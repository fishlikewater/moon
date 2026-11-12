//! codex 无头适配器。
//!
//! CLI 事实（本机 codex-cli 0.130.0-alpha.5 实测）：
//! - 可执行文件在 `%LOCALAPPDATA%\OpenAI\Codex\bin\codex.exe`；
//! - `codex exec <prompt> --json`，续接用 `codex exec resume <SESSION_ID> <prompt> --json`；
//! - prompt 传 `-` 时从 stdin 读取，长度不受 Windows 命令行上限约束；
//! - stdout 为 NDJSON：`thread.started` 给出会话 ID，`item.*` 携带工具/推理/最终消息，
//!   `turn.completed` 携带用量，`turn.failed` 为失败；
//! - `exec resume` 不接受 `-s/--sandbox`，续接时必须改用 `-c sandbox_mode="..."`；
//! - 会话隔离使用独立 `CODEX_HOME`：复用时指向会话存储目录，否则使用临时目录，
//!   首次使用时从用户配置生成最小 `config.toml` 并复制 `auth.json`。

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::domain::agent::{HarnessKind, ModelConfig};
use crate::domain::room::RoomPermissionMode;
use crate::domain::run_event::{RunEvent, RunInput};
use crate::domain::session::RunSessionContext;
use crate::error::{AppError, AppResult, ErrorCode};

use super::types::{EventStream, HarnessAdapter};
use super::{describe_spawn_failure, tail, unique_temp_dir, Cleanup};

const SESSION_NOT_FOUND_MARKER: &str = "no rollout found";

pub struct ResolvedCodexCommand {
    pub command: String,
    pub args_prefix: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// 解析 codex 可执行文件：显式配置 → 环境变量 → Windows 默认安装 → PATH。
pub fn resolve_codex_command(configured_bin: Option<&str>) -> ResolvedCodexCommand {
    let explicit = normalize_bin(configured_bin)
        .or_else(|| normalize_bin(std::env::var("MOON_CODEX_BIN").ok().as_deref()));
    if let Some(bin) = explicit {
        return command_for(&bin);
    }
    if let Some(local_app_data) = std::env::var("LOCALAPPDATA")
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        let candidate = Path::new(&local_app_data)
            .join("OpenAI")
            .join("Codex")
            .join("bin")
            .join("codex.exe");
        if candidate.is_file() {
            return ResolvedCodexCommand {
                command: candidate.to_string_lossy().into_owned(),
                args_prefix: Vec::new(),
                env: Vec::new(),
            };
        }
    }
    ResolvedCodexCommand {
        command: "codex".to_string(),
        args_prefix: Vec::new(),
        env: Vec::new(),
    }
}

fn command_for(bin: &str) -> ResolvedCodexCommand {
    if is_node_script(bin) {
        let runner = std::env::var("MOON_CODEX_NODE_BIN")
            .ok()
            .and_then(|value| normalize_bin(Some(&value)))
            .unwrap_or_else(|| "node".to_string());
        return ResolvedCodexCommand {
            command: runner,
            args_prefix: vec![bin.to_string()],
            env: Vec::new(),
        };
    }
    ResolvedCodexCommand {
        command: bin.to_string(),
        args_prefix: Vec::new(),
        env: Vec::new(),
    }
}

pub struct CodexHomePlan {
    pub home: PathBuf,
    _cleanup: Cleanup,
}

/// 准备本次运行的 CODEX_HOME：复用会话用持久目录，否则用临时目录。
pub fn plan_codex_home(session: Option<&RunSessionContext>) -> AppResult<CodexHomePlan> {
    let (home, cleanup) = match session {
        Some(session) => {
            let home = Path::new(&session.storage_dir).join("codex-home");
            (home, Cleanup::none())
        }
        None => {
            let home = unique_temp_dir("moon-codex-home-").map_err(|error| {
                AppError::with_source(
                    ErrorCode::HarnessRunFailed,
                    "无法创建 Codex 临时目录",
                    error,
                )
            })?;
            (home.clone(), Cleanup::directory(home))
        }
    };

    if let Err(error) = ensure_codex_home(&home) {
        return Err(AppError::with_source(
            ErrorCode::HarnessRunFailed,
            "无法准备 Codex 会话目录",
            error,
        ));
    }
    Ok(CodexHomePlan { home, _cleanup: cleanup })
}

/// 准备会话 CODEX_HOME：生成最小配置并补齐认证文件。
fn ensure_codex_home(home: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(home)?;
    let config_path = home.join("config.toml");
    let source = user_codex_home();
    if !config_path.exists() {
        write_minimal_config(&source.join("config.toml"), &config_path)?;
    }
    copy_if_exists(&source.join("auth.json"), &home.join("auth.json"))
}

