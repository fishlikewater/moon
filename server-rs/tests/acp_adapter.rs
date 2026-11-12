//! ACP 适配器集成测试：用 tests/fixtures/acp_fake_agent.mjs 走真实进程。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use moon_server::adapters::acp::probe_capabilities;
use moon_server::adapters::{AcpAdapter, AdapterFactory, HarnessAdapter};
use moon_server::db::Db;
use moon_server::domain::harness::{CreateCustomHarnessInput, CustomHarness};
use moon_server::domain::room::RoomPermissionMode;
use moon_server::domain::run_event::{RunEvent, RunInput};
use moon_server::domain::session::RunSessionContext;
use moon_server::{AppState, AppStateOptions};
use serde_json::json;

const FAKE_AGENT_SCRIPT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/acp_fake_agent.mjs"
);

/// fake agent 依赖 node；缺失时给出可操作的失败说明。
fn require_node() {
    let ok = std::process::Command::new("node")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    assert!(ok, "集成测试需要 PATH 中可用的 node 来运行 fake ACP agent");
}

struct TestHarness {
    state: AppState,
    workspace: PathBuf,
    custom_harness_id: i64,
    dir: tempfile::TempDir,
}

async fn setup_fake_harness(extra_args: Vec<String>) -> TestHarness {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("moon.db")).await.unwrap();
    let state = AppState::new_with_options(db, dir.path().join("sessions"), AppStateOptions::default())
        .await
        .unwrap();

    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();

    let mut args = vec![FAKE_AGENT_SCRIPT.to_string()];
    args.extend(extra_args);
    let harness = state
        .custom_harness_store
        .create(CreateCustomHarnessInput {
            name: "fake-acp".to_string(),
            command: "node".to_string(),
            args,
            env: BTreeMap::new(),
        })
        .await
        .unwrap();

    TestHarness {
        state,
        workspace,
        custom_harness_id: harness.id,
        dir,
    }
}

fn run_input(harness: &TestHarness, prompt: &str, session: Option<RunSessionContext>) -> RunInput {
    RunInput {
        prompt: prompt.to_string(),
        workspace_dir: harness.workspace.to_string_lossy().to_string(),
        additional_directories: Vec::new(),
        config: None,
        session,
        permission_mode: None,
        timeout: None,
    }
}

async fn collect_events(
    adapter: Arc<dyn HarnessAdapter>,
    input: RunInput,
) -> Vec<RunEvent> {
    let mut events = Vec::new();
    let mut stream = adapter.run(input);
    while let Some(item) = stream.next().await {
        events.push(item.expect("适配器不应返回错误"));
    }
    events
}

fn final_text(events: &[RunEvent]) -> Option<String> {
    events.iter().find_map(|event| match event {
        RunEvent::Final(text) => Some(text.clone()),
        _ => None,
    })
}

#[tokio::test]
async fn acp_adapter_emits_expected_event_sequence() {
    require_node();
    let harness = setup_fake_harness(Vec::new()).await;
    let adapter: Arc<dyn HarnessAdapter> = Arc::new(AcpAdapter::new(
        harness.state.custom_harness_store.clone(),
        Some(harness.custom_harness_id),
    ));

    let events = collect_events(adapter, run_input(&harness, "打个招呼", None)).await;

    // 会话事件在前。
    let RunEvent::Session(session_id) = &events[0] else {
        panic!("第一个事件应为 Session，实际：{events:?}");
    };
    assert!(session_id.starts_with("sess-fake-"));

    assert!(
        events
            .iter()
            .any(|event| matches!(event, RunEvent::Reasoning(text) if text.contains("正在思考"))),
        "缺少思考事件：{events:?}"
    );
    assert!(
        events.iter().any(|event| matches!(event,
            RunEvent::ToolStart { tool_call_id, name, .. }
            if tool_call_id == "call-1" && name == "列出文件")),
        "缺少工具开始事件：{events:?}"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, RunEvent::ToolEnd { ok: true, .. })),
        "缺少工具结束事件：{events:?}"
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            RunEvent::Usage {
                token_count: Some(42),
                context_size: Some(1000),
                ..
            }
        )),
        "缺少用量事件：{events:?}"
    );

    // 增量拼接与最终结论一致。
    let streamed: String = events
        .iter()
        .filter_map(|event| match event {
            RunEvent::TextDelta(text) => Some(text.clone()),
            _ => None,
        })
        .collect();
    let final_text = final_text(&events).expect("应有 Final 事件");
    assert_eq!(streamed, final_text);
    assert!(final_text.contains("我是自定义 ACP agent"));
}

