use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use moon_server::adapters::codex::{
    build_model_args, codex_permission_args, parse_codex_line, plan_codex_home,
    resolve_codex_command,
};
use moon_server::adapters::zcode::{
    parse_zcode_line_detail, plan_prompt_invocation, plan_session_env, split_prompt_for_attachments,
    zcode_permission_args, ZcodeAdapter,
};
use moon_server::adapters::{CodexAdapter, HarnessAdapter};
use moon_server::domain::agent::ModelConfig;
use moon_server::domain::room::RoomPermissionMode;
use moon_server::domain::run_event::{RunEvent, RunInput};
use moon_server::domain::session::RunSessionContext;
use serde_json::{json, Value};

/// 环境变量是进程级共享状态，涉及 CODEX_HOME 的用例串行执行。
static ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &Path) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

fn model_config(value: Value) -> Option<ModelConfig> {
    value.as_object().cloned()
}

#[test]
fn splits_prompts_without_losing_characters() {
    let prompt = "第一段内容\n第二段内容\n第三段内容";
    let parts = split_prompt_for_attachments(prompt, 8).unwrap();
    assert!(parts.len() > 1);
    assert_eq!(parts.concat(), prompt);
    assert!(parts.iter().all(|part| part.chars().count() <= 8));

    let hard_cut = split_prompt_for_attachments(&"a".repeat(25), 10).unwrap();
    assert_eq!(hard_cut.concat(), "a".repeat(25));
    assert!(hard_cut.iter().all(|part| part.chars().count() <= 10));

    assert_eq!(
        split_prompt_for_attachments("short", 10).unwrap(),
        vec!["short".to_string()]
    );
    assert!(split_prompt_for_attachments("x", 0).is_err());
}

#[test]
fn plans_long_prompt_attachments_and_cleans_up() {
    let root = tempfile::tempdir().unwrap();
    let prompt = "长提示词\n".repeat(20);
    let plan = plan_prompt_invocation(&prompt, 30, Some(root.path())).unwrap();

    assert_eq!(plan.args[0], "-p");
    assert!(plan.args[1].contains("附件"));
    let attach_count = plan.args.iter().filter(|value| *value == "--attach").count();
    assert_eq!(attach_count, plan.args.iter().filter(|value| value.ends_with(".md")).count());

    let directory = plan.cleanup_path().unwrap().clone();
    assert!(directory.exists());
    for entry in std::fs::read_dir(&directory).unwrap() {
        let entry = entry.unwrap();
        assert!(entry.path().extension().is_some_and(|ext| ext == "md"));
    }
    drop(plan);
    assert!(!directory.exists());
}

