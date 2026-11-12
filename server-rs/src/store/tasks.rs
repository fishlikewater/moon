//! 任务与任务事件访问层。

use serde_json::{json, Map, Value};
use sqlx::SqlitePool;

use crate::db::Db;
use crate::domain::now_ms;
use crate::domain::task::{Task, TaskEvent, TaskPlan, TaskPriority, TaskStatus};
use crate::error::{AppError, AppResult};

const TASK_COLUMNS: &str = "id, room_id, title, description, status, priority, \
     assignee_agent_id, due_at, scheduled_at, plan_json, created_by_agent_id, created_at, updated_at";

#[derive(Debug, Clone)]
pub struct CreateTaskInput {
    pub room_id: i64,
    pub title: String,
    pub description: String,
    pub status: TaskStatus,
    pub priority: TaskPriority,
    pub assignee_agent_id: Option<i64>,
    pub due_at: Option<i64>,
    pub scheduled_at: Option<i64>,
    pub plan: Option<TaskPlan>,
    pub created_by_agent_id: Option<i64>,
    /// `manual` / `moderator`；缺省时不写该字段，保持既有 created 事件结构。
    pub source: Option<String>,
}

/// 可编辑字段补丁：外层 `None` 表示不修改，内层 `None` 表示显式清空。
#[derive(Debug, Clone, Default)]
pub struct UpdateTaskFields {
    pub title: Option<String>,
    pub description: Option<String>,
    pub priority: Option<TaskPriority>,
    pub assignee_agent_id: Option<Option<i64>>,
    pub due_at: Option<Option<i64>>,
    pub scheduled_at: Option<Option<i64>>,
}

#[derive(Debug, Clone)]
pub enum TransitionResult {
    Updated {
        task: Task,
        previous_status: TaskStatus,
    },
    NotFound,
    Invalid {
        status: TaskStatus,
    },
}

#[derive(Debug, Clone)]
pub struct TaskStore {
    db: Db,
}

impl TaskStore {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    fn pool(&self) -> &SqlitePool {
        self.db.pool()
    }

