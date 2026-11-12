//! 房间队列持久化访问层。
//!
//! 表内只保留「尚未开跑」的任务：入队时插入，开跑或取消时删除。
//! 运行中的任务由 `agent_runs` 与启动恢复负责，不在此表留下历史行。

use sqlx::SqlitePool;

use crate::db::Db;
use crate::error::AppResult;

/// 新入队的待执行任务。
#[derive(Debug, Clone)]
pub struct NewQueueJob {
    pub room_id: i64,
    pub agent_id: i64,
    pub label: String,
    pub kind: String,
    pub instruction: String,
    pub moderator_trigger_message_id: Option<i64>,
    pub enqueued_at: i64,
}

/// 待恢复的队列任务；`kind` 由调用方解析，未知类型直接清理。
#[derive(Debug, Clone)]
pub struct PersistedQueueJob {
    pub id: i64,
    pub room_id: i64,
    pub agent_id: i64,
    pub label: String,
    pub kind: String,
    pub instruction: String,
    pub moderator_trigger_message_id: Option<i64>,
    pub enqueued_at: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct QueueJobRow {
    id: i64,
    room_id: i64,
    agent_id: i64,
    label: String,
    kind: String,
    instruction: String,
    moderator_trigger_message_id: Option<i64>,
    enqueued_at: i64,
}

impl From<QueueJobRow> for PersistedQueueJob {
    fn from(row: QueueJobRow) -> Self {
        Self {
            id: row.id,
            room_id: row.room_id,
            agent_id: row.agent_id,
            label: row.label,
            kind: row.kind,
            instruction: row.instruction,
            moderator_trigger_message_id: row.moderator_trigger_message_id,
            enqueued_at: row.enqueued_at,
        }
    }
}

#[derive(Debug, Clone)]
pub struct QueueStore {
    db: Db,
}

impl QueueStore {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    fn pool(&self) -> &SqlitePool {
        self.db.pool()
    }

    /// 登记一条待执行任务，返回持久化行 id。
    pub async fn insert(&self, job: NewQueueJob) -> AppResult<i64> {
        let result = sqlx::query(
            "INSERT INTO room_queue_jobs (
               room_id, agent_id, label, kind, instruction,
               moderator_trigger_message_id, enqueued_at
             )
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(job.room_id)
        .bind(job.agent_id)
        .bind(job.label)
        .bind(job.kind)
        .bind(job.instruction)
        .bind(job.moderator_trigger_message_id)
        .bind(job.enqueued_at)
        .execute(self.pool())
        .await?;
        Ok(result.last_insert_rowid())
    }

    /// 任务已开跑或已取消：删除待执行行。
    pub async fn delete(&self, id: i64) -> AppResult<()> {
        sqlx::query("DELETE FROM room_queue_jobs WHERE id = ?")
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// 该待执行行是否仍存在；恢复驱动据此判断上一任务是否已离开等待。
    pub async fn exists(&self, id: i64) -> AppResult<bool> {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM room_queue_jobs WHERE id = ?")
                .bind(id)
                .fetch_one(self.pool())
                .await?;
        Ok(count > 0)
    }

    /// 全部待恢复任务，按入队顺序（行 id 升序）。
    pub async fn list_pending(&self) -> AppResult<Vec<PersistedQueueJob>> {
        let rows = sqlx::query_as::<_, QueueJobRow>(
            "SELECT id, room_id, agent_id, label, kind, instruction,
                    moderator_trigger_message_id, enqueued_at
               FROM room_queue_jobs
              ORDER BY id",
        )
        .fetch_all(self.pool())
        .await?;
        Ok(rows.into_iter().map(PersistedQueueJob::from).collect())
    }

    /// 启动时读取仍处于暂停状态的房间。
    pub async fn list_paused_rooms(&self) -> AppResult<Vec<i64>> {
        Ok(sqlx::query_scalar("SELECT id FROM rooms WHERE queue_paused = 1 ORDER BY id")
            .fetch_all(self.pool())
            .await?)
    }

    /// 写入房间级暂停状态。
    pub async fn set_paused(&self, room_id: i64, paused: bool) -> AppResult<()> {
        sqlx::query("UPDATE rooms SET queue_paused = ? WHERE id = ?")
            .bind(if paused { 1_i64 } else { 0_i64 })
            .bind(room_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }
}