#[tokio::test]
async fn acp_adapter_enforces_workspace_whitelist() {
    require_node();
    let harness = setup_fake_harness(Vec::new()).await;
    let adapter: Arc<dyn HarnessAdapter> = Arc::new(AcpAdapter::new(
        harness.state.custom_harness_store.clone(),
        Some(harness.custom_harness_id),
    ));

    // 白名单内：可读并回显内容。
    let hello = harness.workspace.join("hello.txt");
    std::fs::write(&hello, "工作区内内容").unwrap();
    let inside = collect_events(
        adapter.clone(),
        run_input(&harness, &format!("READ:{}", hello.display()), None),
    )
    .await;
    let inside_final = final_text(&inside).expect("白名单内读取应有 Final");
    assert!(
        inside_final.contains("READ_OK:工作区内内容"),
        "白名单内读取失败：{inside_final}"
    );

    // 白名单外：适配器拒绝，fake agent 把拒绝原因写进回复。
    let outside = harness.dir.path().join("outside.txt");
    std::fs::write(&outside, "机密").unwrap();
    let escaped = collect_events(
        adapter.clone(),
        run_input(&harness, &format!("READ:{}", outside.display()), None),
    )
    .await;
    let escaped_final = final_text(&escaped).expect("越界读取也应有 Final（fake agent 回显拒绝原因）");
    assert!(
        escaped_final.contains("READ_FAIL:"),
        "越界读取未被拒绝：{escaped_final}"
    );
    assert!(
        escaped_final.contains("超出工作区白名单"),
        "拒绝原因应指明白名单：{escaped_final}"
    );
    assert!(
        !escaped_final.contains("机密"),
        "越界内容不应被读取：{escaped_final}"
    );

    // 写路径：白名单内成功、越界被拒且文件未产生。
    let inside_target = harness.workspace.join("inside-write.txt");
    let inside_write = collect_events(
        adapter.clone(),
        run_input(
            &harness,
            &format!("WRITE:{}::工作区写入", inside_target.display()),
            None,
        ),
    )
    .await;
    assert!(
        final_text(&inside_write)
            .expect("白名单内写入应有 Final")
            .contains("WRITE_OK"),
        "白名单内写入失败：{inside_write:?}"
    );
    assert_eq!(std::fs::read_to_string(&inside_target).unwrap(), "工作区写入");

    let outside_target = harness.dir.path().join("outside-write.txt");
    let outside_write = collect_events(
        adapter.clone(),
        run_input(
            &harness,
            &format!("WRITE:{}::机密", outside_target.display()),
            None,
        ),
    )
    .await;
    let outside_final = final_text(&outside_write).expect("越界写入应有 Final");
    assert!(
        outside_final.contains("WRITE_FAIL:"),
        "越界写入未被拒绝：{outside_final}"
    );
    assert!(
        !outside_target.exists(),
        "越界写入不应产生文件"
    );

    // 中间目录缺失的新文件：写入应落在白名单内完整路径（父目录回退不丢层级）。
    let nested = harness.workspace.join("newdir").join("nested.txt");
    let nested_write = collect_events(
        adapter.clone(),
        run_input(
            &harness,
            &format!("WRITE:{}::嵌套", nested.display()),
            None,
        ),
    )
    .await;
    assert!(
        final_text(&nested_write)
            .expect("嵌套写入应有 Final")
            .contains("WRITE_OK"),
        "嵌套新路径写入失败：{nested_write:?}"
    );
    assert_eq!(std::fs::read_to_string(&nested).unwrap(), "嵌套");

    // 悬空符号链接：目标不存在，任何层级的符号链接都必须被拒绝。
    let link = harness.workspace.join("dangling.link");
    #[cfg(windows)]
    let create_link = std::os::windows::fs::symlink_file(
        harness.dir.path().join("no-such-target"),
        &link,
    );
    #[cfg(unix)]
    let create_link = std::os::unix::fs::symlink(
        harness.dir.path().join("no-such-target"),
        &link,
    );
    match create_link {
        Ok(()) => {
            let escape = collect_events(
                adapter,
                run_input(&harness, &format!("WRITE:{}::越权", link.display()), None),
            )
            .await;
            let escape_final = final_text(&escape).expect("悬空链接写入应有 Final");
            assert!(
                escape_final.contains("WRITE_FAIL:"),
                "悬空符号链接未被拒绝：{escape_final}"
            );
            assert!(
                !harness.dir.path().join("no-such-target").exists(),
                "悬空符号链接不应被写入"
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            // 无符号链接特权的环境跳过该负例；创建成功路径已由白名单测试覆盖。
            eprintln!("skipped dangling symlink case: {error}");
        }
        Err(error) => panic!("创建悬空符号链接失败：{error}"),
    }
}

#[tokio::test]
async fn acp_adapter_resumes_session_when_reuse_enabled() {
    require_node();
    let record_dir = tempfile::tempdir().unwrap();
    let record_file = record_dir.path().join("requests.jsonl");
    let harness = setup_fake_harness(vec![
        "--mode".to_string(),
        "record".to_string(),
        "--record".to_string(),
        record_file.to_string_lossy().to_string(),
    ])
    .await;
    let adapter: Arc<dyn HarnessAdapter> = Arc::new(AcpAdapter::new(
        harness.state.custom_harness_store.clone(),
        Some(harness.custom_harness_id),
    ));

    let storage_dir = harness.dir.path().join("session-storage");
    std::fs::create_dir_all(&storage_dir).unwrap();

    // 第一次运行：建立会话。
    let first = collect_events(adapter.clone(), run_input(&harness, "第一问", None)).await;
    let first_session = first
        .iter()
        .find_map(|event| match event {
            RunEvent::Session(id) => Some(id.clone()),
            _ => None,
        })
        .expect("第一次运行应有会话事件");

    // 第二次运行：携带同一会话 id，fake agent 应收到 session/load。
    let second = collect_events(
        adapter,
        run_input(
            &harness,
            "第二问",
            Some(RunSessionContext {
                harness_session_id: Some(first_session.clone()),
                storage_dir: storage_dir.to_string_lossy().to_string(),
            }),
        ),
    )
    .await;
    let second_session = second
        .iter()
        .find_map(|event| match event {
            RunEvent::Session(id) => Some(id.clone()),
            _ => None,
        })
        .expect("第二次运行应有会话事件");
    assert_eq!(second_session, first_session);

    let recorded = std::fs::read_to_string(&record_file).unwrap();
    assert!(
        recorded.contains("\"session/load\""),
        "第二次运行应发出 session/load：{recorded}"
    );
    assert!(
        recorded.contains(&format!("\"sessionId\":\"{first_session}\"")),
        "session/load 应携带第一次的会话 id：{recorded}"
    );

    // 首轮 session/new 必须携带 cwd、mcpServers；additionalDirectories 为空时
    // SDK 按协议省略字段（schema 明确「省略与空数组等价」），出现时必须是空数组。
    assert!(
        recorded.contains("\"method\":\"session/new\""),
        "首轮应发出 session/new：{recorded}"
    );
    assert!(
        recorded.contains("\"mcpServers\":[]"),
        "session/new 应携带 mcpServers=[]：{recorded}"
    );
    let additional_directories_present = recorded.contains("\"additionalDirectories\":");
    assert!(
        !additional_directories_present || recorded.contains("\"additionalDirectories\":[]"),
        "session/new 的 additionalDirectories 应为空或省略：{recorded}"
    );
    assert!(
        recorded.contains("\"cwd\":"),
        "session/new 应携带 cwd：{recorded}"
    );
}

#[tokio::test]
async fn acp_adapter_skips_load_when_agent_lacks_capability() {
    require_node();
    let record_dir = tempfile::tempdir().unwrap();
    let record_file = record_dir.path().join("requests.jsonl");
    let harness = setup_fake_harness(vec![
        "--mode".to_string(),
        "record".to_string(),
        "--record".to_string(),
        record_file.to_string_lossy().to_string(),
        "--no-load-session".to_string(),
    ])
    .await;
    let adapter: Arc<dyn HarnessAdapter> = Arc::new(AcpAdapter::new(
        harness.state.custom_harness_store.clone(),
        Some(harness.custom_harness_id),
    ));

    let storage_dir = harness.dir.path().join("session-storage");
    std::fs::create_dir_all(&storage_dir).unwrap();

    let first = collect_events(adapter.clone(), run_input(&harness, "一", None)).await;
    let session_id = first
        .iter()
        .find_map(|event| match event {
            RunEvent::Session(id) => Some(id.clone()),
            _ => None,
        })
        .expect("第一次运行应有会话事件");

    let second = collect_events(
        adapter,
        run_input(
            &harness,
            "二",
            Some(RunSessionContext {
                harness_session_id: Some(session_id),
                storage_dir: storage_dir.to_string_lossy().to_string(),
            }),
        ),
    )
    .await;
    let _ = second;

    let recorded = std::fs::read_to_string(&record_file).unwrap();
    assert!(
        !recorded.contains("\"session/load\""),
        "agent 未宣告 loadSession 时不应发送 session/load：{recorded}"
    );
    assert!(
        recorded.matches("\"session/new\"").count() == 2,
        "两次运行都应新建会话：{recorded}"
    );
}

#[tokio::test]
async fn acp_adapter_reports_missing_custom_harness() {
    require_node();
    let harness = setup_fake_harness(Vec::new()).await;
    let adapter: Arc<dyn HarnessAdapter> =
        Arc::new(AcpAdapter::new(harness.state.custom_harness_store.clone(), None));

    let mut error = None;
    let mut stream = adapter.run(run_input(&harness, "任何输入", None));
    while let Some(item) = stream.next().await {
        if let Err(failure) = item {
            error = Some(failure);
            break;
        }
    }
    let error = error.expect("未绑定定义时应返回错误");
    assert!(error.message.contains("未绑定自定义 ACP agent"));
}

#[tokio::test]
async fn acp_adapter_passes_additional_directories_into_scope() {
    require_node();
    let harness = setup_fake_harness(Vec::new()).await;
    let adapter: Arc<dyn HarnessAdapter> = Arc::new(AcpAdapter::new(
        harness.state.custom_harness_store.clone(),
        Some(harness.custom_harness_id),
    ));

    let extra = harness.dir.path().join("extra");
    std::fs::create_dir_all(&extra).unwrap();
    std::fs::write(extra.join("extra.txt"), "附加目录内容").unwrap();

    // 附加目录在白名单内：读与写都放行。
    let read = collect_events(
        adapter.clone(),
        run_input_with(
            &harness,
            &format!("READ:{}", extra.join("extra.txt").display()),
            vec![extra.to_string_lossy().to_string()],
            None,
        ),
    )
    .await;
    assert!(
        final_text(&read)
            .expect("附加目录读取应有 Final")
            .contains("READ_OK:附加目录内容"),
        "附加目录读取失败：{read:?}"
    );

    let target = extra.join("written.txt");
    let write = collect_events(
        adapter,
        run_input_with(
            &harness,
            &format!("WRITE:{}::附加写入", target.display()),
            vec![extra.to_string_lossy().to_string()],
            None,
        ),
    )
    .await;
    assert!(
        final_text(&write).expect("附加目录写入应有 Final").contains("WRITE_OK"),
        "附加目录写入失败：{write:?}"
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "附加写入");

    // 线上断言：附加目录必须真的经 session/new 传给 agent（record fixture）。
    let record_dir = tempfile::tempdir().unwrap();
    let record_file = record_dir.path().join("dirs.jsonl");
    let recorded_harness = setup_fake_harness(vec![
        "--mode".to_string(),
        "record".to_string(),
        "--record".to_string(),
        record_file.to_string_lossy().to_string(),
    ])
    .await;
    let recorded_adapter: Arc<dyn HarnessAdapter> = Arc::new(AcpAdapter::new(
        recorded_harness.state.custom_harness_store.clone(),
        Some(recorded_harness.custom_harness_id),
    ));
    let recorded_extra = recorded_harness.dir.path().join("extra-recorded");
    std::fs::create_dir_all(&recorded_extra).unwrap();
    let _ = collect_events(
        recorded_adapter,
        run_input_with(
            &recorded_harness,
            "记录附加目录",
            vec![recorded_extra.to_string_lossy().to_string()],
            None,
        ),
    )
    .await;
    let recorded_wire = std::fs::read_to_string(&record_file).unwrap();
    assert!(
        recorded_wire.contains("\"additionalDirectories\":["),
        "非空附加目录应经 session/new 传入：{recorded_wire}"
    );
    // 线上是 JSON 转义后的路径（反斜杠成对），用 serde 序列化出期望字面量。
    let expected_dir = serde_json::to_string(&recorded_extra.to_string_lossy()).unwrap();
    assert!(
        recorded_wire.contains(&format!("\"additionalDirectories\":[{expected_dir}")),
        "additionalDirectories 应携带附加目录绝对路径：{recorded_wire}"
    );
}

#[tokio::test]
async fn acp_adapter_auto_selects_allow_permission_option() {
    require_node();
    let harness = setup_fake_harness(Vec::new()).await;
    let adapter: Arc<dyn HarnessAdapter> = Arc::new(AcpAdapter::new(
        harness.state.custom_harness_store.clone(),
        Some(harness.custom_harness_id),
    ));

    let events = collect_events(adapter, run_input(&harness, "PERMISSION", None)).await;
    let text = final_text(&events).expect("权限用例应有 Final");
    assert!(
        text.contains("PERM_OK:allow-1"),
        "应自动选择第一个 allow 选项：{text}"
    );
}

#[tokio::test]
async fn acp_adapter_applies_room_permission_mode() {
    require_node();
    let record_dir = tempfile::tempdir().unwrap();
    let record_file = record_dir.path().join("requests.jsonl");
    let harness = setup_fake_harness(vec![
        "--mode".to_string(),
        "record".to_string(),
        "--record".to_string(),
        record_file.to_string_lossy().to_string(),
    ])
    .await;
    let adapter: Arc<dyn HarnessAdapter> = Arc::new(AcpAdapter::new(
        harness.state.custom_harness_store.clone(),
        Some(harness.custom_harness_id),
    ));

    for (mode, expected) in [
        (RoomPermissionMode::Plan, "plan"),
        (RoomPermissionMode::Auto, "acceptEdits"),
        (RoomPermissionMode::Full, "bypassPermissions"),
    ] {
        let mut input = run_input(&harness, "映射权限模式", None);
        input.permission_mode = Some(mode);
        let events = collect_events(adapter.clone(), input).await;
        assert!(
            final_text(&events).is_some(),
            "权限映射不应中断运行：{events:?}"
        );
        // 每次运行结束就读一次记录，确认本轮目标已发送，而不是只靠全局出现。
        let recorded = std::fs::read_to_string(&record_file).unwrap();
        assert!(
            recorded.contains(&format!("\"modeId\":\"{expected}\"")),
            "本轮未发送权限模式 {expected} 的 set_mode：{recorded}"
        );
    }

    let recorded = std::fs::read_to_string(&record_file).unwrap();
    for expected in ["plan", "acceptEdits", "bypassPermissions"] {
        assert!(
            recorded.contains(&format!("\"modeId\":\"{expected}\"")),
            "缺少权限模式 {expected} 的 set_mode 请求：{recorded}"
        );
    }
    assert_eq!(
        recorded.matches("\"method\":\"session/set_mode\"").count(),
        3,
        "每次运行都应发送一次 set_mode：{recorded}"
    );
}

#[tokio::test]
async fn agent_reasoning_level_overrides_permission_mode() {
    require_node();
    let record_dir = tempfile::tempdir().unwrap();
    let record_file = record_dir.path().join("requests.jsonl");
    let harness = setup_fake_harness(vec![
        "--mode".to_string(),
        "record".to_string(),
        "--record".to_string(),
        record_file.to_string_lossy().to_string(),
    ])
    .await;
    let adapter: Arc<dyn HarnessAdapter> = Arc::new(AcpAdapter::new(
        harness.state.custom_harness_store.clone(),
        Some(harness.custom_harness_id),
    ));

    // 智能体显式选 plan，房间权限模式 Auto 本应映射 acceptEdits；
    // 显式选择必须优先，且不再发送权限映射请求。
    let mut input = run_input(&harness, "显式模式优先", None);
    input.config = json!({ "reasoningLevel": "plan" }).as_object().cloned();
    input.permission_mode = Some(RoomPermissionMode::Auto);
    let events = collect_events(adapter, input).await;
    assert!(
        final_text(&events).is_some(),
        "显式模式不应中断运行：{events:?}"
    );

    let recorded = std::fs::read_to_string(&record_file).unwrap();
    assert!(
        recorded.contains("\"modeId\":\"plan\""),
        "应发送显式模式 plan 的 set_mode：{recorded}"
    );
    assert!(
        !recorded.contains("\"modeId\":\"acceptEdits\""),
        "显式模式命中后不应再发权限映射请求：{recorded}"
    );
    assert_eq!(
        recorded.matches("\"method\":\"session/set_mode\"").count(),
        1,
        "本轮只应发送一次 set_mode：{recorded}"
    );
}

#[tokio::test]
async fn unknown_reasoning_level_falls_back_to_permission_mode() {
    require_node();
    let record_dir = tempfile::tempdir().unwrap();
    let record_file = record_dir.path().join("requests.jsonl");
    let harness = setup_fake_harness(vec![
        "--mode".to_string(),
        "record".to_string(),
        "--record".to_string(),
        record_file.to_string_lossy().to_string(),
    ])
    .await;
    let adapter: Arc<dyn HarnessAdapter> = Arc::new(AcpAdapter::new(
        harness.state.custom_harness_store.clone(),
        Some(harness.custom_harness_id),
    ));

    // 未宣告的模式只提示，回落到房间权限模式映射（Plan → plan）。
    let mut input = run_input(&harness, "未知模式回落", None);
    input.config = json!({ "reasoningLevel": "zen" }).as_object().cloned();
    input.permission_mode = Some(RoomPermissionMode::Plan);
    let events = collect_events(adapter, input).await;
    assert!(
        final_text(&events).is_some(),
        "未知模式不应中断运行：{events:?}"
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            RunEvent::Status(text) if text.contains("未宣告模式「zen」")
        )),
        "未知模式应有 Status 说明：{events:?}"
    );

    let recorded = std::fs::read_to_string(&record_file).unwrap();
    assert!(
        recorded.contains("\"modeId\":\"plan\""),
        "未知模式应回落到权限映射 plan：{recorded}"
    );
    assert!(
        !recorded.contains("\"modeId\":\"zen\""),
        "未宣告模式不应发送 set_mode：{recorded}"
    );
}

