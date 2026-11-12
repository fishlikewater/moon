//! 房间聊天编排。
//!
//! 职责边界：
//! - 落库（消息/运行记录）与实时广播的编排顺序在此层确定；
//! - harness 适配器只负责 I/O，存储层只负责持久化，WS hub 只负责投递；
//! - 同一房间的运行通过 `RoomRunQueue` 串行，不同房间可并行。
//!
//! @ 提及规则：只识别房间成员，且必须精确匹配成员名称（大小写敏感）；
//! `@名称` 后需是空白、标点或文本结尾，避免 `@a` 误匹配 `@ab`。
//! 多个提及按在文本中出现的先后顺序依次排队运行。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::oneshot;

use crate::adapters::{create_adapter, AdapterFactory};
use crate::domain::agent::{Agent, BuiltinAgentRole};
use crate::domain::discussion::DiscussionStatus;
use crate::domain::collaboration::{
    CollaborationAssignment, CollaborationAssignmentStatus, CollaborationBatch,
    CollaborationStatus, CollaborationStopReason, DEFAULT_COLLABORATION_MAX_ROUNDS,
};
use crate::domain::event::RoomEvent;
use crate::domain::message::{
    CollaborationMessageRole, DiscussionStance, DiscussionStopVote, Message, MessageCollaboration,
    MessageDiscussion, MessageKind, MessageSenderType, MessageStatus,
};
use crate::domain::room::{Room, RoomReplyMode};
use crate::domain::run::{
    AgentRun, ContextPreview, RunActivity, RunActivityKind, RunDetailResponse, RunStatus,
};
use crate::domain::run_event::{RunEvent, RunInput};
use crate::error::{AppError, AppResult};
use crate::queue::{PersistJobSpec, QueueJobInfo, QueueJobKind, RoomRunQueue};
use crate::services::attachments::AttachmentService;
use crate::services::collaborations::{
    build_round_review_instruction, parse_round_review, CollaborationService, ParsedRoundReview,
    RoundReviewMemberResult,
};
use crate::services::context_compaction::{
    compact_transcript, estimate_tokens, CompactionOptions, CompactionResult, LabeledMessage,
    CONTEXT_MAX_MESSAGES,
};
use crate::services::discussion::DiscussionService;
use crate::services::message_artifacts::{
    compact_message_line, context_messages, instruction_with_attachments, save_message_artifact,
    ArtifactInput, ARCHIVE_MIN_CHARS,
};
use crate::services::moderator::{
    build_delegated_instruction, build_moderator_chat_instruction, extract_moderator_action,
    ChatAssignment, ModeratorChatAction, ModeratorChatExtraction, ModeratorDelegation,
    CONCISE_REPLY_RULE,
};
use crate::services::orchestrator::OrchestratorEngine;
use crate::services::run_activity::{
    format_usage_summary, summarize_tool_payload, RunActivityRegistry,
};
use crate::services::sessions::SessionService;
use crate::services::settings::SettingsService;
use crate::services::tasks::TaskService;
use crate::services::parse_id_label;
use crate::store::collaborations::{
    AddCollaborationAssignmentInput, CreateCollaborationBatchInput, FinishCollaborationBatchInput,
    UpdateCollaborationAssignmentInput,
};
use crate::store::messages::{AppendMessageInput, UpdateMessageDiscussionInput, UpdateMessageInput};
use crate::store::queue::PersistedQueueJob;
use crate::store::{AgentStore, MessageStore, RoomStore, RunStore};

/// 流式占位消息落库节流间隔。
const STREAM_FLUSH_INTERVAL_MS: u64 = 60;
/// 推理片段合并阈值。
const REASONING_CHUNK_CHARS: usize = 600;
/// 阶段总结的最小字符数。
pub const MIN_PROGRESS_CHARS: usize = 20;

pub const STAGE_SUMMARY_RULE: &str = "执行过程中：每完成一个工具阶段，先用 1–2 句中文总结本阶段做了什么、发现了什么、下一步是什么，再继续；这些总结会展示给用户。最终回复只给结论与交付物，不要重复阶段总结。";

const MENTION_TERMINATORS: &str = ",，.。!！?？;；:：、)\\]】}\"'”’";

const RUN_LIST_DEFAULT_LIMIT: usize = 20;
const RUN_LIST_MAX_LIMIT: usize = 100;

pub type BroadcastFn = Arc<dyn Fn(i64, &RoomEvent) + Send + Sync>;

