//! 房间智能体会话业务规则。
//!
//! 会话隔离键固定为 `(roomId, agentId)`：同一智能体在不同房间各自持有
//! 独立会话与存储目录。归档 = 结束当前会话（下次运行新建），存储保留；
//! 删除 = 记录与存储目录一起移除。房间有排队/运行中时禁止归档/删除，
//! 避免与运行中的 harness 竞争同一会话库。

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde_json::Value;

use crate::domain::agent::Agent;
use crate::domain::session::{
    AgentSession, AgentSessionStatus, ResolvedRunSession, RoomSessionsResponse,
    RoomMemberSessionSettings, RunSessionContext, SessionOverview, StoredAgentSession,
};
use crate::error::{AppError, AppResult};
use crate::services::{parse_id_label, require_body};
use crate::store::{RoomStore, SessionStore};

type BusyChecker = Arc<dyn Fn(i64) -> bool + Send + Sync>;

pub struct SessionService {
    sessions: SessionStore,
    rooms: RoomStore,
    storage_root: PathBuf,
    /// 房间忙判定；ChatService 装配完成后注入真实队列查询。
    busy_checker: RwLock<Option<BusyChecker>>,
}

impl SessionService {
    pub fn new(
        sessions: SessionStore,
        rooms: RoomStore,
        storage_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            sessions,
            rooms,
            storage_root: storage_root.into(),
            busy_checker: RwLock::new(None),
        }
    }

    /// 由 app 在 ChatService 装配后注入；测试可直接注入。
    pub fn set_busy_checker(&self, checker: BusyChecker) {
        if let Ok(mut guard) = self.busy_checker.write() {
            *guard = Some(checker);
        }
    }

    /// 全部房间的会话总览（含归档）。
    pub async fn list_all(&self) -> AppResult<Vec<SessionOverview>> {
        self.sessions.list_all().await
    }

    /// 房间会话列表与成员复用设置；房间不存在时 404。
    pub async fn list(&self, raw_room_id: &str) -> AppResult<RoomSessionsResponse> {
        let room_id = self.require_room_id(raw_room_id).await?;
        let members = self.rooms.list_member_session_settings(room_id).await?;
        let sessions = self
            .sessions
            .list_by_room(room_id)
            .await?
            .iter()
            .map(StoredAgentSession::to_contract)
            .collect();

        Ok(RoomSessionsResponse { members, sessions })
    }

    /// 设置成员是否复用会话；非成员统一 404。
    pub async fn set_reuse_session(
        &self,
        raw_room_id: &str,
        raw_agent_id: &str,
        body: Value,
    ) -> AppResult<RoomMemberSessionSettings> {
        let room_id = self.require_room_id(raw_room_id).await?;
        let agent_id = parse_id_label(raw_agent_id, "智能体")?;
        let input = require_body(body)?;
        let reuse_session = match input.get("reuseSession") {
            Some(Value::Bool(value)) => *value,
            _ => {
                return Err(AppError::validation(
                    "reuseSession 必须是布尔值",
                ))
            }
        };

        if !self
            .rooms
            .set_member_reuse_session(room_id, agent_id, reuse_session)
            .await?
        {
            return Err(AppError::not_found(format!(
                "智能体 {agent_id} 不是房间 {room_id} 的成员"
            )));
        }
        Ok(RoomMemberSessionSettings {
            agent_id,
            reuse_session,
        })
    }

    /// 归档当前会话：保留存储，下次运行新建会话。
    pub async fn archive(
        &self,
        raw_room_id: &str,
        raw_session_id: &str,
    ) -> AppResult<AgentSession> {
        let session = self.require_session(raw_room_id, raw_session_id).await?;
        self.require_idle_room(session.room_id)?;
        if session.status != AgentSessionStatus::Active {
            return Err(AppError::constraint("会话已归档，无需重复归档"));
        }

        self.sessions.archive(session.id).await?;
        let updated = self.sessions.get_by_id(session.id).await?;
        Ok(updated
            .as_ref()
            .unwrap_or(&session)
            .to_contract())
    }

    /// 删除会话：先移除本地存储目录，再删记录。
    ///
    /// 目录删除失败时保留记录并报错，避免出现「记录没了、目录还在」的孤儿；
    /// 目录本就不存在视为成功（已手动清理过）。
    pub async fn remove(&self, raw_room_id: &str, raw_session_id: &str) -> AppResult<()> {
        let session = self.require_session(raw_room_id, raw_session_id).await?;
        self.require_idle_room(session.room_id)?;

        let dir = self.absolute_dir(&session);
        remove_dir_with_retry(&dir)
            .await
            .map_err(|error| {
                AppError::with_source(
                    crate::error::ErrorCode::InternalError,
                    "无法删除会话存储目录，请稍后重试",
                    error,
                )
            })?;

        self.sessions.remove(session.id).await?;
        Ok(())
    }

    /// 运行前的会话解析。
    ///
    /// 只有房间成员显式开启复用才返回会话上下文；否则返回 None，
    /// 运行保持「每次独立会话」的默认行为。本方法在房间串行队列内调用，
    /// 唯一索引兜底并发创建。
    pub async fn resolve_run_session(
        &self,
        room_id: i64,
        agent: &Agent,
    ) -> AppResult<Option<ResolvedRunSession>> {
        let setting = self
            .rooms
            .list_member_session_settings(room_id)
            .await?
            .into_iter()
            .find(|item| item.agent_id == agent.id);
        let Some(setting) = setting else {
            return Ok(None);
        };
        if !setting.reuse_session {
            return Ok(None);
        }

        let stored = self
            .sessions
            .ensure_active(crate::store::sessions::CreateAgentSessionInput {
                room_id,
                agent_id: agent.id,
                harness: agent.harness,
            })
            .await?;
        // 智能体切换 harness 后旧会话 ID 无法被新适配器续接：归档旧记录，
        // 在新 harness 下重建会话，避免把 sess_* 传给 codex 之类的适配器。
        let current = if stored.harness == agent.harness {
            stored
        } else {
            self.recreate_for_harness(&stored, agent).await?
        };

        // 会话存储目录由服务层负责创建与清理；适配器只负责写入。
        let storage_dir = self.absolute_dir(&current);
        tokio::fs::create_dir_all(&storage_dir)
            .await
            .map_err(|error| {
                AppError::with_source(
                    crate::error::ErrorCode::InternalError,
                    "无法创建会话存储目录",
                    error,
                )
            })?;

        Ok(Some(ResolvedRunSession {
            id: current.id,
            context: RunSessionContext {
                harness_session_id: current.harness_session_id,
                storage_dir: storage_dir.to_string_lossy().into_owned(),
                last_context_message_id: current.last_context_message_id,
            },
        }))
    }

    /// 只读读取当前 active 会话；供上下文预览使用，不创建会话。
    pub async fn active_session(
        &self,
        room_id: i64,
        agent_id: i64,
    ) -> AppResult<Option<StoredAgentSession>> {
        self.sessions.get_active(room_id, agent_id).await
    }

    /// 运行成功后提交上下文水位。
    pub async fn advance_context_watermark(
        &self,
        session_id: i64,
        message_id: i64,
    ) -> AppResult<()> {
        self.sessions
            .advance_context_watermark(session_id, message_id)
            .await
    }

    /// 只读判断：该成员开启复用且已有可续接的 harness 会话。
    ///
    /// 讨论/裁决在构建提示词前用它决定能否省略已经进入会话上下文的
    /// 人设、主题全文与历史发言；不创建会话记录，也不改任何状态。
    pub async fn has_resumable_session(&self, room_id: i64, agent: &Agent) -> AppResult<bool> {
        let setting = self
            .rooms
            .list_member_session_settings(room_id)
            .await?
            .into_iter()
            .find(|item| item.agent_id == agent.id);
        let Some(setting) = setting else {
            return Ok(false);
        };
        if !setting.reuse_session {
            return Ok(false);
        }

        let Some(session) = self.sessions.get_active(room_id, agent.id).await? else {
            return Ok(false);
        };
        Ok(session.harness == agent.harness && session.harness_session_id.is_some())
    }

    /// 运行中拿到 harness 会话 ID 后落库；运行记录的关联由调用方完成。
    pub async fn record_harness_session(
        &self,
        session_id: i64,
        harness_session_id: &str,
        run_id: i64,
    ) -> AppResult<()> {
        self.sessions
            .touch(
                session_id,
                crate::store::sessions::TouchAgentSessionInput {
                    harness_session_id: Some(Some(harness_session_id.to_string())),
                    run_id: Some(Some(run_id)),
                },
            )
            .await
    }

    /// 成员被移出房间时归档其活动会话，避免重新加入后继续续接旧上下文。
    pub async fn archive_by_agent(&self, room_id: i64, agent_id: i64) -> AppResult<bool> {
        self.sessions.archive_by_agent(room_id, agent_id).await
    }

    /// 删除智能体在所有房间的会话存储目录；由智能体删除流程调用。
    ///
    /// 记录已随事务删除，这里失败只留下临时目录，不能再向用户报错。
    pub fn delete_agent_storage(&self, agent_id: i64) {
        let Ok(entries) = std::fs::read_dir(&self.storage_root) else {
            return;
        };

        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !entry.path().is_dir() || !name.starts_with("room-") {
                continue;
            }
            let target = entry.path().join(format!("agent-{agent_id}"));
            if let Err(error) = std::fs::remove_dir_all(&target) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(
                        agent_id,
                        path = %target.display(),
                        error = %error,
                        "agent session storage cleanup failed"
                    );
                }
            }
        }
    }

    /// 删除整个房间的会话存储目录；由房间删除流程调用。
    pub fn delete_room_storage(&self, room_id: i64) {
        let target = self.storage_root.join(format!("room-{room_id}"));
        if let Err(error) = std::fs::remove_dir_all(&target) {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(
                    room_id,
                    path = %target.display(),
                    error = %error,
                    "room session storage cleanup failed"
                );
            }
        }
    }

    /// 将相对存储路径解析为绝对路径。
    pub fn absolute_dir(&self, session: &StoredAgentSession) -> PathBuf {
        self.storage_root.join(&session.storage_dir)
    }

    /// 归档 harness 已变化的旧会话，并在当前 harness 下新建。
    async fn recreate_for_harness(
        &self,
        stale: &StoredAgentSession,
        agent: &Agent,
    ) -> AppResult<StoredAgentSession> {
        self.sessions.archive(stale.id).await?;
        self.sessions
            .ensure_active(crate::store::sessions::CreateAgentSessionInput {
                room_id: stale.room_id,
                agent_id: agent.id,
                harness: agent.harness,
            })
            .await
    }

    async fn require_room_id(&self, raw: &str) -> AppResult<i64> {
        let room_id = parse_id_label(raw, "房间")?;
        if self.rooms.get_by_id(room_id).await?.is_none() {
            return Err(AppError::not_found(format!(
                "房间不存在：{room_id}"
            )));
        }
        Ok(room_id)
    }

    async fn require_session(
        &self,
        raw_room_id: &str,
        raw_session_id: &str,
    ) -> AppResult<StoredAgentSession> {
        let room_id = self.require_room_id(raw_room_id).await?;
        let session_id = parse_id_label(raw_session_id, "会话")?;
        match self.sessions.get_by_id(session_id).await? {
            Some(session) if session.room_id == room_id => Ok(session),
            _ => Err(AppError::not_found(format!(
                "会话不存在：{session_id}"
            ))),
        }
    }

    fn require_idle_room(&self, room_id: i64) -> AppResult<()> {
        let busy = self
            .busy_checker
            .read()
            .ok()
            .and_then(|guard| guard.as_ref().map(|checker| checker(room_id)))
            .unwrap_or(false);
        if busy {
            return Err(AppError::constraint(
                "房间有排队或运行中的任务，请等待完成后再操作会话",
            ));
        }
        Ok(())
    }
}

/// 删除目录；Windows 上文件句柄释放有延迟，短暂重试。
async fn remove_dir_with_retry(path: &Path) -> std::io::Result<()> {
    for attempt in 0..5 {
        match tokio::fs::remove_dir_all(path).await {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) if attempt == 4 => return Err(error),
            Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
    Ok(())
}