#[tokio::test]
async fn acp_adapter_applies_agent_model_and_skips_unknown() {
    require_node();

    // 已宣告模型：m2 与 agent 当前 m1 不同，应发送 set_config_option。
    let record_dir = tempfile::tempdir().unwrap();
    let record_file = record_dir.path().join("requests.jsonl");
    let harness = setup_fake_harness(vec![
        "--mode".to_string(),
        "record".to_string(),
        "--record".to_string(),
        record_file.to_string_lossy().to_string(),
    ])
    .await;
    let adapter: Arc<dyn HarnessAdapter> = Arc::new(AcpAdapter::new(
        harness.state.custom_harness_store.clone(),
        Some(harness.custom_harness_id),
    ));

    let mut input = run_input(&harness, "切换模型", None);
    input.config = json!({ "model": "m2" }).as_object().cloned();
    let events = collect_events(adapter.clone(), input).await;
    assert!(final_text(&events).is_some(), "模型设置不应中断运行");

    let recorded = std::fs::read_to_string(&record_file).unwrap();
    assert!(
        recorded.contains("\"method\":\"session/set_config_option\""),
        "应发送 set_config_option：{recorded}"
    );
    assert!(
        recorded.contains("\"configId\":\"model\"") && recorded.contains("\"value\":\"m2\""),
        "模型设置应携带 configId 与目标值：{recorded}"
    );

    // 未宣告模型：只发 Status，不发送任何模型设置请求。
    let unknown_dir = tempfile::tempdir().unwrap();
    let unknown_file = unknown_dir.path().join("requests.jsonl");
    let unknown_harness = setup_fake_harness(vec![
        "--mode".to_string(),
        "record".to_string(),
        "--record".to_string(),
        unknown_file.to_string_lossy().to_string(),
    ])
    .await;
    let unknown_adapter: Arc<dyn HarnessAdapter> = Arc::new(AcpAdapter::new(
        unknown_harness.state.custom_harness_store.clone(),
        Some(unknown_harness.custom_harness_id),
    ));
    let mut unknown_input = run_input(&unknown_harness, "未知模型", None);
    unknown_input.config = json!({ "model": "m9" }).as_object().cloned();
    let unknown_events = collect_events(unknown_adapter, unknown_input).await;
    assert!(
        unknown_events.iter().any(|event| matches!(
            event,
            RunEvent::Status(text) if text.contains("未宣告模型「m9」")
        )),
        "未知模型应有 Status 说明：{unknown_events:?}"
    );
    let unknown_recorded = std::fs::read_to_string(&unknown_file).unwrap();
    assert!(
        !unknown_recorded.contains("session/set_config_option"),
        "未知模型不应发送设置请求：{unknown_recorded}"
    );
}

