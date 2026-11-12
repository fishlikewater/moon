//! zcode app-server（ZCode Protocol）传输。
//!
//! 与 CLI `--prompt` 的差异：
//! - 通过 NDJSON 双向协议创建/恢复会话，`session/create` 支持
//!   `additionalDirectories`（房间的附加工作目录）；
//! - 事件由 `session/subscribe` 后以 `session/event` 推送，`payload.type`
//!   与 CLI stream-json 的 type 相同，复用 `parse_zcode_event`；
//! - 服务端会反向请求运行时偏好与 MCP 授权，必须在读取循环中应答，
//!   否则会话创建会一直挂起。

use std::process::Stdio;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc;

use crate::domain::room::RoomPermissionMode;
use crate::domain::run_event::RunEvent;
use crate::error::{AppError, AppResult};

use super::zcode::parse_zcode_event;
use super::zcode_config::ZcodeModelSelection;

pub(crate) struct AppServerLaunch {
    pub program: String,
    pub args_prefix: Vec<String>,
    pub env: Vec<(String, String)>,
    pub workspace_dir: String,
    pub additional_directories: Vec<String>,
    pub permission_mode: Option<RoomPermissionMode>,
    pub prompt: String,
    pub resume_session_id: Option<String>,
    /// 智能体选择的模型；app-server 流程必须经 `session/setModel` 应用，
    /// 个人配置文件覆盖对新版 CLI 无效。
    pub model_selection: Option<ZcodeModelSelection>,
    /// 本次运行的总时长上限；None 表示不限制。
    pub timeout: Option<std::time::Duration>,
}

struct ProtocolError {
    code: i64,
    message: String,
}

impl ProtocolError {
    fn into_app_error(self, context: &str) -> AppError {
        AppError::harness_run_failed(format!("{context}：{}", self.message))
    }
}