/// 主持人普通聊天运行选项：不流式输出结构化块，终态前解析并剥离。
fn moderator_chat_options(_room: &Room, trigger_message_id: i64) -> RunAgentOptions {
    RunAgentOptions {
        stream: Some(false),
        moderator_chat: true,
        moderator_trigger_message_id: Some(trigger_message_id),
        ..RunAgentOptions::default()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DiscussionRef {
    pub discussion_id: i64,
    pub round: i64,
}

/// 讨论消息结构化字段的终态补丁；`Some(None)` 表示显式清空。
#[derive(Debug, Clone, Default)]
pub struct DiscussionMetadataPatch {
    pub stance: Option<Option<DiscussionStance>>,
    pub reply_to_message_id: Option<Option<i64>>,
    pub stop_vote: Option<Option<DiscussionStopVote>>,
    pub stop_reason: Option<Option<String>>,
}

#[derive(Debug, Clone)]
pub struct FinalizedContent {
    pub content: String,
    pub discussion: Option<DiscussionMetadataPatch>,
}

/// 成功输出落库前的纯转换；失败时整次运行按解析失败处理。
pub type FinalizeFn = Arc<dyn Fn(&str) -> AppResult<FinalizedContent> + Send + Sync>;

/// 单次运行的扩展选项。
#[derive(Clone, Default)]
pub struct RunAgentOptions {
    /// `stream=false`：不广播 text_delta，只在终态广播完整消息。
    pub stream: Option<bool>,
    /// 主持人普通聊天：使用专用协议提示词并在终态解析动作。
    pub moderator_chat: bool,
    /// 触发本次主持人运行的用户消息 id；委派批次据此关联。
    pub moderator_trigger_message_id: Option<i64>,
    /// instruction 已自带完整上下文，避免外层再包房间历史。
    pub skip_room_context: bool,
    /// 是否把工具阶段之间的可见文本拆成 progress 消息；默认 true。
    pub progress: Option<bool>,
    /// 把占位消息与终态写入讨论轮次。
    pub discussion: Option<DiscussionRef>,
    /// 委派批次归属。
    pub collaboration: Option<MessageCollaboration>,
    /// 成功输出落库前的结构化转换。
    pub finalize: Option<FinalizeFn>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandleUserMessageResult {
    pub user_message: Message,
}

pub struct ChatServiceOptions {
    pub rooms: RoomStore,
    pub messages: MessageStore,
    pub runs: RunStore,
    pub agents: AgentStore,
    pub queue: Arc<RoomRunQueue>,
    pub broadcast: BroadcastFn,
    pub adapter_factory: Option<AdapterFactory>,
    pub session_service: Option<Arc<SessionService>>,
    pub attachments: Arc<AttachmentService>,
    pub settings: Arc<SettingsService>,
}

#[derive(Clone)]
pub struct ChatService {
    rooms: RoomStore,
    messages: MessageStore,
    runs: RunStore,
    agents: AgentStore,
    queue: Arc<RoomRunQueue>,
    broadcast: BroadcastFn,
    adapter_factory: AdapterFactory,
    session_service: Option<Arc<SessionService>>,
    attachments: Arc<AttachmentService>,
    settings: Arc<SettingsService>,
    activity: Arc<RunActivityRegistry>,
    /// 由 AppState 在构造后注入；避免 ChatService ↔ DiscussionService 的初始化环。
    discussion_service: Arc<RwLock<Option<Weak<DiscussionService>>>>,
    task_service: Arc<RwLock<Option<Weak<TaskService>>>>,
    collaborations: Arc<RwLock<Option<Arc<CollaborationService>>>>,
    orchestrator: Arc<RwLock<Option<Weak<OrchestratorEngine>>>>,
}

impl ChatService {
    pub fn new(options: ChatServiceOptions) -> Self {
        Self {
            rooms: options.rooms,
            messages: options.messages,
            runs: options.runs,
            agents: options.agents,
            queue: options.queue,
            broadcast: options.broadcast,
            adapter_factory: options.adapter_factory.unwrap_or_else(|| {
                Arc::new(|agent: &Agent| create_adapter(agent.harness))
            }),
            session_service: options.session_service,
            attachments: options.attachments,
            settings: options.settings,
            activity: Arc::new(RunActivityRegistry::default()),
            discussion_service: Arc::new(RwLock::new(None)),
            task_service: Arc::new(RwLock::new(None)),
            collaborations: Arc::new(RwLock::new(None)),
            orchestrator: Arc::new(RwLock::new(None)),
        }
    }

    pub fn set_discussion_service(&self, service: Weak<DiscussionService>) {
        *self
            .discussion_service
            .write()
            .unwrap_or_else(|error| error.into_inner()) = Some(service);
    }

    pub fn set_task_service(&self, service: Weak<TaskService>) {
        *self
            .task_service
            .write()
            .unwrap_or_else(|error| error.into_inner()) = Some(service);
    }

    pub fn set_collaboration_service(&self, service: Arc<CollaborationService>) {
        *self
            .collaborations
            .write()
            .unwrap_or_else(|error| error.into_inner()) = Some(service);
    }

    pub fn set_orchestrator(&self, service: Weak<OrchestratorEngine>) {
        *self
            .orchestrator
            .write()
            .unwrap_or_else(|error| error.into_inner()) = Some(service);
    }

    /// 房间消息按 seq 升序返回；房间不存在时 404。
    pub async fn list_messages(&self, raw_room_id: &str) -> AppResult<Vec<Message>> {
        let room_id = parse_id_label(raw_room_id, "房间")?;
        self.require_room(room_id).await?;
        self.messages.list_by_room(room_id).await
    }

    /// 房间是否有排队或运行中的运行；供会话归档/历史清理做拦截。
    pub fn is_room_busy(&self, room_id: i64) -> bool {
        self.queue.is_busy(room_id)
    }

    /// 房间最近运行；默认 20 条，最多 100 条。
    pub async fn list_runs(
        &self,
        raw_room_id: &str,
        raw_limit: Option<&str>,
    ) -> AppResult<Vec<AgentRun>> {
        let room_id = parse_id_label(raw_room_id, "房间")?;
        self.require_room(room_id).await?;
        self.runs
            .list_recent_by_room(room_id, parse_run_limit(raw_limit))
            .await
    }

    /// 单次运行详情；跨房间访问与不存在统一返回 404。
    pub async fn get_run_detail(
        &self,
        raw_room_id: &str,
        raw_run_id: &str,
    ) -> AppResult<RunDetailResponse> {
        let room_id = parse_id_label(raw_room_id, "房间")?;
        self.require_room(room_id).await?;
        let run_id = parse_id_label(raw_run_id, "运行")?;
        let Some(run) = self.runs.get_by_id(run_id).await? else {
            return Err(AppError::not_found(format!("运行不存在：{run_id}")));
        };
        if run.room_id != room_id {
            return Err(AppError::not_found(format!("运行不存在：{run_id}")));
        }
        Ok(RunDetailResponse {
            activities: self.activity.list(run_id),
            messages: self.messages.list_by_run(run_id).await?,
            run,
        })
    }

    /// 处理用户发言：落库 → 广播 → 解析 @提及 → 依次排入房间队列。
    pub async fn handle_user_message(
        self: &Arc<Self>,
        raw_room_id: &str,
        body: Value,
    ) -> AppResult<HandleUserMessageResult> {
        let room_id = parse_id_label(raw_room_id, "房间")?;
        let room = self.require_room(room_id).await?;
        let content = body
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let attachment_ids = parse_attachment_ids(body.get("attachmentIds"))?;
        if content.trim().is_empty() && attachment_ids.is_empty() {
            return Err(AppError::validation(
                "content 与 attachmentIds 至少一个非空",
            ));
        }
        let reply_to_id = parse_reply_to_id(body.get("replyToId"))?;
        let reply_target = if let Some(id) = reply_to_id {
            match self.messages.get_by_id(id).await? {
                Some(message) if message.room_id == room_id => Some(message),
                _ => return Err(AppError::validation("被回复的消息不存在")),
            }
        } else {
            None
        };
        self.attachments
            .validate_for_message(room_id, &attachment_ids)
            .await?;

        let user_message = self
            .messages
            .append_with_attachments(
                AppendMessageInput::final_message(
                    room_id,
                    MessageSenderType::User,
                    None,
                    content.clone(),
                )
                .with_reply_to(reply_to_id),
                &attachment_ids,
            )
            .await?;
        self.broadcast(room_id, RoomEvent::MessageFinal {
            room_id,
            message: user_message.clone(),
        });

        let members = self.rooms.list_members(room_id).await?;
        let mentions = parse_mentions(&content, &members);
        // 附件路径与回复引用进入本次指令，会话复用模式下同样携带。
        let instruction_content = match reply_target.as_ref() {
            Some(target) => {
                let sender_label = match target.sender_agent_id {
                    Some(agent_id) => members
                        .iter()
                        .find(|member| member.id == agent_id)
                        .map(|member| member.name.as_str())
                        .unwrap_or("成员"),
                    None => match target.sender_type {
                        MessageSenderType::User => "我",
                        MessageSenderType::System => "系统",
                        MessageSenderType::Agent => "成员",
                    },
                };
                reply_context(&content, target, sender_label)
            }
            None => content.clone(),
        };
        let instruction =
            instruction_with_attachments(&instruction_content, &user_message.attachments);

        if !mentions.is_empty() {
            for agent in mentions {
                // 逐个入队：同一房间 FIFO 串行，多个被提及智能体按提及顺序运行。
                // 显式 @ 内置主持人时也允许其使用讨论/任务协议。
                let options = if agent.builtin_role == Some(BuiltinAgentRole::Moderator) {
                    moderator_chat_options(&room, user_message.id)
                } else {
                    RunAgentOptions::default()
                };
                self.enqueue_run(room.clone(), agent, instruction.clone(), options);
            }
            return Ok(HandleUserMessageResult { user_message });
        }

        if room.reply_mode != RoomReplyMode::Moderator {
            return Ok(HandleUserMessageResult { user_message });
        }

        // 失败讨论的「继续」快路径：复用同一 discussion 从断点续跑，
        // 不再交给主持人重新规划（否则会从第 1 轮重开整场讨论）。
        if is_resume_intent(&content) {
            if let Some(service) = self.discussion_service() {
                if let Some(latest) = service.get_latest(&room.id.to_string()).await? {
                    if latest.status == DiscussionStatus::Failed {
                        match service
                            .resume(&room.id.to_string(), &latest.id.to_string())
                            .await
                        {
                            Ok(_) => {
                                return Ok(HandleUserMessageResult { user_message });
                            }
                            Err(error) => {
                                self.post_system_message(
                                    room.id,
                                    &format!("继续讨论失败：{}", error.message),
                                    None,
                                    None,
                                    None,
                                )
                                .await?;
                            }
                        }
                    }
                }
            }
        }

        let Some(moderator) = members
            .iter()
            .find(|member| member.builtin_role == Some(BuiltinAgentRole::Moderator))
        else {
            self.post_system_message(
                room.id,
                "该房间未配置内置主持人，无法自动接管消息",
                None,
                None,
                None,
            )
            .await?;
            return Ok(HandleUserMessageResult { user_message });
        };

        self.enqueue_run(
            room.clone(),
            moderator.clone(),
            instruction,
            moderator_chat_options(&room, user_message.id),
        );
        Ok(HandleUserMessageResult { user_message })
    }

    /// 入队一次运行；运行创建前的异常也留痕，不静默丢弃。
    /// 队列取消返回 `Ok(None)`，属于用户主动撤回，不记失败日志。
    fn enqueue_run(
        self: &Arc<Self>,
        room: Room,
        agent: Agent,
        instruction: String,
        options: RunAgentOptions,
    ) {
        let service = Arc::clone(self);
        tokio::spawn(async move {
            match service
                .run_agent_queued(room.id, &agent, &instruction, None, options)
                .await
            {
                Ok(_) => {}
                Err(error) => {
                    tracing::error!(
                        event = "agent_run_failed",
                        room_id = room.id,
                        agent_id = agent.id,
                        error = %error.message,
                        "agent run failed before completion"
                    );
                }
            }
        });
    }

    /// 在房间串行队列中执行一次运行；排队阶段被取消时返回 `Ok(None)`。
    pub async fn run_agent_queued(
        &self,
        room_id: i64,
        agent: &Agent,
        instruction: &str,
        task_id: Option<i64>,
        options: RunAgentOptions,
    ) -> AppResult<Option<AgentRun>> {
        let info = queue_job_info(agent, task_id, &options);
        // 只有用户消息触发的聊天/主持人任务可跨重启恢复；讨论/委派/任务
        // 由各自编排状态机的启动恢复负责。
        if info.kind.is_persistable() && self.queue.store().is_some() {
            return Ok(self
                .queue
                .enqueue_managed_persisted(
                    room_id,
                    info,
                    PersistJobSpec {
                        row_id: None,
                        instruction: instruction.to_string(),
                        moderator_trigger_message_id: options.moderator_trigger_message_id,
                    },
                    || async {
                        self.run_agent(room_id, agent, instruction, task_id, &options)
                            .await
                    },
                )
                .await?
                .transpose()?);
        }
        Ok(self
            .queue
            .enqueue_managed(room_id, info, || async {
                self.run_agent(room_id, agent, instruction, task_id, &options)
                    .await
            })
            .await
            .transpose()?)
    }

    /// 队列出队后再检查一次前置条件（如讨论仍运行），不满足则跳过本次运行。
    pub async fn run_agent_queued_checked<G, FutG>(
        &self,
        room_id: i64,
        agent: &Agent,
        instruction: &str,
        task_id: Option<i64>,
        options: RunAgentOptions,
        should_run: G,
    ) -> AppResult<Option<AgentRun>>
    where
        G: Fn() -> FutG,
        FutG: std::future::Future<Output = bool>,
    {
        let info = queue_job_info(agent, task_id, &options);
        Ok(self
            .queue
            .enqueue_managed_when(
                room_id,
                info,
                should_run,
                || async {
                    self.run_agent(room_id, agent, instruction, task_id, &options)
                        .await
                },
            )
            .await
            .transpose()?)
    }

    /// 启动恢复：把上次进程退出时仍未开跑的聊天/主持人任务重新入队。
    ///
    /// 行按 id 升序恢复；每个房间的驱动任务等待上一行登记进房间锁等待
    /// 队列后再派发下一行，保证恢复顺序不受 tokio 调度顺序影响。运行中的
    /// 任务不在此表内，仍由 `recover_interrupted` 标记为中断。
    pub async fn restore_persisted_queue(self: &Arc<Self>) -> AppResult<()> {
        let Some(store) = self.queue.store() else {
            return Ok(());
        };
        let rows = store.list_pending().await?;
        if rows.is_empty() {
            return Ok(());
        }
        let mut by_room: BTreeMap<i64, Vec<PersistedQueueJob>> = BTreeMap::new();
        for row in rows {
            by_room.entry(row.room_id).or_default().push(row);
        }
        for (_, rows) in by_room {
            let service = self.clone();
            tokio::spawn(async move {
                for row in rows {
                    let row_id = row.id;
                    let (waiting, registered) = oneshot::channel();
                    let worker = service.clone();
                    tokio::spawn(async move {
                        if let Err(error) = worker.restore_queue_job(row, waiting).await {
                            tracing::error!(
                                event = "queue_restore_failed",
                                row_id,
                                error = %error.message,
                                "queue restore failed"
                            );
                        }
                    });
                    // 行已失效时 sender 被丢弃，接收端立即返回 Err。
                    match tokio::time::timeout(Duration::from_secs(2), registered).await {
                        Ok(Ok(())) => {}
                        Ok(Err(_)) => {}
                        Err(_) => tracing::warn!(
                            event = "queue_restore_registration_timeout",
                            row_id,
                            "queue restore registration timed out"
                        ),
                    }
                }
            });
        }
        Ok(())
    }

    /// 重建单条恢复任务；房间/智能体缺失、成员已移除或类型不可恢复时清行。
    async fn restore_queue_job(
        &self,
        row: PersistedQueueJob,
        waiting: oneshot::Sender<()>,
    ) -> AppResult<()> {
        let Some(room) = self.rooms.get_by_id(row.room_id).await? else {
            return self.delete_restored_row(row.id).await;
        };
        let Some(agent) = self.agents.get_by_id(row.agent_id).await? else {
            return self.delete_restored_row(row.id).await;
        };
        let Some(kind) = QueueJobKind::parse(&row.kind) else {
            return self.delete_restored_row(row.id).await;
        };
        if !kind.is_persistable() || !room.member_ids.contains(&agent.id) {
            return self.delete_restored_row(row.id).await;
        }

        let options = if kind == QueueJobKind::Moderator {
            RunAgentOptions {
                stream: Some(false),
                moderator_chat: true,
                moderator_trigger_message_id: row.moderator_trigger_message_id,
                ..RunAgentOptions::default()
            }
        } else {
            RunAgentOptions::default()
        };
        let info = QueueJobInfo {
            agent_id: Some(agent.id),
            label: row.label.clone(),
            kind,
            cancelable: true,
        };
        self.queue
            .enqueue_managed_restored(
                row.room_id,
                info,
                PersistJobSpec {
                    row_id: Some(row.id),
                    instruction: row.instruction.clone(),
                    moderator_trigger_message_id: row.moderator_trigger_message_id,
                },
                waiting,
                || async {
                    self.run_agent(row.room_id, &agent, &row.instruction, None, &options)
                        .await
                },
            )
            .await?;
        Ok(())
    }

    async fn delete_restored_row(&self, row_id: i64) -> AppResult<()> {
        if let Some(store) = self.queue.store() {
            if let Err(error) = store.delete(row_id).await {
                tracing::warn!(
                    event = "queue_restore_cleanup_failed",
                    row_id,
                    error = %error.message,
                    "queue restore cleanup failed"
                );
            }
        }
        Ok(())
    }

    /// 只读判断该成员是否有可续接的 harness 会话。
    ///
    /// 讨论/裁决在构建提示词前调用：会话已持有上一轮上下文时，只需发送
    /// 增量指令；未开启复用或尚无会话 ID 时仍发送完整提示词。
    pub async fn has_resumable_session(&self, room_id: i64, agent: &Agent) -> AppResult<bool> {
        match &self.session_service {
            Some(service) => service.has_resumable_session(room_id, agent).await,
            None => Ok(false),
        }
    }

    /// 执行一次智能体运行。
    ///
    /// 调用方负责用 `RoomRunQueue` 保证同房间串行；本方法只负责单次运行的
    /// 持久化、流式广播与失败可见化，不吞掉失败。
    pub async fn run_agent(
        &self,
        room_id: i64,
        agent: &Agent,
        instruction: &str,
        task_id: Option<i64>,
        options: &RunAgentOptions,
    ) -> AppResult<AgentRun> {
        let room = self.require_room(room_id).await?;

        // 复用会话时 harness 侧已持有对话上下文，不再重复拼接房间历史。
        let run_session = match &self.session_service {
            Some(service) => service.resolve_run_session(room_id, agent).await?,
            None => None,
        };
        let history = if options.skip_room_context || run_session.is_some() {
            Vec::new()
        } else {
            self.messages.list_by_room(room_id).await?
        };
        let prompt = self.build_prompt(&room, agent, &history, instruction, options).await?;
        let streaming = options.stream.unwrap_or(true);
        let discussion = options.discussion.map(|reference| MessageDiscussion {
            discussion_id: reference.discussion_id,
            round: reference.round,
            stance: None,
            reply_to_message_id: None,
            stop_vote: None,
            stop_reason: None,
        });
        let collaboration = options.collaboration.clone();

        let run = self
            .runs
            .create(crate::store::runs::CreateRunInput {
                room_id,
                agent_id: agent.id,
                task_id,
                prompt: prompt.clone(),
            })
            .await?;
        let placeholder = self
            .messages
            .append(AppendMessageInput {
                room_id,
                sender_type: MessageSenderType::Agent,
                sender_agent_id: Some(agent.id),
                content: String::new(),
                status: MessageStatus::Streaming,
                kind: MessageKind::Reply,
                run_id: Some(run.id),
                task_id,
                reply_to_message_id: None,
                discussion: discussion.clone(),
                collaboration: collaboration.clone(),
            })
            .await?;

        let started_at = Instant::now();
        tracing::info!(
            event = "agent_run_started",
            room_id,
            agent_id = agent.id,
            run_id = run.id,
            message_id = placeholder.id,
            "agent run started"
        );
        self.broadcast(room_id, RoomEvent::RunStarted {
            room_id,
            run_id: run.id,
            agent_id: agent.id,
            message_id: placeholder.id,
            message: placeholder.clone(),
        });

        let mut current_message = placeholder;
        let mut writer = StreamingWriter::new(
            &self.messages,
            &self.broadcast,
            room_id,
            current_message.id,
        );
        let progress_enabled = options.progress.unwrap_or(!options.moderator_chat);

        let mut segment = String::new();
        let mut progress_count = 0_usize;
        let mut shown_progress = String::new();
        let mut final_text: Option<String> = None;
        let mut failure: Option<String> = None;
        let mut reasoning_buffer = String::new();
        let mut tool_names: HashMap<String, String> = HashMap::new();
        let mut seen_tool_calls: HashSet<String> = HashSet::new();

        let flush_reasoning = |buffer: &mut String| {
            let text = buffer.trim();
            if text.is_empty() {
                buffer.clear();
                return;
            }
            let detail = text.to_string();
            buffer.clear();
            let activity = self.activity.record(
                run.id,
                RunActivityKind::Reasoning,
                "思考",
                Some(&detail),
            );
            self.broadcast(room_id, RoomEvent::RunActivity {
                room_id,
                activity,
            });
        };

        let adapter = (self.adapter_factory)(agent);
        match adapter {
            Err(error) => failure = Some(error.message),
            Ok(adapter) => {
                let members = self.rooms.list_members(room_id).await?;
                let agent_workspace = match agent.workspace_dir.as_deref() {
                    Some(dir) => match tokio::fs::metadata(dir).await {
                        Ok(metadata) if metadata.is_dir() => Some(dir.to_string()),
                        _ => {
                            tracing::warn!(dir, "智能体工作区不可用，回退房间主工作区");
                            None
                        }
                    },
                    None => None,
                };
                let workspace_dir =
                    agent_workspace.unwrap_or_else(|| room.workspace_dir.clone());
                let mut additional_directories: Vec<String> = Vec::new();
                let mut candidates: Vec<&String> = room.workspace_dirs.iter().collect();
                candidates.extend(
                    members
                        .iter()
                        .filter_map(|member| member.workspace_dir.as_ref()),
                );
                for dir in candidates {
                    if dir == &workspace_dir || additional_directories.contains(dir) {
                        continue;
                    }
                    if tokio::fs::metadata(dir)
                        .await
                        .map(|metadata| metadata.is_dir())
                        .unwrap_or(false)
                    {
                        additional_directories.push(dir.clone());
                    }
                }
                let run_config = self
                    .settings
                    .with_harness_bin_override(agent.harness, agent.model_config.clone())
                    .await?;
                let input = RunInput {
                    prompt,
                    workspace_dir,
                    additional_directories,
                    config: run_config,
                    session: run_session.as_ref().map(|session| session.context.clone()),
                    permission_mode: Some(room.permission_mode),
                    timeout: self.settings.run_timeout().await?,
                };
                let mut stream = adapter.run(input);
                while let Some(item) = stream.next().await {
                    let event = match item {
                        Ok(event) => event,
                        Err(error) => {
                            failure = Some(error.message);
                            break;
                        }
                    };

                    if let RunEvent::Reasoning(text) = event {
                        reasoning_buffer.push_str(&text);
                        if reasoning_buffer.chars().count() >= REASONING_CHUNK_CHARS {
                            flush_reasoning(&mut reasoning_buffer);
                        }
                        continue;
                    }
                    flush_reasoning(&mut reasoning_buffer);

                    match event {
                        RunEvent::TextDelta(text) => {
                            segment.push_str(&text);
                            if streaming {
                                writer.push(&text).await;
                            }
                        }
                        RunEvent::Final(text) => final_text = Some(text),
                        RunEvent::ToolStart {
                            tool_call_id,
                            name,
                            input,
                        } => {
                            // model.streaming.tool_call 与 tool.updated.scheduled 可能同时出现。
                            if !seen_tool_calls.insert(tool_call_id.clone()) {
                                continue;
                            }
                            if progress_enabled
                                && segment.trim().chars().count() >= MIN_PROGRESS_CHARS
                            {
                                writer.flush().await;
                                let progress = self
                                    .messages
                                    .update(
                                        current_message.id,
                                        UpdateMessageInput {
                                            content: Some(segment.clone()),
                                            status: Some(MessageStatus::Final),
                                            kind: Some(MessageKind::Progress),
                                        },
                                    )
                                    .await?
                                    .unwrap_or_else(|| {
                                        apply_message_update(
                                            &current_message,
                                            &segment,
                                            MessageStatus::Final,
                                            MessageKind::Progress,
                                        )
                                    });
                                let cleared = self
                                    .messages
                                    .clear_structured_metadata(current_message.id)
                                    .await?;
                                let progress = cleared.unwrap_or(progress);
                                self.broadcast(room_id, RoomEvent::MessageFinal {
                                    room_id,
                                    message: progress,
                                });

                                current_message = self
                                    .messages
                                    .append(AppendMessageInput {
                                        room_id,
                                        sender_type: MessageSenderType::Agent,
                                        sender_agent_id: Some(agent.id),
                                        content: String::new(),
                                        status: MessageStatus::Streaming,
                                        kind: MessageKind::Reply,
                                        run_id: Some(run.id),
                                        task_id,
                                        reply_to_message_id: None,
                                        discussion: discussion.clone(),
                                        collaboration: collaboration.clone(),
                                    })
                                    .await?;
                                self.broadcast(room_id, RoomEvent::MessageStarted {
                                    room_id,
                                    run_id: run.id,
                                    message: current_message.clone(),
                                });
                                writer = StreamingWriter::new(
                                    &self.messages,
                                    &self.broadcast,
                                    room_id,
                                    current_message.id,
                                );
                                progress_count += 1;
                                shown_progress.push_str(&segment);
                                segment.clear();
                            }
                            tool_names.insert(tool_call_id, name.clone());
                            self.record_activity(
                                room_id,
                                run.id,
                                RunActivityKind::Tool,
                                &format!("调用 {name}"),
                                summarize_tool_payload(&input).as_deref(),
                            );
                        }
                        RunEvent::ToolEnd {
                            tool_call_id,
                            name,
                            ok,
                            detail,
                        } => {
                            let resolved_name = name
                                .or_else(|| {
                                    tool_call_id
                                        .as_ref()
                                        .and_then(|id| tool_names.get(id).cloned())
                                })
                                .unwrap_or_else(|| "工具".to_string());
                            let title = format!(
                                "{resolved_name}{}",
                                if ok { " 完成" } else { " 失败" }
                            );
                            self.record_activity(
                                room_id,
                                run.id,
                                RunActivityKind::Tool,
                                &title,
                                summarize_tool_payload(&Value::String(detail)).as_deref(),
                            );
                        }
                        RunEvent::Usage {
                            token_count,
                            context_size,
                            tool_call_count,
                            duration_ms,
                        } => {
                            // 先落库再广播：运行详情即使错过 WS 事件也能读取用量。
                            self.runs
                                .set_usage(run.id, token_count, context_size)
                                .await?;
                            if let Some(summary) = format_usage_summary(
                                token_count,
                                tool_call_count,
                                duration_ms,
                            ) {
                                self.record_activity(
                                    room_id,
                                    run.id,
                                    RunActivityKind::Usage,
                                    "本轮用量",
                                    Some(&summary),
                                );
                            }
                        }
                        RunEvent::Status(text) => {
                            self.record_activity(
                                room_id,
                                run.id,
                                RunActivityKind::Status,
                                "阶段提示",
                                Some(&text),
                            );
                        }
                        RunEvent::Session(harness_session_id) => {
                            // 会话 ID 在运行中途才能确定；失败运行也保留会话记录。
                            if let Some(run_session) = &run_session {
                                if let Some(service) = &self.session_service {
                                    if let Err(error) = service
                                        .record_harness_session(
                                            run_session.id,
                                            &harness_session_id,
                                            run.id,
                                        )
                                        .await
                                    {
                                        failure = Some(error.message);
                                        break;
                                    }
                                    self.runs
                                        .attach_session(run.id, &harness_session_id)
                                        .await?;
                                }
                            }
                        }
                        RunEvent::Error(message) => {
                            failure = Some(message);
                            break;
                        }
                        RunEvent::Reasoning(_) => {}
                    }
                }
            }
        }

        // 流结束时把最后一段推理落定，不丢弃已产生的活动。
        flush_reasoning(&mut reasoning_buffer);

        // 最终回复只取最后一段：阶段已作为 progress 落定，不能重复拼接。
        let final_hint = final_text.filter(|text| !text.trim().is_empty());
        let mut content = if progress_count == 0 {
            final_hint.clone().unwrap_or_else(|| segment.clone())
        } else if !segment.trim().is_empty() {
            segment.clone()
        } else if let Some(hint) = &final_hint {
            hint.strip_prefix(&shown_progress)
                .map(str::to_string)
                .unwrap_or_else(|| hint.clone())
        } else {
            String::new()
        };

        if failure.is_none() && content.trim().is_empty() {
            failure = Some("harness 未返回任何回复内容".to_string());
        }

        // 结构化输出在终态前转换：展示文本剥离协议块，后续动作等消息落地后执行。
        let mut discussion_patch: Option<DiscussionMetadataPatch> = None;
        let mut moderator_extraction: Option<ModeratorChatExtraction> = None;
        if failure.is_none() {
            if let Some(finalize) = &options.finalize {
                match finalize(&content) {
                    Ok(finalized) => {
                        if !finalized.content.trim().is_empty() {
                            content = finalized.content;
                        }
                        discussion_patch = finalized.discussion;
                    }
                    Err(error) => {
                        failure = Some(format!("输出解析失败：{}", error.message));
                    }
                }
            } else if options.moderator_chat
                && agent.builtin_role == Some(BuiltinAgentRole::Moderator)
            {
                let members = self.rooms.list_members(room_id).await?;
                let extraction = extract_moderator_action(&content, agent, &members);
                content = if extraction.text.trim().is_empty() {
                    "（主持人已完成处理）".to_string()
                } else {
                    extraction.text.clone()
                };
                moderator_extraction = Some(extraction);
            }
        }
        if let Some(message) = failure {
            let failed_content = if segment.trim().is_empty() {
                String::new()
            } else {
                segment.clone()
            };
            let failed_message = self
                .messages
                .update(
                    current_message.id,
                    UpdateMessageInput {
                        content: Some(failed_content.clone()),
                        status: Some(MessageStatus::Failed),
                        kind: Some(MessageKind::Reply),
                    },
                )
                .await?
                .unwrap_or_else(|| {
                    apply_message_update(
                        &current_message,
                        &failed_content,
                        MessageStatus::Failed,
                        MessageKind::Reply,
                    )
                });
            let finished = self
                .runs
                .finish(
                    run.id,
                    crate::store::runs::FinishRunInput {
                        status: RunStatus::Failed,
                        result: if failed_content.is_empty() {
                            None
                        } else {
                            Some(failed_content)
                        },
                        error: Some(message.clone()),
                    },
                )
                .await?
                .unwrap_or_else(|| run.clone());
            self.broadcast(room_id, RoomEvent::MessageFinal {
                room_id,
                message: failed_message,
            });
            let system_collaboration = collaboration.as_ref().map(|value| {
                MessageCollaboration {
                    batch_id: value.batch_id,
                    round: value.round,
                    role: CollaborationMessageRole::System,
                }
            });
            self.post_system_message(
                room_id,
                &format!("智能体「{}」运行失败：{message}", agent.name),
                task_id,
                discussion.clone(),
                system_collaboration,
            )
            .await?;
            tracing::error!(
                event = "agent_run_failed",
                room_id,
                agent_id = agent.id,
                run_id = run.id,
                duration_ms = started_at.elapsed().as_millis() as u64,
                error = %message,
                "agent run failed"
            );
            return Ok(finished);
        }

        writer.flush().await;
        let mut final_message = self
            .messages
            .update(
                current_message.id,
                UpdateMessageInput {
                    content: Some(content.clone()),
                    status: Some(MessageStatus::Final),
                    kind: Some(MessageKind::Reply),
                },
            )
            .await?
            .unwrap_or_else(|| {
                apply_message_update(
                    &current_message,
                    &content,
                    MessageStatus::Final,
                    MessageKind::Reply,
                )
            });
        if let Some(patch) = discussion_patch {
            if let Some(updated) = self
                .messages
                .update_discussion(
                    final_message.id,
                    UpdateMessageDiscussionInput {
                        stance: patch.stance,
                        reply_to_message_id: patch.reply_to_message_id,
                        stop_vote: patch.stop_vote,
                        stop_reason: patch.stop_reason,
                    },
                )
                .await?
            {
                final_message = updated;
            }
        }
        let finished = self
            .runs
            .finish(
                run.id,
                crate::store::runs::FinishRunInput {
                    status: RunStatus::Succeeded,
                    result: Some(content.clone()),
                    error: None,
                },
            )
            .await?
            .unwrap_or_else(|| run.clone());
        self.broadcast(room_id, RoomEvent::MessageFinal {
            room_id,
            message: final_message.clone(),
        });
        tracing::info!(
            event = "agent_run_succeeded",
            room_id,
            agent_id = agent.id,
            run_id = run.id,
            duration_ms = started_at.elapsed().as_millis() as u64,
            "agent run succeeded"
        );

        // 超长回复归档到房间工作区；供后续提示词以摘要 + 路径引用全文。
        if content.chars().count() >= ARCHIVE_MIN_CHARS {
            let archived = save_message_artifact(&ArtifactInput {
                workspace_dir: &room.workspace_dir,
                room_id,
                message_id: final_message.id,
                room_name: &room.name,
                sender_label: &agent.name,
                message_seq: final_message.seq,
                content: &content,
                created_at: final_message.created_at,
            });
            if archived.is_none() {
                tracing::warn!(
                    event = "message_archive_failed",
                    room_id,
                    agent_id = agent.id,
                    run_id = run.id,
                    message_id = final_message.id,
                    "message archive failed"
                );
            }
        }

        if let Some(extraction) = moderator_extraction {
            if let Err(error) = self
                .run_moderator_actions(
                    &room,
                    agent,
                    extraction,
                    options.moderator_trigger_message_id,
                )
                .await
            {
                tracing::error!(
                    event = "agent_run_after_failed",
                    room_id,
                    agent_id = agent.id,
                    run_id = run.id,
                    message_id = final_message.id,
                    error = %error.message,
                    "agent run after-action failed"
                );
                let collaboration = collaboration.as_ref().map(|value| MessageCollaboration {
                    batch_id: value.batch_id,
                    round: value.round,
                    role: CollaborationMessageRole::System,
                });
                self.post_system_message(
                    room_id,
                    &format!("智能体「{}」的后续动作失败：{}", agent.name, error.message),
                    task_id,
                    None,
                    collaboration,
                )
                .await?;
            }
        }

        Ok(finished)
    }

    /// 主持人回复落定后的协议动作：提示、讨论、任务或委派。
    async fn run_moderator_actions(
        &self,
        room: &Room,
        moderator: &Agent,
        extraction: ModeratorChatExtraction,
        trigger_message_id: Option<i64>,
    ) -> AppResult<()> {
        for note in &extraction.notes {
            self.post_system_message(room.id, &format!("主持人提示：{note}"), None, None, None)
                .await?;
        }

        match extraction.action {
            ModeratorChatAction::None => {}
            ModeratorChatAction::Delegate(delegation) => {
                let Some(trigger_message_id) = trigger_message_id else {
                    self.post_system_message(
                        room.id,
                        "主持人委派未执行：缺少触发消息。",
                        None,
                        None,
                        None,
                    )
                    .await?;
                    return Ok(());
                };
                self.start_collaboration(room, moderator, trigger_message_id, delegation)
                    .await?;
            }
            ModeratorChatAction::Discussion(draft) => {
                let Some(service) = self.discussion_service() else {
                    self.post_system_message(
                        room.id,
                        "主持人发起讨论失败：讨论处理器未配置",
                        None,
                        None,
                        None,
                    )
                    .await?;
                    return Ok(());
                };
                match service.start(room, moderator, draft).await {
                    Ok(discussion) => service.start_loop(discussion.id),
                    Err(error) => {
                        self.post_system_message(
                            room.id,
                            &format!("主持人发起讨论失败：{}", error.message),
                            None,
                            None,
                            None,
                        )
                        .await?;
                    }
                }
            }
            ModeratorChatAction::Task(draft) => {
                let Some(service) = self.task_service() else {
                    self.post_system_message(
                        room.id,
                        "主持人创建任务失败：任务处理器未配置",
                        None,
                        None,
                        None,
                    )
                    .await?;
                    return Ok(());
                };
                let created = service
                    .create_from_moderator(
                        room.id,
                        moderator.id,
                        crate::services::tasks::ModeratorTaskInput {
                            title: draft.title,
                            description: Some(draft.description),
                            plan: draft.plan,
                        },
                    )
                    .await;
                match created {
                    Ok(task) => {
                        if draft.auto_start {
                            if let Err(error) =
                                service.activate(&task.id.to_string()).await
                            {
                                self.post_system_message(
                                    room.id,
                                    &format!("主持人创建任务失败：{}", error.message),
                                    Some(task.id),
                                    None,
                                    None,
                                )
                                .await?;
                                return Ok(());
                            }
                            if let Some(engine) = self.orchestrator() {
                                engine.start(task.id);
                            }
                        }
                        self.post_system_message(
                            room.id,
                            &format!(
                                "主持人已创建任务 #{}{}",
                                task.id,
                                if draft.auto_start {
                                    " 并启动编排"
                                } else {
                                    "，等待人工启动编排"
                                }
                            ),
                            Some(task.id),
                            None,
                            None,
                        )
                        .await?;
                    }
                    Err(error) => {
                        self.post_system_message(
                            room.id,
                            &format!("主持人创建任务失败：{}", error.message),
                            None,
                            None,
                            None,
                        )
                        .await?;
                    }
                }
            }
        }
        Ok(())
    }

    /// 主持人委派：创建持久化批次并启动第一轮成员运行。
    async fn start_collaboration(
        &self,
        room: &Room,
        moderator: &Agent,
        user_message_id: i64,
        delegation: ModeratorDelegation,
    ) -> AppResult<()> {
        let Some(service) = self.collaborations() else {
            self.post_system_message(
                room.id,
                "主持人委派未执行：服务端未配置委派批次服务。",
                None,
                None,
                None,
            )
            .await?;
            return Ok(());
        };

        if service.store.has_running_for_room(room.id).await? {
            self.post_system_message(
                room.id,
                "已有进行中的委派批次，本次委派未执行。请等待当前批次结束或先停止它。",
                None,
                None,
                None,
            )
            .await?;
            return Ok(());
        }

        let members = self.rooms.list_members(room.id).await?;
        let batch = service
            .store
            .create_batch(CreateCollaborationBatchInput {
                room_id: room.id,
                moderator_agent_id: moderator.id,
                user_message_id,
                max_rounds: DEFAULT_COLLABORATION_MAX_ROUNDS,
            })
            .await?;

        let mut assignments = Vec::new();
        for item in delegation.assignments {
            if !members.iter().any(|member| member.id == item.agent_id) {
                self.post_system_message(
                    room.id,
                    &format!(
                        "主持人委派已跳过：智能体 #{} 已不在房间中",
                        item.agent_id
                    ),
                    None,
                    None,
                    Some(MessageCollaboration {
                        batch_id: batch.id,
                        round: 1,
                        role: CollaborationMessageRole::System,
                    }),
                )
                .await?;
                continue;
            }
            assignments.push(
                service
                    .store
                    .add_assignment(AddCollaborationAssignmentInput {
                        batch_id: batch.id,
                        round: 1,
                        agent_id: item.agent_id,
                        instruction: item.instruction,
                    })
                    .await?,
            );
        }

        if assignments.is_empty() {
            if let Some(failed) = service
                .store
                .finish(
                    batch.id,
                    FinishCollaborationBatchInput {
                        status: CollaborationStatus::Failed,
                        stop_reason: CollaborationStopReason::InternalError,
                        summary: Some("没有可执行的成员指派。".to_string()),
                    },
                )
                .await?
            {
                service.broadcast_batch(&failed);
                self.post_system_message(
                    room.id,
                    "主持人委派未执行：没有可执行的成员。",
                    None,
                    None,
                    Some(MessageCollaboration {
                        batch_id: batch.id,
                        round: 1,
                        role: CollaborationMessageRole::System,
                    }),
                )
                .await?;
            }
            return Ok(());
        }

        if let Some(created) = service.store.get_by_id(batch.id).await? {
            service.broadcast_batch(&created);
        }
        self.spawn_collaboration_round(
            room.clone(),
            moderator.clone(),
            batch.id,
            assignments,
            delegation.summary,
        );
        Ok(())
    }

    fn spawn_collaboration_round(
        &self,
        room: Room,
        moderator: Agent,
        batch_id: i64,
        assignments: Vec<CollaborationAssignment>,
        summary: String,
    ) {
        let chat = self.clone();
        tokio::spawn(async move {
            if let Err(error) = chat
                .run_collaboration_round(room, moderator, batch_id, assignments, summary)
                .await
            {
                let _ = chat
                    .fail_batch(batch_id, CollaborationStopReason::InternalError, &error.message)
                    .await;
            }
        });
    }

    async fn run_collaboration_round(
        &self,
        room: Room,
        moderator: Agent,
        batch_id: i64,
        assignments: Vec<CollaborationAssignment>,
        summary: String,
    ) -> AppResult<()> {
        let Some(service) = self.collaborations() else {
            return Ok(());
        };
        let members = self.rooms.list_members(room.id).await?;

        for assignment in assignments {
            let Some(member) = members.iter().find(|member| member.id == assignment.agent_id) else {
                service
                    .store
                    .update_assignment(
                        assignment.id,
                        UpdateCollaborationAssignmentInput {
                            status: Some(CollaborationAssignmentStatus::Skipped),
                            error: Some(Some("成员已不在房间".to_string())),
                            finished_at: Some(Some(crate::domain::now_ms())),
                            ..UpdateCollaborationAssignmentInput::default()
                        },
                    )
                    .await?;
                continue;
            };

            // 先标记 running，让批次卡片在运行期间可见。
            if let Some(updated) = service
                .store
                .update_assignment(
                    assignment.id,
                    UpdateCollaborationAssignmentInput {
                        status: Some(CollaborationAssignmentStatus::Running),
                        ..UpdateCollaborationAssignmentInput::default()
                    },
                )
                .await?
            {
                let _ = updated;
                if let Some(batch) = service.store.get_by_id(batch_id).await? {
                    service.broadcast_batch(&batch);
                }
            }

            let instruction = build_delegated_instruction(
                &ChatAssignment {
                    agent_id: assignment.agent_id,
                    instruction: assignment.instruction.clone(),
                },
                &summary,
            );
            let options = RunAgentOptions {
                collaboration: Some(MessageCollaboration {
                    batch_id,
                    round: assignment.round,
                    role: CollaborationMessageRole::Assignment,
                }),
                ..RunAgentOptions::default()
            };
            let guard_store = service.store.clone();
            let run = self
                .run_agent_queued_checked(
                    room.id,
                    member,
                    &instruction,
                    None,
                    options,
                    move || {
                        let store = guard_store.clone();
                        async move {
                            matches!(
                                store.get_by_id(batch_id).await,
                                Ok(Some(current))
                                    if current.status == CollaborationStatus::Running
                                        && !current.stop_requested
                            )
                        }
                    },
                )
                .await;

            let now = crate::domain::now_ms();
            let patch = match run {
                Ok(None) => UpdateCollaborationAssignmentInput {
                    status: Some(CollaborationAssignmentStatus::Skipped),
                    error: Some(Some("批次已停止".to_string())),
                    finished_at: Some(Some(now)),
                    ..UpdateCollaborationAssignmentInput::default()
                },
                Ok(Some(run)) => {
                    let message_id = self.last_reply_message_id(run.id).await?;
                    if run.status == RunStatus::Succeeded {
                        UpdateCollaborationAssignmentInput {
                            status: Some(CollaborationAssignmentStatus::Succeeded),
                            run_id: Some(Some(run.id)),
                            message_id: Some(message_id),
                            finished_at: Some(Some(now)),
                            ..UpdateCollaborationAssignmentInput::default()
                        }
                    } else {
                        UpdateCollaborationAssignmentInput {
                            status: Some(CollaborationAssignmentStatus::Failed),
                            run_id: Some(Some(run.id)),
                            message_id: Some(message_id),
                            error: Some(Some(
                                run.error.unwrap_or_else(|| "运行失败".to_string()),
                            )),
                            finished_at: Some(Some(now)),
                            ..UpdateCollaborationAssignmentInput::default()
                        }
                    }
                }
                Err(error) => UpdateCollaborationAssignmentInput {
                    status: Some(CollaborationAssignmentStatus::Failed),
                    error: Some(Some(error.message)),
                    finished_at: Some(Some(now)),
                    ..UpdateCollaborationAssignmentInput::default()
                },
            };
            service.store.update_assignment(assignment.id, patch).await?;
            if let Some(batch) = service.store.get_by_id(batch_id).await? {
                service.broadcast_batch(&batch);
            }
        }

        self.on_round_settled(batch_id, moderator).await
    }

    async fn last_reply_message_id(&self, run_id: i64) -> AppResult<Option<i64>> {
        Ok(self
            .messages
            .list_by_run(run_id)
            .await?
            .into_iter()
            .rev()
            .find(|message| message.kind == MessageKind::Reply)
            .map(|message| message.id))
    }

    async fn on_round_settled(&self, batch_id: i64, moderator: Agent) -> AppResult<()> {
        let Some(service) = self.collaborations() else {
            return Ok(());
        };
        let Some(batch) = service.store.get_by_id(batch_id).await? else {
            return Ok(());
        };
        if batch.status != CollaborationStatus::Running {
            return Ok(());
        }
        if batch.stop_requested {
            self.finish_batch_stopped(&batch).await?;
            return Ok(());
        }
        self.schedule_review(batch_id, moderator).await
    }

    async fn schedule_review(&self, batch_id: i64, moderator: Agent) -> AppResult<()> {
        let Some(service) = self.collaborations() else {
            return Ok(());
        };
        let Some(batch) = service.store.get_by_id(batch_id).await? else {
            return Ok(());
        };
        if batch.status != CollaborationStatus::Running {
            return Ok(());
        }
        if batch.stop_requested {
            self.finish_batch_stopped(&batch).await?;
            return Ok(());
        }
        if !service
            .store
            .claim_review(batch.id, batch.current_round)
            .await?
        {
            return Ok(());
        }

        let Some(room) = self.rooms.get_by_id(batch.room_id).await? else {
            self.fail_batch(
                batch.id,
                CollaborationStopReason::InternalError,
                "委派批次所在房间已不存在。",
            )
            .await?;
            return Ok(());
        };

        let results = self.build_round_review_results(&batch).await?;
        let user_request = self
            .messages
            .get_by_id(batch.user_message_id)
            .await?
            .map(|message| message.content)
            .unwrap_or_default();
        let members = self.rooms.list_members(room.id).await?;
        let instruction = build_round_review_instruction(
            &room,
            &moderator,
            &members,
            &batch,
            &user_request,
            &results,
        );

        let parsed: Arc<Mutex<Option<ParsedRoundReview>>> = Arc::new(Mutex::new(None));
        let slot = parsed.clone();
        let moderator_for_parse = moderator.clone();
        let members_for_parse = members.clone();
        let batch_for_parse = batch.clone();
        let finalize: FinalizeFn = Arc::new(move |raw| {
            let review = parse_round_review(
                raw,
                &moderator_for_parse,
                &members_for_parse,
                batch_for_parse.current_round,
                batch_for_parse.max_rounds,
            );
            let content = review.review.summary.clone();
            *slot.lock().unwrap_or_else(|error| error.into_inner()) = Some(review);
            Ok(FinalizedContent {
                content,
                discussion: None,
            })
        });

        let options = RunAgentOptions {
            stream: Some(false),
            skip_room_context: true,
            collaboration: Some(MessageCollaboration {
                batch_id: batch.id,
                round: batch.current_round,
                role: CollaborationMessageRole::Summary,
            }),
            finalize: Some(finalize),
            ..RunAgentOptions::default()
        };
        let guard_store = service.store.clone();
        let run = self
            .run_agent_queued_checked(room.id, &moderator, &instruction, None, options, move || {
                let store = guard_store.clone();
                async move {
                    matches!(
                        store.get_by_id(batch_id).await,
                        Ok(Some(current)) if current.status == CollaborationStatus::Running
                    )
                }
            })
            .await?;

        let Some(run) = run else {
            if let Some(current) = service.store.get_by_id(batch_id).await? {
                if current.status == CollaborationStatus::Running && current.stop_requested {
                    self.finish_batch_stopped(&current).await?;
                }
            }
            return Ok(());
        };

        if let Some(updated) = service.store.set_review_run_id(batch_id, run.id).await? {
            service.broadcast_batch(&updated);
        }

        if run.status != RunStatus::Succeeded {
            self.fail_batch(
                batch_id,
                CollaborationStopReason::ModeratorFailed,
                "主持人复核运行失败，委派批次已终止。",
            )
            .await?;
            return Ok(());
        }

        let Some(parsed_review) = parsed.lock().unwrap_or_else(|error| error.into_inner()).take()
        else {
            self.fail_batch(
                batch_id,
                CollaborationStopReason::ModeratorFailed,
                "主持人复核输出无法解析，委派批次已终止。",
            )
            .await?;
            return Ok(());
        };
        for note in &parsed_review.notes {
            self.post_batch_system_message(&batch, &format!("主持人提示：{note}"))
                .await?;
        }
        self.apply_round_review(batch_id, moderator, parsed_review)
            .await
    }

    async fn build_round_review_results(
        &self,
        batch: &CollaborationBatch,
    ) -> AppResult<Vec<RoundReviewMemberResult>> {
        let mut results = Vec::new();
        for assignment in batch
            .assignments
            .iter()
            .filter(|assignment| assignment.round == batch.current_round)
        {
            let member = self.agents.get_by_id(assignment.agent_id).await?;
            let output = match assignment.message_id {
                Some(message_id) => self
                    .messages
                    .get_by_id(message_id)
                    .await?
                    .map(|message| message.content)
                    .unwrap_or_default(),
                None => String::new(),
            };
            results.push(RoundReviewMemberResult {
                agent_id: assignment.agent_id,
                name: member
                    .map(|member| member.name)
                    .unwrap_or_else(|| format!("智能体#{}", assignment.agent_id)),
                status: assignment.status,
                instruction: assignment.instruction.clone(),
                output,
                error: assignment.error.clone(),
            });
        }
        Ok(results)
    }

    async fn apply_round_review(
        &self,
        batch_id: i64,
        moderator: Agent,
        parsed: ParsedRoundReview,
    ) -> AppResult<()> {
        let Some(service) = self.collaborations() else {
            return Ok(());
        };
        let Some(batch) = service.store.get_by_id(batch_id).await? else {
            return Ok(());
        };
        if batch.status != CollaborationStatus::Running {
            return Ok(());
        }
        if batch.stop_requested {
            self.finish_batch_stopped(&batch).await?;
            return Ok(());
        }

        if parsed.review.decision == "stop" {
            if let Some(finished) = service
                .store
                .finish(
                    batch_id,
                    FinishCollaborationBatchInput {
                        status: CollaborationStatus::Completed,
                        stop_reason: parsed
                            .stop_reason
                            .unwrap_or(CollaborationStopReason::ModeratorStop),
                        summary: Some(parsed.review.summary),
                    },
                )
                .await?
            {
                service.broadcast_batch(&finished);
            }
            return Ok(());
        }

        if batch.current_round >= batch.max_rounds {
            if let Some(finished) = service
                .store
                .finish(
                    batch_id,
                    FinishCollaborationBatchInput {
                        status: CollaborationStatus::Completed,
                        stop_reason: CollaborationStopReason::MaxRounds,
                        summary: Some(parsed.review.summary),
                    },
                )
                .await?
            {
                service.broadcast_batch(&finished);
            }
            return Ok(());
        }

        let Some(room) = self.rooms.get_by_id(batch.room_id).await? else {
            self.fail_batch(
                batch_id,
                CollaborationStopReason::InternalError,
                "委派批次所在房间已不存在。",
            )
            .await?;
            return Ok(());
        };

        let next_round = batch.current_round + 1;
        let mut next_assignments = Vec::new();
        for item in parsed.review.next_assignments {
            next_assignments.push(
                service
                    .store
                    .add_assignment(AddCollaborationAssignmentInput {
                        batch_id,
                        round: next_round,
                        agent_id: item.agent_id,
                        instruction: item.instruction,
                    })
                    .await?,
            );
        }
        service.store.advance_round(batch_id, next_round).await?;
        if let Some(advanced) = service.store.get_by_id(batch_id).await? {
            service.broadcast_batch(&advanced);
        }
        self.spawn_collaboration_round(room, moderator, batch_id, next_assignments, String::new());
        Ok(())
    }

    async fn finish_batch_stopped(&self, batch: &CollaborationBatch) -> AppResult<()> {
        let Some(service) = self.collaborations() else {
            return Ok(());
        };
        let Some(finished) = service
            .store
            .finish(
                batch.id,
                FinishCollaborationBatchInput {
                    status: CollaborationStatus::Stopped,
                    stop_reason: CollaborationStopReason::UserStop,
                    summary: Some("用户停止了委派批次。".to_string()),
                },
            )
            .await?
        else {
            return Ok(());
        };
        service.broadcast_batch(&finished);
        self.post_batch_system_message(&finished, "主持人委派已停止。")
            .await
            .map(|_| ())
    }

    async fn fail_batch(
        &self,
        batch_id: i64,
        reason: CollaborationStopReason,
        message: &str,
    ) -> AppResult<()> {
        let Some(service) = self.collaborations() else {
            return Ok(());
        };
        let Some(finished) = service
            .store
            .finish(
                batch_id,
                FinishCollaborationBatchInput {
                    status: CollaborationStatus::Failed,
                    stop_reason: reason,
                    summary: Some(message.to_string()),
                },
            )
            .await?
        else {
            return Ok(());
        };
        service.broadcast_batch(&finished);
        self.post_batch_system_message(&finished, message)
            .await
            .map(|_| ())
    }

    async fn post_batch_system_message(
        &self,
        batch: &CollaborationBatch,
        content: &str,
    ) -> AppResult<Message> {
        self.post_system_message(
            batch.room_id,
            content,
            None,
            None,
            Some(MessageCollaboration {
                batch_id: batch.id,
                round: batch.current_round,
                role: CollaborationMessageRole::System,
            }),
        )
        .await
    }

    fn collaborations(&self) -> Option<Arc<CollaborationService>> {
        self.collaborations
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    fn orchestrator(&self) -> Option<Arc<OrchestratorEngine>> {
        self.orchestrator
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .and_then(Weak::upgrade)
    }

    fn discussion_service(&self) -> Option<Arc<DiscussionService>> {
        self.discussion_service
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .and_then(Weak::upgrade)
    }

    fn task_service(&self) -> Option<Arc<TaskService>> {
        self.task_service
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .and_then(Weak::upgrade)
    }

    /// 系统消息与聊天共用落库/广播路径。
    pub async fn post_system_message(
        &self,
        room_id: i64,
        content: &str,
        task_id: Option<i64>,
        discussion: Option<MessageDiscussion>,
        collaboration: Option<MessageCollaboration>,
    ) -> AppResult<Message> {
        self.require_room(room_id).await?;
        let message = self
            .messages
            .append(
                AppendMessageInput::final_message(
                    room_id,
                    MessageSenderType::System,
                    None,
                    content,
                )
                .with_task(task_id)
                .with_discussion(discussion)
                .with_collaboration(collaboration),
            )
            .await?;
        self.broadcast(room_id, RoomEvent::MessageFinal {
            room_id,
            message: message.clone(),
        });
        Ok(message)
    }

    fn broadcast(&self, room_id: i64, event: RoomEvent) {
        (self.broadcast)(room_id, &event);
    }

    fn record_activity(
        &self,
        room_id: i64,
        run_id: i64,
        kind: RunActivityKind,
        title: &str,
        detail: Option<&str>,
    ) {
        let activity = self.activity.record(run_id, kind, title, detail);
        self.broadcast(room_id, RoomEvent::RunActivity {
            room_id,
            activity,
        });
    }

    async fn require_room(&self, room_id: i64) -> AppResult<Room> {
        self.rooms
            .get_by_id(room_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("房间不存在：{room_id}")))
    }

    /// 构建运行提示词：身份与人设 + 房间最近对话 + 当前指令。
    async fn build_prompt(
        &self,
        room: &Room,
        agent: &Agent,
        history: &[Message],
        instruction: &str,
        options: &RunAgentOptions,
    ) -> AppResult<String> {
        // 主持人普通聊天走专用协议提示词；编排循环的提示词由引擎自建。
        if options.moderator_chat && agent.builtin_role == Some(BuiltinAgentRole::Moderator) {
            let members = self.rooms.list_members(room.id).await?;
            let settings = self.settings.get().await?.settings;
            let compaction = CompactionOptions::new(
                settings.context_compression_enabled,
                settings.context_budget_chars,
            );
            return Ok(build_moderator_chat_instruction(
                room,
                agent,
                &members,
                history,
                instruction,
                &compaction,
            ));
        }

        if options.skip_room_context {
            return Ok(instruction.to_string());
        }

        let settings = self.settings.get().await?.settings;
        let compaction = CompactionOptions::new(
            settings.context_compression_enabled,
            settings.context_budget_chars,
        );
        let context = self
            .build_history_transcript(room, history, &compaction)
            .await?;

        let mut lines = vec![
            format!("你是房间「{}」中的智能体「{}」。", room.name, agent.name),
            format!("人设：{}", agent.persona),
            String::new(),
            format!("房间最近对话（按时间顺序，最多 {CONTEXT_MAX_MESSAGES} 条）："),
        ];
        if context.compressed > 0 || context.omitted > 0 {
            lines.push(format!(
                "（已压缩 {} 条早期消息，省略 {} 条）",
                context.compressed, context.omitted
            ));
        }
        lines.push(context.transcript);
        lines.extend([
            String::new(),
            "当前需要你处理的内容：".to_string(),
            instruction.to_string(),
            String::new(),
            format!(
                "请以「{}」的身份直接给出回复。只输出回复正文，不要复述对话记录，不要输出思考过程。",
                agent.name
            ),
        ]);
        if options.progress != Some(false) {
            lines.push(STAGE_SUMMARY_RULE.to_string());
        }
        lines.push(CONCISE_REPLY_RULE.to_string());
        Ok(lines.join("\n"))
    }

    /// 构建房间历史转录；真实提示词与上下文预览共用。
    async fn build_history_transcript(
        &self,
        room: &Room,
        history: &[Message],
        compaction: &CompactionOptions,
    ) -> AppResult<CompactionResult> {
        let members = self.rooms.list_members(room.id).await?;
        let mut member_names = HashMap::new();
        for member in &members {
            member_names.insert(member.id, member.name.clone());
        }

        let mut resolved_sender_names: HashMap<i64, String> = HashMap::new();
        let recent = context_messages(history);
        let mut labeled: Vec<(String, &Message)> = Vec::with_capacity(recent.len());
        for message in &recent {
            let label = match message.sender_type {
                MessageSenderType::System => "系统".to_string(),
                MessageSenderType::User => "用户".to_string(),
                MessageSenderType::Agent => match message.sender_agent_id {
                    None => "智能体".to_string(),
                    Some(sender_id) => {
                        if let Some(name) = member_names.get(&sender_id) {
                            name.clone()
                        } else if let Some(name) = resolved_sender_names.get(&sender_id) {
                            name.clone()
                        } else {
                            let name = self
                                .agents
                                .get_by_id(sender_id)
                                .await?
                                .map(|agent| agent.name)
                                .unwrap_or_else(|| format!("智能体#{sender_id}"));
                            resolved_sender_names.insert(sender_id, name.clone());
                            name
                        }
                    }
                },
            };
            labeled.push((label, message));
        }

        let labeled: Vec<LabeledMessage<'_>> = labeled
            .iter()
            .map(|(label, message)| LabeledMessage {
                label: label.as_str(),
                message,
            })
            .collect();
        Ok(compact_transcript(
            &labeled,
            |message, label| compact_message_line(room, message, label),
            compaction,
        ))
    }

    /// 发送前的上下文估算；会话可续接时只统计增量指令。
    pub async fn context_preview(
        &self,
        raw_room_id: &str,
        query: &HashMap<String, String>,
    ) -> AppResult<ContextPreview> {
        let room_id = parse_id_label(raw_room_id, "房间")?;
        let room = self.require_room(room_id).await?;
        let raw_agent_id = query
            .get("agentId")
            .ok_or_else(|| AppError::validation("缺少 agentId"))?;
        let agent_id = parse_id_label(raw_agent_id, "智能体")?;
        let agent = self
            .agents
            .get_by_id(agent_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("智能体不存在：{agent_id}")))?;
        if !self
            .rooms
            .list_members(room_id)
            .await?
            .iter()
            .any(|member| member.id == agent_id)
        {
            return Err(AppError::not_found(format!(
                "智能体不是房间成员：{agent_id}"
            )));
        }

        let instruction = query
            .get("instruction")
            .map(String::as_str)
            .unwrap_or("");
        let settings = self.settings.get().await?.settings;
        // 与 `resolve_run_session` 保持一致：只要成员开启复用，运行就会清空
        // 房间历史（首次复用也不例外），预览必须如实反映。
        let session_reused = self
            .rooms
            .list_member_session_settings(room_id)
            .await?
            .into_iter()
            .any(|item| item.agent_id == agent.id && item.reuse_session);
        if session_reused {
            let chars = instruction.chars().count();
            return Ok(ContextPreview {
                enabled: settings.context_compression_enabled,
                budget_chars: settings.context_budget_chars,
                raw_chars: chars,
                effective_chars: chars,
                estimated_tokens: estimate_tokens(chars),
                included_messages: 0,
                compressed_messages: 0,
                omitted_messages: 0,
                session_reused: true,
            });
        }

        let history = self.messages.list_by_room(room_id).await?;
        let compaction = CompactionOptions::new(
            settings.context_compression_enabled,
            settings.context_budget_chars,
        );
        let context = self
            .build_history_transcript(&room, &history, &compaction)
            .await?;
        Ok(ContextPreview {
            enabled: settings.context_compression_enabled,
            budget_chars: settings.context_budget_chars,
            raw_chars: context.raw_chars,
            effective_chars: context.effective_chars,
            estimated_tokens: estimate_tokens(context.effective_chars),
            included_messages: context.included,
            compressed_messages: context.compressed,
            omitted_messages: context.omitted,
            session_reused: false,
        })
    }
}

/// 根据运行选项推导队列展示信息；普通聊天与主持人可取消，其余不可取消。
fn queue_job_info(
    agent: &Agent,
    task_id: Option<i64>,
    options: &RunAgentOptions,
) -> QueueJobInfo {
    let kind = if options.discussion.is_some() {
        QueueJobKind::Discussion
    } else if options.collaboration.is_some() {
        QueueJobKind::Collaboration
    } else if task_id.is_some() {
        QueueJobKind::Task
    } else if options.moderator_chat {
        QueueJobKind::Moderator
    } else {
        QueueJobKind::Chat
    };
    QueueJobInfo {
        agent_id: Some(agent.id),
        label: agent.name.clone(),
        kind,
        cancelable: matches!(kind, QueueJobKind::Chat | QueueJobKind::Moderator),
    }
}

/// 解析消息附件 id：缺省为空；只接受正整数数组。
fn parse_attachment_ids(value: Option<&Value>) -> AppResult<Vec<i64>> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                crate::services::as_positive_id(item)
                    .ok_or_else(|| AppError::validation("attachmentIds 必须是正整数数组"))
            })
            .collect(),
        Some(_) => Err(AppError::validation("attachmentIds 必须是正整数数组")),
    }
}

