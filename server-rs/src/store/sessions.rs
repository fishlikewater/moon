//! 房间智能体会话访问层。
//!
//! 会话按 `(room_id, agent_id)` 隔离：同一智能体在不同房间各自持有独立
//! 会话与存储目录，跨房间永不共享。`agent_sessions` 上只有部分唯一索引
//! 约束「每房间每智能体一个 active 会话」，归档后可以再建新会话。
//!
//! `storage_dir` 是相对会话存储根目录的路径（形如
//! `room-1/agent-2/s-<createdAt>-<random>`），绝对路径解析与目录清理
//! 由服务层负责；本层只做数据库读写。

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use sqlx::SqlitePool;

use crate::db::Db;
use crate::domain::agent::HarnessKind;
use crate::domain::now_ms;
use crate::domain::session::{AgentSessionStatus, SessionOverview, StoredAgentSession};
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct SessionStore {
    db: Db,
}

#[derive(Debug, Clone)]
pub struct CreateAgentSessionInput {
    pub room_id: i64,
    pub agent_id: i64,
    pub harness: HarnessKind,
}

#[derive(Debug, Clone, Default)]
pub struct TouchAgentSessionInput {
    /// harness 会话 ID；`Some(Some(_))` 覆盖，`Some(None)` 清空，`None` 保持。
    pub harness_session_id: Option<Option<String>>,
    /// 最近使用该会话的运行 id。
    pub run_id: Option<Option<i64>>,
}

impl SessionStore {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    fn pool(&self) -> &SqlitePool {
        self.db.pool()
    }

    /// 取当前活动会话；没有时返回 None。
    pub async fn get_active(
        &self,
        room_id: i64,
        agent_id: i64,
    ) -> AppResult<Option<StoredAgentSession>> {
        let row = sqlx::query_as::<_, SessionRow>(
            "SELECT id, room_id, agent_id, harness, harness_session_id, status,
                    storage_dir, created_at, last_used_at, last_run_id
               FROM agent_sessions
              WHERE room_id = ? AND agent_id = ? AND status = 'active'",
        )
        .bind(room_id)
        .bind(agent_id)
        .fetch_optional(self.pool())
        .await?;

        row.map(map_session_row).transpose()
    }

    /// 取活动会话，不存在时创建。
    ///
    /// 并发下两个调用可能同时看到「无活动会话」；后插入者会撞上部分唯一
    /// 索引，此时回读已存在的会话返回，保证同一 `(room, agent)` 只有一个
    /// active 记录。
    pub async fn ensure_active(
        &self,
        input: CreateAgentSessionInput,
    ) -> AppResult<StoredAgentSession> {
        if let Some(existing) = self.get_active(input.room_id, input.agent_id).await? {
            return Ok(existing);
        }

        let created_at = now_ms();
        let storage_dir =
            build_storage_dir(input.room_id, input.agent_id, created_at);

        let result = sqlx::query(
            "INSERT INTO agent_sessions
               (room_id, agent_id, harness, harness_session_id, status,
                storage_dir, created_at, last_used_at, last_run_id)
             VALUES (?, ?, ?, NULL, 'active', ?, ?, NULL, NULL)",
        )
        .bind(input.room_id)
        .bind(input.agent_id)
        .bind(input.harness.as_str())
        .bind(&storage_dir)
        .bind(created_at)
        .execute(self.pool())
        .await;

        match result {
            Ok(result) => Ok(StoredAgentSession {
                id: result.last_insert_rowid(),
                room_id: input.room_id,
                agent_id: input.agent_id,
                harness: input.harness,
                harness_session_id: None,
                status: AgentSessionStatus::Active,
                storage_dir,
                created_at,
                last_used_at: None,
                last_run_id: None,
            }),
            Err(error) => {
                if let sqlx::Error::Database(db_error) = &error {
                    if db_error.is_unique_violation() {
                        if let Some(raced) = self
                            .get_active(input.room_id, input.agent_id)
                            .await?
                        {
                            return Ok(raced);
                        }
                    }
                }
                Err(error.into())
            }
        }
    }

