//! ACP（Agent Client Protocol）适配器。
//!
//! Moon 作为 ACP client 与用户在注册表中登记的本地 ACP agent 进程通信，
//! 协议层经由官方 `agent-client-protocol` crate（v1）：
//!
//! - 进程与传输：Moon 负责按注册表定义 spawn agent 进程（cwd/env、进程树
//!   终止、stderr 收集），把 stdio 交给 SDK 的 `ByteStreams`，由它承担
//!   newline-delimited JSON-RPC 的编解码、请求关联与方法分发；
//! - 握手：initialize → session/new（或 session/load 续接既有会话）；
//! - 运行：`session/prompt` 期间把 SDK 的 `SessionNotification` 映射为
//!   RunEvent 流；
//! - 反向请求：应答 `fs/read_text_file` / `fs/write_text_file`（路径必须在
//!   工作区白名单内）与 `session/request_permission`（第一批自动放行）；
//!   其余反向能力（terminal、elicitation 等）未注册处理器，由 SDK 回退为
//!   method-not-found。
//!
//! 与 zcode app-server 传输同构：每次运行 spawn 一个 agent 进程，运行结束
//! 终止整个进程树；文本经 stdin JSON 传递，不受 Windows 命令行长度限制。

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_client_protocol::v1::schema::{
    ClientCapabilities, ContentBlock, FileSystemCapabilities, InitializeRequest,
    LoadSessionRequest, NewSessionRequest, PermissionOptionKind, PromptRequest,
    ReadTextFileRequest, ReadTextFileResponse, RequestPermissionOutcome,
    RequestPermissionRequest, RequestPermissionResponse, SelectedPermissionOutcome,
    SessionConfigId, SessionConfigOptionValue, SessionId, SessionModeId,
    SessionNotification, SessionUpdate, SetSessionConfigOptionRequest, SetSessionModeRequest,
    StopReason, TextContent, ToolCallStatus, WriteTextFileRequest, WriteTextFileResponse,
};
use agent_client_protocol::v1::ConnectionTo;
use agent_client_protocol::{
    Agent, ByteStreams, Client, Error as SdkError, ErrorCode, UntypedMessage,
};
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

/// 裸命令（如 `npx`）按注册表 PATH + PATHEXT 解析；带路径分隔符的命令原样使用。
/// 返回实际程序与来源，供启动失败时诊断。
fn resolved_program(command: &str) -> (String, &'static str) {
    match super::locate::resolve_bare_command(command) {
        Some(resolved) => (resolved, "PATH"),
        None => (command.to_string(), "原样命令"),
    }
}

/// Windows `CREATE_NO_WINDOW`：GUI 进程 spawn 控制台程序时不创建可见窗口。
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

use crate::domain::agent::HarnessKind;
use crate::domain::harness::{
    AcpCapabilities, AcpConfigChoice, AcpConfigOption, AcpMode, AcpModelOption, CustomHarness,
};
use crate::domain::room::RoomPermissionMode;
use crate::domain::run_event::{RunEvent, RunInput};
use crate::error::{AppError, AppResult};
use crate::store::CustomHarnessStore;

use super::types::EventStream;

/// 每次运行独立 spawn 的 ACP 适配器；定义在运行时从注册表读取。
pub struct AcpAdapter {
    store: CustomHarnessStore,
    custom_harness_id: Option<i64>,
}

impl AcpAdapter {
    pub fn new(store: CustomHarnessStore, custom_harness_id: Option<i64>) -> Self {
        Self {
            store,
            custom_harness_id,
        }
    }
}

impl crate::adapters::HarnessAdapter for AcpAdapter {
    fn kind(&self) -> HarnessKind {
        HarnessKind::Acp
    }

    fn run(&self, input: RunInput) -> EventStream {
        let (sender, receiver) = mpsc::channel(128);
        let store = self.store.clone();
        let custom_harness_id = self.custom_harness_id;
        tokio::spawn(async move {
            let result = match custom_harness_id {
                Some(id) => match store.get_by_id(id).await {
                    Ok(Some(harness)) => run_with_harness(&harness, input, &sender).await,
                    Ok(None) => Err(AppError::harness_run_failed(format!(
                        "自定义 ACP agent 不存在：{id}"
                    ))),
                    Err(error) => Err(error),
                },
                None => Err(AppError::validation(
                    "智能体未绑定自定义 ACP agent，请编辑智能体后重新选择",
                )),
            };
            if let Err(error) = result {
                let _ = sender.send(Err(error)).await;
            }
        });
        Box::pin(ReceiverStream::new(receiver))
    }
}