/// 解析普通消息的回复目标；缺省与 null 都表示不回复。
fn parse_reply_to_id(value: Option<&Value>) -> AppResult<Option<i64>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(item) => crate::services::as_positive_id(item)
            .map(Some)
            .ok_or_else(|| AppError::validation("replyToId 必须是正整数")),
    }
}

/// 把被回复消息压成一行引用，作为本轮指令的上下文前缀。
fn reply_context(content: &str, target: &Message, sender_label: &str) -> String {
    let flattened = target.content.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = flattened.chars();
    let snippet: String = chars.by_ref().take(120).collect();
    let ellipsis = if chars.next().is_some() { "…" } else { "" };
    format!("（回复 {sender_label}：{snippet}{ellipsis}）\n{content}")
}

/// 解析消息中的 @提及，只返回房间成员，按文本出现顺序去重。
pub fn parse_mentions(content: &str, members: &[Agent]) -> Vec<Agent> {
    if members.is_empty() || !content.contains('@') {
        return Vec::new();
    }

    let mut names: Vec<(&str, &Agent)> = members
        .iter()
        .map(|member| (member.name.as_str(), member))
        .collect();
    // 长名称优先，避免短名称成为长名称的前缀时抢占匹配。
    names.sort_by(|left, right| right.0.chars().count().cmp(&left.0.chars().count()));

    let mut seen = HashSet::new();
    let mut mentioned = Vec::new();
    let mut offset = 0_usize;
    while offset < content.len() {
        let Some(character) = content[offset..].chars().next() else {
            break;
        };
        if character != '@' {
            offset += character.len_utf8();
            continue;
        }

        let rest_start = offset + character.len_utf8();
        let rest = &content[rest_start..];
        let matched = names.iter().find(|(name, _)| {
            rest.starts_with(*name) && is_mention_boundary(&rest[name.len()..])
        });
        let Some((name, agent)) = matched else {
            offset = rest_start;
            continue;
        };

        if seen.insert(agent.id) {
            mentioned.push((*agent).clone());
        }
        offset = rest_start + name.len();
    }
    mentioned
}

