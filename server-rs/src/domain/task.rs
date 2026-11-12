//! 任务、任务事件领域类型。

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Backlog,
    InProgress,
    Review,
    Done,
    Blocked,
    Cancelled,
}

impl TaskStatus {
    pub const ALL: [Self; 6] = [
        Self::Backlog,
        Self::InProgress,
        Self::Review,
        Self::Done,
        Self::Blocked,
        Self::Cancelled,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Backlog => "backlog",
            Self::InProgress => "in_progress",
            Self::Review => "review",
            Self::Done => "done",
            Self::Blocked => "blocked",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "backlog" => Some(Self::Backlog),
            "in_progress" => Some(Self::InProgress),
            "review" => Some(Self::Review),
            "done" => Some(Self::Done),
            "blocked" => Some(Self::Blocked),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

/// 任务优先级；历史任务缺省为 `normal`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskPriority {
    Low,
    #[default]
    Normal,
    High,
    Urgent,
}

impl TaskPriority {
    pub const ALL: [Self; 4] = [Self::Low, Self::Normal, Self::High, Self::Urgent];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Normal => "normal",
            Self::High => "high",
            Self::Urgent => "urgent",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "low" => Some(Self::Low),
            "normal" => Some(Self::Normal),
            "high" => Some(Self::High),
            "urgent" => Some(Self::Urgent),
            _ => None,
        }
    }
}

/// 任务锚点：主持人创建任务时写入的目标、验收标准、验证方式与范围。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskPlan {
    pub goal: String,
    pub acceptance_criteria: Vec<String>,
    pub verification: Vec<String>,
    pub scope: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: i64,
    pub room_id: i64,
    pub title: String,
    pub description: String,
    pub status: TaskStatus,
    pub priority: TaskPriority,
    /// 指派的房间成员；未指派为 None。
    pub assignee_agent_id: Option<i64>,
    /// 截止时间（epoch 毫秒）；未设置为 None。
    pub due_at: Option<i64>,
    /// 定时自动启动时间（epoch 毫秒）；仅 backlog 任务可设置。
    pub scheduled_at: Option<i64>,
    pub plan: Option<TaskPlan>,
    pub created_by_agent_id: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskEvent {
    pub id: i64,
    pub task_id: i64,
    #[serde(rename = "type")]
    pub event_type: String,
    pub payload: Option<Map<String, Value>>,
    pub created_at: i64,
}
