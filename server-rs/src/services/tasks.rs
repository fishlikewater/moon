//! 任务业务规则：发布、列表、受控状态流转与编排激活。

use serde_json::Value;

use crate::domain::event::RoomEvent;
use crate::domain::task::{Task, TaskPlan, TaskPriority, TaskStatus};
use crate::error::{AppError, AppResult};
use crate::services::chat::BroadcastFn;
use crate::services::parse_id;
use crate::services::require_body;
use crate::services::require_non_empty_string;
use crate::services::task_plan::parse_task_plan;
use crate::store::tasks::{CreateTaskInput, TransitionResult, UpdateTaskFields};
use crate::store::{RoomStore, TaskStore};

/// 状态机唯一判定入口；启动恢复等复用处必须引用同一张表。
pub fn is_allowed_transition(from: TaskStatus, to: TaskStatus) -> bool {
    match from {
        TaskStatus::Backlog => matches!(
            to,
            TaskStatus::InProgress | TaskStatus::Blocked | TaskStatus::Cancelled
        ),
        TaskStatus::InProgress => matches!(
            to,
            TaskStatus::Review
                | TaskStatus::Blocked
                | TaskStatus::Backlog
                | TaskStatus::Cancelled
        ),
        TaskStatus::Review => matches!(
            to,
            TaskStatus::Done
                | TaskStatus::InProgress
                | TaskStatus::Blocked
                | TaskStatus::Cancelled
        ),
        TaskStatus::Blocked => matches!(
            to,
            TaskStatus::InProgress | TaskStatus::Backlog | TaskStatus::Cancelled
        ),
        TaskStatus::Done | TaskStatus::Cancelled => false,
    }
}

/// 主持人创建任务的内部输入；plan 已由协议层校验。
#[derive(Debug, Clone)]
pub struct ModeratorTaskInput {
    pub title: String,
    pub description: Option<String>,
    pub plan: TaskPlan,
}

pub struct TaskServiceOptions {
    pub rooms: RoomStore,
    pub tasks: TaskStore,
    pub broadcast: BroadcastFn,
}

pub struct TaskService {
    rooms: RoomStore,
    tasks: TaskStore,
    broadcast: BroadcastFn,
}

impl TaskService {
    pub fn new(options: TaskServiceOptions) -> Self {
        Self {
            rooms: options.rooms,
            tasks: options.tasks,
            broadcast: options.broadcast,
        }
    }

    /// 发布任务：状态固定从 `backlog` 开始，created 事件由 store 同事务写入。
    pub async fn create(&self, raw_room_id: &str, body: Value) -> AppResult<Task> {
        let room_id = parse_id(raw_room_id)?;
        self.require_room(room_id).await?;

        let body = require_body(body)?;
        let title = require_non_empty_string(body.get("title"), "title")?;
        let description = parse_description(body.get("description"))?;
        let plan = parse_optional_plan(body.get("plan"))?;
        let priority = parse_priority(body.get("priority"))?;
        let assignee_agent_id = self
            .resolve_assignee(room_id, body.get("assigneeAgentId"))
            .await?;
        let due_at = parse_optional_timestamp(body.get("dueAt"), "dueAt")?;
        let scheduled_at = parse_optional_timestamp(body.get("scheduledAt"), "scheduledAt")?;

        self.tasks
            .create(CreateTaskInput {
                room_id,
                title,
                description: description.unwrap_or_default(),
                status: TaskStatus::Backlog,
                priority,
                assignee_agent_id,
                due_at,
                scheduled_at,
                plan,
                created_by_agent_id: None,
                source: None,
            })
            .await
    }

    /// 主持人创建任务：记录发起者与结构化计划，跳过客户端可伪造的字段。
    pub async fn create_from_moderator(
        &self,
        room_id: i64,
        moderator_agent_id: i64,
        input: ModeratorTaskInput,
    ) -> AppResult<Task> {
        self.require_room(room_id).await?;

        let title = input.title;
        if title.trim().is_empty() {
            return Err(AppError::validation("title 必须是非空字符串"));
        }
        let description = input.description.unwrap_or_default();
        let plan = input.plan;

        self.tasks
            .create(CreateTaskInput {
                room_id,
                title,
                description,
                status: TaskStatus::Backlog,
                priority: TaskPriority::Normal,
                assignee_agent_id: None,
                due_at: None,
                scheduled_at: None,
                plan: Some(plan),
                created_by_agent_id: Some(moderator_agent_id),
                source: Some("moderator".to_string()),
            })
            .await
    }

    /// 房间内任务按 id 升序返回；房间不存在时 404。
    pub async fn list_by_room(&self, raw_room_id: &str) -> AppResult<Vec<Task>> {
        let room_id = parse_id(raw_room_id)?;
        self.require_room(room_id).await?;
        self.tasks.list_by_room(room_id).await
    }

