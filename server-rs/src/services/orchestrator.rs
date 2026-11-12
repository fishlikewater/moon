//! 主持人自动编排引擎。
//!
//! 不变量：
//! - 同一任务同一时刻最多一个编排循环；
//! - 同一房间的成员运行经 ChatService/房间队列串行；
//! - 每轮开始前与每次出队派发前都检查房间暂停开关；
//! - 轮次上限固定为 `MAX_ROUNDS`，不可配置关闭；
//! - 任何失败都以「任务 blocked + 系统消息」显式暴露。

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;

use crate::domain::agent::Agent;
use crate::domain::event::RoomEvent;
use crate::domain::message::{Message, MessageSenderType};
use crate::domain::room::Room;
use crate::domain::task::Task;
use crate::error::{AppError, AppResult};
use crate::services::chat::{BroadcastFn, ChatService, RunAgentOptions};
use crate::services::moderator::{
    build_moderator_instruction, format_assignment_instruction, parse_plan, AssignInstruction,
    AssignmentKind, ModeratorPlan, PlanParseError,
};
use crate::services::tasks::TaskService;
use crate::store::messages::AppendMessageInput;
use crate::store::{AgentStore, MessageStore, RoomStore, TaskStore};

/// 最大编排轮次；模块常量，不可通过配置关闭熔断。
pub const MAX_ROUNDS: i64 = 8;
/// 主持人计划解析失败后的总尝试次数（首次 + 重试 1 次）。
const MODERATOR_PLAN_ATTEMPTS: usize = 2;
/// 暂停期间轮询恢复的间隔；固定值。
pub const PAUSE_POLL_INTERVAL_MS: u64 = 50;

/// 房间级暂停开关。
#[derive(Default)]
pub struct PauseControl {
    paused_rooms: Mutex<HashSet<i64>>,
}

impl PauseControl {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn pause(&self, room_id: i64) {
        self.paused_rooms
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(room_id);
    }

    pub fn resume(&self, room_id: i64) {
        self.paused_rooms
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&room_id);
    }

    pub fn is_paused(&self, room_id: i64) -> bool {
        self.paused_rooms
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains(&room_id)
    }

    pub async fn assert_not_paused(&self, room_id: i64) {
        while self.is_paused(room_id) {
            tokio::time::sleep(Duration::from_millis(PAUSE_POLL_INTERVAL_MS)).await;
        }
    }
}

enum EngineFailure {
    Plan(PlanParseError),
    App(AppError),
    Message(String),
}

impl From<AppError> for EngineFailure {
    fn from(error: AppError) -> Self {
        Self::App(error)
    }
}

impl std::fmt::Display for EngineFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Plan(error) => write!(formatter, "{error}"),
            Self::App(error) => write!(formatter, "{}", error.message),
            Self::Message(message) => formatter.write_str(message),
        }
    }
}

struct ModeratorContext {
    task: Task,
    room: Room,
    members: Vec<Agent>,
    moderator: Agent,
    round: i64,
}

pub struct OrchestratorEngine {
    rooms: RoomStore,
    tasks: TaskStore,
    agents: AgentStore,
    messages: MessageStore,
    chat: Arc<ChatService>,
    task_service: Arc<TaskService>,
    broadcast: BroadcastFn,
    pause_control: Arc<PauseControl>,
    /// taskId → 当前编排循环；用于重复启动检测。
    active: Mutex<HashSet<i64>>,
}

impl OrchestratorEngine {
    pub fn new(
        rooms: RoomStore,
        tasks: TaskStore,
        agents: AgentStore,
        messages: MessageStore,
        chat: Arc<ChatService>,
        task_service: Arc<TaskService>,
        broadcast: BroadcastFn,
        pause_control: Arc<PauseControl>,
    ) -> Self {
        Self {
            rooms,
            tasks,
            agents,
            messages,
            chat,
            task_service,
            broadcast,
            pause_control,
            active: Mutex::new(HashSet::new()),
        }
    }