/// 一次 ACP 运行的完整流程：spawn → 握手 → prompt → 映射 → 收尾。
async fn run_with_harness(
    harness: &CustomHarness,
    input: RunInput,
    sender: &mpsc::Sender<AppResult<RunEvent>>,
) -> AppResult<()> {
    let (program, source) = resolved_program(&harness.command);
    let mut command = Command::new(&program);
    command
        .args(&harness.args)
        .current_dir(&input.workspace_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    for (key, value) in &harness.env {
        command.env(key, value);
    }

    let mut child = command.spawn().map_err(|error| {
        AppError::harness_run_failed(format!(
            "无法启动自定义 ACP agent「{}」（程序 {program}，来源 {source}）：{error}",
            harness.name
        ))
    })?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| AppError::harness_run_failed("无法写入自定义 ACP agent 的标准输入"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::harness_run_failed("无法读取自定义 ACP agent 的标准输出"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| AppError::harness_run_failed("无法读取自定义 ACP agent 的错误输出"))?;
    let mut stderr_task = tokio::spawn(async move {
        let mut buffer = String::new();
        let _ = stderr.read_to_string(&mut buffer).await;
        buffer
    });

    let scope = Arc::new(WorkspaceScope::new(
        &input.workspace_dir,
        &input.additional_directories,
    ));
    let result = super::with_run_timeout(
        input.timeout,
        run_protocol(&input, scope, stdin, stdout, sender),
    )
    .await;

    terminate_process_tree(&mut child).await;
    let stderr_text = match tokio::time::timeout(
        std::time::Duration::from_millis(800),
        &mut stderr_task,
    )
    .await
    {
        Ok(Ok(text)) => text,
        _ => {
            stderr_task.abort();
            String::new()
        }
    };

    match result {
        Err(error) => {
            // 字符安全截断；与 codex/zcode 共用同一份实现。
            let tail = super::tail(&stderr_text);
            let tail = tail.trim();
            if tail.is_empty() {
                Err(error)
            } else {
                let message = format!("{}：{}", error.message, tail);
                Err(AppError::new(error.code, message))
            }
        }
        ok => ok,
    }
}

/// 探测 ACP agent 能力：initialize + session/new，不发 prompt；
/// 无论成功、失败或超时，都终止整个进程树。
///
/// 探测阶段不注册反向请求处理器；agent 在探测期间发起的 fs/权限请求由
/// SDK 回退 method-not-found（与 terminal/elicitation 同路径）。
pub async fn probe_capabilities(
    harness: &CustomHarness,
    workspace_dir: &str,
    timeout: Duration,
) -> AppResult<AcpCapabilities> {
    let (program, source) = resolved_program(&harness.command);
    let mut command = Command::new(&program);
    command
        .args(&harness.args)
        .current_dir(workspace_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    for (key, value) in &harness.env {
        command.env(key, value);
    }

    let mut child = command.spawn().map_err(|error| {
        AppError::harness_run_failed(format!(
            "无法启动自定义 ACP agent「{}」（程序 {program}，来源 {source}）：{error}",
            harness.name
        ))
    })?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| AppError::harness_run_failed("无法写入自定义 ACP agent 的标准输入"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::harness_run_failed("无法读取自定义 ACP agent 的标准输出"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| AppError::harness_run_failed("无法读取自定义 ACP agent 的错误输出"))?;
    let mut stderr_task = tokio::spawn(async move {
        let mut buffer = String::new();
        let _ = stderr.read_to_string(&mut buffer).await;
        buffer
    });

    let outcome = tokio::time::timeout(timeout, async {
        let new_session = UntypedMessage::new(
            "session/new",
            NewSessionRequest::new(PathBuf::from(workspace_dir)).mcp_servers(Vec::new()),
        )
        .map_err(|error| protocol_error("创建 ACP 探测会话失败", error))?;
        Client
            .v1()
            .connect_with(
                ByteStreams::new(stdin.compat_write(), stdout.compat()),
                |connection: ConnectionTo<Agent>| async move {
                    let _ = connection
                        .send_request(initialize_request())
                        .block_task()
                        .await
                        .map_err(|error| protocol_error("ACP initialize 失败", error))?;

                    let value = connection
                        .send_request(new_session)
                        .block_task()
                        .await
                        .map_err(|error| protocol_error("创建 ACP 探测会话失败", error))?;
                    value
                        .get("sessionId")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            SdkError::new(ErrorCode::INTERNAL_ERROR, "ACP agent 未返回 sessionId")
                        })?;
                    Ok::<_, SdkError>(parse_capabilities(&value))
                },
            )
            .await
    })
    .await;

    terminate_process_tree(&mut child).await;
    let stderr_text = match tokio::time::timeout(Duration::from_millis(800), &mut stderr_task).await {
        Ok(Ok(text)) => text,
        _ => {
            stderr_task.abort();
            String::new()
        }
    };

    match outcome {
        Err(_) => Err(AppError::harness_run_failed(format!(
            "ACP 能力探测超过 {} 秒未完成，已终止",
            timeout.as_secs()
        ))),
        Ok(Err(error)) => {
            let tail = super::tail(&stderr_text);
            let tail = tail.trim();
            let message = error.message;
            if tail.is_empty() {
                Err(AppError::harness_run_failed(message))
            } else {
                Err(AppError::harness_run_failed(format!(
                    "{message}：{tail}"
                )))
            }
        }
        Ok(Ok(capabilities)) => Ok(capabilities),
    }
}

/// 通过 SDK 运行一次完整会话：注册通知与反向请求处理器，随后在
/// `connect_with` 的主流程里完成握手、偏好设置与 prompt。
async fn run_protocol(
    input: &RunInput,
    scope: Arc<WorkspaceScope>,
    stdin: ChildStdin,
    stdout: ChildStdout,
    sender: &mpsc::Sender<AppResult<RunEvent>>,
) -> AppResult<()> {
    // prompt 期间的文本增量在通知处理器里累积，prompt 响应返回后拼装 Final。
    let final_text = Arc::new(Mutex::new(String::new()));

    let notify_sender = sender.clone();
    let notify_final = Arc::clone(&final_text);
    let read_scope = Arc::clone(&scope);
    let write_scope = Arc::clone(&scope);

    let outcome = Client
        .v1()
        .on_receive_notification(
            async move |notification: SessionNotification, _cx| {
                // 已声明差异：旧实现发送失败即终止运行；SDK 对无请求目标的
                // 处理器错误只记 warn，agent 会继续跑到 turn 结束或运行超时。
                // 消费端提前退出（如运行详情中途关闭）依赖总时长上限兜底。
                if let Some(event) = map_update(notification.update) {
                    if let RunEvent::TextDelta(delta) = &event {
                        let mut guard = lock_final(&notify_final);
                        guard.push_str(delta);
                    }
                    send_event(&notify_sender, event).await.map_err(sdk_internal)?;
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: ReadTextFileRequest, responder, _connection| {
                match read_workspace_file(&read_scope, &request.path.to_string_lossy()).await {
                    Ok(content) => responder.respond(ReadTextFileResponse::new(content))?,
                    Err(error) => responder.respond_with_error(SdkError::new(
                        ErrorCode::INVALID_PARAMS,
                        error.message,
                    ))?,
                }
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: WriteTextFileRequest, responder, _connection| {
                match write_workspace_file(
                    &write_scope,
                    &request.path.to_string_lossy(),
                    &request.content,
                )
                .await
                {
                    Ok(()) => responder.respond(WriteTextFileResponse::new())?,
                    Err(error) => responder.respond_with_error(SdkError::new(
                        ErrorCode::INVALID_PARAMS,
                        error.message,
                    ))?,
                }
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: RequestPermissionRequest, responder, _connection| {
                // 第一批限制：无人工审批入口，自动选择第一个 allow 选项；
                // agent 未提供 allow 选项时取消，不静默允许。
                let outcome = request
                    .options
                    .iter()
                    .find(|option| {
                        matches!(
                            option.kind,
                            PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways
                        )
                    })
                    .map(|option| RequestPermissionOutcome::Selected(
                        SelectedPermissionOutcome::new(option.option_id.clone()),
                    ))
                    .unwrap_or(RequestPermissionOutcome::Cancelled);
                responder.respond(RequestPermissionResponse::new(outcome))?;
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(
            ByteStreams::new(stdin.compat_write(), stdout.compat()),
            |connection: ConnectionTo<Agent>| async move {
                run_session(connection, input, sender, &final_text).await
            },
        )
        .await;

    outcome.map_err(|error| AppError::harness_run_failed(error.message))
}

/// `connect_with` 主流程：initialize → 会话 → 偏好 → prompt。
async fn run_session(
    connection: ConnectionTo<Agent>,
    input: &RunInput,
    sender: &mpsc::Sender<AppResult<RunEvent>>,
    final_text: &Arc<Mutex<String>>,
) -> Result<(), SdkError> {
    // 1. initialize：宣告客户端能力；fs 反向读写按需开启。
    let init_response = connection
        .send_request(initialize_request())
        .block_task()
        .await
        .map_err(|error| protocol_error("ACP initialize 失败", error))?;
    let load_session = init_response.agent_capabilities.load_session;

    // 2. 会话：优先续接；load 失败回退新建。响应体携带 agent 自报的能力快照。
    let (session_id, capabilities) = match input
        .session
        .as_ref()
        .and_then(|session| session.harness_session_id.clone())
    {
        Some(existing) if load_session => {
            match load_session_message(&connection, &existing, input).await {
                Ok(value) => (existing, parse_capabilities(&value)),
                Err(_) => create_session(&connection, input).await?,
            }
        }
        _ => create_session(&connection, input).await?,
    };

    send_event(sender, RunEvent::Session(session_id.clone()))
        .await
        .map_err(sdk_internal)?;

    // 3. 偏好：权限模式与模型是 best-effort 设置，失败只提示，不中断运行。
    apply_session_preferences(input, &session_id, &capabilities, &connection, sender).await?;

    // 4. prompt：响应只在 turn 结束后返回；期间以通知与反向请求为主。
    //    agent 返回的错误信息保持原样透传，与既有行为一致。
    let prompt_response = connection
        .send_request(PromptRequest::new(
            SessionId::from(session_id.clone()),
            vec![ContentBlock::Text(TextContent::new(input.prompt.clone()))],
        ))
        .block_task()
        .await?;

    let stop_reason = stop_reason_text(prompt_response.stop_reason);
    let streamed = lock_final(final_text).clone();
    if stop_reason != "end_turn" && streamed.is_empty() {
        return Err(SdkError::new(
            ErrorCode::INTERNAL_ERROR,
            format!("ACP agent 提前停止（{stop_reason}），未产生回复"),
        ));
    }
    if !streamed.is_empty() {
        let text = if stop_reason == "end_turn" {
            streamed
        } else {
            format!("{streamed}\n\n（ACP agent 停止原因：{stop_reason}）")
        };
        send_event(sender, RunEvent::Final(text))
            .await
            .map_err(sdk_internal)?;
    }

    // 运行成功即视为 agent 正常退出；child 的进程树终止由调用方收尾完成。
    Ok(())
}

/// 构造 initialize 请求：宣告客户端能力，fs 反向读写按需开启。
fn initialize_request() -> InitializeRequest {
    InitializeRequest::new(agent_client_protocol::schema::ProtocolVersion::V1)
        .client_capabilities(
            ClientCapabilities::new().fs(
                FileSystemCapabilities::new()
                    .read_text_file(true)
                    .write_text_file(true),
            ),
        )
}

/// 新建会话；附加目录一次传入，agent 不支持时按协议忽略。
///
/// 会话响应沿用 Moon 的容错解析（含旧式 `models` 兜底），因此经 SDK 的
/// raw 消息通道发送：请求参数仍是 SDK 类型序列化，响应保持原始 JSON。
async fn create_session(
    connection: &ConnectionTo<Agent>,
    input: &RunInput,
) -> Result<(String, AcpCapabilities), SdkError> {
    let request = NewSessionRequest::new(PathBuf::from(&input.workspace_dir))
        .mcp_servers(Vec::new())
        .additional_directories(
            input
                .additional_directories
                .iter()
                .map(PathBuf::from)
                .collect(),
        );
    let message = UntypedMessage::new("session/new", &request)
        .map_err(|error| protocol_error("创建 ACP 会话失败", error))?;
    let value = connection
        .send_request(message)
        .block_task()
        .await
        .map_err(|error| protocol_error("创建 ACP 会话失败", error))?;

    let session_id = value
        .get("sessionId")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            SdkError::new(ErrorCode::INTERNAL_ERROR, "ACP agent 未返回 sessionId")
        })?;
    Ok((session_id.to_string(), parse_capabilities(&value)))
}

/// 续接既有会话；响应保持原始 JSON 以复用容错解析。
async fn load_session_message(
    connection: &ConnectionTo<Agent>,
    session_id: &str,
    input: &RunInput,
) -> Result<Value, SdkError> {
    let request =
        LoadSessionRequest::new(session_id.to_string(), PathBuf::from(&input.workspace_dir));
    let message = UntypedMessage::new("session/load", &request)?;
    connection
        .send_request(message)
        .block_task()
        .await
        .map_err(|error| protocol_error("加载 ACP 会话失败", error))
}

/// 把 session 响应解析为能力快照；缺字段或类型不符时回退为空快照。
fn parse_capabilities(value: &Value) -> AcpCapabilities {
    let mut capabilities = AcpCapabilities::default();

    if let Some(modes) = value.get("modes").and_then(Value::as_object) {
        capabilities.current_mode_id = modes
            .get("currentModeId")
            .and_then(Value::as_str)
            .map(str::to_string);
        capabilities.modes = modes
            .get("availableModes")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(parse_mode).collect())
            .unwrap_or_default();
    }

    if let Some(options) = value.get("configOptions").and_then(Value::as_array) {
        capabilities.config_options = options.iter().filter_map(parse_config_option).collect();
    }

    if let Some(option) = model_config_option(&capabilities) {
        let models: Vec<AcpModelOption> = option
            .options
            .iter()
            .map(|choice| AcpModelOption {
                value: choice.value.clone(),
                name: choice.name.clone(),
            })
            .collect();
        let current_model_id = option.current_value.clone();
        capabilities.models = models;
        capabilities.current_model_id = current_model_id;
    }

    // 旧式 `models` 契约：configOptions 未提供模型时兜底。
    if let Some(models) = value.get("models") {
        let legacy = parse_legacy_models(models);
        if capabilities.models.is_empty() {
            capabilities.models = legacy.models;
        }
        if capabilities.current_model_id.is_none() {
            capabilities.current_model_id = legacy.current_model_id;
        }
    }

    capabilities
}

fn parse_mode(value: &Value) -> Option<AcpMode> {
    let id = value.get("id").and_then(Value::as_str)?.trim();
    if id.is_empty() {
        return None;
    }
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(id);
    Some(AcpMode {
        id: id.to_string(),
        name: name.to_string(),
    })
}

fn parse_config_option(value: &Value) -> Option<AcpConfigOption> {
    let id = value.get("id").and_then(Value::as_str)?.trim();
    if id.is_empty() {
        return None;
    }
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(id);
    let options = value
        .get("options")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(parse_choice).collect())
        .unwrap_or_default();
    Some(AcpConfigOption {
        id: id.to_string(),
        name: name.to_string(),
        category: value
            .get("category")
            .and_then(Value::as_str)
            .map(str::to_string),
        option_type: value
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_string),
        current_value: value
            .get("currentValue")
            .and_then(Value::as_str)
            .map(str::to_string),
        options,
    })
}

fn parse_choice(value: &Value) -> Option<AcpConfigChoice> {
    let raw = value.get("value").and_then(Value::as_str)?.trim();
    if raw.is_empty() {
        return None;
    }
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(raw);
    Some(AcpConfigChoice {
        value: raw.to_string(),
        name: name.to_string(),
    })
}

fn parse_legacy_models(value: &Value) -> AcpCapabilities {
    let mut capabilities = AcpCapabilities::default();
    if let Some(object) = value.as_object() {
        capabilities.current_model_id = object
            .get("currentModelId")
            .and_then(Value::as_str)
            .map(str::to_string);
        capabilities.models = object
            .get("availableModels")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(parse_legacy_model).collect())
            .unwrap_or_default();
    } else if let Some(items) = value.as_array() {
        capabilities.models = items.iter().filter_map(parse_legacy_model).collect();
    }
    capabilities
}

fn parse_legacy_model(value: &Value) -> Option<AcpModelOption> {
    let raw = ["modelId", "value", "id"]
        .iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))?
        .trim();
    if raw.is_empty() {
        return None;
    }
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(raw);
    Some(AcpModelOption {
        value: raw.to_string(),
        name: name.to_string(),
    })
}