    /// 建任务与 `created` 审计事件在同一事务内完成，事件不会缺失。
    pub async fn create(&self, input: CreateTaskInput) -> AppResult<Task> {
        let created_at = now_ms();
        let plan_json = input
            .plan
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;

        let mut payload = Map::new();
        payload.insert(
            "status".to_string(),
            Value::String(input.status.as_str().to_string()),
        );
        if let Some(plan) = &input.plan {
            payload.insert("plan".to_string(), serde_json::to_value(plan)?);
        }
        if let Some(agent_id) = input.created_by_agent_id {
            payload.insert(
                "createdByAgentId".to_string(),
                Value::Number(agent_id.into()),
            );
        }
        if let Some(source) = &input.source {
            payload.insert("source".to_string(), Value::String(source.clone()));
        }

        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let result = sqlx::query(
            "INSERT INTO tasks
               (room_id, title, description, status, priority, assignee_agent_id, due_at,
                scheduled_at, plan_json, created_by_agent_id, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(input.room_id)
        .bind(&input.title)
        .bind(&input.description)
        .bind(input.status.as_str())
        .bind(input.priority.as_str())
        .bind(input.assignee_agent_id)
        .bind(input.due_at)
        .bind(input.scheduled_at)
        .bind(&plan_json)
        .bind(input.created_by_agent_id)
        .bind(created_at)
        .bind(created_at)
        .execute(&mut *tx)
        .await?;
        let task_id = result.last_insert_rowid();

        sqlx::query(
            "INSERT INTO task_events (task_id, type, payload_json, created_at)
             VALUES (?, 'created', ?, ?)",
        )
        .bind(task_id)
        .bind(serde_json::to_string(&payload)?)
        .bind(created_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        Ok(Task {
            id: task_id,
            room_id: input.room_id,
            title: input.title,
            description: input.description,
            status: input.status,
            priority: input.priority,
            assignee_agent_id: input.assignee_agent_id,
            due_at: input.due_at,
            scheduled_at: input.scheduled_at,
            plan: input.plan,
            created_by_agent_id: input.created_by_agent_id,
            created_at,
            updated_at: created_at,
        })
    }

    pub async fn get_by_id(&self, id: i64) -> AppResult<Option<Task>> {
        let row = sqlx::query_as::<_, TaskRow>(&format!(
            "SELECT {TASK_COLUMNS} FROM tasks WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(self.pool())
        .await?;
        row.map(map_task_row).transpose()
    }

    pub async fn list_by_room(&self, room_id: i64) -> AppResult<Vec<Task>> {
        let rows = sqlx::query_as::<_, TaskRow>(&format!(
            "SELECT {TASK_COLUMNS} FROM tasks WHERE room_id = ? ORDER BY id"
        ))
        .bind(room_id)
        .fetch_all(self.pool())
        .await?;
        rows.into_iter().map(map_task_row).collect()
    }

    /// 更新可编辑字段：读旧行 → 与旧值比较合并 → 单条 UPDATE → `updated` 事件。
    ///
    /// 返回 `None` 表示任务不存在；`Some((task, false))` 表示补丁与现有值相同，
    /// 未写库、未产生事件。不改 status/plan/发起者。
    pub async fn update_fields(
        &self,
        id: i64,
        input: UpdateTaskFields,
    ) -> AppResult<Option<(Task, bool)>> {
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let row = sqlx::query_as::<_, TaskRow>(&format!(
            "SELECT {TASK_COLUMNS} FROM tasks WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            tx.rollback().await?;
            return Ok(None);
        };
        // 状态在写事务内复核，避免「读 status → 写定时」之间被并发推进。
        let current_status = parse_status(&row.status)?;
        if input.scheduled_at.is_some() && current_status != TaskStatus::Backlog {
            tx.rollback().await?;
            return Err(AppError::validation("只有待办任务可以设置定时启动"));
        }
        let mut task = map_task_row(row)?;

        let mut changed: Vec<&str> = Vec::new();
        let mut payload = Map::new();
        if let Some(value) = input.title {
            if value != task.title {
                task.title = value;
                changed.push("title");
                payload.insert("title".to_string(), Value::String(task.title.clone()));
            }
        }
        if let Some(value) = input.description {
            if value != task.description {
                task.description = value;
                changed.push("description");
                payload.insert(
                    "description".to_string(),
                    Value::String(task.description.clone()),
                );
            }
        }
        if let Some(value) = input.priority {
            if value != task.priority {
                task.priority = value;
                changed.push("priority");
                payload.insert(
                    "priority".to_string(),
                    Value::String(value.as_str().to_string()),
                );
            }
        }
        if let Some(value) = input.assignee_agent_id {
            if value != task.assignee_agent_id {
                task.assignee_agent_id = value;
                changed.push("assigneeAgentId");
                payload.insert("assigneeAgentId".to_string(), json!(value));
            }
        }
        if let Some(value) = input.due_at {
            if value != task.due_at {
                task.due_at = value;
                changed.push("dueAt");
                payload.insert("dueAt".to_string(), json!(value));
            }
        }
        if let Some(value) = input.scheduled_at {
            if value != task.scheduled_at {
                task.scheduled_at = value;
                changed.push("scheduledAt");
                payload.insert("scheduledAt".to_string(), json!(value));
            }
        }

        if changed.is_empty() {
            tx.rollback().await?;
            return Ok(Some((task, false)));
        }

        task.updated_at = now_ms();
        sqlx::query(
            "UPDATE tasks SET title = ?, description = ?, priority = ?, assignee_agent_id = ?,
                due_at = ?, scheduled_at = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&task.title)
        .bind(&task.description)
        .bind(task.priority.as_str())
        .bind(task.assignee_agent_id)
        .bind(task.due_at)
        .bind(task.scheduled_at)
        .bind(task.updated_at)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        payload.insert("changed".to_string(), json!(changed));
        sqlx::query(
            "INSERT INTO task_events (task_id, type, payload_json, created_at)
             VALUES (?, 'updated', ?, ?)",
        )
        .bind(id)
        .bind(serde_json::to_string(&payload)?)
        .bind(task.updated_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Some((task, true)))
    }

    /// 到期待启动的任务：仅 backlog 且定时时间已到，按定时与 id 升序。
    pub async fn list_due_scheduled(&self, now_ms: i64) -> AppResult<Vec<Task>> {
        let rows = sqlx::query_as::<_, TaskRow>(&format!(
            "SELECT {TASK_COLUMNS} FROM tasks
             WHERE status = 'backlog' AND scheduled_at IS NOT NULL AND scheduled_at <= ?
             ORDER BY scheduled_at, id"
        ))
        .bind(now_ms)
        .fetch_all(self.pool())
        .await?;
        rows.into_iter().map(map_task_row).collect()
    }

    /// 清除定时；返回是否确有定时被清除（用于区分幂等重复）。
    pub async fn clear_schedule(&self, id: i64) -> AppResult<bool> {
        let result = sqlx::query(
            "UPDATE tasks SET scheduled_at = NULL, updated_at = ? WHERE id = ? AND scheduled_at IS NOT NULL",
        )
        .bind(now_ms())
        .bind(id)
        .execute(self.pool())
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// 事务内完成「读当前状态 → 校验 → 更新 → 追加 status_changed 事件」。
    pub async fn transition(
        &self,
        id: i64,
        to: TaskStatus,
        is_allowed: impl Fn(TaskStatus, TaskStatus) -> bool,
    ) -> AppResult<TransitionResult> {
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let row = sqlx::query_as::<_, TaskRow>(&format!(
            "SELECT {TASK_COLUMNS} FROM tasks WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            tx.rollback().await?;
            return Ok(TransitionResult::NotFound);
        };
        let previous_status = parse_status(&row.status)?;
        if !is_allowed(previous_status, to) {
            tx.rollback().await?;
            return Ok(TransitionResult::Invalid {
                status: previous_status,
            });
        }

        let updated_at = now_ms();
        sqlx::query("UPDATE tasks SET status = ?, updated_at = ? WHERE id = ?")
            .bind(to.as_str())
            .bind(updated_at)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO task_events (task_id, type, payload_json, created_at)
             VALUES (?, 'status_changed', ?, ?)",
        )
        .bind(id)
        .bind(serde_json::to_string(&json!({
            "from": previous_status.as_str(),
            "to": to.as_str(),
        }))?)
        .bind(updated_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        let mut task = map_task_row(row)?;
        task.status = to;
        task.updated_at = updated_at;
        Ok(TransitionResult::Updated {
            task,
            previous_status,
        })
    }

    /// 追加任务审计事件；`payload` 显式序列化为 JSON。
    pub async fn append_event(
        &self,
        task_id: i64,
        event_type: &str,
        payload: Option<&Map<String, Value>>,
    ) -> AppResult<TaskEvent> {
        let created_at = now_ms();
        let payload_json = payload.map(serde_json::to_string).transpose()?;
        let result = sqlx::query(
            "INSERT INTO task_events (task_id, type, payload_json, created_at)
             VALUES (?, ?, ?, ?)",
        )
        .bind(task_id)
        .bind(event_type)
        .bind(payload_json)
        .bind(created_at)
        .execute(self.pool())
        .await?;

        Ok(TaskEvent {
            id: result.last_insert_rowid(),
            task_id,
            event_type: event_type.to_string(),
            payload: payload.cloned(),
            created_at,
        })
    }

    /// 任务事件按写入顺序返回（同毫秒时以自增 id 定序）。
    pub async fn list_events(&self, task_id: i64) -> AppResult<Vec<TaskEvent>> {
        let rows = sqlx::query_as::<_, TaskEventRow>(
            "SELECT id, task_id, type, payload_json, created_at
               FROM task_events WHERE task_id = ? ORDER BY id",
        )
        .bind(task_id)
        .fetch_all(self.pool())
        .await?;
        rows.into_iter().map(map_task_event_row).collect()
    }
}

#[derive(sqlx::FromRow)]
struct TaskRow {
    id: i64,
    room_id: i64,
    title: String,
    description: String,
    status: String,
    priority: String,
    assignee_agent_id: Option<i64>,
    due_at: Option<i64>,
    scheduled_at: Option<i64>,
    plan_json: Option<String>,
    created_by_agent_id: Option<i64>,
    created_at: i64,
    updated_at: i64,
}

#[derive(sqlx::FromRow)]
struct TaskEventRow {
    id: i64,
    task_id: i64,
    #[sqlx(rename = "type")]
    event_type: String,
    payload_json: Option<String>,
    created_at: i64,
}

fn map_task_row(row: TaskRow) -> AppResult<Task> {
    let status = parse_status(&row.status)?;
    let priority = parse_priority(&row.priority)?;
    let plan = row
        .plan_json
        .as_deref()
        .map(serde_json::from_str::<TaskPlan>)
        .transpose()
        .map_err(|error| {
            AppError::with_source(
                crate::error::ErrorCode::InternalError,
                "tasks.plan_json 不是合法任务计划",
                error,
            )
        })?;
    Ok(Task {
        id: row.id,
        room_id: row.room_id,
        title: row.title,
        description: row.description,
        status,
        priority,
        assignee_agent_id: row.assignee_agent_id,
        due_at: row.due_at,
        scheduled_at: row.scheduled_at,
        plan,
        created_by_agent_id: row.created_by_agent_id,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

fn map_task_event_row(row: TaskEventRow) -> AppResult<TaskEvent> {
    let payload = row
        .payload_json
        .as_deref()
        .map(serde_json::from_str::<Map<String, Value>>)
        .transpose()
        .map_err(|error| {
            AppError::with_source(
                crate::error::ErrorCode::InternalError,
                "task_events.payload_json 不是合法 JSON 对象",
                error,
            )
        })?;
    Ok(TaskEvent {
        id: row.id,
        task_id: row.task_id,
        event_type: row.event_type,
        payload,
        created_at: row.created_at,
    })
}

fn parse_status(raw: &str) -> AppResult<TaskStatus> {
    TaskStatus::parse(raw)
        .ok_or_else(|| AppError::internal(format!("tasks.status 非法：{raw}")))
}

fn parse_priority(raw: &str) -> AppResult<TaskPriority> {
    TaskPriority::parse(raw)
        .ok_or_else(|| AppError::internal(format!("tasks.priority 非法：{raw}")))
}