#[test]
fn parses_zcode_stdout_events() {
    let parsed = parse_zcode_line_detail(
        r#"{"type":"model.streaming","payload":{"kind":"text_delta","delta":"OK"},"sessionId":"sess-1"}"#,
    );
    assert_eq!(parsed.session_id.as_deref(), Some("sess-1"));
    assert!(matches!(parsed.event, Some(RunEvent::TextDelta(text)) if text == "OK"));

    let reasoning = parse_zcode_line_detail(
        r#"{"type":"model.streaming","payload":{"kind":"reasoning_delta","delta":"想想"}}"#,
    );
    assert!(matches!(reasoning.event, Some(RunEvent::Reasoning(text)) if text == "想想"));

    let tool = parse_zcode_line_detail(
        r#"{"type":"tool.updated","payload":{"kind":"scheduled","toolCallId":"t1","toolName":"shell","input":{"command":"ls"}}}"#,
    );
    assert!(matches!(
        tool.event,
        Some(RunEvent::ToolStart { tool_call_id, name, .. }) if tool_call_id == "t1" && name == "shell"
    ));

    let tool_end = parse_zcode_line_detail(
        r#"{"type":"tool.updated","payload":{"kind":"result","toolCallId":"t1","toolName":"shell","result":{"display":"done"}}}"#,
    );
    assert!(matches!(
        tool_end.event,
        Some(RunEvent::ToolEnd { ok: true, detail, .. }) if detail == "done"
    ));

    let usage = parse_zcode_line_detail(
        r#"{"type":"turn.completed","payload":{"tokenCount":12,"toolCallCount":2,"duration":1500}}"#,
    );
    assert!(matches!(
        usage.event,
        Some(RunEvent::Usage {
            token_count: Some(12),
            context_size: None,
            tool_call_count: Some(2),
            duration_ms: Some(1500)
        })
    ));

    let failed = parse_zcode_line_detail(
        r#"{"type":"turn.failed","payload":{"error":{"message":"余额不足"}}}"#,
    );
    assert!(matches!(failed.event, Some(RunEvent::Status(text)) if text == "余额不足"));

    let result = parse_zcode_line_detail(r#"{"type":"result","payload":{"response":"完成"}}"#);
    assert!(matches!(result.event, Some(RunEvent::Final(text)) if text == "完成"));

    assert!(parse_zcode_line_detail("not json").event.is_none());
}

#[test]
fn maps_zcode_permission_modes() {
    assert!(zcode_permission_args(None).is_empty());
    assert_eq!(
        zcode_permission_args(Some(RoomPermissionMode::Plan)),
        vec!["--mode", "plan"]
    );
    assert_eq!(
        zcode_permission_args(Some(RoomPermissionMode::Auto)),
        vec!["--mode", "guarded"]
    );
    assert_eq!(
        zcode_permission_args(Some(RoomPermissionMode::Full)),
        vec!["--mode", "yolo"]
    );
}

#[test]
fn parses_codex_stdout_events() {
    let session = parse_codex_line(r#"{"type":"thread.started","thread_id":"thread-1"}"#);
    assert!(matches!(session, Some(RunEvent::Session(id)) if id == "thread-1"));

    let message = parse_codex_line(
        r#"{"type":"item.completed","item":{"id":"i1","type":"agent_message","text":"收到"}}"#,
    );
    assert!(matches!(message, Some(RunEvent::Final(text)) if text == "收到"));

    let reasoning = parse_codex_line(
        r#"{"type":"item.completed","item":{"id":"i2","type":"reasoning","text":"思考"}}"#,
    );
    assert!(matches!(reasoning, Some(RunEvent::Reasoning(text)) if text == "思考"));

    let tool_start = parse_codex_line(
        r#"{"type":"item.started","item":{"id":"c1","type":"command_execution","command":"cargo test"}}"#,
    );
    assert!(matches!(
        tool_start,
        Some(RunEvent::ToolStart { name, input, .. }) if name == "shell" && input["command"] == "cargo test"
    ));

    let tool_end = parse_codex_line(
        r#"{"type":"item.completed","item":{"id":"c1","type":"command_execution","exit_code":0,"aggregated_output":"ok"}}"#,
    );
    assert!(matches!(
        tool_end,
        Some(RunEvent::ToolEnd { ok: true, detail, .. }) if detail == "ok"
    ));

    let usage = parse_codex_line(
        r#"{"type":"turn.completed","usage":{"input_tokens":10,"output_tokens":5}}"#,
    );
    assert!(matches!(
        usage,
        Some(RunEvent::Usage { token_count: Some(15), .. })
    ));

    let failed = parse_codex_line(
        r#"{"type":"turn.failed","error":{"message":"额度耗尽"}}"#,
    );
    assert!(matches!(failed, Some(RunEvent::Error(text)) if text == "额度耗尽"));

    assert!(parse_codex_line("{}").is_none());
}

#[test]
fn maps_codex_permission_and_model_args() {
    assert_eq!(
        codex_permission_args(None, false),
        vec!["--dangerously-bypass-approvals-and-sandbox"]
    );
    assert_eq!(
        codex_permission_args(Some(RoomPermissionMode::Plan), false),
        vec!["-s", "read-only"]
    );
    assert_eq!(
        codex_permission_args(Some(RoomPermissionMode::Auto), false),
        vec!["-s", "workspace-write", "-c", "approval_policy=\"never\""]
    );
    assert_eq!(
        codex_permission_args(Some(RoomPermissionMode::Full), false),
        vec!["--dangerously-bypass-approvals-and-sandbox"]
    );

    let resume = codex_permission_args(Some(RoomPermissionMode::Auto), true);
    assert_eq!(
        resume,
        vec![
            "-c",
            "sandbox_mode=\"workspace-write\"",
            "-c",
            "approval_policy=\"never\""
        ]
    );
    assert!(!resume.contains(&"-s".to_string()), "resume 不接受 -s");

    assert_eq!(
        build_model_args(model_config(json!({
            "model": "glm-4.6",
            "reasoningLevel": "high",
        })).as_ref()),
        vec!["-m", "glm-4.6", "-c", "model_reasoning_effort=\"high\""]
    );
    assert!(build_model_args(None).is_empty());
}

#[test]
fn codex_home_filters_desktop_only_config() {
    let _guard = ENV_LOCK.lock().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let user_home = fixture.path().join("user-codex");
    std::fs::create_dir_all(&user_home).unwrap();
    std::fs::write(
        user_home.join("config.toml"),
        "model = \"custom\"\n\
         model_catalog_json = \"catalog.json\"\n\
         [features]\n\
         web_search = true\n\
         [model_providers.local]\n\
         name = \"Local\"\n\
         base_url = \"http://localhost:8080\"\n",
    )
    .unwrap();
    std::fs::write(user_home.join("auth.json"), "{\"token\":\"fixture\"}").unwrap();
    let _env = EnvGuard::set("MOON_CODEX_USER_HOME", &user_home);

    let plan = plan_codex_home(None).unwrap();
    let config = std::fs::read_to_string(plan.home.join("config.toml")).unwrap();
    assert!(config.contains("model = \"custom\""));
    assert!(config.contains("[model_providers.local]"));
    assert!(!config.contains("model_catalog_json"));
    assert!(!config.contains("[features]"));
    assert_eq!(
        std::fs::read_to_string(plan.home.join("auth.json")).unwrap(),
        "{\"token\":\"fixture\"}"
    );
    let home = plan.home.clone();
    assert!(home.exists());
    drop(plan);
    assert!(!home.exists(), "临时 CODEX_HOME 应被清理");
}

#[tokio::test]
async fn zcode_fake_cli_receives_resume_permission_and_attachments() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _transport = EnvGuard::set("MOON_ZCODE_TRANSPORT", std::path::Path::new("cli"));
    let workspace = tempfile::tempdir().unwrap();
    let script = workspace.path().join("fake-zcode.js");
    std::fs::write(
        &script,
        r#"
const fs = require('fs');
const path = require('path');
fs.writeFileSync(path.join(__dirname, 'invocation.json'), JSON.stringify({
  args: process.argv.slice(2),
  sessionDb: process.env.ZCODE_SESSION_DB_PATH || null,
}));
console.log(JSON.stringify({ type: 'model.streaming', payload: { kind: 'text_delta', delta: '你好' }, sessionId: 'sess-1' }));
console.log(JSON.stringify({ type: 'result', payload: { response: '你好' } }));
"#,
    )
    .unwrap();
    let storage = workspace.path().join("session-storage");
    let input = RunInput {
        prompt: "请回复".to_string(),
        workspace_dir: workspace.path().to_string_lossy().into_owned(),
        additional_directories: Vec::new(),
        config: model_config(json!({ "zcodeBin": script.to_string_lossy() })),
        session: Some(RunSessionContext {
            harness_session_id: Some("old-session".to_string()),
            storage_dir: storage.to_string_lossy().into_owned(),
        }),
        permission_mode: Some(RoomPermissionMode::Plan),
        timeout: None,
    };

    let adapter = ZcodeAdapter;
    let mut stream = adapter.run(input);
    let mut events = Vec::new();
    while let Some(item) = stream.next().await {
        events.push(item.expect("zcode event"));
    }
    assert!(matches!(events.first(), Some(RunEvent::Session(id)) if id == "sess-1"));
    assert!(matches!(events.iter().find(|event| matches!(event, RunEvent::Final(_))), Some(RunEvent::Final(text)) if text == "你好"));

    let invocation: Value = serde_json::from_str(
        &std::fs::read_to_string(workspace.path().join("invocation.json")).unwrap(),
    )
    .unwrap();
    let args = invocation["args"].as_array().unwrap();
    let args: Vec<&str> = args.iter().map(|value| value.as_str().unwrap()).collect();
    assert!(args.windows(2).any(|pair| pair == ["--resume", "old-session"]));
    assert!(args.windows(2).any(|pair| pair == ["--mode", "plan"]));
    assert!(args.windows(2).any(|pair| pair == ["-p", "请回复"]));
    assert!(args.windows(2).any(|pair| pair == ["--output-format", "stream-json"]));
    assert!(args.windows(2).any(|pair| pair == ["--cwd", workspace.path().to_string_lossy().as_ref()]));
    assert_eq!(
        invocation["sessionDb"].as_str().unwrap(),
        storage.join("db.sqlite").to_string_lossy()
    );
}

#[tokio::test]
async fn zcode_failure_translation_uses_session_reset_hint() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _transport = EnvGuard::set("MOON_ZCODE_TRANSPORT", std::path::Path::new("cli"));
    let workspace = tempfile::tempdir().unwrap();
    let script = workspace.path().join("failing-zcode.js");
    std::fs::write(
        &script,
        "process.stderr.write('Session not found\\n'); process.exit(1);\n",
    )
    .unwrap();
    let adapter = ZcodeAdapter;
    let mut stream = adapter.run(RunInput {
        prompt: "hi".to_string(),
        workspace_dir: workspace.path().to_string_lossy().into_owned(),
        additional_directories: Vec::new(),
        config: model_config(json!({ "zcodeBin": script.to_string_lossy() })),
        session: None,
        permission_mode: None,
        timeout: None,
    });

    let mut saw_error_event = false;
    let mut failure = None;
    while let Some(item) = stream.next().await {
        match item {
            Ok(RunEvent::Error(message)) => {
                saw_error_event = true;
                assert!(message.contains("zcode 会话不存在"));
            }
            Ok(_) => {}
            Err(error) => failure = Some(error.message),
        }
    }
    assert!(saw_error_event);
    assert!(failure.unwrap().contains("zcode 会话不存在"));
}