fn model_config_option(capabilities: &AcpCapabilities) -> Option<&AcpConfigOption> {
    capabilities.config_options.iter().find(|option| {
        option.category.as_deref() == Some("model") || option.id.eq_ignore_ascii_case("model")
    })
}

/// 房间权限模式 → agent 宣告的模式 id；只从 availableModes 中选择。
fn map_permission_mode(
    mode: RoomPermissionMode,
    capabilities: &AcpCapabilities,
) -> Option<String> {
    let candidates: &[&str] = match mode {
        RoomPermissionMode::Plan => &["plan", "architect", "ask"],
        RoomPermissionMode::Auto => &["acceptedits", "accept_edits", "code", "default"],
        RoomPermissionMode::Full => &[
            "bypasspermissions",
            "bypass_permissions",
            "full-access",
            "full_access",
            "fullaccess",
            "code",
            "default",
        ],
    };

    for candidate in candidates {
        if let Some(found) = capabilities.modes.iter().find(|available| {
            let id = available.id.to_lowercase();
            let name = available.name.to_lowercase();
            id.contains(candidate) || name.contains(candidate)
        }) {
            return Some(found.id.clone());
        }
    }
    None
}

/// 应用会话模式与思考等级偏好；任何协议错误只发 Status，不影响后续 prompt。
///
/// 会话模式三级优先链：智能体显式会话模式（`model_config.sessionMode`）>
/// 旧 `reasoningLevel` 命中 modes（兼容已存智能体）> 房间权限模式映射。
/// 思考等级独立下发：`reasoningLevel` 不被会话模式链消费时，若命中思考类
/// configOption（category 或 id 不区分大小写含 think/reason）的候选值，
/// 经 set_config_option 下发；agent 未宣告思考选项时静默跳过。
async fn apply_session_preferences(
    input: &RunInput,
    session_id: &str,
    capabilities: &AcpCapabilities,
    connection: &ConnectionTo<Agent>,
    sender: &mpsc::Sender<AppResult<RunEvent>>,
) -> Result<(), SdkError> {
    let config = input.config.as_ref();
    let session_mode = config
        .and_then(|config| config.get("sessionMode"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let reasoning_level = config
        .and_then(|config| config.get("reasoningLevel"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());

    // ---- 会话模式链：决定 mode_from_config，进而决定是否走房间权限映射。
    let mut mode_from_config = false;
    // sessionMode 命中过 modes 时，旧 reasoningLevel 兼容链必须跳过（互斥优先链）。
    let mut session_mode_applied = false;
    if let Some(requested) = session_mode {
        if capabilities.modes.iter().any(|mode| mode.id == requested) {
            if capabilities.current_mode_id.as_deref() != Some(requested) {
                let result = connection
                    .send_request(SetSessionModeRequest::new(
                        SessionId::from(session_id.to_string()),
                        SessionModeId::new(requested.to_string()),
                    ))
                    .block_task()
                    .await;
                if let Err(error) = result {
                    send_event(
                        sender,
                        RunEvent::Status(format!("设置 ACP 会话模式失败：{}", error.message)),
                    )
                    .await
                    .map_err(sdk_internal)?;
                }
            }
            mode_from_config = true;
            session_mode_applied = true;
        } else {
            // 未宣告：仅此处提示；命中时不得产生误导性 Status。
            send_event(
                sender,
                RunEvent::Status(format!(
                    "ACP agent 未宣告会话模式「{requested}」，沿用房间权限模式映射"
                )),
            )
            .await
            .map_err(sdk_internal)?;
        }
    }
    // 旧数据兼容：reasoningLevel 存的是 mode id（sessionMode 之前的契约）；
    // 仅当 sessionMode 未命中时才尝试，避免双 set_mode 相互覆盖。
    if !session_mode_applied {
        if let Some(requested) = reasoning_level {
            if capabilities.modes.iter().any(|mode| mode.id == requested) {
                if capabilities.current_mode_id.as_deref() != Some(requested) {
                    let result = connection
                        .send_request(SetSessionModeRequest::new(
                            SessionId::from(session_id.to_string()),
                            SessionModeId::new(requested.to_string()),
                        ))
                        .block_task()
                        .await;
                    if let Err(error) = result {
                        send_event(
                            sender,
                            RunEvent::Status(format!("设置 ACP 模式失败：{}", error.message)),
                        )
                        .await
                        .map_err(sdk_internal)?;
                    }
                }
                mode_from_config = true;
            } else {
                // 未命中 modes：仅当也不命中思考选项候选值时才提示（既有文案保持不变），
                // 否则思考链会成功下发而 Status 却说「未宣告」，与拆分语义相悖。
                let hits_thinking = reasoning_config_option(capabilities).is_some_and(|option| {
                    option.options.iter().any(|choice| choice.value == requested)
                });
                if !hits_thinking {
                    send_event(
                        sender,
                        RunEvent::Status(format!(
                            "ACP agent 未宣告模式「{requested}」，沿用房间权限模式映射"
                        )),
                    )
                    .await
                    .map_err(sdk_internal)?;
                }
            }
        }
    }

    if !mode_from_config {
        apply_permission_mode_mapping(input, session_id, capabilities, connection, sender).await?;
    }

    // ---- 思考等级链（独立于会话模式链）：reasoningLevel 未被当作 mode 消费时，
    // 尝试思考类 configOption；未宣告或未命中时不再重复提示。
    if let Some(requested) = reasoning_level {
        let consumed_as_mode = capabilities.modes.iter().any(|mode| mode.id == requested);
        if !consumed_as_mode {
            if let Some(option) = reasoning_config_option(capabilities) {
                if option.options.iter().any(|choice| choice.value == requested) {
                    if option.current_value.as_deref() != Some(requested) {
                        let result = connection
                            .send_request(SetSessionConfigOptionRequest::new(
                                SessionId::from(session_id.to_string()),
                                SessionConfigId::new(option.id.clone()),
                                SessionConfigOptionValue::from(requested),
                            ))
                            .block_task()
                            .await;
                        if let Err(error) = result {
                            send_event(
                                sender,
                                RunEvent::Status(format!(
                                    "设置 ACP 思考等级失败：{}",
                                    error.message
                                )),
                            )
                            .await
                            .map_err(sdk_internal)?;
                        }
                    }
                }
            }
        }
    }

    apply_model_preference(input, session_id, capabilities, connection, sender).await
}

/// 识别 agent 宣告的思考类配置项：category 或 id 不区分大小写含 think/reason。
/// 与前端 `acpAgentCapabilities.ts` 的识别规则保持一致。
fn reasoning_config_option(capabilities: &AcpCapabilities) -> Option<&AcpConfigOption> {
    capabilities.config_options.iter().find(|option| {
        let id = option.id.to_lowercase();
        let category = option.category.as_deref().unwrap_or("").to_lowercase();
        id.contains("think")
            || id.contains("reason")
            || category.contains("think")
            || category.contains("reason")
    })
}

/// 房间权限模式 → agent 宣告模式的映射应用；未命中时只提示不改模式。
async fn apply_permission_mode_mapping(
    input: &RunInput,
    session_id: &str,
    capabilities: &AcpCapabilities,
    connection: &ConnectionTo<Agent>,
    sender: &mpsc::Sender<AppResult<RunEvent>>,
) -> Result<(), SdkError> {
    if let Some(mode) = input.permission_mode {
        match map_permission_mode(mode, capabilities) {
            Some(target) if capabilities.current_mode_id.as_deref() != Some(target.as_str()) => {
                let result = connection
                    .send_request(SetSessionModeRequest::new(
                        SessionId::from(session_id.to_string()),
                        SessionModeId::new(target),
                    ))
                    .block_task()
                    .await;
                if let Err(error) = result {
                    send_event(
                        sender,
                        RunEvent::Status(format!("设置 ACP 权限模式失败：{}", error.message)),
                    )
                    .await
                    .map_err(sdk_internal)?;
                }
            }
            Some(_) => {}
            None => {
                send_event(
                    sender,
                    RunEvent::Status(format!(
                        "ACP agent 未宣告与「{}」匹配的权限模式，沿用 agent 默认模式",
                        mode.as_str()
                    )),
                )
                .await
                .map_err(sdk_internal)?;
            }
        }
    }

    Ok(())
}

/// 应用智能体的模型偏好：新式 configOptions 走 set_config_option，
/// 旧式 models 走 set_model；未宣告时只提示。
async fn apply_model_preference(
    input: &RunInput,
    session_id: &str,
    capabilities: &AcpCapabilities,
    connection: &ConnectionTo<Agent>,
    sender: &mpsc::Sender<AppResult<RunEvent>>,
) -> Result<(), SdkError> {
    let Some(requested) = input
        .config
        .as_ref()
        .and_then(|config| config.get("model"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };

    if let Some(option) = model_config_option(capabilities) {
        if !option.options.iter().any(|choice| choice.value == requested) {
            send_event(
                sender,
                RunEvent::Status(format!(
                    "ACP agent 未宣告模型「{requested}」，沿用 agent 默认模型"
                )),
            )
            .await
            .map_err(sdk_internal)?;
        } else if option.current_value.as_deref() != Some(requested) {
            let result = connection
                .send_request(SetSessionConfigOptionRequest::new(
                    SessionId::from(session_id.to_string()),
                    SessionConfigId::new(option.id.clone()),
                    SessionConfigOptionValue::from(requested),
                ))
                .block_task()
                .await;
            if let Err(error) = result {
                send_event(
                    sender,
                    RunEvent::Status(format!("设置 ACP 模型失败：{}", error.message)),
                )
                .await
                .map_err(sdk_internal)?;
            }
        }
    } else if capabilities
        .models
        .iter()
        .any(|model| model.value == requested)
    {
        // 旧式 models 没有 configOptions，使用 ACP 早期的 set_model 请求。
        // 官方 SDK 未建模该方法，走 SDK 的 raw 消息通道保持既有行为。
        if capabilities.current_model_id.as_deref() != Some(requested) {
            let message = UntypedMessage::new(
                "session/set_model",
                json!({ "sessionId": session_id, "modelId": requested }),
            )?;
            let result = connection.send_request(message).block_task().await;
            if let Err(error) = result {
                send_event(
                    sender,
                    RunEvent::Status(format!("设置 ACP 模型失败：{}", error.message)),
                )
                .await
                .map_err(sdk_internal)?;
            }
        }
    } else {
        send_event(
            sender,
            RunEvent::Status(format!(
                "ACP agent 未宣告模型「{requested}」，沿用 agent 默认模型"
            )),
        )
        .await
        .map_err(sdk_internal)?;
    }

    Ok(())
}

/// 把 SDK 的会话更新映射为 RunEvent；无法识别的 update 忽略。
fn map_update(update: SessionUpdate) -> Option<RunEvent> {
    match update {
        SessionUpdate::AgentMessageChunk(chunk) => {
            let text = content_block_text(&chunk.content)?;
            (!text.is_empty()).then(|| RunEvent::TextDelta(text.to_string()))
        }
        SessionUpdate::AgentThoughtChunk(chunk) => {
            let text = content_block_text(&chunk.content)?;
            (!text.is_empty()).then(|| RunEvent::Reasoning(text.to_string()))
        }
        SessionUpdate::ToolCall(call) => Some(RunEvent::ToolStart {
            tool_call_id: call.tool_call_id.to_string(),
            name: call.title,
            input: json!({
                "kind": serde_json::to_value(call.kind).ok(),
                "status": serde_json::to_value(call.status).ok(),
            }),
        }),
        SessionUpdate::ToolCallUpdate(update) => {
            let status = update.fields.status?;
            let finished = matches!(status, ToolCallStatus::Completed | ToolCallStatus::Failed);
            finished.then(|| {
                let detail = update
                    .fields
                    .content
                    .as_ref()
                    .map(|content| serde_json::to_string(content).unwrap_or_default())
                    .filter(|detail| !detail.is_empty())
                    .unwrap_or_else(|| tool_call_status_text(status).to_string());
                RunEvent::ToolEnd {
                    tool_call_id: Some(update.tool_call_id.to_string()),
                    name: update.fields.title,
                    ok: status == ToolCallStatus::Completed,
                    detail,
                }
            })
        }
        SessionUpdate::UsageUpdate(usage) => Some(RunEvent::Usage {
            token_count: i64::try_from(usage.used).ok(),
            context_size: i64::try_from(usage.size).ok(),
            tool_call_count: None,
            duration_ms: None,
        }),
        _ => None,
    }
}

/// 提取文本内容块；只认 `{ type: "text", text }`。
fn content_block_text(content: &ContentBlock) -> Option<&str> {
    match content {
        ContentBlock::Text(text) => {
            let text = text.text.as_str();
            (!text.is_empty()).then_some(text)
        }
        _ => None,
    }
}

/// `StopReason` 的线上字符串；SDK 未提供字符串助手，按 serde 命名对齐。
fn stop_reason_text(reason: StopReason) -> &'static str {
    match reason {
        StopReason::EndTurn => "end_turn",
        StopReason::MaxTokens => "max_tokens",
        StopReason::MaxTurnRequests => "max_turn_requests",
        StopReason::Refusal => "refusal",
        StopReason::Cancelled => "cancelled",
        // `#[non_exhaustive]`：未来版本新增的停止原因按未知处理。
        _ => "unknown",
    }
}

fn tool_call_status_text(status: ToolCallStatus) -> &'static str {
    match status {
        ToolCallStatus::Pending => "pending",
        ToolCallStatus::InProgress => "in_progress",
        ToolCallStatus::Completed => "completed",
        ToolCallStatus::Failed => "failed",
        _ => "unknown",
    }
}

/// 协议失败包装：保留既有中文前缀文案，底层 agent 消息原样拼接。
fn protocol_error(context: &str, error: SdkError) -> SdkError {
    SdkError::new(
        ErrorCode::INTERNAL_ERROR,
        format!("{context}：{}", error.message),
    )
}

/// 事件通道关闭等本地失败 → SDK 错误（保留 Moon 文案）。
fn sdk_internal(error: AppError) -> SdkError {
    SdkError::new(ErrorCode::INTERNAL_ERROR, error.message)
}

/// 读取最终文本；锁中毒时取回内部值，不让单次 panic 拖垮整个运行。
fn lock_final(final_text: &Arc<Mutex<String>>) -> std::sync::MutexGuard<'_, String> {
    final_text
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

async fn read_workspace_file(scope: &WorkspaceScope, raw: &str) -> AppResult<String> {
    let path = scope.ensure_allowed(raw)?;
    tokio::fs::read_to_string(&path)
        .await
        .map_err(|error| {
            AppError::harness_run_failed(format!("读取工作区文件失败：{error}"))
        })
}

async fn write_workspace_file(scope: &WorkspaceScope, raw: &str, content: &str) -> AppResult<()> {
    let path = scope.ensure_allowed(raw)?;
    // 新文件的父目录可能尚不存在；路径已通过白名单校验，这里补齐目录。
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|error| {
            AppError::harness_run_failed(format!("创建写入目录失败：{error}"))
        })?;
    }
    tokio::fs::write(&path, content)
        .await
        .map_err(|error| {
            AppError::harness_run_failed(format!("写入工作区文件失败：{error}"))
        })
}

