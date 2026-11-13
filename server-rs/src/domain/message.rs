//! 消息领域类型与结构化元数据。

use serde::{Deserialize, Serialize};

use crate::domain::attachment::MessageAttachment;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageSenderType {
    User,
    Agent,
    System,
}

impl MessageSenderType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Agent => "agent",
            Self::System => "system",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageStatus {
    Final,
    Streaming,
    Failed,
}

impl MessageStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Final => "final",
            Self::Streaming => "streaming",
            Self::Failed => "failed",
        }
    }
}

/// 消息用途：`reply` 参与上下文，`progress` 只用于展示。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageKind {
    Reply,
    Progress,
}

impl MessageKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reply => "reply",
            Self::Progress => "progress",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiscussionStance {
    Propose,
    Support,
    Oppose,
    Question,
}

impl DiscussionStance {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Propose => "propose",
            Self::Support => "support",
            Self::Oppose => "oppose",
            Self::Question => "question",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "propose" => Some(Self::Propose),
            "support" => Some(Self::Support),
            "oppose" => Some(Self::Oppose),
            "question" => Some(Self::Question),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiscussionStopVote {
    Continue,
    Stop,
}

impl DiscussionStopVote {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Continue => "continue",
            Self::Stop => "stop",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "continue" => Some(Self::Continue),
            "stop" => Some(Self::Stop),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CollaborationMessageRole {
    Assignment,
    Summary,
    System,
}

impl CollaborationMessageRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Assignment => "assignment",
            Self::Summary => "summary",
            Self::System => "system",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "assignment" => Some(Self::Assignment),
            "summary" => Some(Self::Summary),
            "system" => Some(Self::System),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageDiscussion {
    pub discussion_id: i64,
    pub round: i64,
    pub stance: Option<DiscussionStance>,
    pub reply_to_message_id: Option<i64>,
    pub stop_vote: Option<DiscussionStopVote>,
    pub stop_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageCollaboration {
    pub batch_id: i64,
    pub round: i64,
    pub role: CollaborationMessageRole,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageReaction {
    pub emoji: String,
    /// 同一 emoji 的反应数；当前单用户产品通常为 1。
    pub count: i64,
    /// 当前本地用户是否已添加该反应。
    pub reacted_by_me: bool,
}

/// 单用户产品固定使用本地用户身份记录反应。
pub const LOCAL_REACTION_ACTOR: &str = "user";

/// 前后端共用的固定反应调色板；顺序决定 UI 展示顺序。
pub const REACTION_EMOJIS: [&str; 6] = ["👍", "❤️", "😄", "🎉", "👀", "🙏"];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: i64,
    pub room_id: i64,
    /// 房间内严格递增的序号，跨房间独立计数。
    pub seq: i64,
    pub sender_type: MessageSenderType,
    pub sender_agent_id: Option<i64>,
    pub status: MessageStatus,
    pub kind: MessageKind,
    pub run_id: Option<i64>,
    pub content: String,
    pub task_id: Option<i64>,
    /// 普通消息的回复目标；讨论消息仍使用 `discussion.replyToMessageId`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to_message_id: Option<i64>,
    /// 非讨论消息不携带该字段。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discussion: Option<MessageDiscussion>,
    /// 非委派消息不携带该字段。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collaboration: Option<MessageCollaboration>,
    /// 用户消息的附件；旧数据与无附件消息为空数组。
    pub attachments: Vec<MessageAttachment>,
    /// 表情反应；旧数据与无反应消息为空数组。
    pub reactions: Vec<MessageReaction>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageCleanupResult {
    /// 实际删除的消息条数。
    pub deleted: u64,
    /// 因讨论/任务/委派记录保留而从聊天窗口隐藏的消息条数。
    pub hidden: u64,
    /// 实际删除的归档文件数。
    pub artifacts_deleted: u64,
    /// 实际删除的附件行数与磁盘文件数（按成功删除行的附件计）。
    pub attachments_deleted: u64,
}