fn run_input_with(
    harness: &TestHarness,
    prompt: &str,
    additional_directories: Vec<String>,
    session: Option<RunSessionContext>,
) -> RunInput {
    let mut input = run_input(harness, prompt, session);
    input.additional_directories = additional_directories;
    input
}

#[tokio::test]
async fn default_factory_builds_acp_adapter_from_registry() {
    require_node();
    // 不注入工厂：直接使用 lib.rs 默认工厂的构造逻辑（注册表 → AcpAdapter）。
    let harness = setup_fake_harness(Vec::new()).await;
    let agent = harness
        .state
        .agent_service
        .create(serde_json::json!({
            "name": "acp-成员",
            "persona": "自定义 ACP 成员",
            "harness": "acp",
            "customHarnessId": harness.custom_harness_id,
        }))
        .await
        .unwrap();
    assert_eq!(
        agent.harness,
        moon_server::domain::agent::HarnessKind::Acp
    );
    assert_eq!(agent.custom_harness_id, Some(harness.custom_harness_id));

    let factory: AdapterFactory =
        moon_server::adapters::default_adapter_factory(harness.state.custom_harness_store.clone());
    let adapter = factory(&agent).expect("默认工厂应能构造 acp 适配器");
    let events = collect_events(adapter, run_input(&harness, "你好", None)).await;
    let text = final_text(&events).expect("默认工厂链路应有 Final");
    assert!(text.contains("我是自定义 ACP agent"), "实际回复：{text}");
}