/// 工作区路径白名单：工作目录 + 附加目录，全部按真实路径解析后前缀匹配。
struct WorkspaceScope {
    roots: Vec<PathBuf>,
}

impl WorkspaceScope {
    fn new(workspace_dir: &str, additional_directories: &[String]) -> Self {
        let mut roots = vec![PathBuf::from(workspace_dir)];
        for directory in additional_directories {
            if !directory.trim().is_empty() {
                roots.push(PathBuf::from(directory));
            }
        }
        Self { roots }
    }

    /// 校验路径并返回可直接使用的绝对路径。
    ///
    /// - 已存在的路径：`canonicalize` 解析符号链接与真实大小写后必须落在
    ///   白名单内（符号链接指向白名单外会被拒绝）；
    /// - 不存在的路径（写入新文件）：从最深已存在祖先的 canonical 形式
    ///   拼回**完整剩余后缀**（中间目录不丢失），任何一层出现悬空符号
    ///   链接都直接拒绝。
    fn ensure_allowed(&self, raw: &str) -> AppResult<PathBuf> {
        let candidate = PathBuf::from(raw);
        if !candidate.is_absolute() {
            return Err(AppError::harness_run_failed(
                "ACP 文件请求必须使用绝对路径",
            ));
        }

        if let Ok(resolved) = candidate.canonicalize() {
            self.ensure_within(&resolved, raw)?;
            return Ok(resolved);
        }

        // 路径不存在：逐级回退。symlink_metadata 成功说明该前缀真实存在
        // 但无法 canonicalize（悬空符号链接或权限问题），直接拒绝。
        let mut missing: Vec<std::ffi::OsString> = Vec::new();
        let mut ancestor = candidate.clone();
        loop {
            if let Ok(base) = ancestor.canonicalize() {
                self.ensure_within(&base, raw)?;
                missing.reverse();
                let mut resolved = base;
                for part in missing {
                    resolved = resolved.join(part);
                }
                return Ok(resolved);
            }
            if std::fs::symlink_metadata(&ancestor).is_ok() {
                return Err(AppError::harness_run_failed(format!(
                    "路径包含符号链接或无法解析，超出工作区白名单：{raw}"
                )));
            }
            match ancestor.file_name() {
                Some(name) => missing.push(name.to_os_string()),
                None => {
                    return Err(AppError::harness_run_failed(format!(
                        "路径超出工作区白名单：{raw}"
                    )))
                }
            }
            if !ancestor.pop() || ancestor.as_os_str().is_empty() {
                return Err(AppError::harness_run_failed(format!(
                    "路径超出工作区白名单：{raw}"
                )));
            }
        }
    }