fn is_mention_boundary(rest: &str) -> bool {
    match rest.chars().next() {
        None => true,
        Some(character) => {
            character.is_whitespace() || MENTION_TERMINATORS.contains(character)
        }
    }
}

/// 用户是否在请求「继续刚才中断的事」。
///
/// 只接受短指令本身，避免「继续优化接口」这类新请求误触发续跑。
fn is_resume_intent(content: &str) -> bool {
    matches!(
        content.trim().to_lowercase().as_str(),
        "继续"
            | "继续吧"
            | "继续讨论"
            | "接着"
            | "接着来"
            | "接着继续"
            | "重试"
            | "重试一下"
            | "continue"
    )
}

fn parse_run_limit(raw: Option<&str>) -> i64 {
    let Some(text) = raw else {
        return RUN_LIST_DEFAULT_LIMIT as i64;
    };
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return RUN_LIST_DEFAULT_LIMIT as i64;
    }
    match text.parse::<i64>() {
        Ok(value) if value > 0 => value.min(RUN_LIST_MAX_LIMIT as i64),
        _ => RUN_LIST_DEFAULT_LIMIT as i64,
    }
}

fn apply_message_update(
    existing: &Message,
    content: &str,
    status: MessageStatus,
    kind: MessageKind,
) -> Message {
    Message {
        content: content.to_string(),
        status,
        kind,
        ..existing.clone()
    }
}