#[tokio::test]
async fn acp_adapter_times_out_and_terminates_agent() {
    require_node();
    let harness = setup_fake_harness(vec![
        "--mode".to_string(),
        "hang".to_string(),
        "--pid-file".to_string(),
        "hang-agent.pid".to_string(),
    ])
    .await;
    let pid_path = harness.workspace.join("hang-agent.pid");

    let adapter: Arc<dyn HarnessAdapter> = Arc::new(AcpAdapter::new(
        harness.state.custom_harness_store.clone(),
        Some(harness.custom_harness_id),
    ));
    let mut input = run_input(&harness, "挂起等待", None);
    input.timeout = Some(Duration::from_secs(1));

    let mut stream = adapter.run(input);
    let started = Instant::now();
    let mut failure = None;
    while let Some(item) = stream.next().await {
        if let Err(error) = item {
            failure = Some(error.message);
            break;
        }
    }
    let elapsed = started.elapsed();

    assert!(
        failure
            .as_deref()
            .is_some_and(|message| message.contains("运行超过")),
        "应返回超时错误：{failure:?}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "超时应在 5 秒内生效：{elapsed:?}"
    );

    let pid_text = std::fs::read_to_string(&pid_path)
        .unwrap_or_else(|error| panic!("读取 PID 文件失败：{error}"));
    let pid: u32 = pid_text.trim().parse().expect("PID 文件应是数字");
    assert!(
        wait_for_process_exit(pid, Duration::from_secs(3)),
        "超时后 fake agent 进程仍然存活：pid={pid}"
    );
}