    /// 受控状态流转；非法迁移 409 且不产生事件、不改动任务。
    pub async fn transition(&self, raw_task_id: &str, body: Value) -> AppResult<Task> {
        let task_id = parse_id(raw_task_id)?;
        let body = require_body(body)?;
        let to = parse_status(body.get("status"))?;

        let result = self
            .tasks
            .transition(task_id, to, is_allowed_transition)
            .await?;
        match result {
            TransitionResult::NotFound => Err(AppError::not_found(format!(
                "任务不存在：{task_id}"
            ))),
            TransitionResult::Invalid { status } => Err(AppError::invalid_transition(format!(
                "任务状态不允许从 {} 迁移到 {}",
                status.as_str(),
                to.as_str()
            ))),
            TransitionResult::Updated {
                mut task,
                previous_status: _,
            } => {
                // 离开待办后定时不再有意义（含取消与手动启动）；清理失败只告警，不掩盖流转结果。
                if task.status != TaskStatus::Backlog && task.scheduled_at.is_some() {
                    if let Err(error) = self.tasks.clear_schedule(task.id).await {
                        tracing::warn!(task_id = task.id, %error, "清理任务定时失败");
                    } else {
                        task.scheduled_at = None;
                    }
                }
                // 事务已提交，看板收到的事件与落库状态一致。
                (self.broadcast)(
                    task.room_id,
                    &RoomEvent::TaskChanged {
                        room_id: task.room_id,
                        task_id: task.id,
                        status: task.status,
                    },
                );
                Ok(task)
            }
        }
    }

    /// 更新可编辑字段；未出现的键保持不变，显式 null 清除可空字段。
    pub async fn update(&self, raw_task_id: &str, body: Value) -> AppResult<Task> {
        let task_id = parse_id(raw_task_id)?;
        let Some(task) = self.tasks.get_by_id(task_id).await? else {
            return Err(AppError::not_found(format!("任务不存在：{task_id}")));
        };
        let body = require_body(body)?;

        let mut fields = UpdateTaskFields::default();
        if let Some(value) = body.get("title") {
            fields.title = Some(require_non_empty_string(Some(value), "title")?);
        }
        if let Some(value) = body.get("description") {
            fields.description = Some(parse_description(Some(value))?.unwrap_or_default());
        }
        if let Some(value) = body.get("priority") {
            fields.priority = Some(parse_priority(Some(value))?);
        }
        if let Some(value) = body.get("assigneeAgentId") {
            fields.assignee_agent_id = Some(
                self.resolve_assignee(task.room_id, Some(value))
                    .await?,
            );
        }
        if let Some(value) = body.get("dueAt") {
            fields.due_at = Some(parse_optional_timestamp(Some(value), "dueAt")?);
        }
        if let Some(value) = body.get("scheduledAt") {
            let scheduled_at = parse_optional_timestamp(Some(value), "scheduledAt")?;
            if scheduled_at.is_some() && task.status != TaskStatus::Backlog {
                return Err(AppError::validation("只有待办任务可以设置定时启动"));
            }
            fields.scheduled_at = Some(scheduled_at);
        }

        let Some((updated, changed)) = self.tasks.update_fields(task_id, fields).await? else {
            return Err(AppError::not_found(format!("任务不存在：{task_id}")));
        };
        // 补丁与现有值相同时不广播，避免看板收到空更新。
        if changed {
            (self.broadcast)(
                updated.room_id,
                &RoomEvent::TaskChanged {
                    room_id: updated.room_id,
                    task_id: updated.id,
                    status: updated.status,
                },
            );
        }
        Ok(updated)
    }

    /// 指派人必须是房间成员；`null`/缺省表示不指派。
    async fn resolve_assignee(
        &self,
        room_id: i64,
        value: Option<&Value>,
    ) -> AppResult<Option<i64>> {
        let Some(value) = value else {
            return Ok(None);
        };
        if value.is_null() {
            return Ok(None);
        }
        let agent_id = match value {
            Value::Number(number) => number
                .as_i64()
                .filter(|id| *id > 0)
                .ok_or_else(|| AppError::validation("assigneeAgentId 必须是正整数或 null"))?,
            Value::String(text) => text
                .parse::<i64>()
                .ok()
                .filter(|id| *id > 0)
                .ok_or_else(|| AppError::validation("assigneeAgentId 必须是正整数或 null"))?,
            _ => {
                return Err(AppError::validation(
                    "assigneeAgentId 必须是正整数或 null",
                ))
            }
        };
        let Some(room) = self.rooms.get_by_id(room_id).await? else {
            return Err(AppError::not_found(format!("房间不存在：{room_id}")));
        };
        if !room.member_ids.contains(&agent_id) {
            return Err(AppError::validation("指派对象必须是房间成员"));
        }
        Ok(Some(agent_id))
    }

