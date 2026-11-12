//! zcode 无头适配器。
//!
//! CLI 事实（本机 zcode 0.17.1 实测）：
//! - 入口为 `ZCode/resources/glm/zcode.cjs`；桌面版没有独立 node，使用
//!   `ZCode.exe` 的 `ELECTRON_RUN_AS_NODE=1` 模式执行脚本。
//! - 无头参数：`-p <prompt> --output-format stream-json --cwd <workspace> --mode <mode>`；
//!   `--resume <sessionId>` 续接会话；长提示词拆分后用重复 `--attach` 传递。
//! - stdout 为 NDJSON，一行一个事件；`model.streaming` 携带文本/推理增量，
//!   `tool.updated` 携带工具开始/结束，`turn.completed` 携带用量，
//!   `result.payload.response` 为完整回复；失败通过非 0 退出码与 stderr 判定。
//! - 会话库通过 `ZCODE_SESSION_DB_PATH` 隔离：复用会话指向持久目录，
//!   否则指向临时目录并在结束后删除，避免污染用户 ZCode 会话列表。

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::domain::agent::HarnessKind;
use crate::domain::room::RoomPermissionMode;
use crate::domain::run_event::{RunEvent, RunInput};
use crate::domain::session::RunSessionContext;
use crate::error::{AppError, AppResult, ErrorCode};

use super::types::{EventStream, HarnessAdapter};
use super::{
    describe_spawn_failure, tail, unique_temp_dir, unique_temp_dir_in, Cleanup,
};

/// 安装目录下的 zcode 脚本；由协议处理器/卸载注册表/App Paths 给出的目录推导。
const ZCODE_SCRIPT_RELATIVE_PATHS: &[&str] = &["resources/glm/zcode.cjs"];
/// PATH 上的 zcode CLI 命令名。
const ZCODE_PATH_NAMES: &[&str] = &["zcode"];
const SESSION_NOT_FOUND_MARKER: &str = "Session not found";

/// 单个附件的最大字符数，留出 Windows 32,767 命令行上限的余量。
pub const PROMPT_ATTACHMENT_LIMIT: usize = 24_000;

pub fn build_attachment_instruction(part_count: usize) -> String {
    format!(
        "本次提示词较长，已按顺序拆分为 {part_count} 个附件（part-1.md … part-{part_count}.md）。\
         请完整阅读全部附件，把它们合起来当作本次完整提示词严格执行。\
         直接输出最终回复正文，不要复述附件内容，不要输出思考过程。"
    )
}

/// 把提示词切成若干片，保证 `parts.join("") == prompt`。
///
/// 优先在片尾窗口内最后一个换行处切分；单行本身超过 limit 时退回硬切。
pub fn split_prompt_for_attachments(
    prompt: &str,
    limit: usize,
) -> AppResult<Vec<String>> {
    if limit == 0 {
        return Err(AppError::harness_run_failed("提示词分片上限非法：0"));
    }
    let chars: Vec<char> = prompt.chars().collect();
    if chars.len() <= limit {
        return Ok(vec![prompt.to_string()]);
    }

    let mut parts = Vec::new();
    let mut offset = 0;
    while offset < chars.len() {
        let mut end = (offset + limit).min(chars.len());
        if end < chars.len() {
            if let Some(position) = chars[offset..end].iter().rposition(|value| *value == '\n') {
                if position >= 1 {
                    end = offset + position + 1;
                }
            }
        }
        parts.push(chars[offset..end].iter().collect());
        offset = end;
    }
    Ok(parts)
}

pub struct PromptInvocationPlan {
    pub args: Vec<String>,
    cleanup: Cleanup,
}

impl PromptInvocationPlan {
    pub fn cleanup_path(&self) -> Option<&PathBuf> {
        self.cleanup.path()
    }
}