async fn stored_harness(harness: &TestHarness) -> CustomHarness {
    harness
        .state
        .custom_harness_store
        .get_by_id(harness.custom_harness_id)
        .await
        .unwrap()
        .expect("注册表条目应存在")
}

/// 探测只做 initialize + session/new，不发 prompt（AC-002 probe 语义）。
#[tokio::test]
async fn acp_probe_sends_only_initialize_and_session_new() {
    require_node();
    let record_dir = tempfile::tempdir().unwrap();
    let record_file = record_dir.path().join("probe.jsonl");
    let harness = setup_fake_harness(vec![
        "--mode".to_string(),
        "record".to_string(),
        "--record".to_string(),
        record_file.to_string_lossy().to_string(),
    ])
    .await;
    let custom = stored_harness(&harness).await;

    let capabilities = probe_capabilities(
        &custom,
        &harness.workspace.to_string_lossy(),
        Duration::from_secs(15),
    )
    .await
    .expect("record 模式探测应成功");

    assert_eq!(capabilities.modes.len(), 4, "应解析到 fake 自报的模式");
    assert_eq!(capabilities.current_model_id.as_deref(), Some("m1"));

    let recorded = std::fs::read_to_string(&record_file).unwrap();
    assert!(
        recorded.contains("\"method\":\"initialize\"")
            && recorded.contains("\"method\":\"session/new\""),
        "探测应发出 initialize 与 session/new：{recorded}"
    );
    assert!(
        !recorded.contains("\"method\":\"session/prompt\""),
        "探测不应发送 prompt：{recorded}"
    );
}

/// 探测超时必须终止 agent 进程树。
#[tokio::test]
async fn acp_probe_times_out_and_terminates_agent() {
    require_node();
    let harness = setup_fake_harness(vec![
        "--hang-handshake".to_string(),
        "--pid-file".to_string(),
        "probe-hang.pid".to_string(),
    ])
    .await;
    let custom = stored_harness(&harness).await;
    let pid_path = harness.workspace.join("probe-hang.pid");

    // 外层再兜一层超时：若探测自身的超时包裹被误删，表现为明确失败而非挂起。
    let started = Instant::now();
    let outcome = tokio::time::timeout(
        Duration::from_secs(5),
        probe_capabilities(
            &custom,
            &harness.workspace.to_string_lossy(),
            // 留足 fixture 写 PID 文件的余量，避免负载下的窄竞态。
            Duration::from_secs(1),
        ),
    )
    .await
    .expect("探测应按自身超时返回，不应挂满外层上限");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "探测应在内部超时上限附近返回"
    );

    let error = outcome.expect_err("握手挂起时探测应超时");
    assert!(
        error.message.contains("能力探测超过"),
        "应返回探测超时错误：{}",
        error.message
    );
    let pid_text = std::fs::read_to_string(&pid_path)
        .unwrap_or_else(|error| panic!("读取 PID 文件失败：{error}"));
    let pid: u32 = pid_text.trim().parse().expect("PID 文件应是数字");
    assert!(
        wait_for_process_exit(pid, Duration::from_secs(3)),
        "探测超时后 fake agent 进程仍然存活：pid={pid}"
    );
}

/// session/load 失败必须回退新建会话，运行不中断。
#[tokio::test]
async fn acp_adapter_falls_back_to_new_session_when_load_fails() {
    require_node();
    let record_dir = tempfile::tempdir().unwrap();
    let record_file = record_dir.path().join("requests.jsonl");
    let harness = setup_fake_harness(vec![
        "--mode".to_string(),
        "record".to_string(),
        "--record".to_string(),
        record_file.to_string_lossy().to_string(),
        "--fail-load".to_string(),
    ])
    .await;
    let adapter: Arc<dyn HarnessAdapter> = Arc::new(AcpAdapter::new(
        harness.state.custom_harness_store.clone(),
        Some(harness.custom_harness_id),
    ));

    let first = collect_events(adapter.clone(), run_input(&harness, "第一问", None)).await;
    let first_session = first
        .iter()
        .find_map(|event| match event {
            RunEvent::Session(id) => Some(id.clone()),
            _ => None,
        })
        .expect("第一次运行应有会话事件");

    let second = collect_events(
        adapter,
        run_input(
            &harness,
            "第二问",
            Some(RunSessionContext {
                harness_session_id: Some(first_session.clone()),
                storage_dir: harness.dir.path().join("storage").to_string_lossy().to_string(),
            }),
        ),
    )
    .await;
    let second_session = second
        .iter()
        .find_map(|event| match event {
            RunEvent::Session(id) => Some(id.clone()),
            _ => None,
        })
        .expect("回退后仍应有会话事件");
    // 每次运行 spawn 独立 fake 进程、session id 从 1 重新计数，因此回退新建
    // 的 id 与首次相同；「回退为新建」由下面的线上计数证明。
    assert_eq!(
        second_session, first_session,
        "回退新建的会话 id 应来自新的 session/new（计数从 1 起）"
    );
    assert!(
        final_text(&second).is_some(),
        "回退运行不应中断：{second:?}"
    );

    let recorded = std::fs::read_to_string(&record_file).unwrap();
    assert!(
        recorded.contains("\"method\":\"session/load\""),
        "应先尝试 session/load：{recorded}"
    );
    assert_eq!(
        recorded.matches("\"method\":\"session/new\"").count(),
        2,
        "load 失败后第二次运行应回退新建会话：{recorded}"
    );
}