pub(crate) async fn run(
    launch: AppServerLaunch,
    sender: mpsc::Sender<AppResult<RunEvent>>,
) -> AppResult<()> {
    let mut command = Command::new(&launch.program);
    command
        .args(&launch.args_prefix)
        .arg("app-server")
        .current_dir(&launch.workspace_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in &launch.env {
        command.env(key, value);
    }

    let mut child = command.spawn().map_err(|error| {
        AppError::harness_run_failed(format!("无法启动 zcode app-server：{error}"))
    })?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| AppError::harness_run_failed("无法写入 zcode app-server"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::harness_run_failed("无法读取 zcode app-server 输出"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| AppError::harness_run_failed("无法读取 zcode app-server 错误输出"))?;
    let mut stderr_task = tokio::spawn(async move {
        let mut buffer = String::new();
        let _ = stderr.read_to_string(&mut buffer).await;
        buffer
    });
    let mut lines = BufReader::new(stdout).lines();

    let result = super::with_run_timeout(
        launch.timeout,
        run_session(&launch, &mut stdin, &mut lines, &sender),
    )
    .await;
    terminate_process_tree(&mut child).await;
    let stderr_text = match tokio::time::timeout(Duration::from_millis(800), &mut stderr_task).await {
        Ok(Ok(text)) => text,
        _ => {
            // MCP 子进程可能继承并长期持有 stderr；不能因此阻塞本次运行收尾。
            stderr_task.abort();
            String::new()
        }
    };

    match result {
        Err(error) => {
            let tail = stderr_text.trim();
            if tail.is_empty() {
                Err(error)
            } else {
                let mut message = error.message;
                message.push('：');
                message.push_str(&tail[tail.len().saturating_sub(800)..]);
                Err(AppError::new(error.code, message))
            }
        }
        ok => ok,
    }
}

/// 结束 app-server 及其 MCP/插件子进程；只杀父进程会留下孤儿进程。
async fn terminate_process_tree(child: &mut Child) {
    #[cfg(windows)]
    if let Some(pid) = child.id() {
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(0x0800_0000)
                .status(),
        )
        .await;
    }
    let _ = tokio::time::timeout(Duration::from_secs(2), child.kill()).await;
}

async fn run_session(
    launch: &AppServerLaunch,
    stdin: &mut ChildStdin,
    lines: &mut Lines<BufReader<ChildStdout>>,
    sender: &mpsc::Sender<AppResult<RunEvent>>,
) -> AppResult<()> {
    let mut request_id = 0_i64;
    let session_id = match launch.resume_session_id.as_deref() {
        Some(existing) => {
            request_id += 1;
            match request(
                stdin,
                lines,
                request_id,
                "session/resume",
                json!({ "sessionId": existing }),
            )
            .await
            {
                Ok(value) => session_id_from(&value).unwrap_or_else(|| existing.to_string()),
                Err(error)
                    if error.code == -32004 || error.message.contains("Session not found") =>
                {
                    request_id += 1;
                    create_session(launch, stdin, lines, request_id).await?
                }
                Err(error) => {
                    return Err(error.into_app_error("恢复 zcode 会话失败"));
                }
            }
        }
        None => {
            request_id += 1;
            create_session(launch, stdin, lines, request_id).await?
        }
    };

    send_event(sender, RunEvent::Session(session_id.clone())).await?;

    request_id += 1;
    request(
        stdin,
        lines,
        request_id,
        "session/subscribe",
        json!({
            "sessionId": session_id,
            "deliveryKind": "desktop-continuous",
        }),
    )
    .await
    .map_err(|error| error.into_app_error("订阅 zcode 会话事件失败"))?;

    // 模型选择必须在发送消息前应用；否则 CLI 回落默认 provider，
    // 用户在界面上选的模型不生效。
    if let Some(selection) = &launch.model_selection {
        // registry 在 setModel 时一并校验思考等级，等级必须内联在 options。
        let mut model = json!({
            "providerId": selection.provider_id,
            "modelId": selection.model_id,
        });
        if let Some(level) = &selection.reasoning_level {
            model["options"] = json!({ "reasoningLevel": level });
        }
        request_id += 1;
        request(
            stdin,
            lines,
            request_id,
            "session/setModel",
            json!({
                "sessionId": session_id,
                "model": model,
            }),
        )
        .await
        .map_err(|error| error.into_app_error("设置 zcode 会话模型失败"))?;
    }

    request_id += 1;
    request(
        stdin,
        lines,
        request_id,
        "session/send",
        json!({ "sessionId": session_id, "content": launch.prompt }),
    )
    .await
    .map_err(|error| error.into_app_error("发送 zcode 消息失败"))?;

    let mut saw_stream_text = false;

    let final_text: Option<String> = loop {
        let Some(message) = next_message(lines).await? else {
            return Err(AppError::harness_run_failed(
                "zcode app-server 在回复完成前退出",
            ));
        };
        if is_server_request(&message) {
            respond_to_server_request(stdin, &message).await?;
            continue;
        }

        match message.get("method").and_then(Value::as_str) {
            Some("session/event") => {
                let Some(params) = message.get("params").and_then(Value::as_object) else {
                    continue;
                };
                let Some(payload) = params.get("payload").and_then(Value::as_object) else {
                    continue;
                };
                // 真实 app-server 把事件类型放在 params.type；CLI stream-json
                // 历史形态则放在 payload.type。两者都接受。
                let mut event = params.clone();
                if !event.contains_key("type") {
                    if let Some(kind) = payload.get("type").cloned() {
                        event.insert("type".to_string(), kind);
                    }
                }
                match event.get("type").and_then(Value::as_str) {
                    Some("model.streaming") => {
                        if payload.get("kind").and_then(Value::as_str) == Some("text_delta") {
                            saw_stream_text = true;
                        }
                        if let Some(run_event) = parse_zcode_event(&event, payload) {
                            send_event(sender, run_event).await?;
                        }
                    }
                    Some("tool.updated") => {
                        if let Some(run_event) = parse_zcode_event(&event, payload) {
                            send_event(sender, run_event).await?;
                        }
                    }
                    Some("part.delta") => {
                        // 没有 model.streaming 文本时，用消息部件增量兜底。
                        if !saw_stream_text
                            && payload.get("field").and_then(Value::as_str) == Some("text")
                        {
                            if let Some(delta) =
                                payload.get("delta").and_then(Value::as_str).filter(|text| !text.is_empty())
                            {
                                send_event(sender, RunEvent::TextDelta(delta.to_string())).await?;
                            }
                        }
                    }
                    Some("turn.completed") => {
                        send_event(
                            sender,
                            RunEvent::Usage {
                                token_count: non_negative_int(payload.get("tokenCount")),
                                context_size: None,
                                tool_call_count: non_negative_int(payload.get("toolCallCount")),
                                duration_ms: non_negative_int(payload.get("duration")),
                            },
                        )
                        .await?;
                        break payload
                            .get("response")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                    }
                    Some("turn.failed") => {
                        let message = payload
                            .get("error")
                            .and_then(Value::as_object)
                            .and_then(|error| error.get("message"))
                            .and_then(Value::as_str)
                            .unwrap_or("zcode 运行失败");
                        return Err(AppError::harness_run_failed(message.to_string()));
                    }
                    _ => {}
                }
            }
            Some("session/lifecycle/changed") => {
                let params = message.get("params");
                if params.and_then(|value| value.get("lastError")).is_some()
                    && params.and_then(|value| value.get("state")).and_then(Value::as_str)
                        == Some("unloaded")
                {
                    let error = params
                        .and_then(|value| value.get("lastError"))
                        .and_then(|value| value.get("message"))
                        .and_then(Value::as_str)
                        .unwrap_or("zcode 会话加载失败");
                    return Err(AppError::harness_run_failed(error.to_string()));
                }
            }
            _ => {}
        }
    };

    if let Some(text) = final_text.filter(|text| !text.is_empty()) {
        send_event(sender, RunEvent::Final(text)).await?;
    }
    Ok(())
}

async fn create_session(
    launch: &AppServerLaunch,
    stdin: &mut ChildStdin,
    lines: &mut Lines<BufReader<ChildStdout>>,
    request_id: i64,
) -> AppResult<String> {
    let params = json!({
        "workspace": {
            "workspacePath": launch.workspace_dir,
            "workspaceKey": launch.workspace_dir,
        },
        "additionalDirectories": launch.additional_directories,
        "mode": permission_mode_str(launch.permission_mode),
    });
    let value = request(stdin, lines, request_id, "session/create", params)
        .await
        .map_err(|error| error.into_app_error("创建 zcode 会话失败"))?;
    session_id_from(&value)
        .ok_or_else(|| AppError::harness_run_failed("zcode 会话未返回 sessionId"))
}

async fn request(
    stdin: &mut ChildStdin,
    lines: &mut Lines<BufReader<ChildStdout>>,
    id: i64,
    method: &str,
    params: Value,
) -> Result<Value, ProtocolError> {
    write_message(
        stdin,
        &json!({ "id": id, "method": method, "params": params }),
    )
    .await
    .map_err(|error| ProtocolError {
        code: -1,
        message: error.message,
    })?;

    loop {
        let message = match next_message(lines).await {
            Ok(Some(message)) => message,
            Ok(None) => {
                return Err(ProtocolError {
                    code: -1,
                    message: "zcode app-server 在响应前退出".to_string(),
                })
            }
            Err(error) => {
                return Err(ProtocolError {
                    code: -1,
                    message: error.message,
                })
            }
        };
        if is_server_request(&message) {
            respond_to_server_request(stdin, &message)
                .await
                .map_err(|error| ProtocolError {
                    code: -1,
                    message: error.message,
                })?;
            continue;
        }
        if message.get("id").and_then(Value::as_i64) != Some(id) {
            continue;
        }
        if let Some(error) = message.get("error") {
            return Err(ProtocolError {
                code: error.get("code").and_then(Value::as_i64).unwrap_or(-1),
                message: error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("zcode 协议请求失败")
                    .to_string(),
            });
        }
        return Ok(message.get("result").cloned().unwrap_or(Value::Null));
    }
}

async fn respond_to_server_request(
    stdin: &mut ChildStdin,
    message: &Value,
) -> AppResult<()> {
    let Some(id) = message.get("id") else {
        return Ok(());
    };
    let result = match message.get("method").and_then(Value::as_str) {
        Some("session/requestRuntimePreferences") => {
            json!({ "nativeSearchEnhancementsEnabled": false })
        }
        Some("interaction/requestPermission") => {
            // 无头运行没有人工审批入口；明确拒绝，危险操作不会静默放行。
            json!({ "approved": false, "allowed": false })
        }
        Some("interaction/requestUserInput") => json!({ "cancelled": true }),
        _ => json!({}),
    };
    tracing::debug!(
        method = message.get("method").and_then(|value| value.as_str()).unwrap_or(""),
        id = ?id,
        "respond to zcode app-server request"
    );
    write_message(stdin, &json!({ "id": id, "result": result })).await
}

async fn write_message(stdin: &mut ChildStdin, message: &Value) -> AppResult<()> {
    let mut line = serde_json::to_string(message)
        .map_err(|error| AppError::with_source(
            crate::error::ErrorCode::InternalError,
            "无法序列化 zcode 协议消息",
            error,
        ))?;
    line.push('\n');
    stdin.write_all(line.as_bytes()).await.map_err(|error| {
        AppError::with_source(
            crate::error::ErrorCode::HarnessRunFailed,
            "写入 zcode app-server 失败",
            error,
        )
    })?;
    stdin.flush().await.map_err(|error| {
        AppError::with_source(
            crate::error::ErrorCode::HarnessRunFailed,
            "刷新 zcode app-server 输入失败",
            error,
        )
    })?;
    Ok(())
}

async fn next_message(
    lines: &mut Lines<BufReader<ChildStdout>>,
) -> AppResult<Option<Value>> {
    loop {
        let Some(line) = lines.next_line().await.map_err(|error| {
            AppError::with_source(
                crate::error::ErrorCode::HarnessRunFailed,
                "读取 zcode app-server 输出失败",
                error,
            )
        })? else {
            return Ok(None);
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(trimmed) {
            Ok(message) => {
                tracing::debug!(
                    method = message.get("method").and_then(|value| value.as_str()).unwrap_or(""),
                    id = ?message.get("id"),
                    payload_type = message
                        .pointer("/params/payload/type")
                        .and_then(|value| value.as_str())
                        .unwrap_or(""),
                    "zcode app-server message"
                );
                return Ok(Some(message));
            }
            Err(_) => continue,
        }
    }
}

fn is_server_request(message: &Value) -> bool {
    message.get("method").is_some()
        && message.get("id").is_some()
        && message.get("result").is_none()
        && message.get("error").is_none()
}

async fn send_event(
    sender: &mpsc::Sender<AppResult<RunEvent>>,
    event: RunEvent,
) -> AppResult<()> {
    sender
        .send(Ok(event))
        .await
        .map_err(|_| AppError::harness_run_failed("zcode 事件接收端已关闭"))
}

fn session_id_from(value: &Value) -> Option<String> {
    value
        .get("session")
        .and_then(|session| session.get("sessionId"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .or_else(|| {
            value
                .get("sessionId")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
        })
}

fn permission_mode_str(mode: Option<RoomPermissionMode>) -> &'static str {
    match mode {
        Some(RoomPermissionMode::Plan) => "plan",
        Some(RoomPermissionMode::Auto) => "guarded",
        Some(RoomPermissionMode::Full) | None => "yolo",
    }
}

fn non_negative_int(value: Option<&Value>) -> Option<i64> {
    value
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite() && *number >= 0.0)
        .map(|number| number as i64)
}