/// 只保留根级模型字段与 `[model_providers.*]` 段，丢弃桌面端专用配置。
fn write_minimal_config(source: &Path, target: &Path) -> std::io::Result<()> {
    let Ok(text) = std::fs::read_to_string(source) else {
        return std::fs::write(target, "");
    };
    let mut kept: Vec<String> = Vec::new();
    let mut section = String::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            section = trimmed
                .split(|value: char| value.is_whitespace() || value == '#')
                .next()
                .unwrap_or("")
                .to_string();
            if section == "[model_providers]" || section.starts_with("[model_providers.") {
                kept.push(line.to_string());
            }
            continue;
        }

        if section.is_empty() {
            if trimmed.starts_with("model_catalog_json") {
                continue;
            }
            let key = trimmed.split('=').next().unwrap_or("").trim();
            if trimmed.contains('=')
                && matches!(
                    key,
                    "model_provider" | "model" | "model_reasoning_effort"
                )
            {
                kept.push(line.to_string());
            }
            continue;
        }

        if section == "[model_providers]" || section.starts_with("[model_providers.") {
            kept.push(line.to_string());
        }
    }
    std::fs::write(target, format!("{}\n", kept.join("\n")))
}

fn user_codex_home() -> PathBuf {
    if let Some(value) = std::env::var("MOON_CODEX_USER_HOME")
        .ok()
        .and_then(|value| normalize_bin(Some(&value)))
    {
        return PathBuf::from(value);
    }
    if let Some(value) = std::env::var("CODEX_HOME")
        .ok()
        .and_then(|value| normalize_bin(Some(&value)))
    {
        return PathBuf::from(value);
    }
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    Path::new(&home).join(".codex")
}

/// 把房间权限模式映射为 codex 沙箱/审批参数。
///
/// `exec` 是非交互模式，审批策略必须为 never；`exec resume` 不接受 `-s`，
/// 复用会话时改用等价的 `-c sandbox_mode="..."`。未传模式保持 bypass。
pub fn codex_permission_args(mode: Option<RoomPermissionMode>, resume: bool) -> Vec<String> {
    let Some(mode) = mode else {
        return vec!["--dangerously-bypass-approvals-and-sandbox".to_string()];
    };
    let sandbox = |value: &str| -> Vec<String> {
        if resume {
            vec![
                "-c".to_string(),
                format!("sandbox_mode=\"{value}\""),
            ]
        } else {
            vec!["-s".to_string(), value.to_string()]
        }
    };
    match mode {
        RoomPermissionMode::Plan => sandbox("read-only"),
        RoomPermissionMode::Auto => {
            let mut args = sandbox("workspace-write");
            args.push("-c".to_string());
            args.push("approval_policy=\"never\"".to_string());
            args
        }
        RoomPermissionMode::Full => {
            vec!["--dangerously-bypass-approvals-and-sandbox".to_string()]
        }
    }
}

/// 模型与思考等级通过 CLI 参数覆盖；未声明时沿用 config.toml。
pub fn build_model_args(config: Option<&ModelConfig>) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(model) = config
        .and_then(|config| config.get("model"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        args.push("-m".to_string());
        args.push(model.to_string());
    }
    if let Some(level) = config
        .and_then(|config| config.get("reasoningLevel"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        args.push("-c".to_string());
        args.push(format!("model_reasoning_effort=\"{level}\""));
    }
    args
}

/// 纯函数：把一行 stdout 解析为运行事件；未知/非法输入返回 None。
pub fn parse_codex_line(line: &str) -> Option<RunEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let Value::Object(parsed) = serde_json::from_str::<Value>(trimmed).ok()? else {
        return None;
    };

    match parsed.get("type").and_then(Value::as_str)? {
        "thread.started" => read_string(parsed.get("thread_id")).map(RunEvent::Session),
        "item.started" | "item.completed" => {
            let phase = parsed.get("type").and_then(Value::as_str)?;
            parse_item_event(phase, parsed.get("item"))
        }
        "turn.completed" => parse_turn_completed(parsed.get("usage")),
        "turn.failed" => parsed
            .get("error")
            .and_then(Value::as_object)
            .and_then(|error| read_string(error.get("message")))
            .map(RunEvent::Error),
        "error" => read_string(parsed.get("message")).map(RunEvent::Status),
        _ => None,
    }
}