/// 旧式 models 契约走 session/set_model（SDK 未建模，经 raw 通道保持既有行为）。
#[tokio::test]
async fn acp_adapter_uses_set_model_for_legacy_models_contract() {
    require_node();
    let record_dir = tempfile::tempdir().unwrap();
    let record_file = record_dir.path().join("requests.jsonl");
    let harness = setup_fake_harness(vec![
        "--mode".to_string(),
        "record".to_string(),
        "--record".to_string(),
        record_file.to_string_lossy().to_string(),
        "--legacy-models".to_string(),
    ])
    .await;
    let adapter: Arc<dyn HarnessAdapter> = Arc::new(AcpAdapter::new(
        harness.state.custom_harness_store.clone(),
        Some(harness.custom_harness_id),
    ));

    let mut input = run_input(&harness, "切换旧式模型", None);
    input.config = json!({ "model": "m2" }).as_object().cloned();
    let events = collect_events(adapter, input).await;
    assert!(
        final_text(&events).is_some(),
        "旧式模型设置不应中断运行：{events:?}"
    );

    let recorded = std::fs::read_to_string(&record_file).unwrap();
    assert!(
        recorded.contains("\"method\":\"session/set_model\"")
            && recorded.contains("\"modelId\":\"m2\""),
        "旧式 models 应发送 session/set_model：{recorded}"
    );
    assert!(
        !recorded.contains("session/set_config_option"),
        "旧式 models 不应发送 set_config_option：{recorded}"
    );
}

/// 显式会话模式优先于房间权限模式映射，且不触发思考等级下发。
#[tokio::test]
async fn acp_adapter_session_mode_overrides_permission_mode() {
    require_node();
    let record_dir = tempfile::tempdir().unwrap();
    let record_file = record_dir.path().join("requests.jsonl");
    let harness = setup_fake_harness(vec![
        "--mode".to_string(),
        "record".to_string(),
        "--record".to_string(),
        record_file.to_string_lossy().to_string(),
    ])
    .await;
    let adapter: Arc<dyn HarnessAdapter> = Arc::new(AcpAdapter::new(
        harness.state.custom_harness_store.clone(),
        Some(harness.custom_harness_id),
    ));

    let mut input = run_input(&harness, "会话模式优先", None);
    input.config = json!({ "sessionMode": "bypassPermissions" }).as_object().cloned();
    input.permission_mode = Some(RoomPermissionMode::Auto);
    let events = collect_events(adapter, input).await;
    assert!(
        final_text(&events).is_some(),
        "会话模式不应中断运行：{events:?}"
    );
    assert!(
        !events.iter().any(|event| matches!(
            event,
            RunEvent::Status(text) if text.contains("未宣告会话模式")
        )),
        "有效会话模式命中后不应出现「未宣告会话模式」Status：{events:?}"
    );

    let recorded = std::fs::read_to_string(&record_file).unwrap();
    assert!(
        recorded.contains("\"method\":\"session/set_mode\"")
            && recorded.contains("\"modeId\":\"bypassPermissions\""),
        "应下发显式会话模式：{recorded}"
    );
    assert!(
        !recorded.contains("\"modeId\":\"acceptEdits\""),
        "显式会话模式命中后不应再发权限映射：{recorded}"
    );
    assert_eq!(
        recorded.matches("\"method\":\"session/set_mode\"").count(),
        1,
        "本轮只应发送一次 set_mode：{recorded}"
    );
    assert!(
        !recorded.contains("session/set_config_option"),
        "未设置思考等级时不应发送 set_config_option：{recorded}"
    );
}

/// 未宣告的会话模式只提示并回落房间权限映射。
#[tokio::test]
async fn acp_adapter_unknown_session_mode_falls_back() {
    require_node();
    let record_dir = tempfile::tempdir().unwrap();
    let record_file = record_dir.path().join("requests.jsonl");
    let harness = setup_fake_harness(vec![
        "--mode".to_string(),
        "record".to_string(),
        "--record".to_string(),
        record_file.to_string_lossy().to_string(),
    ])
    .await;
    let adapter: Arc<dyn HarnessAdapter> = Arc::new(AcpAdapter::new(
        harness.state.custom_harness_store.clone(),
        Some(harness.custom_harness_id),
    ));

    let mut input = run_input(&harness, "未知会话模式回落", None);
    input.config = json!({ "sessionMode": "zen2" }).as_object().cloned();
    input.permission_mode = Some(RoomPermissionMode::Plan);
    let events = collect_events(adapter, input).await;
    assert!(
        final_text(&events).is_some(),
        "未知会话模式不应中断运行：{events:?}"
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            RunEvent::Status(text) if text.contains("未宣告会话模式「zen2」")
        )),
        "未知会话模式应有 Status 说明：{events:?}"
    );

    let recorded = std::fs::read_to_string(&record_file).unwrap();
    assert!(
        recorded.contains("\"modeId\":\"plan\""),
        "未知会话模式应回落到权限映射 plan：{recorded}"
    );
    assert!(
        !recorded.contains("\"modeId\":\"zen2\""),
        "未宣告会话模式不应发送 set_mode：{recorded}"
    );
}