#[tokio::test]
async fn codex_fake_cli_receives_stdin_resume_and_no_sandbox_flag() {
    let _guard = ENV_LOCK.lock().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let user_home = workspace.path().join("user-codex");
    std::fs::create_dir_all(&user_home).unwrap();
    std::fs::write(user_home.join("config.toml"), "model = \"fixture\"\n").unwrap();
    std::fs::write(user_home.join("auth.json"), "{\"token\":\"fixture\"}").unwrap();
    let _env = EnvGuard::set("MOON_CODEX_USER_HOME", &user_home);

    let script = workspace.path().join("fake-codex.js");
    std::fs::write(
        &script,
        r#"
const fs = require('fs');
const path = require('path');
let stdin = '';
process.stdin.setEncoding('utf8');
process.stdin.on('data', (chunk) => { stdin += chunk; });
process.stdin.on('end', () => {
  fs.writeFileSync(path.join(__dirname, 'invocation.json'), JSON.stringify({
    args: process.argv.slice(2),
    stdin,
    codexHome: process.env.CODEX_HOME || null,
  }));
  console.log(JSON.stringify({ type: 'thread.started', thread_id: 'thread-1' }));
  console.log(JSON.stringify({ type: 'item.completed', item: { id: 'i1', type: 'agent_message', text: '收到' } }));
  console.log(JSON.stringify({ type: 'turn.completed', usage: { input_tokens: 10, output_tokens: 5 } }));
});
"#,
    )
    .unwrap();

    let storage = workspace.path().join("codex-session");
    let input = RunInput {
        prompt: "请处理这个任务".to_string(),
        workspace_dir: workspace.path().to_string_lossy().into_owned(),
        additional_directories: Vec::new(),
        config: model_config(json!({
            "codexBin": script.to_string_lossy(),
            "model": "glm-4.6",
            "reasoningLevel": "high",
        })),
        session: Some(RunSessionContext {
            harness_session_id: Some("thread-old".to_string()),
            storage_dir: storage.to_string_lossy().into_owned(),
        }),
        permission_mode: Some(RoomPermissionMode::Auto),
        timeout: None,
    };

    let adapter = CodexAdapter;
    let mut stream = adapter.run(input);
    let mut events = Vec::new();
    while let Some(item) = stream.next().await {
        events.push(item.expect("codex event"));
    }
    assert!(matches!(events.first(), Some(RunEvent::Session(id)) if id == "thread-1"));
    assert!(matches!(events.iter().find(|event| matches!(event, RunEvent::Final(_))), Some(RunEvent::Final(text)) if text == "收到"));

    let invocation: Value = serde_json::from_str(
        &std::fs::read_to_string(workspace.path().join("invocation.json")).unwrap(),
    )
    .unwrap();
    let args: Vec<&str> = invocation["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect();
    assert_eq!(&args[0..5], &["exec", "resume", "thread-old", "-", "--json"]);
    assert!(args.contains(&"--skip-git-repo-check"));
    assert!(args.windows(2).any(|pair| pair == ["-c", "sandbox_mode=\"workspace-write\""]));
    assert!(args.windows(2).any(|pair| pair == ["-c", "approval_policy=\"never\""]));
    assert!(args.windows(2).any(|pair| pair == ["-m", "glm-4.6"]));
    assert!(!args.contains(&"-s"), "resume 不能使用 -s");
    assert!(!args.contains(&"--ephemeral"), "复用会话不能 ephemeral");
    assert_eq!(invocation["stdin"].as_str().unwrap(), "请处理这个任务");

    let home = PathBuf::from(invocation["codexHome"].as_str().unwrap());
    assert_eq!(home, storage.join("codex-home"));
    assert!(home.join("config.toml").exists());
    assert!(home.join("auth.json").exists());
}

#[test]
fn resolves_codex_command_from_explicit_config() {
    let resolved = resolve_codex_command(Some("E:/tools/codex.exe"));
    assert_eq!(resolved.command, "E:/tools/codex.exe");
    assert!(resolved.args_prefix.is_empty());
}

#[test]
fn isolated_session_keeps_persistent_storage() {
    let root = tempfile::tempdir().unwrap();
    let storage = root.path().join("session");
    let plan = plan_session_env(Some(&RunSessionContext {
        harness_session_id: None,
        storage_dir: storage.to_string_lossy().into_owned(),
    }))
    .unwrap();
    assert_eq!(
        plan.env[0].1,
        storage.join("db.sqlite").to_string_lossy()
    );
    assert!(storage.exists(), "持久会话目录不被清理");
}

#[tokio::test]
async fn zcode_app_server_streams_events_and_passes_additional_directories() {
    let _guard = ENV_LOCK.lock().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let script = workspace.path().join("fake-app-server.js");
    std::fs::write(
        &script,
        r#"
const fs = require('fs');
const path = require('path');
let buffer = '';
let pendingCreate = null;
process.stdin.setEncoding('utf8');
process.stdin.on('data', (chunk) => {
  buffer += chunk;
  let index;
  while ((index = buffer.indexOf('\n')) >= 0) {
    const line = buffer.slice(0, index);
    buffer = buffer.slice(index + 1);
    if (!line.trim()) continue;
    const message = JSON.parse(line);
    if (message.method === 'session/create') {
      pendingCreate = message;
      process.stdout.write(JSON.stringify({
        id: 'server-1',
        method: 'session/requestRuntimePreferences',
        params: { sessionId: 'sess-app-1', scope: 'runtime-materialization' },
      }) + '\n');
    } else if (pendingCreate && message.id === 'server-1') {
      fs.writeFileSync(path.join(__dirname, 'app-server-invocation.json'), JSON.stringify({
        params: pendingCreate.params,
        preferences: message.result,
        sessionDb: process.env.ZCODE_SESSION_DB_PATH || null,
      }));
      process.stdout.write(JSON.stringify({
        id: pendingCreate.id,
        result: { session: { sessionId: 'sess-app-1' } },
      }) + '\n');
      pendingCreate = null;
    } else if (message.method === 'session/subscribe') {
      process.stdout.write(JSON.stringify({
        id: message.id,
        result: { eventSeq: 0, events: [], sessionId: 'sess-app-1' },
      }) + '\n');
    } else if (message.method === 'session/send') {
      process.stdout.write(JSON.stringify({
        id: message.id,
        result: { accepted: true, sessionId: 'sess-app-1' },
      }) + '\n');
      process.stdout.write(JSON.stringify({
        method: 'session/event',
        params: {
          sessionId: 'sess-app-1',
          type: 'model.streaming',
          payload: { kind: 'text_delta', delta: '你好' },
        },
      }) + '\n');
      process.stdout.write(JSON.stringify({
        method: 'session/event',
        params: {
          sessionId: 'sess-app-1',
          type: 'turn.completed',
          payload: {
            response: '你好，世界',
            tokenCount: 12,
            toolCallCount: 0,
            duration: 5,
          },
        },
      }) + '\n');
    } else if (message.method) {
      process.stdout.write(JSON.stringify({ id: message.id, result: {} }) + '\n');
    }
  }
});
"#,
    )
    .unwrap();

    let input = RunInput {
        prompt: "请回复".to_string(),
        workspace_dir: workspace.path().to_string_lossy().into_owned(),
        additional_directories: vec!["E:/extra".to_string()],
        config: model_config(json!({ "zcodeBin": script.to_string_lossy() })),
        session: None,
        permission_mode: Some(RoomPermissionMode::Plan),
        timeout: None,
    };

    let adapter = ZcodeAdapter;
    let mut stream = adapter.run(input);
    let mut events = Vec::new();
    while let Some(item) = stream.next().await {
        events.push(item.expect("zcode app-server event"));
    }
    assert!(matches!(events.first(), Some(RunEvent::Session(id)) if id == "sess-app-1"));
    assert!(events
        .iter()
        .any(|event| matches!(event, RunEvent::TextDelta(text) if text == "你好")));
    assert!(events
        .iter()
        .any(|event| matches!(event, RunEvent::Final(text) if text == "你好，世界")));
    assert!(events.iter().any(
        |event| matches!(event, RunEvent::Usage { token_count: Some(12), .. })
    ));

    let invocation: Value = serde_json::from_str(
        &std::fs::read_to_string(workspace.path().join("app-server-invocation.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(invocation["params"]["additionalDirectories"][0], "E:/extra");
    assert_eq!(
        invocation["params"]["workspace"]["workspacePath"],
        workspace.path().to_string_lossy().as_ref()
    );
    assert_eq!(invocation["params"]["mode"], "plan");
    assert_eq!(
        invocation["preferences"]["nativeSearchEnhancementsEnabled"],
        false
    );
}

#[tokio::test]
async fn zcode_app_server_applies_model_selection_via_set_model() {
    let _guard = ENV_LOCK.lock().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let script = workspace.path().join("fake-app-server-model.js");
    std::fs::write(
        &script,
        r#"
const fs = require('fs');
const path = require('path');
let buffer = '';
const protocolCalls = [];
process.stdin.setEncoding('utf8');
process.stdin.on('data', (chunk) => {
  buffer += chunk;
  let index;
  while ((index = buffer.indexOf('\n')) >= 0) {
    const line = buffer.slice(0, index);
    buffer = buffer.slice(index + 1);
    if (!line.trim()) continue;
    const message = JSON.parse(line);
    if (message.method === 'session/create') {
      process.stdout.write(JSON.stringify({
        id: message.id,
        result: { session: { sessionId: 'sess-model-1' } },
      }) + '\n');
    } else if (message.method === 'session/setModel') {
      protocolCalls.push({ method: message.method, params: message.params });
      process.stdout.write(JSON.stringify({ id: message.id, result: {} }) + '\n');
    } else if (message.method === 'session/subscribe') {
      process.stdout.write(JSON.stringify({
        id: message.id,
        result: { eventSeq: 0, events: [], sessionId: 'sess-model-1' },
      }) + '\n');
    } else if (message.method === 'session/send') {
      fs.writeFileSync(path.join(__dirname, 'model-calls.json'), JSON.stringify(protocolCalls));
      process.stdout.write(JSON.stringify({
        id: message.id,
        result: { accepted: true, sessionId: 'sess-model-1' },
      }) + '\n');
      process.stdout.write(JSON.stringify({
        method: 'session/event',
        params: {
          sessionId: 'sess-model-1',
          type: 'turn.completed',
          payload: { response: '收到', tokenCount: 3, toolCallCount: 0, duration: 1 },
        },
      }) + '\n');
    } else if (message.method) {
      process.stdout.write(JSON.stringify({ id: message.id, result: {} }) + '\n');
    }
  }
});
"#,
    )
    .unwrap();

    let input = RunInput {
        prompt: "请回复".to_string(),
        workspace_dir: workspace.path().to_string_lossy().into_owned(),
        additional_directories: Vec::new(),
        config: model_config(json!({
            "zcodeBin": script.to_string_lossy(),
            "providerId": "account:demo-plan",
            "model": "GLM-5.3-Flash",
            "reasoningLevel": "high",
        })),
        session: None,
        permission_mode: Some(RoomPermissionMode::Plan),
        timeout: None,
    };

    let adapter = ZcodeAdapter;
    let mut stream = adapter.run(input);
    let mut events = Vec::new();
    while let Some(item) = stream.next().await {
        events.push(item.expect("zcode app-server event"));
    }
    assert!(events
        .iter()
        .any(|event| matches!(event, RunEvent::Final(text) if text == "收到")));

    let calls: Value = serde_json::from_str(
        &std::fs::read_to_string(workspace.path().join("model-calls.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(calls[0]["method"], "session/setModel");
    assert_eq!(calls[0]["params"]["model"]["providerId"], "account:demo-plan");
    assert_eq!(calls[0]["params"]["model"]["modelId"], "GLM-5.3-Flash");
    assert_eq!(
        calls[0]["params"]["model"]["options"]["reasoningLevel"],
        "high"
    );
    assert_eq!(
        calls.as_array().map(Vec::len),
        Some(1),
        "思考等级必须内联在 setModel 的 options 中，不单独调用 setThoughtLevel"
    );
}

#[tokio::test]
async fn zcode_app_server_failure_returns_without_waiting_for_grandchild_stderr() {
    let _guard = ENV_LOCK.lock().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let script = workspace.path().join("fake-app-server-failure.js");
    std::fs::write(
        &script,
        r#"
const { spawn } = require('child_process');
let buffer = '';
process.stdin.setEncoding('utf8');

// 模拟 MCP 子进程：继承 stderr 并比 app-server 活得更久。
spawn(process.execPath, ['-e', 'setTimeout(() => {}, 30000)'], {
  stdio: ['ignore', 'ignore', 'inherit'],
});

process.stdin.on('data', (chunk) => {
  buffer += chunk;
  let index;
  while ((index = buffer.indexOf('\n')) >= 0) {
    const line = buffer.slice(0, index);
    buffer = buffer.slice(index + 1);
    if (!line.trim()) continue;
    const message = JSON.parse(line);
    if (message.method === 'session/create') {
      process.stdout.write(JSON.stringify({
        id: message.id,
        result: { session: { sessionId: 'sess-fail-1' } },
      }) + '\n');
    } else if (message.method === 'session/subscribe') {
      process.stdout.write(JSON.stringify({
        id: message.id,
        result: { eventSeq: 0, events: [], sessionId: 'sess-fail-1' },
      }) + '\n');
    } else if (message.method === 'session/send') {
      process.stdout.write(JSON.stringify({
        id: message.id,
        result: { accepted: true, sessionId: 'sess-fail-1' },
      }) + '\n');
      process.stdout.write(JSON.stringify({
        method: 'session/event',
        params: {
          sessionId: 'sess-fail-1',
          type: 'turn.failed',
          payload: { error: { message: '余额不足，额度耗尽' } },
        },
      }) + '\n');
    } else if (message.method) {
      process.stdout.write(JSON.stringify({ id: message.id, result: {} }) + '\n');
    }
  }
});

// 保持 app-server 存活，等待 Rust 侧收尾。
setInterval(() => {}, 1000);
"#,
    )
    .unwrap();

    let input = RunInput {
        prompt: "请回复".to_string(),
        workspace_dir: workspace.path().to_string_lossy().into_owned(),
        additional_directories: Vec::new(),
        config: model_config(json!({ "zcodeBin": script.to_string_lossy() })),
        session: None,
        permission_mode: None,
        timeout: None,
    };

    let started = Instant::now();
    let adapter = ZcodeAdapter;
    let mut stream = adapter.run(input);
    let outcome = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(item) = stream.next().await {
            if let Err(error) = item {
                return error;
            }
        }
        panic!("失败运行应以错误结束");
    })
    .await
    .expect("失败收尾被孙进程 stderr 阻塞");

    assert!(outcome.message.contains("余额不足"));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "失败应在收尾超时内返回：{:?}",
        started.elapsed()
    );
}

/// 生成一个读取 stdin 后永久挂起的脚本；用于验证运行超时。
fn write_hanging_script(dir: &Path, name: &str) -> PathBuf {
    let script = dir.join(name);
    std::fs::write(
        &script,
        "process.stdin.resume(); setInterval(() => {}, 1000);\n",
    )
    .unwrap();
    script
}

async fn collect_failure(
    adapter: &dyn HarnessAdapter,
    input: RunInput,
) -> (Option<String>, Duration) {
    let started = Instant::now();
    let mut stream = adapter.run(input);
    let mut failure = None;
    while let Some(item) = stream.next().await {
        if let Err(error) = item {
            failure = Some(error.message);
            break;
        }
    }
    (failure, started.elapsed())
}

#[tokio::test]
async fn codex_times_out_hanging_cli() {
    let workspace = tempfile::tempdir().unwrap();
    let script = write_hanging_script(workspace.path(), "hanging-codex.js");
    let input = RunInput {
        prompt: "hi".to_string(),
        workspace_dir: workspace.path().to_string_lossy().into_owned(),
        additional_directories: Vec::new(),
        config: model_config(json!({ "codexBin": script.to_string_lossy() })),
        session: None,
        permission_mode: None,
        timeout: Some(Duration::from_millis(500)),
    };

    let (failure, elapsed) = collect_failure(&CodexAdapter, input).await;
    assert!(
        failure
            .as_deref()
            .is_some_and(|message| message.contains("运行超过")),
        "应返回超时错误：{failure:?}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "超时应及时返回：{elapsed:?}"
    );
}

#[tokio::test]
async fn zcode_cli_times_out_hanging_cli() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _transport = EnvGuard::set("MOON_ZCODE_TRANSPORT", Path::new("cli"));
    let workspace = tempfile::tempdir().unwrap();
    let script = write_hanging_script(workspace.path(), "hanging-zcode.js");
    let input = RunInput {
        prompt: "hi".to_string(),
        workspace_dir: workspace.path().to_string_lossy().into_owned(),
        additional_directories: Vec::new(),
        config: model_config(json!({ "zcodeBin": script.to_string_lossy() })),
        session: None,
        permission_mode: None,
        timeout: Some(Duration::from_millis(500)),
    };

    let (failure, elapsed) = collect_failure(&ZcodeAdapter, input).await;
    assert!(
        failure
            .as_deref()
            .is_some_and(|message| message.contains("运行超过")),
        "应返回超时错误：{failure:?}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "超时应及时返回：{elapsed:?}"
    );
}

#[tokio::test]
async fn zcode_app_server_times_out_hanging_cli() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _transport = EnvGuard::set("MOON_ZCODE_TRANSPORT", Path::new("app-server"));
    let workspace = tempfile::tempdir().unwrap();
    let script = write_hanging_script(workspace.path(), "hanging-app-server.js");
    let input = RunInput {
        prompt: "hi".to_string(),
        workspace_dir: workspace.path().to_string_lossy().into_owned(),
        additional_directories: Vec::new(),
        config: model_config(json!({ "zcodeBin": script.to_string_lossy() })),
        session: None,
        permission_mode: None,
        timeout: Some(Duration::from_millis(500)),
    };

    let (failure, elapsed) = collect_failure(&ZcodeAdapter, input).await;
    assert!(
        failure
            .as_deref()
            .is_some_and(|message| message.contains("运行超过")),
        "应返回超时错误：{failure:?}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "超时应及时返回：{elapsed:?}"
    );
}
