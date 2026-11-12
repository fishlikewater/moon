//! 任务时间线条目与排序契约。

use serde::Serialize;

use crate::domain::message::Message;
use crate::domain::run::AgentRun;
use crate::domain::task::TaskEvent;

/// 时间线条目类型：任务审计事件 / 消息 / harness 运行。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum TimelineEntry {
    TaskEvent {
        /// 来源表 `task_events.id`。
        id: i64,
        task_id: i64,
        created_at: i64,
        event: TaskEvent,
    },
    Message {
        id: i64,
        task_id: i64,
        created_at: i64,
        message: Message,
    },
    Run {
        id: i64,
        task_id: i64,
        created_at: i64,
        run: AgentRun,
    },
}

impl TimelineEntry {
    /// 同毫秒定序：task_event → run → message。
    pub fn kind_order(&self) -> u8 {
        match self {
            Self::TaskEvent { .. } => 0,
            Self::Run { .. } => 1,
            Self::Message { .. } => 2,
        }
    }

    pub fn created_at(&self) -> i64 {
        match self {
            Self::TaskEvent { created_at, .. }
            | Self::Message { created_at, .. }
            | Self::Run { created_at, .. } => *created_at,
        }
    }

    pub fn entry_id(&self) -> i64 {
        match self {
            Self::TaskEvent { id, .. }
            | Self::Message { id, .. }
            | Self::Run { id, .. } => *id,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskTimelineResponse {
    pub task_id: i64,
    pub entries: Vec<TimelineEntry>,
}