/// 流式消息写入器：内存累积、按固定间隔节流入库，并立即广播增量。
struct StreamingWriter<'a> {
    messages: &'a MessageStore,
    broadcast: &'a BroadcastFn,
    room_id: i64,
    message_id: i64,
    buffered: String,
    persisted: String,
    last_persist: Instant,
}

impl<'a> StreamingWriter<'a> {
    fn new(
        messages: &'a MessageStore,
        broadcast: &'a BroadcastFn,
        room_id: i64,
        message_id: i64,
    ) -> Self {
        Self {
            messages,
            broadcast,
            room_id,
            message_id,
            buffered: String::new(),
            persisted: String::new(),
            last_persist: Instant::now(),
        }
    }

    async fn push(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.buffered.push_str(text);
        (self.broadcast)(self.room_id, &RoomEvent::MessageDelta {
            room_id: self.room_id,
            message_id: self.message_id,
            text: text.to_string(),
        });
        if self.last_persist.elapsed() >= Duration::from_millis(STREAM_FLUSH_INTERVAL_MS) {
            self.persist().await;
        }
    }

    async fn flush(&mut self) {
        self.persist().await;
    }

    async fn persist(&mut self) {
        if self.buffered == self.persisted {
            return;
        }
        let _ = self
            .messages
            .update(
                self.message_id,
                UpdateMessageInput {
                    content: Some(self.buffered.clone()),
                    ..UpdateMessageInput::default()
                },
            )
            .await;
        self.persisted = self.buffered.clone();
        self.last_persist = Instant::now();
    }
}