/// 规划一次运行的提示词参数：短提示词 `-p <prompt>`，长提示词写临时附件。
pub fn plan_prompt_invocation(
    prompt: &str,
    limit: usize,
    temp_root: Option<&Path>,
) -> AppResult<PromptInvocationPlan> {
    let parts = split_prompt_for_attachments(prompt, limit)?;
    if parts.len() == 1 {
        return Ok(PromptInvocationPlan {
            args: vec!["-p".to_string(), prompt.to_string()],
            cleanup: Cleanup::none(),
        });
    }

    let root = temp_root
        .map(Path::to_path_buf)
        .unwrap_or_else(std::env::temp_dir);
    let dir = unique_temp_dir_in(&root, "moon-prompt-").map_err(|error| {
        AppError::with_source(
            ErrorCode::HarnessRunFailed,
            "无法创建提示词附件目录",
            error,
        )
    })?;
    let cleanup = Cleanup::directory(dir.clone());

    let mut paths = Vec::with_capacity(parts.len());
    for (index, part) in parts.iter().enumerate() {
        let file_path = dir.join(format!("part-{}.md", index + 1));
        // 显式 UTF-8：std::fs::write 写入字符串的 UTF-8 字节。
        if let Err(error) = std::fs::write(&file_path, part) {
            return Err(AppError::with_source(
                ErrorCode::HarnessRunFailed,
                "无法写入提示词附件",
                error,
            ));
        }
        paths.push(file_path);
    }

    let mut args = vec![
        "-p".to_string(),
        build_attachment_instruction(parts.len()),
    ];
    for path in paths {
        args.push("--attach".to_string());
        args.push(path.to_string_lossy().into_owned());
    }
    Ok(PromptInvocationPlan { args, cleanup })
}

pub struct SessionDbPlan {
    pub env: Vec<(String, String)>,
    _cleanup: Cleanup,
}

/// 规划会话库：复用会话用持久目录，否则隔离到临时库并在结束后删除。
pub fn plan_session_env(session: Option<&RunSessionContext>) -> AppResult<SessionDbPlan> {
    if let Some(session) = session {
        std::fs::create_dir_all(&session.storage_dir).map_err(|error| {
            AppError::with_source(
                ErrorCode::HarnessRunFailed,
                "无法创建会话存储目录",
                error,
            )
        })?;
        return Ok(SessionDbPlan {
            env: vec![(
                "ZCODE_SESSION_DB_PATH".to_string(),
                Path::new(&session.storage_dir)
                    .join("db.sqlite")
                    .to_string_lossy()
                    .into_owned(),
            )],
            _cleanup: Cleanup::none(),
        });
    }

    let dir = unique_temp_dir("moon-zcode-session-").map_err(|error| {
        AppError::with_source(
            ErrorCode::HarnessRunFailed,
            "无法创建临时会话库目录",
            error,
        )
    })?;
    Ok(SessionDbPlan {
        env: vec![(
            "ZCODE_SESSION_DB_PATH".to_string(),
            dir.join("db.sqlite").to_string_lossy().into_owned(),
        )],
        _cleanup: Cleanup::directory(dir),
    })
}

pub struct ResolvedZcodeCommand {
    pub command: String,
    pub args_prefix: Vec<String>,
    pub env: Vec<(String, String)>,
    /// 定位时尝试过的来源；启动失败时拼入错误，便于诊断 `program not found`。
    pub searched: Vec<String>,
}

/// 定位 zcode 脚本或 CLI：显式配置 → 协议处理器 → 卸载注册表 → App Paths → PATH。
///
/// 不做目录猜测：安装器写入的注册表元数据优先，最后才回退到 PATH。
pub fn locate_zcode_program(configured_bin: Option<&str>) -> super::locate::LocatedProgram {
    let explicit = normalize_bin(configured_bin)
        .or_else(|| normalize_bin(std::env::var("MOON_ZCODE_BIN").ok().as_deref()));
    super::locate::locate_program(&super::locate::LocateRequest {
        explicit: explicit.as_deref(),
        protocol: Some("zcode"),
        uninstall_prefix: Some("ZCode"),
        app_paths_exe: Some("ZCode.exe"),
        directory_candidates: ZCODE_SCRIPT_RELATIVE_PATHS,
        path_names: ZCODE_PATH_NAMES,
        vendor_defaults: &[],
    })
}

/// 解析 zcode 可执行文件（脚本时自动选择 node / ZCode.exe 作为 runner）。
pub fn resolve_zcode_command(configured_bin: Option<&str>) -> ResolvedZcodeCommand {
    let located = locate_zcode_program(configured_bin);
    let mut resolved = command_for(&located.program);
    resolved.searched = located.searched;
    resolved
}

