//! 委派批次与指派的持久化访问层。
//!
//! `review_run_id = 0` 是「已认领、尚未拿到 run id」的哨兵值，避免两个
//! 完成回调同时调度同一轮汇总；真实 run id 在运行创建后再写回。

use sqlx::SqlitePool;

use crate::db::Db;
use crate::domain::collaboration::{
    CollaborationAssignment, CollaborationAssignmentStatus, CollaborationBatch, CollaborationStatus,
    CollaborationStopReason,
};
use crate::domain::now_ms;
use crate::error::{AppError, AppResult};

const BATCH_COLUMNS: &str = "id, room_id, moderator_agent_id, user_message_id, status, current_round, \
     max_rounds, stop_reason, stop_requested, summary, review_run_id, created_at, updated_at";

const ASSIGNMENT_COLUMNS: &str = "id, batch_id, round, agent_id, instruction, status, run_id, \
     message_id, error, created_at, finished_at";

#[derive(Debug, Clone)]
pub struct CreateCollaborationBatchInput {
    pub room_id: i64,
    pub moderator_agent_id: i64,
    pub user_message_id: i64,
    pub max_rounds: i64,
}

#[derive(Debug, Clone)]
pub struct AddCollaborationAssignmentInput {
    pub batch_id: i64,
    pub round: i64,
    pub agent_id: i64,
    pub instruction: String,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateCollaborationAssignmentInput {
    pub status: Option<CollaborationAssignmentStatus>,
    pub run_id: Option<Option<i64>>,
    pub message_id: Option<Option<i64>>,
    pub error: Option<Option<String>>,
    pub finished_at: Option<Option<i64>>,
}

#[derive(Debug, Clone)]
pub struct FinishCollaborationBatchInput {
    pub status: CollaborationStatus,
    pub stop_reason: CollaborationStopReason,
    pub summary: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CollaborationStore {
    db: Db,
}

impl CollaborationStore {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    fn pool(&self) -> &SqlitePool {
        self.db.pool()
    }

    pub async fn create_batch(
        &self,
        input: CreateCollaborationBatchInput,
    ) -> AppResult<CollaborationBatch> {
        let created_at = now_ms();
        let result = sqlx::query(
            "INSERT INTO collaboration_batches
               (room_id, moderator_agent_id, user_message_id, status, current_round,
                max_rounds, stop_reason, stop_requested, summary, review_run_id,
                created_at, updated_at)
             VALUES (?, ?, ?, 'running', 1, ?, NULL, 0, NULL, NULL, ?, ?)",
        )
        .bind(input.room_id)
        .bind(input.moderator_agent_id)
        .bind(input.user_message_id)
        .bind(input.max_rounds)
        .bind(created_at)
        .bind(created_at)
        .execute(self.pool())
        .await?;
        self.require_batch(result.last_insert_rowid()).await
    }

    pub async fn add_assignment(
        &self,
        input: AddCollaborationAssignmentInput,
    ) -> AppResult<CollaborationAssignment> {
        let created_at = now_ms();
        let result = sqlx::query(
            "INSERT INTO collaboration_assignments
               (batch_id, round, agent_id, instruction, status, run_id, message_id,
                error, created_at, finished_at)
             VALUES (?, ?, ?, ?, 'queued', NULL, NULL, NULL, ?, NULL)",
        )
        .bind(input.batch_id)
        .bind(input.round)
        .bind(input.agent_id)
        .bind(&input.instruction)
        .bind(created_at)
        .execute(self.pool())
        .await?;
        self.require_assignment(result.last_insert_rowid()).await
    }

    pub async fn update_assignment(
        &self,
        id: i64,
        patch: UpdateCollaborationAssignmentInput,
    ) -> AppResult<Option<CollaborationAssignment>> {
        let Some(existing) = self.get_assignment_by_id(id).await? else {
            return Ok(None);
        };
        let status = patch.status.unwrap_or(existing.status);
        let run_id = patch.run_id.unwrap_or(existing.run_id);
        let message_id = patch.message_id.unwrap_or(existing.message_id);
        let error = patch.error.unwrap_or(existing.error);
        let finished_at = patch.finished_at.unwrap_or(existing.finished_at);

        sqlx::query(
            "UPDATE collaboration_assignments
                SET status = ?, run_id = ?, message_id = ?, error = ?, finished_at = ?
              WHERE id = ?",
        )
        .bind(status.as_str())
        .bind(run_id)
        .bind(message_id)
        .bind(&error)
        .bind(finished_at)
        .bind(id)
        .execute(self.pool())
        .await?;

        Ok(Some(CollaborationAssignment {
            status,
            run_id,
            message_id,
            error,
            finished_at,
            ..existing
        }))
    }