    /// 该任务是否已有编排循环在运行。
    pub fn is_running(&self, task_id: i64) -> bool {
        self.active
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains(&task_id)
    }

    /// 启动编排；同一任务重复调用幂等，不产生第二个循环。
    pub fn start(self: &Arc<Self>, task_id: i64) {
        {
            let mut active = self
                .active
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if !active.insert(task_id) {
                return;
            }
        }
        let engine = Arc::clone(self);
        tokio::spawn(async move {
            if let Err(error) = engine.execute(task_id).await {
                tracing::error!(
                    event = "orchestration_failed",
                    task_id,
                    error = %error,
                    "orchestration failed"
                );
            }
            engine
                .active
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .remove(&task_id);
        });
    }

    async fn execute(&self, task_id: i64) -> Result<(), EngineFailure> {
        let Some(task) = self.tasks.get_by_id(task_id).await? else {
            return Err(EngineFailure::App(AppError::not_found(format!(
                "任务不存在：{task_id}"
            ))));
        };
        let Some(room) = self.rooms.get_by_id(task.room_id).await? else {
            return Err(EngineFailure::App(AppError::not_found(format!(
                "房间不存在：{}",
                task.room_id
            ))));
        };
        let moderator = self.require_moderator(&room).await?;
        let members = self.rooms.list_members(room.id).await?;

        tracing::info!(
            event = "orchestration_started",
            task_id,
            agent_id = moderator.id,
            "orchestration started"
        );

        let mut executed_work = 0_usize;
        let mut completed_reviews = 0_usize;
        let mut review_gate_corrected = false;

        let outcome: Result<(), EngineFailure> = async {
            for round in 1..=MAX_ROUNDS {
                self.pause_control.assert_not_paused(room.id).await;

                let plan = self
                    .request_plan(ModeratorContext {
                        task: task.clone(),
                        room: room.clone(),
                        members: members.clone(),
                        moderator: moderator.clone(),
                        round,
                    })
                    .await?;

                match plan {
                    ModeratorPlan::Done {
                        summary,
                        verification,
                    } => {
                        if executed_work == 0 {
                            return Err(EngineFailure::Message(
                                "主持人尚未安排任何成员执行工作，不能宣布完成".to_string(),
                            ));
                        }
                        if completed_reviews == 0 {
                            if !review_gate_corrected {
                                review_gate_corrected = true;
                                self.post_system_message(
                                    room.id,
                                    "主持人计划缺少成员复核：请先安排至少一次 kind:\"review\" 的评审并取得结论，再输出 done。",
                                    task_id,
                                )
                                .await?;
                                tracing::warn!(
                                    event = "review_gate_rejected",
                                    task_id,
                                    agent_id = moderator.id,
                                    round,
                                    "review gate rejected"
                                );
                                continue;
                            }
                            return Err(EngineFailure::Message(
                                "主持人连续两轮未安排成员复核，任务不能进入评审".to_string(),
                            ));
                        }

                        self.tasks
                            .append_event(
                                task_id,
                                "moderator_done",
                                json!({
                                    "round": round,
                                    "summary": summary,
                                    "verification": verification,
                                })
                                .as_object(),
                            )
                            .await?;
                        self.task_service
                            .transition(
                                &task_id.to_string(),
                                json!({ "status": "review" }),
                            )
                            .await?;
                        let mut content = format!("主持人总结：{summary}\n验收证据：");
                        for (index, item) in verification.iter().enumerate() {
                            content.push_str(&format!("\n{}. {item}", index + 1));
                        }
                        self.post_system_message(room.id, &content, task_id).await?;
                        tracing::info!(
                            event = "orchestration_done",
                            task_id,
                            agent_id = moderator.id,
                            round,
                            "orchestration done"
                        );
                        return Ok(());
                    }
                    ModeratorPlan::Assign { assignments } => {
                        self.validate_assignments(&assignments, &members, &moderator)?;

                        let assignments_json: Vec<serde_json::Value> = assignments
                            .iter()
                            .map(|assignment| {
                                let mut item = json!({
                                    "agentId": assignment.agent_id,
                                    "kind": assignment.kind.as_str(),
                                    "instruction": assignment.instruction,
                                });
                                if let Some(expected) = &assignment.expected_output {
                                    item["expectedOutput"] = json!(expected);
                                }
                                if let Some(criteria) = &assignment.acceptance_criteria {
                                    item["acceptanceCriteria"] = json!(criteria);
                                }
                                item
                            })
                            .collect();
                        self.tasks
                            .append_event(
                                task_id,
                                "moderator_plan",
                                json!({
                                    "round": round,
                                    "assignments": assignments_json,
                                })
                                .as_object(),
                            )
                            .await?;

                        for assignment in assignments {
                            let Some(member) = members
                                .iter()
                                .find(|member| member.id == assignment.agent_id)
                            else {
                                return Err(EngineFailure::App(
                                    AppError::constraint(format!(
                                        "主持人指派了非房间成员：{}",
                                        assignment.agent_id
                                    )),
                                ));
                            };
                            let instruction = format_assignment_instruction(&assignment);
                            let run = self
                                .run_in_room_queue(
                                    room.id,
                                    member,
                                    &instruction,
                                    task_id,
                                    RunAgentOptions::default(),
                                )
                                .await?;
                            if run.status != crate::domain::run::RunStatus::Succeeded {
                                return Err(EngineFailure::Message(format!(
                                    "成员「{}」运行失败：{}",
                                    member.name,
                                    run.error
                                        .clone()
                                        .unwrap_or_else(|| run.status.as_str().to_string())
                                )));
                            }
                            if assignment.kind == AssignmentKind::Work {
                                executed_work += 1;
                            } else {
                                completed_reviews += 1;
                            }
                            tracing::info!(
                                event = "assignment_dispatched",
                                task_id,
                                agent_id = member.id,
                                run_id = run.id,
                                round,
                                "assignment dispatched"
                            );
                        }
                    }
                }
            }

            self.block(task_id, room.id, "已达最大轮次，熔断等待人工介入")
                .await?;
            Ok(())
        }
        .await;

        if let Err(error) = outcome {
            let message = match &error {
                EngineFailure::Plan(parse_error) => format!(
                    "主持人输出无法解析为约定 JSON，编排已熔断：{parse_error}"
                ),
                EngineFailure::App(error) => {
                    format!("编排失败，已熔断等待人工介入：{}", error.message)
                }
                EngineFailure::Message(message) => {
                    format!("编排失败，已熔断等待人工介入：{message}")
                }
            };
            self.block(task_id, room.id, &message).await?;
        }
        Ok(())
    }