    /// canonical 形式必须落在某个白名单根内。
    fn ensure_within(&self, resolved: &std::path::Path, raw: &str) -> AppResult<()> {
        for root in &self.roots {
            let Ok(root) = root.canonicalize() else {
                continue;
            };
            if resolved.starts_with(&root) {
                return Ok(());
            }
        }
        Err(AppError::harness_run_failed(format!(
            "路径超出工作区白名单：{raw}"
        )))
    }
}

/// 与 zcode app-server 相同的收尾策略：Windows 下先按进程树终止，
/// 避免留下孤儿子进程；再兜底 kill。
async fn terminate_process_tree(child: &mut Child) {
    #[cfg(windows)]
    if let Some(pid) = child.id() {
        use tokio::process::Command as TokioCommand;
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            TokioCommand::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(CREATE_NO_WINDOW)
                .status(),
        )
        .await;
    }
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), child.kill()).await;
}

/// 发送一条 RunEvent；下游关闭时静默结束。
async fn send_event(
    sender: &mpsc::Sender<AppResult<RunEvent>>,
    event: RunEvent,
) -> AppResult<()> {
    sender
        .send(Ok(event))
        .await
        .map_err(|_| AppError::harness_run_failed("运行事件通道已关闭"))
}
#[cfg(test)]
mod tests {
    use super::*;

