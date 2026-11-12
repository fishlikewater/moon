//! 任务定时调度器：扫描到期任务并自动启动。
//!
//! 启动必须走 `TaskService::activate`，保证状态机校验、`status_changed` 事件与
//! WS 广播与手工启动一致；调度器只负责发现「到期」与记录调度结果。

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Map};

use crate::domain::now_ms;
use crate::domain::task::TaskStatus;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::services::tasks::TaskService;
use crate::store::TaskStore;

/// 单轮扫描结果；`failed` 只统计因前置条件不满足而放弃的任务。
#[derive(Debug, Default, Clone)]
pub struct TickReport {
    pub started: Vec<i64>,
    pub failed: Vec<i64>,
}

pub struct TaskScheduler {
    tasks: TaskStore,
    service: Arc<TaskService>,
}

impl TaskScheduler {
    pub fn new(tasks: TaskStore, service: Arc<TaskService>) -> Self {
        Self { tasks, service }
    }

    /// 扫描并启动到期任务。幂等：同一到期快照被并发处理时只有真正完成
    /// backlog→in_progress 的那次写 `scheduled_started`，其余静默跳过。
    pub async fn tick(&self, now_ms: i64) -> AppResult<TickReport> {
        let due = self.tasks.list_due_scheduled(now_ms).await?;
        let mut report = TickReport::default();
        for task in due {
            let scheduled_at = task.scheduled_at;
            match self.service.activate(&task.id.to_string()).await {
                Ok((started, started_now)) => {
                    // 已启用的定时没有继续保留的意义；原定时时间记入事件。
                    self.tasks.clear_schedule(started.id).await?;
                    if !started_now {
                        // 其它路径（手动启动/并发 tick）已先推进状态，不重复记事件。
                        continue;
                    }
                    let mut payload = Map::new();
                    payload.insert("scheduledAt".to_string(), json!(scheduled_at));
                    payload.insert("startedAt".to_string(), json!(now_ms));
                    self.tasks
                        .append_event(started.id, "scheduled_started", Some(&payload))
                        .await?;
                    report.started.push(started.id);
                }
                Err(error) if is_permanent_schedule_failure(&error) => {
                    // 取消/流转/并发清理都可能让快照过期；只有任务仍是 backlog
                    // 且定时值未变时才记录失败并清除，避免覆盖用户新设置。
                    let Some(current) = self.tasks.get_by_id(task.id).await? else {
                        continue;
                    };
                    if current.status != TaskStatus::Backlog
                        || current.scheduled_at != scheduled_at
                    {
                        continue;
                    }
                    // 任务/房间/主持人前置不满足：清除定时并记录原因，避免每轮重复失败。
                    self.tasks.clear_schedule(task.id).await?;
                    let mut payload = Map::new();
                    payload.insert("reason".to_string(), json!(error.message));
                    payload.insert("scheduledAt".to_string(), json!(scheduled_at));
                    self.tasks
                        .append_event(task.id, "schedule_failed", Some(&payload))
                        .await?;
                    report.failed.push(task.id);
                }
                Err(error) => {
                    // 内部错误保留定时，下一轮重试。
                    tracing::warn!(task_id = task.id, %error, "定时启动失败，保留定时等待重试");
                }
            }
        }
        Ok(report)
    }
}

fn is_permanent_schedule_failure(error: &AppError) -> bool {
    matches!(
        error.code,
        ErrorCode::NotFound | ErrorCode::ConstraintViolation | ErrorCode::InvalidTransition
    )
}

/// 生产启动循环：首轮延迟一个周期，避免与启动恢复抢跑。
pub fn spawn_loop(scheduler: Arc<TaskScheduler>, period: Duration) {
    tokio::spawn(async move {
        tokio::time::sleep(period).await;
        let mut interval = tokio::time::interval(period);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            match scheduler.tick(now_ms()).await {
                Ok(report) if !report.started.is_empty() || !report.failed.is_empty() => {
                    tracing::info!(
                        started = report.started.len(),
                        failed = report.failed.len(),
                        "task scheduler tick"
                    );
                }
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, "task scheduler tick failed"),
            }
        }
    });
}