    /// 分配单校验：自派、重复成员、非成员都必须阻塞。
    fn validate_assignments(
        &self,
        assignments: &[AssignInstruction],
        members: &[Agent],
        moderator: &Agent,
    ) -> Result<(), EngineFailure> {
        let mut seen = HashSet::new();
        for assignment in assignments {
            if assignment.agent_id == moderator.id {
                return Err(EngineFailure::App(AppError::constraint(
                    "主持人不能指派自己",
                )));
            }
            if !seen.insert(assignment.agent_id) {
                return Err(EngineFailure::App(AppError::constraint(format!(
                    "主持人重复指派成员：{}",
                    assignment.agent_id
                ))));
            }
            if !members
                .iter()
                .any(|member| member.id == assignment.agent_id)
            {
                return Err(EngineFailure::App(AppError::constraint(format!(
                    "主持人指派了非房间成员：{}",
                    assignment.agent_id
                ))));
            }
        }
        Ok(())
    }

    async fn request_plan(
        &self,
        context: ModeratorContext,
    ) -> Result<ModeratorPlan, EngineFailure> {
        let mut last_error: Option<PlanParseError> = None;

        for _ in 0..MODERATOR_PLAN_ATTEMPTS {
            let history = self.messages.list_by_room(context.room.id).await?;
            let instruction = build_moderator_instruction(
                &context.task,
                &context.room,
                &context.members,
                &history,
                context.round,
                MAX_ROUNDS,
            );
            let run = self
                .run_in_room_queue(
                    context.room.id,
                    &context.moderator,
                    &instruction,
                    context.task.id,
                    RunAgentOptions {
                        skip_room_context: true,
                        ..RunAgentOptions::default()
                    },
                )
                .await?;

            if run.status != crate::domain::run::RunStatus::Succeeded {
                return Err(EngineFailure::Message(format!(
                    "主持人运行{}：{}",
                    run.status.as_str(),
                    run.error.clone().unwrap_or_else(|| "无输出".to_string())
                )));
            }
            let Some(result) = run.result.clone() else {
                return Err(EngineFailure::Message(
                    "主持人运行没有返回任何内容".to_string(),
                ));
            };

            match parse_plan(&result) {
                Ok(plan) => return Ok(plan),
                Err(error) => {
                    tracing::warn!(
                        event = "moderator_plan_invalid",
                        task_id = context.task.id,
                        agent_id = context.moderator.id,
                        run_id = run.id,
                        round = context.round,
                        "moderator plan invalid"
                    );
                    last_error = Some(error);
                }
            }
        }

        Err(EngineFailure::Plan(
            last_error.unwrap_or_else(|| PlanParseError("主持人输出解析失败".to_string())),
        ))
    }

