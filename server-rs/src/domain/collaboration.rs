//! 委派批次与指派的领域类型。

use serde::{Deserialize, Serialize};

/// 主持人委派单个批次的最大轮次；固定值，不可配置关闭。
pub const DEFAULT_COLLABORATION_MAX_ROUNDS: i64 = 3;
/// 单轮最多指派人数。
pub const MAX_COLLABORATION_ASSIGNMENTS: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CollaborationStatus {
    Running,
    Completed,
    Stopped,
    Failed,
}

impl CollaborationStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "running" => Some(Self::Running),
            "completed" => Some(Self::Completed),
            "stopped" => Some(Self::Stopped),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollaborationStopReason {
    ModeratorStop,
    MaxRounds,
    UserStop,
    Interrupted,
    InvalidNextAssignments,
    ModeratorFailed,
    InternalError,
}

impl CollaborationStopReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ModeratorStop => "moderator_stop",
            Self::MaxRounds => "max_rounds",
            Self::UserStop => "user_stop",
            Self::Interrupted => "interrupted",
            Self::InvalidNextAssignments => "invalid_next_assignments",
            Self::ModeratorFailed => "moderator_failed",
            Self::InternalError => "internal_error",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "moderator_stop" => Some(Self::ModeratorStop),
            "max_rounds" => Some(Self::MaxRounds),
            "user_stop" => Some(Self::UserStop),
            "interrupted" => Some(Self::Interrupted),
            "invalid_next_assignments" => Some(Self::InvalidNextAssignments),
            "moderator_failed" => Some(Self::ModeratorFailed),
            "internal_error" => Some(Self::InternalError),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CollaborationAssignmentStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Skipped,
}

impl CollaborationAssignmentStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(Self::Queued),
            "running" => Some(Self::Running),
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            "skipped" => Some(Self::Skipped),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollaborationAssignment {
    pub id: i64,
    pub batch_id: i64,
    pub round: i64,
    pub agent_id: i64,
    pub instruction: String,
    pub status: CollaborationAssignmentStatus,
    pub run_id: Option<i64>,
    pub message_id: Option<i64>,
    pub error: Option<String>,
    pub created_at: i64,
    pub finished_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollaborationBatch {
    pub id: i64,
    pub room_id: i64,
    pub moderator_agent_id: i64,
    /// 触发批次的主持人回复之前那条用户消息。
    pub user_message_id: i64,
    pub status: CollaborationStatus,
    pub current_round: i64,
    pub max_rounds: i64,
    pub stop_reason: Option<CollaborationStopReason>,
    /// 用户请求停止；当前轮结束后不再派发下一轮。
    pub stop_requested: bool,
    /// 主持人最终汇总；未结束时为 null。
    pub summary: Option<String>,
    /// 正在生成或已生成当前轮汇总的主持人运行 id。
    pub review_run_id: Option<i64>,
    /// 按轮次与创建顺序排列。
    pub assignments: Vec<CollaborationAssignment>,
    pub created_at: i64,
    pub updated_at: i64,
}