/// reasoningLevel 命中思考类 configOption 时经 set_config_option 下发。
#[tokio::test]
async fn acp_adapter_reasoning_level_applies_to_thinking_option() {
    require_node();
    let record_dir = tempfile::tempdir().unwrap();
    let record_file = record_dir.path().join("requests.jsonl");
    let harness = setup_fake_harness(vec![
        "--mode".to_string(),
        "record".to_string(),
        "--record".to_string(),
        record_file.to_string_lossy().to_string(),
    ])
    .await;
    let adapter: Arc<dyn HarnessAdapter> = Arc::new(AcpAdapter::new(
        harness.state.custom_harness_store.clone(),
        Some(harness.custom_harness_id),
    ));

    let mut input = run_input(&harness, "思考等级", None);
    input.config = json!({ "reasoningLevel": "high" }).as_object().cloned();
    let events = collect_events(adapter, input).await;
    assert!(
        final_text(&events).is_some(),
        "思考等级设置不应中断运行：{events:?}"
    );
    assert!(
        !events.iter().any(|event| matches!(
            event,
            RunEvent::Status(text) if text.contains("未宣告模式")
        )),
        "思考等级命中思考选项后不应出现「未宣告模式」Status：{events:?}"
    );

    let recorded = std::fs::read_to_string(&record_file).unwrap();
    assert!(
        recorded.contains("\"configId\":\"reasoning_effort\"")
            && recorded.contains("\"value\":\"high\""),
        "思考等级应经 set_config_option 下发：{recorded}"
    );
    assert!(
        !recorded.contains("\"method\":\"session/set_mode\""),
        "思考等级不是模式，不应发送 set_mode：{recorded}"
    );
}

/// 会话模式与思考等级相互独立：两条偏好都下发。
#[tokio::test]
async fn acp_adapter_session_mode_and_thinking_apply_independently() {
    require_node();
    let record_dir = tempfile::tempdir().unwrap();
    let record_file = record_dir.path().join("requests.jsonl");
    let harness = setup_fake_harness(vec![
        "--mode".to_string(),
        "record".to_string(),
        "--record".to_string(),
        record_file.to_string_lossy().to_string(),
    ])
    .await;
    let adapter: Arc<dyn HarnessAdapter> = Arc::new(AcpAdapter::new(
        harness.state.custom_harness_store.clone(),
        Some(harness.custom_harness_id),
    ));

    let mut input = run_input(&harness, "会话模式与思考等级并存", None);
    input.config = json!({ "sessionMode": "plan", "reasoningLevel": "high" })
        .as_object()
        .cloned();
    let events = collect_events(adapter, input).await;
    assert!(
        final_text(&events).is_some(),
        "并存偏好不应中断运行：{events:?}"
    );

    let recorded = std::fs::read_to_string(&record_file).unwrap();
    assert!(
        recorded.contains("\"modeId\":\"plan\""),
        "会话模式应下发 set_mode：{recorded}"
    );
    assert!(
        recorded.contains("\"configId\":\"reasoning_effort\"")
            && recorded.contains("\"value\":\"high\""),
        "思考等级应下发 set_config_option：{recorded}"
    );
}

/// 三级优先链互斥：sessionMode 与旧 reasoningLevel 同为 mode id 时，
/// 只发显式 sessionMode，旧 reasoningLevel 不再覆盖。
#[tokio::test]
async fn acp_adapter_session_mode_wins_over_legacy_reasoning_mode() {
    require_node();
    let record_dir = tempfile::tempdir().unwrap();
    let record_file = record_dir.path().join("requests.jsonl");
    let harness = setup_fake_harness(vec![
        "--mode".to_string(),
        "record".to_string(),
        "--record".to_string(),
        record_file.to_string_lossy().to_string(),
    ])
    .await;
    let adapter: Arc<dyn HarnessAdapter> = Arc::new(AcpAdapter::new(
        harness.state.custom_harness_store.clone(),
        Some(harness.custom_harness_id),
    ));

    let mut input = run_input(&harness, "显式会话模式优先于旧字段", None);
    input.config = json!({ "sessionMode": "plan", "reasoningLevel": "acceptEdits" })
        .as_object()
        .cloned();
    input.permission_mode = Some(RoomPermissionMode::Auto);
    let events = collect_events(adapter, input).await;
    assert!(
        final_text(&events).is_some(),
        "并存模式偏好不应中断运行：{events:?}"
    );
    assert!(
        !events.iter().any(|event| matches!(
            event,
            RunEvent::Status(text) if text.contains("未宣告")
        )),
        "两个偏好都命中时不应出现任何「未宣告」Status：{events:?}"
    );

    let recorded = std::fs::read_to_string(&record_file).unwrap();
    assert_eq!(
        recorded.matches("\"method\":\"session/set_mode\"").count(),
        1,
        "只应发送一次 set_mode：{recorded}"
    );
    assert!(
        recorded.contains("\"modeId\":\"plan\""),
        "显式 sessionMode 应胜出：{recorded}"
    );
    assert!(
        !recorded.contains("\"modeId\":\"acceptEdits\""),
        "旧 reasoningLevel 不应覆盖显式 sessionMode：{recorded}"
    );
}

fn wait_for_process_exit(pid: u32, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if !process_exists(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn process_exists(pid: u32) -> bool {
    #[cfg(windows)]
    {
        let output = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output();
        match output {
            Ok(output) => String::from_utf8_lossy(&output.stdout).contains(&pid.to_string()),
            Err(_) => false,
        }
    }
    #[cfg(unix)]
    {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
}