fn parse_item_event(phase: &str, raw_item: Option<&Value>) -> Option<RunEvent> {
    let item = raw_item?.as_object()?;
    let item_type = read_string(item.get("type"))?;
    let item_id = read_string(item.get("id")).unwrap_or_default();

    match item_type.as_str() {
        "agent_message" if phase == "item.completed" => {
            read_string(item.get("text")).map(RunEvent::Final)
        }
        "reasoning" if phase == "item.completed" => {
            read_string(item.get("text")).map(RunEvent::Reasoning)
        }
        "command_execution" => {
            if phase == "item.started" {
                return Some(RunEvent::ToolStart {
                    tool_call_id: item_id,
                    name: "shell".to_string(),
                    input: json!({ "command": item.get("command").cloned().unwrap_or(Value::Null) }),
                });
            }
            let exit_code = item.get("exit_code").and_then(Value::as_i64);
            Some(RunEvent::ToolEnd {
                tool_call_id: (!item_id.is_empty()).then_some(item_id),
                name: Some("shell".to_string()),
                ok: exit_code.is_none_or(|code| code == 0),
                detail: item
                    .get("aggregated_output")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            })
        }
        "mcp_tool_call" => {
            let server = read_string(item.get("server")).unwrap_or_else(|| "mcp".to_string());
            let tool = read_string(item.get("tool")).unwrap_or_else(|| "tool".to_string());
            let name = format!("{server}.{tool}");
            if phase == "item.started" {
                return Some(RunEvent::ToolStart {
                    tool_call_id: item_id,
                    name,
                    input: item.get("arguments").cloned().unwrap_or(Value::Null),
                });
            }
            let error_message = item
                .get("error")
                .and_then(Value::as_object)
                .and_then(|error| read_string(error.get("message")));
            Some(RunEvent::ToolEnd {
                tool_call_id: (!item_id.is_empty()).then_some(item_id),
                name: Some(name),
                ok: error_message.is_none(),
                detail: error_message.unwrap_or_else(|| describe_value(item.get("result"))),
            })
        }
        _ if phase != "item.completed" => None,
        "file_change" => Some(RunEvent::ToolEnd {
            tool_call_id: (!item_id.is_empty()).then_some(item_id),
            name: Some("file_change".to_string()),
            ok: true,
            detail: describe_file_changes(item.get("changes")),
        }),
        "web_search" => Some(RunEvent::ToolEnd {
            tool_call_id: (!item_id.is_empty()).then_some(item_id),
            name: Some("web_search".to_string()),
            ok: true,
            detail: read_string(item.get("query")).unwrap_or_default(),
        }),
        "error" => read_string(item.get("message")).map(RunEvent::Status),
        _ => None,
    }
}

fn parse_turn_completed(raw_usage: Option<&Value>) -> Option<RunEvent> {
    let usage = raw_usage?.as_object()?;
    let input = read_non_negative_int(usage.get("input_tokens"));
    let output = read_non_negative_int(usage.get("output_tokens"));
    let total = if input.is_none() && output.is_none() {
        None
    } else {
        Some(input.unwrap_or(0) + output.unwrap_or(0))
    };
    Some(RunEvent::Usage {
        token_count: total,
        context_size: None,
        tool_call_count: None,
        duration_ms: None,
    })
}

fn describe_file_changes(value: Option<&Value>) -> String {
    let Some(Value::Array(items)) = value else {
        return String::new();
    };
    let mut parts = Vec::new();
    for item in items {
        let Some(item) = item.as_object() else {
            continue;
        };
        let path = read_string(item.get("path")).unwrap_or_default();
        let kind = read_string(item.get("kind")).unwrap_or_default();
        if !path.is_empty() {
            parts.push(if kind.is_empty() {
                path
            } else {
                format!("{kind} {path}")
            });
        }
    }
    parts.join("; ")
}

