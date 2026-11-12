//! 讨论与参与者的持久化访问层；状态变更只允许从 running 进入终态。

use sqlx::SqlitePool;

use crate::db::Db;
use crate::domain::discussion::{Discussion, DiscussionStatus, DiscussionStopReason};
use crate::domain::now_ms;
use crate::error::{AppError, AppResult};

const DISCUSSION_COLUMNS: &str = "id, room_id, topic, status, current_round, max_rounds, stop_reason, \
     summary, created_by_agent_id, created_at, updated_at";

#[derive(Debug, Clone)]
pub struct CreateDiscussionInput {
    pub room_id: i64,
    pub topic: String,
    pub max_rounds: i64,
    pub participant_ids: Vec<i64>,
    pub created_by_agent_id: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct FinishDiscussionInput {
    pub status: DiscussionStatus,
    pub stop_reason: DiscussionStopReason,
    pub summary: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DiscussionStore {
    db: Db,
}

impl DiscussionStore {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    fn pool(&self) -> &SqlitePool {
        self.db.pool()
    }

    /// 讨论与参与者顺序在同一事务内写入，避免出现无参与者的半初始化讨论。
    pub async fn create(&self, input: CreateDiscussionInput) -> AppResult<Discussion> {
        let created_at = now_ms();
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let result = sqlx::query(
            "INSERT INTO discussions
               (room_id, topic, status, current_round, max_rounds,
                stop_reason, summary, created_by_agent_id, created_at, updated_at)
             VALUES (?, ?, 'running', 1, ?, NULL, NULL, ?, ?, ?)",
        )
        .bind(input.room_id)
        .bind(&input.topic)
        .bind(input.max_rounds)
        .bind(input.created_by_agent_id)
        .bind(created_at)
        .bind(created_at)
        .execute(&mut *tx)
        .await?;
        let discussion_id = result.last_insert_rowid();

        for (position, agent_id) in input.participant_ids.iter().enumerate() {
            sqlx::query(
                "INSERT INTO discussion_participants (discussion_id, agent_id, position)
                 VALUES (?, ?, ?)",
            )
            .bind(discussion_id)
            .bind(agent_id)
            .bind(position as i64)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;

        Ok(Discussion {
            id: discussion_id,
            room_id: input.room_id,
            topic: input.topic,
            status: DiscussionStatus::Running,
            current_round: 1,
            max_rounds: input.max_rounds,
            participant_ids: input.participant_ids,
            created_by_agent_id: input.created_by_agent_id,
            stop_reason: None,
            summary: None,
            created_at,
            updated_at: created_at,
        })
    }

    pub async fn get_by_id(&self, id: i64) -> AppResult<Option<Discussion>> {
        let row = sqlx::query_as::<_, DiscussionRow>(&format!(
            "SELECT {DISCUSSION_COLUMNS} FROM discussions WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(self.pool())
        .await?;
        match row {
            None => Ok(None),
            Some(row) => self.map_row(row).await.map(Some),
        }
    }

    /// 房间最新讨论（含已结束），供刷新时水合状态条。
    pub async fn latest_by_room(&self, room_id: i64) -> AppResult<Option<Discussion>> {
        let row = sqlx::query_as::<_, DiscussionRow>(&format!(
            "SELECT {DISCUSSION_COLUMNS} FROM discussions
              WHERE room_id = ? ORDER BY id DESC LIMIT 1"
        ))
        .bind(room_id)
        .fetch_optional(self.pool())
        .await?;
        match row {
            None => Ok(None),
            Some(row) => self.map_row(row).await.map(Some),
        }
    }

    pub async fn find_running_by_room(&self, room_id: i64) -> AppResult<Option<Discussion>> {
        let row = sqlx::query_as::<_, DiscussionRow>(&format!(
            "SELECT {DISCUSSION_COLUMNS} FROM discussions
              WHERE room_id = ? AND status = 'running' ORDER BY id DESC LIMIT 1"
        ))
        .bind(room_id)
        .fetch_optional(self.pool())
        .await?;
        match row {
            None => Ok(None),
            Some(row) => self.map_row(row).await.map(Some),
        }
    }

    /// 房间是否存在运行中讨论；用于删除房间/清理历史前拦截。
    pub async fn has_running_for_room(&self, room_id: i64) -> AppResult<bool> {
        Ok(sqlx::query_scalar::<_, i64>(
            "SELECT 1 FROM discussions WHERE room_id = ? AND status = 'running' LIMIT 1",
        )
        .bind(room_id)
        .fetch_optional(self.pool())
        .await?
        .is_some())
    }

    /// 智能体是否正在参与运行中讨论；用于删除智能体前拦截。
    pub async fn has_running_for_agent(&self, agent_id: i64) -> AppResult<bool> {
        Ok(sqlx::query_scalar::<_, i64>(
            "SELECT 1
               FROM discussion_participants AS p
               JOIN discussions AS d ON d.id = p.discussion_id
              WHERE p.agent_id = ? AND d.status = 'running'
              LIMIT 1",
        )
        .bind(agent_id)
        .fetch_optional(self.pool())
        .await?
        .is_some())
    }

    /// 启动恢复用：当前所有未终结讨论。
    pub async fn list_running(&self) -> AppResult<Vec<Discussion>> {
        let rows = sqlx::query_as::<_, DiscussionRow>(&format!(
            "SELECT {DISCUSSION_COLUMNS} FROM discussions WHERE status = 'running' ORDER BY id"
        ))
        .fetch_all(self.pool())
        .await?;
        let mut discussions = Vec::with_capacity(rows.len());
        for row in rows {
            discussions.push(self.map_row(row).await?);
        }
        Ok(discussions)
    }

    /// 推进到下一轮；讨论已停止/结束时返回当前快照而不改写状态。
    pub async fn advance_round(&self, id: i64, round: i64) -> AppResult<Option<Discussion>> {
        sqlx::query(
            "UPDATE discussions SET current_round = ?, updated_at = ?
              WHERE id = ? AND status = 'running'",
        )
        .bind(round)
        .bind(now_ms())
        .bind(id)
        .execute(self.pool())
        .await?;
        self.get_by_id(id).await
    }

    /// 续跑失败讨论：failed → running，并清空上一次的终态原因与摘要。
    ///
    /// 返回 None 表示状态已不是 failed（例如被并发续跑抢先），调用方必须
    /// 按失败处理，避免同一讨论被两个循环同时补跑。
    pub async fn reopen(&self, id: i64) -> AppResult<Option<Discussion>> {
        let result = sqlx::query(
            "UPDATE discussions
                SET status = 'running', stop_reason = NULL, summary = NULL, updated_at = ?
              WHERE id = ? AND status = 'failed'",
        )
        .bind(now_ms())
        .bind(id)
        .execute(self.pool())
        .await?;
        if result.rows_affected() != 1 {
            return Ok(None);
        }
        self.get_by_id(id).await
    }

    /// 从 running 进入终态；条件更新避免手动停止与自动收尾互相覆盖。
    ///
    /// 返回 None 表示状态已不是 running（例如用户刚手动停止），调用方
    /// 不得再追加「讨论已结束/失败」系统消息。
    pub async fn finish(
        &self,
        id: i64,
        input: FinishDiscussionInput,
    ) -> AppResult<Option<Discussion>> {
        let result = sqlx::query(
            "UPDATE discussions
                SET status = ?, stop_reason = ?, summary = ?, updated_at = ?
              WHERE id = ? AND status = 'running'",
        )
        .bind(input.status.as_str())
        .bind(input.stop_reason.as_str())
        .bind(input.summary)
        .bind(now_ms())
        .bind(id)
        .execute(self.pool())
        .await?;
        if result.rows_affected() == 0 {
            return Ok(None);
        }
        self.get_by_id(id).await
    }

    /// 启动恢复：把所有残留 running 讨论标记为失败，返回更新后的快照。
    pub async fn fail_running(
        &self,
        stop_reason: DiscussionStopReason,
    ) -> AppResult<Vec<Discussion>> {
        let running = self.list_running().await?;
        if running.is_empty() {
            return Ok(Vec::new());
        }

        let updated_at = now_ms();
        let mut updated = Vec::with_capacity(running.len());
        for discussion in running {
            let result = sqlx::query(
                "UPDATE discussions SET status = 'failed', stop_reason = ?, updated_at = ?
                  WHERE id = ? AND status = 'running'",
            )
            .bind(stop_reason.as_str())
            .bind(updated_at)
            .bind(discussion.id)
            .execute(self.pool())
            .await?;
            if result.rows_affected() == 0 {
                continue;
            }
            if let Some(snapshot) = self.get_by_id(discussion.id).await? {
                updated.push(snapshot);
            }
        }
        Ok(updated)
    }

    async fn map_row(&self, row: DiscussionRow) -> AppResult<Discussion> {
        let status = DiscussionStatus::parse(&row.status)
            .ok_or_else(|| AppError::internal(format!("discussions.status 非法：{}", row.status)))?;
        let stop_reason = row
            .stop_reason
            .as_deref()
            .map(|raw| {
                DiscussionStopReason::parse(raw).ok_or_else(|| {
                    AppError::internal(format!("discussions.stop_reason 非法：{raw}"))
                })
            })
            .transpose()?;
        let participant_ids = sqlx::query_scalar::<_, i64>(
            "SELECT agent_id FROM discussion_participants
              WHERE discussion_id = ? ORDER BY position",
        )
        .bind(row.id)
        .fetch_all(self.pool())
        .await?;

        Ok(Discussion {
            id: row.id,
            room_id: row.room_id,
            topic: row.topic,
            status,
            current_round: row.current_round,
            max_rounds: row.max_rounds,
            participant_ids,
            created_by_agent_id: row.created_by_agent_id,
            stop_reason,
            summary: row.summary,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct DiscussionRow {
    id: i64,
    room_id: i64,
    topic: String,
    status: String,
    current_round: i64,
    max_rounds: i64,
    stop_reason: Option<String>,
    summary: Option<String>,
    created_by_agent_id: Option<i64>,
    created_at: i64,
    updated_at: i64,
}
