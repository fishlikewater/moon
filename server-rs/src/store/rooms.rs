//! 房间与成员访问层。

use sqlx::SqlitePool;

use crate::db::Db;
use crate::domain::agent::Agent;
use crate::domain::room::{
    CreateRoomInput, Room, RoomPermissionMode, RoomReplyMode, UpdateRoomInput,
};
use crate::domain::session::RoomMemberSessionSettings;
use crate::domain::now_ms;
use crate::error::{AppError, AppResult};
use crate::store::agents::{map_agent_row, AgentRow};

const ROOM_COLUMNS: &str =
    "id, name, workspace_dir, workspace_dirs, moderator_agent_id, reply_mode, discussion_max_rounds, permission_mode, created_at";

#[derive(Debug, Clone)]
pub struct RoomStore {
    db: Db,
}

impl RoomStore {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    fn pool(&self) -> &SqlitePool {
        self.db.pool()
    }

    /// 房间与成员在同一事务内写入，避免出现半初始化房间。
    pub async fn create(&self, input: CreateRoomInput) -> AppResult<Room> {
        let created_at = now_ms();
        let mut member_ids = input.member_ids;
        member_ids.sort_unstable();
        member_ids.dedup();

        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let workspace_dirs = if input.workspace_dirs.is_empty() {
            vec![input.workspace_dir.clone()]
        } else {
            input.workspace_dirs
        };
        let workspace_dirs_json =
            serde_json::to_string(&workspace_dirs).map_err(|error| {
                AppError::with_source(
                    crate::error::ErrorCode::InternalError,
                    "无法序列化房间工作目录",
                    error,
                )
            })?;
        let result = sqlx::query(
            "INSERT INTO rooms
               (name, workspace_dir, workspace_dirs, moderator_agent_id, reply_mode,
                discussion_max_rounds, permission_mode, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&input.name)
        .bind(&input.workspace_dir)
        .bind(&workspace_dirs_json)
        .bind(input.moderator_agent_id)
        .bind(input.reply_mode.as_str())
        .bind(input.discussion_max_rounds)
        .bind(input.permission_mode.as_str())
        .bind(created_at)
        .execute(&mut *tx)
        .await?;
        let id = result.last_insert_rowid();

        for agent_id in &member_ids {
            sqlx::query("INSERT INTO room_members (room_id, agent_id) VALUES (?, ?)")
                .bind(id)
                .bind(agent_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;

        Ok(Room {
            id,
            name: input.name,
            workspace_dir: input.workspace_dir,
            workspace_dirs,
            moderator_agent_id: input.moderator_agent_id,
            member_ids,
            reply_mode: input.reply_mode,
            discussion_max_rounds: input.discussion_max_rounds,
            permission_mode: input.permission_mode,
            created_at,
            last_activity_at: None,
        })
    }

    pub async fn get_by_id(&self, id: i64) -> AppResult<Option<Room>> {
        let row = sqlx::query_as::<_, RoomRow>(&format!(
            "SELECT {ROOM_COLUMNS} FROM rooms WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(self.pool())
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };
        let member_ids = self.member_ids_of(id).await?;
        Ok(Some(map_room_row(row, member_ids)?))
    }

    pub async fn update(&self, id: i64, patch: UpdateRoomInput) -> AppResult<Option<Room>> {
        let Some(existing) = self.get_by_id(id).await? else {
            return Ok(None);
        };

        let name = patch.name.unwrap_or_else(|| existing.name.clone());
        let (workspace_dir, workspace_dirs) = match patch.workspace_dirs {
            Some(dirs) if !dirs.is_empty() => {
                let primary = dirs[0].clone();
                (primary, dirs)
            }
            _ => match patch.workspace_dir {
                Some(single) => (single.clone(), vec![single]),
                None => (existing.workspace_dir.clone(), existing.workspace_dirs.clone()),
            },
        };
        let workspace_dirs_json = serde_json::to_string(&workspace_dirs).map_err(|error| {
            AppError::with_source(
                crate::error::ErrorCode::InternalError,
                "无法序列化房间工作目录",
                error,
            )
        })?;
        let moderator_agent_id = match patch.moderator_agent_id {
            Some(value) => value,
            None => existing.moderator_agent_id,
        };
        let reply_mode = patch.reply_mode.unwrap_or(existing.reply_mode);
        let discussion_max_rounds = patch
            .discussion_max_rounds
            .unwrap_or(existing.discussion_max_rounds);
        let permission_mode = patch
            .permission_mode
            .unwrap_or(existing.permission_mode);

        sqlx::query(
            "UPDATE rooms
                SET name = ?, workspace_dir = ?, workspace_dirs = ?, moderator_agent_id = ?,
                    reply_mode = ?, discussion_max_rounds = ?, permission_mode = ?
              WHERE id = ?",
        )
        .bind(&name)
        .bind(&workspace_dir)
        .bind(&workspace_dirs_json)
        .bind(moderator_agent_id)
        .bind(reply_mode.as_str())
        .bind(discussion_max_rounds)
        .bind(permission_mode.as_str())
        .bind(id)
        .execute(self.pool())
        .await?;

        Ok(Some(Room {
            id,
            name,
            workspace_dir,
            workspace_dirs,
            moderator_agent_id,
            member_ids: existing.member_ids,
            reply_mode,
            discussion_max_rounds,
            permission_mode,
            created_at: existing.created_at,
            last_activity_at: existing.last_activity_at,
        }))
    }

    /// 加成员是幂等的：已是成员时不报错，便于前端重复提交。
    pub async fn add_member(&self, room_id: i64, agent_id: i64) -> AppResult<()> {
        sqlx::query("INSERT OR IGNORE INTO room_members (room_id, agent_id) VALUES (?, ?)")
            .bind(room_id)
            .bind(agent_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// 返回是否真的移除了成员关系。
    pub async fn remove_member(&self, room_id: i64, agent_id: i64) -> AppResult<bool> {
        let result = sqlx::query("DELETE FROM room_members WHERE room_id = ? AND agent_id = ?")
            .bind(room_id)
            .bind(agent_id)
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// 设置成员是否复用会话；成员不存在时返回 false。
    pub async fn set_member_reuse_session(
        &self,
        room_id: i64,
        agent_id: i64,
        reuse_session: bool,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            "UPDATE room_members SET reuse_session = ? WHERE room_id = ? AND agent_id = ?",
        )
        .bind(i64::from(reuse_session))
        .bind(room_id)
        .bind(agent_id)
        .execute(self.pool())
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// 房间全部成员的会话复用设置；按 agent id 升序。
    pub async fn list_member_session_settings(
        &self,
        room_id: i64,
    ) -> AppResult<Vec<RoomMemberSessionSettings>> {
        let rows = sqlx::query_as::<_, (i64, i64)>(
            "SELECT agent_id, reuse_session FROM room_members
              WHERE room_id = ? ORDER BY agent_id",
        )
        .bind(room_id)
        .fetch_all(self.pool())
        .await?;

        Ok(rows
            .into_iter()
            .map(|(agent_id, reuse_session)| RoomMemberSessionSettings {
                agent_id,
                reuse_session: reuse_session != 0,
            })
            .collect())
    }

    /// 删除房间及其全部房间级数据。
    ///
    /// 删除顺序按外键依赖排列：先清任务事件与委派指派；批次与消息互相引用
    /// （批次指向触发它的用户消息，消息指向所属批次），所以要先断开消息上的
    /// 批次引用，再删批次和消息；之后清运行、讨论参与者、任务、讨论与成员
    /// 关系，最后删房间本身。整个过程在同一事务内。
    pub async fn delete(&self, id: i64) -> AppResult<bool> {
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;

        sqlx::query(
            "DELETE FROM task_events WHERE task_id IN (SELECT id FROM tasks WHERE room_id = ?)",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "DELETE FROM collaboration_assignments
              WHERE batch_id IN (SELECT id FROM collaboration_batches WHERE room_id = ?)",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE messages SET collaboration_batch_id = NULL WHERE room_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM collaboration_batches WHERE room_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM messages WHERE room_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM agent_runs WHERE room_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "DELETE FROM discussion_participants
              WHERE discussion_id IN (SELECT id FROM discussions WHERE room_id = ?)",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM discussions WHERE room_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM tasks WHERE room_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM agent_sessions WHERE room_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM room_members WHERE room_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;

        let result = sqlx::query("DELETE FROM rooms WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn list(&self) -> AppResult<Vec<Room>> {
        let rows = sqlx::query_as::<_, RoomRow>(&format!(
            "SELECT {ROOM_COLUMNS} FROM rooms ORDER BY id"
        ))
        .fetch_all(self.pool())
        .await?;

        // 一次查询取回全部成员关系再分组，避免按房间逐条查询。
        let links = sqlx::query_as::<_, (i64, i64)>(
            "SELECT room_id, agent_id FROM room_members ORDER BY room_id, agent_id",
        )
        .fetch_all(self.pool())
        .await?;

        let mut member_ids_by_room: std::collections::HashMap<i64, Vec<i64>> =
            std::collections::HashMap::new();
        for (room_id, agent_id) in links {
            member_ids_by_room.entry(room_id).or_default().push(agent_id);
        }

        // 最近活动时间：最近一条消息的 created_at；无消息的房间不出现在结果里。
        let activities = sqlx::query_as::<_, (i64, Option<i64>)>(
            "SELECT room_id, MAX(created_at) FROM messages GROUP BY room_id",
        )
        .fetch_all(self.pool())
        .await?;
        let activity_by_room: std::collections::HashMap<i64, i64> = activities
            .into_iter()
            .filter_map(|(room_id, last)| last.map(|value| (room_id, value)))
            .collect();

        rows.into_iter()
            .map(|row| {
                let id = row.id;
                let mut room = map_room_row(
                    row,
                    member_ids_by_room.get(&id).cloned().unwrap_or_default(),
                )?;
                room.last_activity_at = activity_by_room.get(&id).copied();
                Ok(room)
            })
            .collect()
    }

    /// 房间成员及其人设；按 agent id 升序，供 @ 提及解析使用。
    pub async fn list_members(&self, room_id: i64) -> AppResult<Vec<Agent>> {
        let rows = sqlx::query_as::<_, AgentRow>(&format!(
            "SELECT a.id, a.name, a.persona, a.harness, a.model_config_json,
                    a.builtin_role, a.custom_harness_id, a.workspace_dir, a.created_at
               FROM room_members m
               JOIN agents a ON a.id = m.agent_id
              WHERE m.room_id = ?
              ORDER BY a.id"
        ))
        .bind(room_id)
        .fetch_all(self.pool())
        .await?;

        rows.into_iter().map(map_agent_row).collect()
    }

    async fn member_ids_of(&self, room_id: i64) -> AppResult<Vec<i64>> {
        let rows = sqlx::query_scalar::<_, i64>(
            "SELECT agent_id FROM room_members WHERE room_id = ? ORDER BY agent_id",
        )
        .bind(room_id)
        .fetch_all(self.pool())
        .await?;
        Ok(rows)
    }

    pub async fn has_running_discussion(&self, room_id: i64) -> AppResult<bool> {
        let exists: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM discussions WHERE room_id = ? AND status = 'running')",
        )
        .bind(room_id)
        .fetch_one(self.pool())
        .await?;
        Ok(exists != 0)
    }

    pub async fn has_running_collaboration(&self, room_id: i64) -> AppResult<bool> {
        let exists: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM collaboration_batches
                            WHERE room_id = ? AND status = 'running')",
        )
        .bind(room_id)
        .fetch_one(self.pool())
        .await?;
        Ok(exists != 0)
    }
}

#[derive(sqlx::FromRow)]
struct RoomRow {
    id: i64,
    name: String,
    workspace_dir: String,
    workspace_dirs: Option<String>,
    moderator_agent_id: Option<i64>,
    reply_mode: String,
    discussion_max_rounds: i64,
    permission_mode: String,
    created_at: i64,
}

fn map_room_row(row: RoomRow, member_ids: Vec<i64>) -> AppResult<Room> {
    let reply_mode = RoomReplyMode::parse(&row.reply_mode)
        .map_err(|_| AppError::internal(format!("rooms.reply_mode 非法：{}", row.reply_mode)))?;
    let permission_mode =
        RoomPermissionMode::parse(&row.permission_mode).map_err(|_| {
            AppError::internal(format!(
                "rooms.permission_mode 非法：{}",
                row.permission_mode
            ))
        })?;

    let mut workspace_dirs: Vec<String> = row
        .workspace_dirs
        .as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_default();
    if workspace_dirs.is_empty() {
        workspace_dirs.push(row.workspace_dir.clone());
    }
    if workspace_dirs[0] != row.workspace_dir {
        workspace_dirs.retain(|dir| dir != &row.workspace_dir);
        workspace_dirs.insert(0, row.workspace_dir.clone());
    }

    Ok(Room {
        id: row.id,
        name: row.name,
        workspace_dir: row.workspace_dir,
        workspace_dirs,
        moderator_agent_id: row.moderator_agent_id,
        member_ids,
        reply_mode,
        discussion_max_rounds: row.discussion_max_rounds,
        permission_mode,
        created_at: row.created_at,
        last_activity_at: None,
    })
}
