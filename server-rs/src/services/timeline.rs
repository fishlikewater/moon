//! 任务时间线聚合：只读合并审计事件、关联消息与运行记录。

use crate::domain::timeline::{TaskTimelineResponse, TimelineEntry};
use crate::error::{AppError, AppResult};
use crate::services::parse_id_label;
use crate::store::{MessageStore, RunStore, TaskStore};

pub struct TimelineService {
    tasks: TaskStore,
    messages: MessageStore,
    runs: RunStore,
}

impl TimelineService {
    pub fn new(tasks: TaskStore, messages: MessageStore, runs: RunStore) -> Self {
        Self {
            tasks,
            messages,
            runs,
        }
    }

    /// 任务不存在 404；非法 id 400；成功返回任务与其全部条目。
    pub async fn get_timeline(&self, raw_task_id: &str) -> AppResult<TaskTimelineResponse> {
        let task_id = parse_id_label(raw_task_id, "任务")?;
        if self.tasks.get_by_id(task_id).await?.is_none() {
            return Err(AppError::not_found(format!("任务不存在：{task_id}")));
        }

        let mut entries = Vec::new();
        for event in self.tasks.list_events(task_id).await? {
            entries.push(TimelineEntry::TaskEvent {
                id: event.id,
                task_id,
                created_at: event.created_at,
                event,
            });
        }
        for run in self.runs.list_by_task(task_id).await? {
            entries.push(TimelineEntry::Run {
                id: run.id,
                task_id,
                created_at: run.created_at,
                run,
            });
        }
        for message in self.messages.list_by_task(task_id).await? {
            entries.push(TimelineEntry::Message {
                id: message.id,
                task_id,
                created_at: message.created_at,
                message,
            });
        }

        entries.sort_by_key(|entry| (entry.created_at(), entry.kind_order(), entry.entry_id()));
        Ok(TaskTimelineResponse { task_id, entries })
    }
}