fn command_for(bin: &str) -> ResolvedZcodeCommand {
    if is_node_script(bin) {
        let runner = std::env::var("MOON_ZCODE_NODE_BIN")
            .ok()
            .and_then(|value| normalize_bin(Some(&value)))
            .or_else(|| derive_zcode_exe(bin))
            .unwrap_or_else(|| "node".to_string());
        let mut env = Vec::new();
        if runner
            .rsplit(['/', '\\'])
            .next()
            .is_some_and(|name| name.eq_ignore_ascii_case("zcode.exe"))
        {
            env.push(("ELECTRON_RUN_AS_NODE".to_string(), "1".to_string()));
        }
        return ResolvedZcodeCommand {
            command: runner,
            args_prefix: vec![bin.to_string()],
            env,
            searched: Vec::new(),
        };
    }
    ResolvedZcodeCommand {
        command: bin.to_string(),
        args_prefix: Vec::new(),
        env: Vec::new(),
        searched: Vec::new(),
    }
}

fn derive_zcode_exe(bin: &str) -> Option<String> {
    let root = Path::new(bin).parent()?.parent()?.parent()?;
    let candidate = root.join("ZCode.exe");
    candidate.is_file().then(|| candidate.to_string_lossy().into_owned())
}

/// 把房间权限模式映射为 zcode `--mode`；未传时保持 CLI 默认（yolo）。
pub fn zcode_permission_args(mode: Option<RoomPermissionMode>) -> Vec<String> {
    let mapped = match mode {
        None => return Vec::new(),
        Some(RoomPermissionMode::Plan) => "plan",
        Some(RoomPermissionMode::Auto) => "guarded",
        Some(RoomPermissionMode::Full) => "yolo",
    };
    vec!["--mode".to_string(), mapped.to_string()]
}

#[derive(Debug, Clone, Default)]
pub struct ParsedZcodeLine {
    pub event: Option<RunEvent>,
    pub session_id: Option<String>,
}

/// 纯函数：把一行 stdout 解析为事件与 sessionId；异常输入返回空结果。
pub fn parse_zcode_line_detail(line: &str) -> ParsedZcodeLine {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return ParsedZcodeLine::default();
    }
    let Ok(Value::Object(parsed)) = serde_json::from_str::<Value>(trimmed) else {
        return ParsedZcodeLine::default();
    };
    let session_id = parsed
        .get("sessionId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let Some(Value::Object(payload)) = parsed.get("payload") else {
        return ParsedZcodeLine {
            event: None,
            session_id,
        };
    };

    ParsedZcodeLine {
        event: parse_zcode_event(&parsed, payload),
        session_id,
    }
}

pub fn parse_zcode_line(line: &str) -> Option<RunEvent> {
    parse_zcode_line_detail(line).event
}

pub(crate) fn parse_zcode_event(
    parsed: &serde_json::Map<String, Value>,
    payload: &serde_json::Map<String, Value>,
) -> Option<RunEvent> {
    match parsed.get("type").and_then(Value::as_str) {
        Some("model.streaming") => match payload.get("kind").and_then(Value::as_str) {
            Some("text_delta") => payload
                .get("delta")
                .and_then(Value::as_str)
                .map(|text| RunEvent::TextDelta(text.to_string())),
            Some("reasoning_delta") => payload
                .get("delta")
                .and_then(Value::as_str)
                .map(|text| RunEvent::Reasoning(text.to_string())),
            Some("tool_call") => {
                let tool_call_id = read_string(payload.get("toolCallId"))?;
                let name = read_string(payload.get("toolName"))?;
                Some(RunEvent::ToolStart {
                    tool_call_id,
                    name,
                    input: payload.get("input").cloned().unwrap_or(Value::Null),
                })
            }
            _ => None,
        },
        Some("tool.updated") => parse_tool_updated(payload),
        Some("turn.completed") => Some(RunEvent::Usage {
            token_count: read_non_negative_int(payload.get("tokenCount")),
            context_size: None,
            tool_call_count: read_non_negative_int(payload.get("toolCallCount")),
            duration_ms: read_non_negative_number(payload.get("duration")),
        }),
        Some("turn.failed") => payload
            .get("error")
            .and_then(Value::as_object)
            .and_then(|error| read_string(error.get("message")))
            .map(RunEvent::Status),
        Some("result") => payload
            .get("response")
            .and_then(Value::as_str)
            .map(|text| RunEvent::Final(text.to_string())),
        _ => None,
    }
}