    fn capabilities_with_modes(ids: &[(&str, &str)]) -> AcpCapabilities {
        AcpCapabilities {
            modes: ids
                .iter()
                .map(|(id, name)| AcpMode {
                    id: (*id).to_string(),
                    name: (*name).to_string(),
                })
                .collect(),
            current_mode_id: ids.first().map(|(id, _)| (*id).to_string()),
            ..AcpCapabilities::default()
        }
    }

    #[test]
    fn parse_capabilities_reads_modes_and_model_config() {
        let capabilities = parse_capabilities(&json!({
            "sessionId": "sess-1",
            "modes": {
                "currentModeId": "default",
                "availableModes": [
                    { "id": "default", "name": "Default" },
                    { "id": "plan", "name": "Plan" },
                ],
            },
            "configOptions": [
                {
                    "id": "model",
                    "name": "Model",
                    "category": "model",
                    "type": "select",
                    "currentValue": "m1",
                    "options": [
                        { "value": "m1", "name": "Model 1" },
                        { "value": "m2", "name": "Model 2" },
                    ],
                },
                {
                    "id": "approval",
                    "name": "Approval",
                    "type": "select",
                    "options": [{ "value": "always", "name": "Always" }],
                },
            ],
        }));

        assert_eq!(capabilities.current_mode_id.as_deref(), Some("default"));
        assert_eq!(capabilities.modes.len(), 2);
        assert_eq!(capabilities.current_model_id.as_deref(), Some("m1"));
        assert_eq!(capabilities.models.len(), 2);
        assert_eq!(capabilities.models[1].value, "m2");
        assert_eq!(capabilities.config_options.len(), 2);
        assert_eq!(capabilities.config_options[1].id, "approval");
    }