/// 运行活动快照类型在 chat 内部通过 registry 产出；此别名避免调用方误用。
pub type ChatRunActivity = RunActivity;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::HarnessKind;

    fn agent(id: i64, name: &str) -> Agent {
        Agent {
            id,
            name: name.to_string(),
            persona: String::new(),
            harness: HarnessKind::Fake,
            model_config: None,
            builtin_role: None,
            custom_harness_id: None,
            workspace_dir: None,
            created_at: 0,
        }
    }

    #[test]
    fn parses_mentions_longest_first_and_deduped() {
        let members = vec![agent(1, "a"), agent(2, "ab"), agent(3, "小明")];
        assert_eq!(
            parse_mentions("@ab 与 @a 和 @小明，再来一次 @a", &members)
                .iter()
                .map(|value| value.id)
                .collect::<Vec<_>>(),
            vec![2, 1, 3]
        );
        assert!(parse_mentions("@abc", &members).is_empty());
        assert!(parse_mentions("邮箱 a@b.com", &members).is_empty());
    }

    #[test]
    fn mention_boundaries_include_punctuation_and_end() {
        let members = vec![agent(1, "小明")];
        assert_eq!(parse_mentions("@小明", &members).len(), 1);
        assert_eq!(parse_mentions("@小明。", &members).len(), 1);
        assert_eq!(parse_mentions("@小明abc", &members).len(), 0);
    }

    #[test]
    fn parses_run_limits() {
        assert_eq!(parse_run_limit(None), 20);
        assert_eq!(parse_run_limit(Some("abc")), 20);
        assert_eq!(parse_run_limit(Some("0")), 20);
        assert_eq!(parse_run_limit(Some("7")), 7);
        assert_eq!(parse_run_limit(Some("999")), 100);
    }

    #[test]
    fn resume_intent_only_matches_short_continuations() {
        for text in ["继续", " 继续吧 ", "继续讨论", "接着", "重试", "Continue"] {
            assert!(is_resume_intent(text), "应命中：{text}");
        }
        for text in ["继续优化接口", "继续讨论一下团购券接口", "重试一下数据库", "新的请求"] {
            assert!(!is_resume_intent(text), "不应命中：{text}");
        }
    }

    fn reply_target(content: &str) -> Message {
        Message {
            id: 7,
            room_id: 1,
            seq: 3,
            sender_type: MessageSenderType::Agent,
            sender_agent_id: Some(2),
            status: MessageStatus::Final,
            kind: MessageKind::Reply,
            run_id: None,
            content: content.to_string(),
            task_id: None,
            reply_to_message_id: None,
            discussion: None,
            collaboration: None,
            attachments: Vec::new(),
            reactions: Vec::new(),
            created_at: 0,
        }
    }

    #[test]
    fn reply_context_flattens_and_truncates_reference() {
        let target = reply_target(&format!("第一行\n\n  第二行 {}", "长".repeat(130)));
        let instruction = reply_context("请解释这里", &target, "小明");
        assert!(instruction.starts_with("（回复 小明：第一行 第二行 "));
        assert!(instruction.ends_with("…）\n请解释这里"));
        let quote_line = instruction.lines().next().unwrap();
        let snippet = quote_line
            .strip_prefix("（回复 小明：")
            .unwrap()
            .strip_suffix("）")
            .unwrap();
        // 120 字摘要 + 一个省略号。
        assert_eq!(snippet.chars().count(), 121);
    }

    #[test]
    fn reply_context_keeps_short_reference_unchanged() {
        let instruction = reply_context("继续", &reply_target("原消息"), "我");
        assert_eq!(instruction, "（回复 我：原消息）\n继续");
    }

    #[test]
    fn parse_reply_to_id_accepts_only_positive_integers() {
        assert_eq!(parse_reply_to_id(None).unwrap(), None);
        assert_eq!(parse_reply_to_id(Some(&Value::Null)).unwrap(), None);
        assert_eq!(
            parse_reply_to_id(Some(&serde_json::json!(12))).unwrap(),
            Some(12)
        );
        assert!(parse_reply_to_id(Some(&serde_json::json!(0))).is_err());
        assert!(parse_reply_to_id(Some(&serde_json::json!(-1))).is_err());
        assert!(parse_reply_to_id(Some(&serde_json::json!(1.5))).is_err());
        assert!(parse_reply_to_id(Some(&serde_json::json!("12"))).is_err());
    }
}