    pub async fn get_assignment_by_id(
        &self,
        id: i64,
    ) -> AppResult<Option<CollaborationAssignment>> {
        let row = sqlx::query_as::<_, AssignmentRow>(&format!(
            "SELECT {ASSIGNMENT_COLUMNS} FROM collaboration_assignments WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(self.pool())
        .await?;
        row.map(map_assignment_row).transpose()
    }

    pub async fn get_by_id(&self, id: i64) -> AppResult<Option<CollaborationBatch>> {
        let row = sqlx::query_as::<_, BatchRow>(&format!(
            "SELECT {BATCH_COLUMNS} FROM collaboration_batches WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(self.pool())
        .await?;
        match row {
            None => Ok(None),
            Some(row) => self.map_batch_row(row).await.map(Some),
        }
    }

    /// 房间最新批次（含已结束），供刷新时水合状态条。
    pub async fn latest_by_room(&self, room_id: i64) -> AppResult<Option<CollaborationBatch>> {
        let row = sqlx::query_as::<_, BatchRow>(&format!(
            "SELECT {BATCH_COLUMNS} FROM collaboration_batches
              WHERE room_id = ? ORDER BY id DESC LIMIT 1"
        ))
        .bind(room_id)
        .fetch_optional(self.pool())
        .await?;
        match row {
            None => Ok(None),
            Some(row) => self.map_batch_row(row).await.map(Some),
        }
    }

    pub async fn list_assignments(&self, batch_id: i64) -> AppResult<Vec<CollaborationAssignment>> {
        let rows = sqlx::query_as::<_, AssignmentRow>(&format!(
            "SELECT {ASSIGNMENT_COLUMNS} FROM collaboration_assignments
              WHERE batch_id = ? ORDER BY round, id"
        ))
        .bind(batch_id)
        .fetch_all(self.pool())
        .await?;
        rows.into_iter().map(map_assignment_row).collect()
    }

    pub async fn list_running(&self) -> AppResult<Vec<CollaborationBatch>> {
        let rows = sqlx::query_as::<_, BatchRow>(&format!(
            "SELECT {BATCH_COLUMNS} FROM collaboration_batches
              WHERE status = 'running' ORDER BY id"
        ))
        .fetch_all(self.pool())
        .await?;
        let mut batches = Vec::with_capacity(rows.len());
        for row in rows {
            batches.push(self.map_batch_row(row).await?);
        }
        Ok(batches)
    }

    pub async fn has_running_for_room(&self, room_id: i64) -> AppResult<bool> {
        Ok(sqlx::query_scalar::<_, i64>(
            "SELECT 1 FROM collaboration_batches
              WHERE room_id = ? AND status = 'running' LIMIT 1",
        )
        .bind(room_id)
        .fetch_optional(self.pool())
        .await?
        .is_some())
    }

    /// 智能体是否仍属于运行中的批次；批次收口前不能删除其指派。
    pub async fn has_running_for_agent(&self, agent_id: i64) -> AppResult<bool> {
        Ok(sqlx::query_scalar::<_, i64>(
            "SELECT 1
               FROM collaboration_assignments AS a
               JOIN collaboration_batches AS b ON b.id = a.batch_id
              WHERE a.agent_id = ? AND b.status = 'running'
              LIMIT 1",
        )
        .bind(agent_id)
        .fetch_optional(self.pool())
        .await?
        .is_some())
    }

    /// 认领当前轮的汇总运行；只有 running、未请求停止且本轮回调尚未认领时成功。
    pub async fn claim_review(&self, batch_id: i64, round: i64) -> AppResult<bool> {
        let result = sqlx::query(
            "UPDATE collaboration_batches
                SET review_run_id = 0, updated_at = ?
              WHERE id = ?
                AND status = 'running'
                AND current_round = ?
                AND stop_requested = 0
                AND review_run_id IS NULL",
        )
        .bind(now_ms())
        .bind(batch_id)
        .bind(round)
        .execute(self.pool())
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn set_review_run_id(
        &self,
        id: i64,
        run_id: i64,
    ) -> AppResult<Option<CollaborationBatch>> {
        sqlx::query(
            "UPDATE collaboration_batches
                SET review_run_id = ?, updated_at = ?
              WHERE id = ? AND review_run_id = 0",
        )
        .bind(run_id)
        .bind(now_ms())
        .bind(id)
        .execute(self.pool())
        .await?;
        self.get_by_id(id).await
    }

    /// 进入下一轮；同时清空本轮的汇总认领标记。
    pub async fn advance_round(
        &self,
        id: i64,
        round: i64,
    ) -> AppResult<Option<CollaborationBatch>> {
        sqlx::query(
            "UPDATE collaboration_batches
                SET current_round = ?, review_run_id = NULL, updated_at = ?
              WHERE id = ? AND status = 'running'",
        )
        .bind(round)
        .bind(now_ms())
        .bind(id)
        .execute(self.pool())
        .await?;
        self.get_by_id(id).await
    }

    /// 标记用户请求停止；当前轮收尾后不再派发下一轮。
    pub async fn request_stop(
        &self,
        id: i64,
    ) -> AppResult<Option<CollaborationBatch>> {
        sqlx::query(
            "UPDATE collaboration_batches
                SET stop_requested = 1, updated_at = ?
              WHERE id = ? AND status = 'running'",
        )
        .bind(now_ms())
        .bind(id)
        .execute(self.pool())
        .await?;
        self.get_by_id(id).await
    }

    /// 从 running 进入终态；条件更新避免用户停止与自动收尾互相覆盖。
    pub async fn finish(
        &self,
        id: i64,
        input: FinishCollaborationBatchInput,
    ) -> AppResult<Option<CollaborationBatch>> {
        let result = sqlx::query(
            "UPDATE collaboration_batches
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

    /// 启动恢复：残留批次与未完成指派统一失败化，返回更新后的批次快照。
    pub async fn fail_running(
        &self,
        stop_reason: CollaborationStopReason,
    ) -> AppResult<Vec<CollaborationBatch>> {
        let running = self.list_running().await?;
        if running.is_empty() {
            return Ok(Vec::new());
        }
        let now = now_ms();
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        for batch in &running {
            sqlx::query(
                "UPDATE collaboration_batches
                    SET status = 'failed', stop_reason = ?, updated_at = ?
                  WHERE id = ? AND status = 'running'",
            )
            .bind(stop_reason.as_str())
            .bind(now)
            .bind(batch.id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE collaboration_assignments
                    SET status = 'failed', error = ?, finished_at = ?
                  WHERE batch_id = ? AND status IN ('queued', 'running')",
            )
            .bind(stop_reason.as_str())
            .bind(now)
            .bind(batch.id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;

        let mut updated = Vec::with_capacity(running.len());
        for batch in running {
            if let Some(snapshot) = self.get_by_id(batch.id).await? {
                if snapshot.status == CollaborationStatus::Failed {
                    updated.push(snapshot);
                }
            }
        }
        Ok(updated)
    }

    async fn map_batch_row(&self, row: BatchRow) -> AppResult<CollaborationBatch> {
        let status = CollaborationStatus::parse(&row.status)
            .ok_or_else(|| AppError::internal(format!("collaboration_batches.status 非法：{}", row.status)))?;
        let stop_reason = row
            .stop_reason
            .as_deref()
            .map(|raw| {
                CollaborationStopReason::parse(raw).ok_or_else(|| {
                    AppError::internal(format!(
                        "collaboration_batches.stop_reason 非法：{raw}"
                    ))
                })
            })
            .transpose()?;
        let assignments = self.list_assignments(row.id).await?;
        Ok(CollaborationBatch {
            id: row.id,
            room_id: row.room_id,
            moderator_agent_id: row.moderator_agent_id,
            user_message_id: row.user_message_id,
            status,
            current_round: row.current_round,
            max_rounds: row.max_rounds,
            stop_reason,
            stop_requested: row.stop_requested != 0,
            summary: row.summary,
            review_run_id: row.review_run_id,
            assignments,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    async fn require_batch(&self, id: i64) -> AppResult<CollaborationBatch> {
        self.get_by_id(id)
            .await?
            .ok_or_else(|| AppError::internal(format!("委派批次不存在：{id}")))
    }

    async fn require_assignment(&self, id: i64) -> AppResult<CollaborationAssignment> {
        self.get_assignment_by_id(id)
            .await?
            .ok_or_else(|| AppError::internal(format!("委派指派不存在：{id}")))
    }
}

#[derive(sqlx::FromRow)]
struct BatchRow {
    id: i64,
    room_id: i64,
    moderator_agent_id: i64,
    user_message_id: i64,
    status: String,
    current_round: i64,
    max_rounds: i64,
    stop_reason: Option<String>,
    stop_requested: i64,
    summary: Option<String>,
    review_run_id: Option<i64>,
    created_at: i64,
    updated_at: i64,
}

#[derive(sqlx::FromRow)]
struct AssignmentRow {
    id: i64,
    batch_id: i64,
    round: i64,
    agent_id: i64,
    instruction: String,
    status: String,
    run_id: Option<i64>,
    message_id: Option<i64>,
    error: Option<String>,
    created_at: i64,
    finished_at: Option<i64>,
}

fn map_assignment_row(row: AssignmentRow) -> AppResult<CollaborationAssignment> {
    let status = CollaborationAssignmentStatus::parse(&row.status).ok_or_else(|| {
        AppError::internal(format!(
            "collaboration_assignments.status 非法：{}",
            row.status
        ))
    })?;
    Ok(CollaborationAssignment {
        id: row.id,
        batch_id: row.batch_id,
        round: row.round,
        agent_id: row.agent_id,
        instruction: row.instruction,
        status,
        run_id: row.run_id,
        message_id: row.message_id,
        error: row.error,
        created_at: row.created_at,
        finished_at: row.finished_at,
    })
}