    #[test]
    fn parse_capabilities_tolerates_malformed_shapes() {
        let capabilities = parse_capabilities(&json!({
            "modes": "not-an-object",
            "configOptions": [{ "id": 1 }, "bad"],
            "models": 42,
        }));
        assert_eq!(capabilities, AcpCapabilities::default());
    }

    #[test]
    fn parse_capabilities_falls_back_to_legacy_models() {
        let capabilities = parse_capabilities(&json!({
            "models": {
                "currentModelId": "m2",
                "availableModels": [
                    { "modelId": "m1", "name": "Model 1" },
                    { "modelId": "m2" },
                ],
            },
        }));
        assert_eq!(capabilities.current_model_id.as_deref(), Some("m2"));
        assert_eq!(capabilities.models.len(), 2);
        assert_eq!(capabilities.models[1].name, "m2");
    }

    #[test]
    fn parse_capabilities_prefers_config_options_over_legacy_models() {
        let capabilities = parse_capabilities(&json!({
            "configOptions": [{
                "id": "model",
                "category": "model",
                "currentValue": "new",
                "options": [{ "value": "new", "name": "New" }],
            }],
            "models": {
                "currentModelId": "old",
                "availableModels": [{ "modelId": "old" }],
            },
        }));
        assert_eq!(capabilities.current_model_id.as_deref(), Some("new"));
        assert_eq!(capabilities.models[0].value, "new");
    }