    /// 记录会话 ID 与最近使用信息；未传的字段保持原值。
    pub async fn touch(&self, id: i64, input: TouchAgentSessionInput) -> AppResult<()> {
        let Some(existing) = self.get_by_id(id).await? else {
            return Ok(());
        };

        let harness_session_id = match input.harness_session_id {
            Some(value) => value,
            None => existing.harness_session_id,
        };
        let last_run_id = match input.run_id {
            Some(value) => value,
            None => existing.last_run_id,
        };

        sqlx::query(
            "UPDATE agent_sessions
                SET harness_session_id = ?, last_used_at = ?, last_run_id = ?
              WHERE id = ?",
        )
        .bind(harness_session_id)
        .bind(now_ms())
        .bind(last_run_id)
        .bind(id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// 归档：保留记录与存储，后续运行不再续接该会话。
    pub async fn archive(&self, id: i64) -> AppResult<bool> {
        let result = sqlx::query(
            "UPDATE agent_sessions SET status = 'archived' WHERE id = ? AND status = 'active'",
        )
        .bind(id)
        .execute(self.pool())
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// 删除记录；存储目录由服务层删除。
    pub async fn remove(&self, id: i64) -> AppResult<bool> {
        let result = sqlx::query("DELETE FROM agent_sessions WHERE id = ?")
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn get_by_id(&self, id: i64) -> AppResult<Option<StoredAgentSession>> {
        let row = sqlx::query_as::<_, SessionRow>(
            "SELECT id, room_id, agent_id, harness, harness_session_id, status,
                    storage_dir, created_at, last_used_at, last_run_id
               FROM agent_sessions WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await?;

        row.map(map_session_row).transpose()
    }

    /// 全部房间的会话（含归档），JOIN 房间名与智能体名；
    /// 按最近使用倒序，未使用过用创建时间，id 倒序兜底。
    pub async fn list_all(&self) -> AppResult<Vec<SessionOverview>> {
        let rows = sqlx::query_as::<_, SessionOverviewRow>(
            "SELECT s.id, s.room_id, s.agent_id, s.harness, s.harness_session_id,
                    s.status, s.created_at, s.last_used_at, s.last_run_id,
                    r.name AS room_name, a.name AS agent_name
               FROM agent_sessions AS s
               JOIN rooms AS r ON r.id = s.room_id
               JOIN agents AS a ON a.id = s.agent_id
              ORDER BY COALESCE(s.last_used_at, s.created_at) DESC, s.id DESC",
        )
        .fetch_all(self.pool())
        .await?;

        rows.into_iter().map(map_overview_row).collect()
    }

    /// 房间全部会话（含归档），按创建顺序倒序。
    pub async fn list_by_room(&self, room_id: i64) -> AppResult<Vec<StoredAgentSession>> {
        let rows = sqlx::query_as::<_, SessionRow>(
            "SELECT id, room_id, agent_id, harness, harness_session_id, status,
                    storage_dir, created_at, last_used_at, last_run_id
               FROM agent_sessions
              WHERE room_id = ?
              ORDER BY id DESC",
        )
        .bind(room_id)
        .fetch_all(self.pool())
        .await?;

        rows.into_iter().map(map_session_row).collect()
    }

    /// 成员被移出房间时归档其活动会话，避免会话继续被续接。
    pub async fn archive_by_agent(&self, room_id: i64, agent_id: i64) -> AppResult<bool> {
        let result = sqlx::query(
            "UPDATE agent_sessions SET status = 'archived'
              WHERE room_id = ? AND agent_id = ? AND status = 'active'",
        )
        .bind(room_id)
        .bind(agent_id)
        .execute(self.pool())
        .await?;
        Ok(result.rows_affected() > 0)
    }
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    id: i64,
    room_id: i64,
    agent_id: i64,
    harness: String,
    harness_session_id: Option<String>,
    status: String,
    storage_dir: String,
    created_at: i64,
    last_used_at: Option<i64>,
    last_run_id: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct SessionOverviewRow {
    id: i64,
    room_id: i64,
    agent_id: i64,
    harness: String,
    harness_session_id: Option<String>,
    status: String,
    created_at: i64,
    last_used_at: Option<i64>,
    last_run_id: Option<i64>,
    room_name: String,
    agent_name: String,
}

fn map_overview_row(row: SessionOverviewRow) -> AppResult<SessionOverview> {
    let harness = HarnessKind::parse(&row.harness).map_err(|_| {
        AppError::internal(format!("agent_sessions.harness 非法：{}", row.harness))
    })?;
    Ok(SessionOverview {
        id: row.id,
        room_id: row.room_id,
        room_name: row.room_name,
        agent_id: row.agent_id,
        agent_name: row.agent_name,
        harness,
        harness_session_id: row.harness_session_id,
        status: if row.status == "archived" {
            AgentSessionStatus::Archived
        } else {
            AgentSessionStatus::Active
        },
        created_at: row.created_at,
        last_used_at: row.last_used_at,
        last_run_id: row.last_run_id,
    })
}

fn map_session_row(row: SessionRow) -> AppResult<StoredAgentSession> {
    let harness = HarnessKind::parse(&row.harness).map_err(|_| {
        AppError::internal(format!("agent_sessions.harness 非法：{}", row.harness))
    })?;
    Ok(StoredAgentSession {
        id: row.id,
        room_id: row.room_id,
        agent_id: row.agent_id,
        harness,
        harness_session_id: row.harness_session_id,
        status: if row.status == "archived" {
            AgentSessionStatus::Archived
        } else {
            AgentSessionStatus::Active
        },
        storage_dir: row.storage_dir,
        created_at: row.created_at,
        last_used_at: row.last_used_at,
        last_run_id: row.last_run_id,
    })
}

/// 生成会话存储目录（相对路径）。
///
/// 目录名携带创建时间与随机后缀，创建前即可确定，不需要插入后二次更新；
/// 归档会话保留原目录，新会话天然使用新目录，互不覆盖。
fn build_storage_dir(room_id: i64, agent_id: i64, created_at: i64) -> String {
    format!(
        "room-{room_id}/agent-{agent_id}/s-{created_at}-{}",
        random_suffix()
    )
}

/// 4 位 base36 后缀；只需避免同毫秒内的目录碰撞，不做密码学随机。
fn random_suffix() -> String {
    const ALPHABET: [u8; 36] = *b"0123456789abcdefghijklmnopqrstuvwxyz";
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos() as u64)
        .unwrap_or(0);
    let mut value = nanos
        ^ COUNTER
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut suffix = String::with_capacity(4);
    for _ in 0..4 {
        suffix.push(ALPHABET[(value % 36) as usize] as char);
        value /= 36;
    }
    suffix
}
