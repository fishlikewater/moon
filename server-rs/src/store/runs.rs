//! 运行记录访问层。

use sqlx::SqlitePool;

use crate::db::Db;
use crate::domain::now_ms;
use crate::domain::run::{AgentRun, RunStatus};
use crate::error::{AppError, AppResult};

const RUN_COLUMNS: &str = "id, room_id, agent_id, task_id, status, prompt, result, error, \
     token_count, context_size, harness_session_id, created_at, finished_at";

#[derive(Debug, Clone)]
pub struct CreateRunInput {
    pub room_id: i64,
    pub agent_id: i64,
    pub task_id: Option<i64>,
    pub prompt: String,
}

/// 运行只能从 `running` 进入终态，因此终态不接受 `running`。
#[derive(Debug, Clone)]
pub struct FinishRunInput {
    pub status: RunStatus,
    pub result: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RunStore {
    db: Db,
}

impl RunStore {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    fn pool(&self) -> &SqlitePool {
        self.db.pool()
    }

    /// 新建运行记录，初始状态固定为 `running`。
    pub async fn create(&self, input: CreateRunInput) -> AppResult<AgentRun> {
        let created_at = now_ms();
        let result = sqlx::query(
            "INSERT INTO agent_runs
               (room_id, agent_id, task_id, status, prompt, result, error,
                token_count, context_size, created_at, finished_at)
             VALUES (?, ?, ?, 'running', ?, NULL, NULL, NULL, NULL, ?, NULL)",
        )
        .bind(input.room_id)
        .bind(input.agent_id)
        .bind(input.task_id)
        .bind(&input.prompt)
        .bind(created_at)
        .execute(self.pool())
        .await?;

        Ok(AgentRun {
            id: result.last_insert_rowid(),
            room_id: input.room_id,
            agent_id: input.agent_id,
            task_id: input.task_id,
            status: RunStatus::Running,
            prompt: input.prompt,
            result: None,
            error: None,
            token_count: None,
            context_size: None,
            harness_session_id: None,
            created_at,
            finished_at: None,
        })
    }

    /// 记录本次运行使用的 harness 会话 ID；会话在运行中途才确定。
    pub async fn attach_session(&self, id: i64, harness_session_id: &str) -> AppResult<()> {
        sqlx::query("UPDATE agent_runs SET harness_session_id = ? WHERE id = ?")
            .bind(harness_session_id)
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// 记录 harness 上报的用量；多次上报时只覆盖本次非空字段。
    pub async fn set_usage(
        &self,
        id: i64,
        token_count: Option<i64>,
        context_size: Option<i64>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE agent_runs
                SET token_count = COALESCE(?, token_count),
                    context_size = COALESCE(?, context_size)
              WHERE id = ?",
        )
        .bind(token_count)
        .bind(context_size)
        .bind(id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn get_by_id(&self, id: i64) -> AppResult<Option<AgentRun>> {
        let row = sqlx::query_as::<_, RunRow>(&format!(
            "SELECT {RUN_COLUMNS} FROM agent_runs WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(self.pool())
        .await?;
        row.map(map_run_row).transpose()
    }

    pub async fn list_by_room(&self, room_id: i64) -> AppResult<Vec<AgentRun>> {
        let rows = sqlx::query_as::<_, RunRow>(&format!(
            "SELECT {RUN_COLUMNS} FROM agent_runs WHERE room_id = ? ORDER BY id"
        ))
        .bind(room_id)
        .fetch_all(self.pool())
        .await?;
        rows.into_iter().map(map_run_row).collect()
    }

    /// 房间最近运行：按创建时间倒序，供运行详情面板展示。
    pub async fn list_recent_by_room(
        &self,
        room_id: i64,
        limit: i64,
    ) -> AppResult<Vec<AgentRun>> {
        let rows = sqlx::query_as::<_, RunRow>(&format!(
            "SELECT {RUN_COLUMNS} FROM agent_runs
              WHERE room_id = ?
              ORDER BY created_at DESC, id DESC
              LIMIT ?"
        ))
        .bind(room_id)
        .bind(limit)
        .fetch_all(self.pool())
        .await?;
        rows.into_iter().map(map_run_row).collect()
    }

    pub async fn list_by_task(&self, task_id: i64) -> AppResult<Vec<AgentRun>> {
        let rows = sqlx::query_as::<_, RunRow>(&format!(
            "SELECT {RUN_COLUMNS} FROM agent_runs WHERE task_id = ? ORDER BY id"
        ))
        .bind(task_id)
        .fetch_all(self.pool())
        .await?;
        rows.into_iter().map(map_run_row).collect()
    }

    pub async fn list_by_status(&self, status: RunStatus) -> AppResult<Vec<AgentRun>> {
        let rows = sqlx::query_as::<_, RunRow>(&format!(
            "SELECT {RUN_COLUMNS} FROM agent_runs WHERE status = ? ORDER BY id"
        ))
        .bind(status.as_str())
        .fetch_all(self.pool())
        .await?;
        rows.into_iter().map(map_run_row).collect()
    }

    /// 结束运行并记录终态时间；未传入的 result/error 保持原值。
    pub async fn finish(
        &self,
        id: i64,
        input: FinishRunInput,
    ) -> AppResult<Option<AgentRun>> {
        let Some(existing) = self.get_by_id(id).await? else {
            return Ok(None);
        };

        let finished_at = now_ms();
        let result = input.result.or(existing.result.clone());
        let error = input.error.or(existing.error.clone());

        sqlx::query(
            "UPDATE agent_runs
                SET status = ?, result = ?, error = ?, finished_at = ?
              WHERE id = ?",
        )
        .bind(input.status.as_str())
        .bind(&result)
        .bind(&error)
        .bind(finished_at)
        .bind(id)
        .execute(self.pool())
        .await?;

        Ok(Some(AgentRun {
            status: input.status,
            result,
            error,
            finished_at: Some(finished_at),
            ..existing
        }))
    }
}

#[derive(sqlx::FromRow)]
struct RunRow {
    id: i64,
    room_id: i64,
    agent_id: i64,
    task_id: Option<i64>,
    status: String,
    prompt: String,
    result: Option<String>,
    error: Option<String>,
    token_count: Option<i64>,
    context_size: Option<i64>,
    harness_session_id: Option<String>,
    created_at: i64,
    finished_at: Option<i64>,
}

fn map_run_row(row: RunRow) -> AppResult<AgentRun> {
    let status = RunStatus::parse(&row.status)
        .ok_or_else(|| AppError::internal(format!("agent_runs.status 非法：{}", row.status)))?;
    Ok(AgentRun {
        id: row.id,
        room_id: row.room_id,
        agent_id: row.agent_id,
        task_id: row.task_id,
        status,
        prompt: row.prompt,
        result: row.result,
        error: row.error,
        token_count: row.token_count,
        context_size: row.context_size,
        harness_session_id: row.harness_session_id,
        created_at: row.created_at,
        finished_at: row.finished_at,
    })
}