    #[test]
    fn map_permission_mode_uses_declared_modes_only() {
        let capabilities = capabilities_with_modes(&[
            ("default", "Default"),
            ("acceptEdits", "Accept Edits"),
            ("plan", "Plan"),
            ("bypassPermissions", "Bypass"),
        ]);
        assert_eq!(
            map_permission_mode(RoomPermissionMode::Plan, &capabilities).as_deref(),
            Some("plan")
        );
        assert_eq!(
            map_permission_mode(RoomPermissionMode::Auto, &capabilities).as_deref(),
            Some("acceptEdits")
        );
        assert_eq!(
            map_permission_mode(RoomPermissionMode::Full, &capabilities).as_deref(),
            Some("bypassPermissions")
        );

        let only_default = capabilities_with_modes(&[("default", "Default")]);
        assert_eq!(map_permission_mode(RoomPermissionMode::Plan, &only_default), None);
        assert_eq!(
            map_permission_mode(RoomPermissionMode::Auto, &only_default).as_deref(),
            Some("default")
        );
        assert_eq!(
            map_permission_mode(RoomPermissionMode::Full, &only_default).as_deref(),
            Some("default")
        );

        assert_eq!(
            map_permission_mode(RoomPermissionMode::Plan, &AcpCapabilities::default()),
            None
        );
    }

    #[test]
    fn map_permission_mode_matches_names_case_insensitively() {
        let capabilities = capabilities_with_modes(&[
            ("1", "Architect"),
            ("2", "Ask Agent"),
        ]);
        assert_eq!(
            map_permission_mode(RoomPermissionMode::Plan, &capabilities).as_deref(),
            Some("1")
        );

        let ask_only = capabilities_with_modes(&[("2", "Ask Agent")]);
        assert_eq!(
            map_permission_mode(RoomPermissionMode::Plan, &ask_only).as_deref(),
            Some("2")
        );
    }
}