fn describe_value(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        None | Some(Value::Null) => String::new(),
        Some(value) => serde_json::to_string(value).unwrap_or_default(),
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

fn normalize_bin(value: Option<&str>) -> Option<String> {
    let trimmed = value?.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn is_node_script(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".cjs") || lower.ends_with(".mjs") || lower.ends_with(".js")
}

fn copy_if_exists(from: &Path, to: &Path) -> std::io::Result<()> {
    if !from.is_file() {
        return Ok(());
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(from, to).map(|_| ())
}

pub struct CodexAdapter;

impl HarnessAdapter for CodexAdapter {
    fn kind(&self) -> HarnessKind {
        HarnessKind::Codex
    }

    fn run(&self, input: RunInput) -> EventStream {
        let (sender, receiver) = mpsc::channel(128);
        tokio::spawn(async move {
            if let Err(error) = run_codex(input, sender.clone()).await {
                let _ = sender.send(Err(error)).await;
            }
        });
        Box::pin(tokio_stream::wrappers::ReceiverStream::new(receiver))
    }
}

async fn run_codex(
    input: RunInput,
    sender: mpsc::Sender<AppResult<RunEvent>>,
) -> AppResult<()> {
    let home_plan = plan_codex_home(input.session.as_ref())?;
    let resolved = resolve_codex_command(
        input
            .config
            .as_ref()
            .and_then(|config| config.get("codexBin"))
            .and_then(Value::as_str),
    );
    let resume_session_id = input
        .session
        .as_ref()
        .and_then(|session| session.harness_session_id.clone());
    let resume = resume_session_id.is_some();

    let mut args = resolved.args_prefix.clone();
    args.push("exec".to_string());
    if let Some(session_id) = &resume_session_id {
        args.push("resume".to_string());
        args.push(session_id.clone());
    }
    args.push("-".to_string());
    args.push("--json".to_string());
    args.push("--skip-git-repo-check".to_string());
    args.extend(codex_permission_args(input.permission_mode, resume));
    if input.session.is_none() {
        args.push("--ephemeral".to_string());
    }
    args.extend(build_model_args(input.config.as_ref()));

    let mut command = Command::new(&resolved.command);
    command
        .args(&args)
        .current_dir(&input.workspace_dir)
        .env("CODEX_HOME", &home_plan.home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in &resolved.env {
        command.env(key, value);
    }

    let mut child = command.spawn().map_err(|error| spawn_error(error, "codex"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| AppError::harness_run_failed("无法写入 codex 提示词"))?;
    // 提示词走 stdin，长度不受 Windows 命令行上限约束。
    stdin
        .write_all(input.prompt.as_bytes())
        .await
        .map_err(|error| {
            AppError::with_source(ErrorCode::HarnessRunFailed, "写入 codex 提示词失败", error)
        })?;
    drop(stdin);

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::harness_run_failed("无法读取 codex 输出"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| AppError::harness_run_failed("无法读取 codex 错误输出"))?;
    let stderr_task = tokio::spawn(async move {
        let mut buffer = String::new();
        let _ = stderr.read_to_string(&mut buffer).await;
        buffer
    });

    let mut lines = BufReader::new(stdout).lines();

    let read_result = super::with_run_timeout(input.timeout, async {
        let mut reported_session = false;
        let mut failure: Option<String> = None;

        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    let Some(event) = parse_codex_line(&line) else {
                        continue;
                    };
                    if let RunEvent::Session(_) = &event {
                        if reported_session {
                            continue;
                        }
                        reported_session = true;
                    }
                    if let RunEvent::Error(message) = &event {
                        failure = Some(message.clone());
                    }
                    let is_error = failure.is_some();
                    if sender.send(Ok(event)).await.is_err() {
                        return Ok(false);
                    }
                    if is_error {
                        return Err(AppError::harness_run_failed(
                            failure.unwrap_or_default(),
                        ));
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    return Err(AppError::with_source(
                        ErrorCode::HarnessRunFailed,
                        "读取 codex 输出失败",
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
        AppError::with_source(ErrorCode::HarnessRunFailed, "等待 codex 退出失败", error)
    })?;
    let stderr_text = stderr_task.await.unwrap_or_default();
    let stderr_text = stderr_text.trim();

    if status.code().unwrap_or(1) != 0 {
        let message = if stderr_text.contains(SESSION_NOT_FOUND_MARKER) {
            "Codex 会话不存在（可能已被清理）：请在房间的会话面板重置该智能体的会话后重试"
                .to_string()
        } else if stderr_text.is_empty() {
            format!(
                "codex 运行失败（退出码 {}）",
                code_label(status.code())
            )
        } else {
            format!(
                "codex 运行失败（退出码 {}）：{}",
                code_label(status.code()),
                tail(stderr_text)
            )
        };
        let _ = sender.send(Ok(RunEvent::Error(message.clone()))).await;
        return Err(AppError::harness_run_failed(message));
    }
    Ok(())
}

fn code_label(code: Option<i32>) -> String {
    code.map(|value| value.to_string())
        .unwrap_or_else(|| "未知".to_string())
}

fn spawn_error(error: std::io::Error, harness: &str) -> AppError {
    let message = describe_spawn_failure(&error, harness)
        .unwrap_or_else(|| format!("{harness} 启动失败：{error}"));
    AppError::with_source(ErrorCode::HarnessRunFailed, message, error)
}

async fn terminate(child: &mut tokio::process::Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}