    /// 经房间串行队列执行一次运行，并在出队后、派发前再次检查暂停。
    async fn run_in_room_queue(
        &self,
        room_id: i64,
        agent: &Agent,
        instruction: &str,
        task_id: i64,
        options: RunAgentOptions,
    ) -> Result<crate::domain::run::AgentRun, EngineFailure> {
        loop {
            let pause = self.pause_control.clone();
            let outcome = self
                .chat
                .run_agent_queued_checked(
                    room_id,
                    agent,
                    instruction,
                    Some(task_id),
                    options.clone(),
                    move || {
                        let pause = pause.clone();
                        async move { !pause.is_paused(room_id) }
                    },
                )
                .await?;
            match outcome {
                Some(run) => return Ok(run),
                None => self.pause_control.assert_not_paused(room_id).await,
            }
        }
    }

    /// 任务置 blocked 并留下可见系统消息；错误路径不允许静默。
    async fn block(
        &self,
        task_id: i64,
        room_id: i64,
        content: &str,
    ) -> Result<(), EngineFailure> {
        let _ = self
            .task_service
            .transition(&task_id.to_string(), json!({ "status": "blocked" }))
            .await;
        self.post_system_message(room_id, content, task_id).await?;
        tracing::error!(
            event = "orchestration_blocked",
            task_id,
            "orchestration blocked"
        );
        Ok(())
    }

    /// 系统消息与聊天同一条落库 + 广播路径，并关联任务以供时间线聚合。
    async fn post_system_message(
        &self,
        room_id: i64,
        content: &str,
        task_id: i64,
    ) -> AppResult<Message> {
        let message = self
            .messages
            .append(
                AppendMessageInput::final_message(
                    room_id,
                    MessageSenderType::System,
                    None,
                    content,
                )
                .with_task(Some(task_id)),
            )
            .await?;
        (self.broadcast)(
            room_id,
            &RoomEvent::MessageFinal {
                room_id,
                message: message.clone(),
            },
        );
        Ok(message)
    }

    async fn require_moderator(&self, room: &Room) -> Result<Agent, EngineFailure> {
        let Some(moderator_agent_id) = room.moderator_agent_id else {
            return Err(EngineFailure::App(AppError::constraint(
                "房间未设置主持人，无法启动编排",
            )));
        };
        if !room.member_ids.contains(&moderator_agent_id) {
            return Err(EngineFailure::App(AppError::constraint(
                "房间主持人不是房间成员，无法启动编排",
            )));
        }
        self.agents
            .get_by_id(moderator_agent_id)
            .await?
            .ok_or_else(|| {
                EngineFailure::App(AppError::constraint(format!(
                    "房间主持人不存在：{moderator_agent_id}"
                )))
            })
    }
}