    /// 编排激活入口：只有 backlog/in_progress 可激活，且主持人必须是房间成员。
    ///
    /// 返回 `(task, started_now)`：`started_now` 只有在本次调用把任务从
    /// `backlog` 推进到 `in_progress` 时为 true；已是 in_progress 时为 false，
    /// 调用方可据此避免重复记录「已启动」。
    pub async fn activate(&self, raw_task_id: &str) -> AppResult<(Task, bool)> {
        let task_id = parse_id(raw_task_id)?;
        let Some(task) = self.tasks.get_by_id(task_id).await? else {
            return Err(AppError::not_found(format!("任务不存在：{task_id}")));
        };
        if !matches!(task.status, TaskStatus::Backlog | TaskStatus::InProgress) {
            return Err(AppError::invalid_transition(format!(
                "任务状态 {} 不允许激活编排",
                task.status.as_str()
            )));
        }

        let Some(room) = self.rooms.get_by_id(task.room_id).await? else {
            return Err(AppError::not_found(format!(
                "房间不存在：{}",
                task.room_id
            )));
        };
        let Some(moderator_agent_id) = room.moderator_agent_id else {
            return Err(AppError::constraint("房间未设置主持人，无法启动编排"));
        };
        if !room.member_ids.contains(&moderator_agent_id) {
            return Err(AppError::constraint(
                "房间主持人不是房间成员，无法启动编排",
            ));
        }

        if task.status == TaskStatus::Backlog {
            let updated = self
                .transition(&task_id.to_string(), serde_json::json!({ "status": "in_progress" }))
                .await?;
            return Ok((updated, true));
        }
        Ok((task, false))
    }

    async fn require_room(&self, room_id: i64) -> AppResult<()> {
        if self.rooms.get_by_id(room_id).await?.is_none() {
            return Err(AppError::not_found(format!("房间不存在：{room_id}")));
        }
        Ok(())
    }
}

fn parse_description(value: Option<&Value>) -> AppResult<Option<String>> {
    match value {
        None => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(AppError::validation("description 必须是字符串")),
    }
}

fn parse_optional_plan(value: Option<&Value>) -> AppResult<Option<TaskPlan>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => parse_task_plan(value).map(Some).map_err(AppError::validation),
    }
}

fn parse_status(value: Option<&Value>) -> AppResult<TaskStatus> {
    match value.and_then(Value::as_str).and_then(TaskStatus::parse) {
        Some(status) => Ok(status),
        None => Err(AppError::validation(format!(
            "status 必须是 {} 之一",
            TaskStatus::ALL.map(TaskStatus::as_str).join(" | ")
        ))),
    }
}

fn parse_priority(value: Option<&Value>) -> AppResult<TaskPriority> {
    match value {
        None | Some(Value::Null) => Ok(TaskPriority::Normal),
        Some(Value::String(text)) => TaskPriority::parse(text).ok_or_else(|| {
            AppError::validation(format!(
                "priority 必须是 {} 之一",
                TaskPriority::ALL.map(TaskPriority::as_str).join(" | ")
            ))
        }),
        Some(_) => Err(AppError::validation("priority 必须是字符串")),
    }
}

fn parse_optional_timestamp(value: Option<&Value>, field: &str) -> AppResult<Option<i64>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number
            .as_i64()
            .filter(|ms| *ms >= 0)
            .map(Some)
            .ok_or_else(|| AppError::validation(format!("{field} 必须是 epoch 毫秒或 null"))),
        Some(_) => Err(AppError::validation(format!(
            "{field} 必须是 epoch 毫秒或 null"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_machine_matches_contract() {
        assert!(is_allowed_transition(TaskStatus::Backlog, TaskStatus::InProgress));
        assert!(is_allowed_transition(TaskStatus::Review, TaskStatus::Done));
        assert!(!is_allowed_transition(TaskStatus::Done, TaskStatus::InProgress));
        assert!(!is_allowed_transition(TaskStatus::Backlog, TaskStatus::Done));
    }

    #[test]
    fn every_non_terminal_status_can_cancel() {
        for status in [
            TaskStatus::Backlog,
            TaskStatus::InProgress,
            TaskStatus::Review,
            TaskStatus::Blocked,
        ] {
            assert!(
                is_allowed_transition(status, TaskStatus::Cancelled),
                "{} 应可取消",
                status.as_str()
            );
        }
        assert!(!is_allowed_transition(TaskStatus::Done, TaskStatus::Cancelled));
        assert!(!is_allowed_transition(TaskStatus::Cancelled, TaskStatus::Backlog));
    }

    #[test]
    fn priority_defaults_to_normal_and_rejects_unknown() {
        assert_eq!(parse_priority(None).unwrap(), TaskPriority::Normal);
        assert_eq!(
            parse_priority(Some(&serde_json::json!("urgent"))).unwrap(),
            TaskPriority::Urgent
        );
        assert!(parse_priority(Some(&serde_json::json!("p0"))).is_err());
    }

    #[test]
    fn optional_timestamp_accepts_null_and_non_negative_integer() {
        assert_eq!(parse_optional_timestamp(None, "dueAt").unwrap(), None);
        assert_eq!(
            parse_optional_timestamp(Some(&serde_json::json!(null)), "dueAt").unwrap(),
            None
        );
        assert_eq!(
            parse_optional_timestamp(Some(&serde_json::json!(1_700_000_000_000i64)), "dueAt")
                .unwrap(),
            Some(1_700_000_000_000)
        );
        assert!(parse_optional_timestamp(Some(&serde_json::json!(-1)), "dueAt").is_err());
        assert!(parse_optional_timestamp(Some(&serde_json::json!("soon")), "dueAt").is_err());
    }
}