fn parse_tool_updated(payload: &serde_json::Map<String, Value>) -> Option<RunEvent> {
    let tool_call_id = read_string(payload.get("toolCallId"));
    let name = read_string(payload.get("toolName"));

    if payload.get("kind").and_then(Value::as_str) == Some("error") {
        let message = payload
            .get("error")
            .and_then(Value::as_object)
            .and_then(|error| read_string(error.get("message")))?;
        return Some(RunEvent::ToolEnd {
            tool_call_id,
            name,
            ok: false,
            detail: message,
        });
    }

    let tool_call_id = tool_call_id?;
    match payload.get("kind").and_then(Value::as_str) {
        Some("scheduled") => Some(RunEvent::ToolStart {
            tool_call_id,
            name: name?,
            input: payload.get("input").cloned().unwrap_or(Value::Null),
        }),
        Some("result") => Some(RunEvent::ToolEnd {
            tool_call_id: Some(tool_call_id),
            name,
            ok: true,
            detail: describe_tool_result(payload.get("result")),
        }),
        _ => None,
    }
}

fn describe_tool_result(result: Option<&Value>) -> String {
    match result {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Object(map)) => {
            if let Some(display) = read_string(map.get("display")) {
                return display;
            }
            if let Some(content) = read_string(map.get("content")) {
                return content;
            }
            if let Some(message) = map
                .get("error")
                .and_then(Value::as_object)
                .and_then(|error| read_string(error.get("message")))
            {
                return message;
            }
            serde_json::to_string(result.unwrap()).unwrap_or_default()
        }
        Some(value) => serde_json::to_string(value).unwrap_or_default(),
        None => String::new(),
    }
}

fn read_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn read_non_negative_int(value: Option<&Value>) -> Option<i64> {
    value
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite() && *number >= 0.0)
        .map(|number| number.floor() as i64)
}

fn read_non_negative_number(value: Option<&Value>) -> Option<i64> {
    value
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite() && *number >= 0.0)
        .map(|number| number as i64)
}

fn read_zcode_bin(config: Option<&crate::domain::agent::ModelConfig>) -> Option<String> {
    config
        .and_then(|config| config.get("zcodeBin"))
        .and_then(Value::as_str)
        .and_then(|value| normalize_bin(Some(value)))
}

fn normalize_bin(value: Option<&str>) -> Option<String> {
    let trimmed = value?.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn is_node_script(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".cjs") || lower.ends_with(".mjs") || lower.ends_with(".js")
}

pub struct ZcodeAdapter;

impl HarnessAdapter for ZcodeAdapter {
    fn kind(&self) -> HarnessKind {
        HarnessKind::Zcode
    }

    fn run(&self, input: RunInput) -> EventStream {
        let (sender, receiver) = mpsc::channel(128);
        tokio::spawn(async move {
            if let Err(error) = run_zcode(input, sender.clone()).await {
                let _ = sender.send(Err(error)).await;
            }
        });
        Box::pin(tokio_stream::wrappers::ReceiverStream::new(receiver))
    }
}

async fn run_zcode(
    input: RunInput,
    sender: mpsc::Sender<AppResult<RunEvent>>,
) -> AppResult<()> {
    // 默认走 app-server 以支持房间附加目录；`MOON_ZCODE_TRANSPORT=cli`
    // 可回退到成熟的 --prompt 传输，便于排查协议问题。
    if std::env::var("MOON_ZCODE_TRANSPORT")
        .ok()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("cli"))
    {
        return run_zcode_cli(input, sender).await;
    }
    run_zcode_app_server(input, sender).await
}

