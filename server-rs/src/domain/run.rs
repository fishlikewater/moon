//! 运行、运行活动与运行详情领域类型。

use serde::{Deserialize, Serialize};

use crate::domain::message::Message;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Running,
    Succeeded,
    Failed,
    Interrupted,
}

impl RunStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "running" => Some(Self::Running),
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            "interrupted" => Some(Self::Interrupted),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRun {
    pub id: i64,
    pub room_id: i64,
    pub agent_id: i64,
    pub task_id: Option<i64>,
    pub status: RunStatus,
    pub prompt: String,
    pub result: Option<String>,
    pub error: Option<String>,
    /// harness 上报的累计 token 数；未知时为 None。
    pub token_count: Option<i64>,
    /// harness 的上下文窗口/已用大小；仅 ACP 上报。
    pub context_size: Option<i64>,
    pub harness_session_id: Option<String>,
    pub created_at: i64,
    pub finished_at: Option<i64>,
}

/// 发送前的上下文估算；`estimated_tokens` 固定为字符数除以 4 向上取整。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextPreview {
    pub enabled: bool,
    pub budget_chars: i64,
    pub raw_chars: usize,
    pub effective_chars: usize,
    pub estimated_tokens: usize,
    pub included_messages: usize,
    pub compressed_messages: usize,
    pub omitted_messages: usize,
    pub session_reused: bool,
}

/// 运行活动类型：推理、工具调用、用量与阶段状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunActivityKind {
    Reasoning,
    Tool,
    Usage,
    Status,
}

impl RunActivityKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reasoning => "reasoning",
            Self::Tool => "tool",
            Self::Usage => "usage",
            Self::Status => "status",
        }
    }
}

/// 一次 harness 运行期间的一条可展示活动；不落库。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunActivity {
    pub run_id: i64,
    pub seq: i64,
    pub kind: RunActivityKind,
    pub title: String,
    pub detail: Option<String>,
    pub at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDetailResponse {
    pub run: AgentRun,
    pub activities: Vec<RunActivity>,
    pub messages: Vec<Message>,
}
