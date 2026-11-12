//! 讨论领域类型、协议边界与轮次快照。

use serde::{Deserialize, Serialize};

use crate::domain::room::MAX_ROOM_DISCUSSION_ROUNDS;

/// 讨论参与者数量与轮次范围；实际轮数上限由房间设置决定。
pub const MIN_DISCUSSION_PARTICIPANTS: usize = 2;
pub const MAX_DISCUSSION_PARTICIPANTS: usize = 5;
pub const DEFAULT_DISCUSSION_ROUNDS: i64 = 3;
/// 协议层的绝对上限，房间 `discussionMaxRounds` 不得超过该值。
pub const MAX_DISCUSSION_ROUNDS: i64 = MAX_ROOM_DISCUSSION_ROUNDS;

/// 参与者提示词最多注入的讨论消息数；每条内容单独截断。
pub const DISCUSSION_TRANSCRIPT_LIMIT: usize = 12;
/// 主持人裁决最多注入的讨论消息数。
pub const DISCUSSION_JUDGE_TRANSCRIPT_LIMIT: usize = 30;
pub const DISCUSSION_MESSAGE_CHARS: usize = 1_500;
pub const DISCUSSION_TOPIC_CHARS: usize = 1_000;
pub const DISCUSSION_REASON_CHARS: usize = 1_000;
pub const DISCUSSION_SUMMARY_CHARS: usize = 4_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiscussionStatus {
    Running,
    Completed,
    Stopped,
    Failed,
}

impl DiscussionStatus {
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
pub enum DiscussionStopReason {
    AllVotedStop,
    ModeratorDecision,
    MaxRounds,
    UserStop,
    ParticipantFailed,
    ModeratorFailed,
    Interrupted,
    InternalError,
}

impl DiscussionStopReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AllVotedStop => "all_voted_stop",
            Self::ModeratorDecision => "moderator_decision",
            Self::MaxRounds => "max_rounds",
            Self::UserStop => "user_stop",
            Self::ParticipantFailed => "participant_failed",
            Self::ModeratorFailed => "moderator_failed",
            Self::Interrupted => "interrupted",
            Self::InternalError => "internal_error",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "all_voted_stop" => Some(Self::AllVotedStop),
            "moderator_decision" => Some(Self::ModeratorDecision),
            "max_rounds" => Some(Self::MaxRounds),
            "user_stop" => Some(Self::UserStop),
            "participant_failed" => Some(Self::ParticipantFailed),
            "moderator_failed" => Some(Self::ModeratorFailed),
            "interrupted" => Some(Self::Interrupted),
            "internal_error" => Some(Self::InternalError),
            _ => None,
        }
    }

    /// 讨论结束系统消息使用的可读原因。
    pub const fn label(self) -> &'static str {
        match self {
            Self::AllVotedStop => "全员同意结束",
            Self::ModeratorDecision => "主持人裁决结束",
            Self::MaxRounds => "达到最大轮次",
            Self::UserStop => "用户手动停止",
            Self::ParticipantFailed => "参与者运行失败",
            Self::ModeratorFailed => "主持人裁决失败",
            Self::Interrupted => "服务重启中断",
            Self::InternalError => "内部错误",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Discussion {
    pub id: i64,
    pub room_id: i64,
    pub topic: String,
    pub status: DiscussionStatus,
    pub current_round: i64,
    pub max_rounds: i64,
    pub participant_ids: Vec<i64>,
    pub created_by_agent_id: Option<i64>,
    pub stop_reason: Option<DiscussionStopReason>,
    pub summary: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 主持人发起讨论的协议草稿；`max_rounds = None` 表示省略，由房间上限解析。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModeratorDiscussionDraft {
    pub topic: String,
    pub participant_ids: Vec<i64>,
    pub max_rounds: Option<i64>,
}