async fn run_zcode_app_server(
    input: RunInput,
    sender: mpsc::Sender<AppResult<RunEvent>>,
) -> AppResult<()> {
    let session_plan = plan_session_env(input.session.as_ref())?;
    let model_selection =
        crate::adapters::zcode_config::read_zcode_model_selection(input.config.as_ref());
    // env 覆盖仅对旧版 CLI 的 TUI/CLI 流程有意义；app-server 流程必须
    // 经 session/setModel 应用（见 AppServerLaunch.model_selection）。
    let model_override = match &model_selection {
        Some(selection) => Some(
            crate::adapters::zcode_config::create_zcode_model_override(selection).map_err(
                |error| AppError::harness_run_failed(format!("无法应用所选模型：{error}")),
            )?,
        ),
        None => None,
    };
    let resolved = resolve_zcode_command(read_zcode_bin(input.config.as_ref()).as_deref());

    let mut env = resolved.env.clone();
    if let Some(model_override) = &model_override {
        env.extend(model_override.env.clone());
    }
    env.extend(session_plan.env.clone());
    if let Some(builtin_path) =
        crate::adapters::zcode_config::resolve_zcode_builtin_config_path()
    {
        env.push((
            "ZCODE_BUILTIN_PROVIDER_CONFIG_FILE".to_string(),
            builtin_path.to_string_lossy().into_owned(),
        ));
    }

    let launch = crate::adapters::zcode_app_server::AppServerLaunch {
        program: resolved.command,
        args_prefix: resolved.args_prefix,
        searched: resolved.searched,
        env,
        workspace_dir: input.workspace_dir.clone(),
        additional_directories: input.additional_directories.clone(),
        permission_mode: input.permission_mode,
        prompt: input.prompt.clone(),
        resume_session_id: input
            .session
            .as_ref()
            .and_then(|session| session.harness_session_id.clone()),
        model_selection,
        timeout: input.timeout,
    };
    let result = crate::adapters::zcode_app_server::run(launch, sender).await;
    drop(model_override);
    drop(session_plan);
    result
}

async fn run_zcode_cli(
    input: RunInput,
    sender: mpsc::Sender<AppResult<RunEvent>>,
) -> AppResult<()> {
    let prompt_plan = plan_prompt_invocation(
        &input.prompt,
        PROMPT_ATTACHMENT_LIMIT,
        None,
    )?;
    let session_plan = plan_session_env(input.session.as_ref())?;
    // 模型配置只对本次运行生效；临时副本随运行结束清理。
    let model_override = match crate::adapters::zcode_config::read_zcode_model_selection(
        input.config.as_ref(),
    ) {
        Some(selection) => Some(
            crate::adapters::zcode_config::create_zcode_model_override(&selection).map_err(
                |error| {
                    AppError::harness_run_failed(format!("无法应用所选模型：{error}"))
                },
            )?,
        ),
        None => None,
    };
    let resolved = resolve_zcode_command(read_zcode_bin(input.config.as_ref()).as_deref());
    let resume_args = match input.session.as_ref().and_then(|session| session.harness_session_id.as_ref()) {
        Some(session_id) => vec!["--resume".to_string(), session_id.clone()],
        None => Vec::new(),
    };
    let mut args = resolved.args_prefix.clone();
    args.extend(resume_args);
    args.extend(zcode_permission_args(input.permission_mode));
    args.extend(prompt_plan.args.iter().cloned());
    args.push("--output-format".to_string());
    args.push("stream-json".to_string());
    args.push("--cwd".to_string());
    args.push(input.workspace_dir.clone());

    let mut command = Command::new(&resolved.command);
    command
        .args(&args)
        .current_dir(&input.workspace_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in &resolved.env {
        command.env(key, value);
    }
    if let Some(model_override) = &model_override {
        for (key, value) in &model_override.env {
            command.env(key, value);
        }
    }
    for (key, value) in &session_plan.env {
        command.env(key, value);
    }
    // 桌面端从快捷方式启动时没有 ZCode 客户端注入的环境变量，
    // 需显式指向按版本生成的内置 provider 配置，否则 CLI 会回退到安装目录探测并退出。
    if let Some(builtin_path) =
        crate::adapters::zcode_config::resolve_zcode_builtin_config_path()
    {
        command.env(
            "ZCODE_BUILTIN_PROVIDER_CONFIG_FILE",
            builtin_path,
        );
    }

    let mut child = command
        .spawn()
        .map_err(|error| spawn_error(error, "zcode", &resolved.searched))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::harness_run_failed("无法读取 zcode 输出"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| AppError::harness_run_failed("无法读取 zcode 错误输出"))?;
    let stderr_task = tokio::spawn(async move {
        let mut buffer = String::new();
        let _ = stderr.read_to_string(&mut buffer).await;
        buffer
    });

    let mut lines = BufReader::new(stdout).lines();
    let mut accumulated = String::new();
    let mut saw_final = false;

    let read_result = super::with_run_timeout(input.timeout, async {
        let mut reported_session = false;
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    let parsed = parse_zcode_line_detail(&line);
                    if let Some(session_id) = parsed.session_id {
                        if !reported_session {
                            reported_session = true;
                            if sender
                                .send(Ok(RunEvent::Session(session_id)))
                                .await
                                .is_err()
                            {
                                return Ok(false);
                            }
                        }
                    }
                    let Some(event) = parsed.event else {
                        continue;
                    };
                    if let RunEvent::TextDelta(ref text) = event {
                        accumulated.push_str(text);
                    }
                    if matches!(event, RunEvent::Final(_)) {
                        saw_final = true;
                    }
                    if sender.send(Ok(event)).await.is_err() {
                        return Ok(false);
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    return Err(AppError::with_source(
                        ErrorCode::HarnessRunFailed,
                        "读取 zcode 输出失败",
                        error,
                    ));
                }
            }
        }
        Ok(true)
    })
    .await;

    match read_result {
        Ok(true) => {}
        Ok(false) => {
            terminate(&mut child).await;
            return Ok(());
        }
        Err(error) => {
            terminate(&mut child).await;
            return Err(error);
        }
    }

    let status = child.wait().await.map_err(|error| {
        AppError::with_source(ErrorCode::HarnessRunFailed, "等待 zcode 退出失败", error)
    })?;
    let stderr_text = stderr_task.await.unwrap_or_default();
    let stderr_text = stderr_text.trim();

    match status.code() {
        Some(0) => {
            if !saw_final && !accumulated.is_empty() {
                let _ = sender.send(Ok(RunEvent::Final(accumulated))).await;
            }
            Ok(())
        }
        code => {
            let message = if stderr_text.contains(SESSION_NOT_FOUND_MARKER) {
                "zcode 会话不存在（可能已被清理）：请在房间的会话面板重置该智能体的会话后重试"
                    .to_string()
            } else if stderr_text.is_empty() {
                format!("zcode 运行失败（退出码 {}）", code_label(code))
            } else {
                format!(
                    "zcode 运行失败（退出码 {}）：{}",
                    code_label(code),
                    tail(stderr_text)
                )
            };
            let _ = sender.send(Ok(RunEvent::Error(message.clone()))).await;
            Err(AppError::harness_run_failed(message))
        }
    }
}

