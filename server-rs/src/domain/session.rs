//! 房间智能体会话领域类型。

use serde::{Deserialize, Serialize};

use crate::domain::agent::HarnessKind;

/// 房间内智能体会话状态；归档后不再被运行续接。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentSessionStatus {
    Active,
    Archived,
}

/// 对外会话契约；`storage_dir` 属于服务端内部路径，不进入响应。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSession {
    pub id: i64,
    pub room_id: i64,
    pub agent_id: i64,
    pub harness: HarnessKind,
    pub harness_session_id: Option<String>,
    pub status: AgentSessionStatus,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub last_run_id: Option<i64>,
}

/// store 层会话对象，额外携带相对存储目录。
#[derive(Debug, Clone)]
pub struct StoredAgentSession {
    pub id: i64,
    pub room_id: i64,
    pub agent_id: i64,
    pub harness: HarnessKind,
    pub harness_session_id: Option<String>,
    pub status: AgentSessionStatus,
    pub storage_dir: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub last_run_id: Option<i64>,
    /// 该会话已投递到的全局消息 id；新会话为 0。
    pub last_context_message_id: i64,
}

impl StoredAgentSession {
    pub fn to_contract(&self) -> AgentSession {
        AgentSession {
            id: self.id,
            room_id: self.room_id,
            agent_id: self.agent_id,
            harness: self.harness,
            harness_session_id: self.harness_session_id.clone(),
            status: self.status,
            created_at: self.created_at,
            last_used_at: self.last_used_at,
            last_run_id: self.last_run_id,
        }
    }
}

/// 全局会话总览：会话字段 + 房间与智能体名称，供 /api/sessions 使用。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionOverview {
    pub id: i64,
    pub room_id: i64,
    pub room_name: String,
    pub agent_id: i64,
    pub agent_name: String,
    pub harness: HarnessKind,
    pub harness_session_id: Option<String>,
    pub status: AgentSessionStatus,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub last_run_id: Option<i64>,
}

/// 房间成员的会话复用设置。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomMemberSessionSettings {
    pub agent_id: i64,
    pub reuse_session: bool,
}

/// `GET /api/rooms/:id/sessions` 响应。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomSessionsResponse {
    /// 房间全部成员及其复用开关。
    pub members: Vec<RoomMemberSessionSettings>,
    /// 房间内全部会话（含归档），按创建顺序倒序。
    pub sessions: Vec<AgentSession>,
}

/// 一次运行解析出的会话上下文；未开启复用的成员不返回。
#[derive(Debug, Clone)]
pub struct RunSessionContext {
    /// 已知 harness 会话 ID；None 表示本次运行需要建立新会话。
    pub harness_session_id: Option<String>,
    /// 会话隔离存储目录（绝对路径，由服务层解析）。
    pub storage_dir: String,
    /// 该会话已投递到的全局消息 id；首次运行为 0。
    pub last_context_message_id: i64,
}

#[derive(Debug, Clone)]
pub struct ResolvedRunSession {
    pub id: i64,
    pub context: RunSessionContext,
}