fn code_label(code: Option<i32>) -> String {
    code.map(|value| value.to_string())
        .unwrap_or_else(|| "未知".to_string())
}

fn spawn_error(error: std::io::Error, harness: &str, searched: &[String]) -> AppError {
    let message = spawn_error_message(&error, harness, searched);
    AppError::with_source(ErrorCode::HarnessRunFailed, message, error)
}

/// 拼出可诊断的启动失败消息；有尝试来源时附在末尾。
fn spawn_error_message(error: &std::io::Error, harness: &str, searched: &[String]) -> String {
    let base = describe_spawn_failure(error, harness)
        .unwrap_or_else(|| format!("{harness} 启动失败：{error}"));
    if searched.is_empty() {
        base
    } else {
        format!("{base}（已尝试：{}）", searched.join("、"))
    }
}

async fn terminate(child: &mut tokio::process::Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

#[cfg(test)]
mod tests {
    use super::spawn_error_message;

    #[test]
    fn spawn_failure_lists_attempted_sources() {
        let error = std::io::Error::new(std::io::ErrorKind::NotFound, "program not found");
        let message = spawn_error_message(
            &error,
            "zcode",
            &[
                "protocol:zcode".to_string(),
                "app_paths:ZCode.exe".to_string(),
                "path:zcode".to_string(),
            ],
        );
        assert!(message.contains("未找到可执行的 zcode"), "{message}");
        assert!(
            message.contains("已尝试：protocol:zcode、app_paths:ZCode.exe、path:zcode"),
            "{message}"
        );
    }

    #[test]
    fn spawn_failure_without_sources_has_no_suffix() {
        let error = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let message = spawn_error_message(&error, "zcode", &[]);
        assert!(message.contains("没有权限执行 zcode"), "{message}");
        assert!(!message.contains("已尝试"), "{message}");
    }
}
