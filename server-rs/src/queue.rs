//! 房间级串行运行队列。
//!
//! 不变量：同一房间同一时刻最多一个 harness 运行；不同房间互不阻塞。
//! 使用 `tokio::sync::Mutex` 的公平等待保证 FIFO；`pending` 计数覆盖
//! 排队与运行中，供会话归档/历史清理做 busy 拦截。
//!
//! 受管入口 `enqueue_managed` 额外登记可见任务快照，并允许在任务尚未
//! 开跑前取消；取消只阻止 job 执行，不改变串行与公平等待语义。

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tokio::sync::oneshot;

use crate::domain::now_ms;
use crate::error::{AppError, AppResult};
use crate::store::queue::{NewQueueJob, QueueStore};

/// 任务类型；用于前端文案与取消边界。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum QueueJobKind {
    Chat,
    Moderator,
    Discussion,
    Collaboration,
    Task,
}

impl QueueJobKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Moderator => "moderator",
            Self::Discussion => "discussion",
            Self::Collaboration => "collaboration",
            Self::Task => "task",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "chat" => Some(Self::Chat),
            "moderator" => Some(Self::Moderator),
            "discussion" => Some(Self::Discussion),
            "collaboration" => Some(Self::Collaboration),
            "task" => Some(Self::Task),
            _ => None,
        }
    }

    /// 只有用户消息触发的聊天/主持人任务可跨重启恢复；
    /// 讨论/委派/任务属于各自编排状态机，由启动恢复统一标记终态。
    pub const fn is_persistable(self) -> bool {
        matches!(self, Self::Chat | Self::Moderator)
    }
}

/// 任务状态：仅排队与运行中；结束后直接从快照移除。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum QueueJobState {
    Queued,
    Running,
}

/// 入队时由调用方提供的展示信息。
#[derive(Debug, Clone)]
pub struct QueueJobInfo {
    pub agent_id: Option<i64>,
    pub label: String,
    pub kind: QueueJobKind,
    pub cancelable: bool,
}

/// 队列快照中的单个任务；字段直接序列化为前端类型。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueJob {
    pub id: u64,
    pub room_id: i64,
    pub agent_id: Option<i64>,
    pub label: String,
    pub kind: QueueJobKind,
    pub state: QueueJobState,
    /// 运行中为 0；排队任务从 1 开始，按入队顺序递增。
    pub position: usize,
    pub enqueued_at: i64,
    pub started_at: Option<i64>,
}

/// 房间队列快照。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueSnapshot {
    pub room_id: i64,
    /// 暂停只阻止后续任务启动；运行中的任务不中断。
    pub paused: bool,
    pub jobs: Vec<QueueJob>,
}

/// 持久化入队参数；`row_id` 为 `Some` 表示启动恢复既有行。
#[derive(Debug, Clone)]
pub struct PersistJobSpec {
    pub row_id: Option<i64>,
    pub instruction: String,
    pub moderator_trigger_message_id: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueCancelError {
    NotFound,
    Running,
    NotCancelable,
}

struct JobEntry {
    id: u64,
    room_id: i64,
    agent_id: Option<i64>,
    label: String,
    kind: QueueJobKind,
    cancelable: bool,
    state: QueueJobState,
    enqueued_at: i64,
    started_at: Option<i64>,
    cancel: Arc<AtomicBool>,
    /// 对应 `room_queue_jobs.id`；未持久化任务为 None。
    persist_row: Option<i64>,
}

struct RoomSlot {
    lock: tokio::sync::Mutex<()>,
    pending: AtomicUsize,
    jobs: Mutex<HashMap<u64, JobEntry>>,
    next_id: AtomicU64,
    paused: AtomicBool,
    resume: tokio::sync::Notify,
}

#[derive(Default)]
pub struct RoomRunQueue {
    slots: Mutex<HashMap<i64, Arc<RoomSlot>>>,
    store: Option<QueueStore>,
}

impl RoomRunQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// 带持久化的运行时装配；单测可用 `new()` 保持纯内存。
    pub fn with_store(store: QueueStore) -> Self {
        Self {
            store: Some(store),
            ..Self::default()
        }
    }

    /// 持久化后端；`None` 表示当前队列不落库（单测或未配置）。
    pub fn store(&self) -> Option<&QueueStore> {
        self.store.as_ref()
    }

    fn slot(&self, room_id: i64) -> Arc<RoomSlot> {
        let mut slots = self.slots.lock().unwrap_or_else(|error| error.into_inner());
        slots
            .entry(room_id)
            .or_insert_with(|| {
                Arc::new(RoomSlot {
                    lock: tokio::sync::Mutex::new(()),
                    pending: AtomicUsize::new(0),
                    jobs: Mutex::new(HashMap::new()),
                    next_id: AtomicU64::new(1),
                    paused: AtomicBool::new(false),
                    resume: tokio::sync::Notify::new(),
                })
            })
            .clone()
    }

    /// 启动时把库中暂停状态写入内存；没有任务的暂停房间也预建 slot。
    pub fn seed_paused(&self, room_ids: &[i64]) {
        for room_id in room_ids {
            self.slot(*room_id).paused.store(true, Ordering::SeqCst);
        }
    }

    /// 更新暂停状态并落库；恢复时唤醒正在等待闸门的任务。
    pub async fn set_paused(&self, room_id: i64, paused: bool) -> AppResult<QueueSnapshot> {
        if let Some(store) = &self.store {
            store.set_paused(room_id, paused).await?;
        }
        let slot = self.slot(room_id);
        slot.paused.store(paused, Ordering::SeqCst);
        if !paused {
            // 只允许一个任务持有房间锁等待闸门，notify_one 足够且不会丢唤醒。
            slot.resume.notify_one();
        }
        Ok(self.snapshot(room_id))
    }

    /// 把任务排入指定房间的 FIFO 队列，返回该任务自身的结果。
    ///
    /// 等待与执行期间 `is_busy` 均为 true；任务 panic 时由 guard 归还计数。
    pub async fn enqueue<F, Fut, T>(&self, room_id: i64, job: F) -> T
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = T>,
    {
        let slot = self.slot(room_id);
        slot.pending.fetch_add(1, Ordering::SeqCst);
        let _pending = PendingGuard(slot.clone());
        let _guard = slot.lock.lock().await;
        job().await
    }

    /// 出队后先异步检查前置条件；不满足时跳过任务并返回 None。
    ///
    /// 用于讨论停止等场景：排队期间状态可能已变化，不能再启动运行。
    pub async fn enqueue_when<G, FutG, F, Fut, T>(
        &self,
        room_id: i64,
        should_run: G,
        job: F,
    ) -> Option<T>
    where
        G: Fn() -> FutG,
        FutG: Future<Output = bool>,
        F: FnOnce() -> Fut,
        Fut: Future<Output = T>,
    {
        let slot = self.slot(room_id);
        slot.pending.fetch_add(1, Ordering::SeqCst);
        let _pending = PendingGuard(slot.clone());
        let _guard = slot.lock.lock().await;
        if !should_run().await {
            return None;
        }
        Some(job().await)
    }

    /// 登记并执行一个可见任务；开跑前已被取消时跳过 job 并返回 None。
    ///
    /// 取消判定在获得房间锁之后、调用 job 之前，并在 `jobs` 锁内完成，
    /// 因此「取消」与「刚开始运行」不会同时成立。
    pub async fn enqueue_managed<F, Fut, T>(
        &self,
        room_id: i64,
        info: QueueJobInfo,
        job: F,
    ) -> Option<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = T>,
    {
        self.enqueue_managed_when(room_id, info, || async { true }, job)
            .await
    }

    /// 受管入口的带前置条件版本：出队后先异步检查 `should_run`，
    /// 不满足时跳过并返回 None（讨论停止等场景）。
    pub async fn enqueue_managed_when<G, FutG, F, Fut, T>(
        &self,
        room_id: i64,
        info: QueueJobInfo,
        should_run: G,
        job: F,
    ) -> Option<T>
    where
        G: Fn() -> FutG,
        FutG: Future<Output = bool>,
        F: FnOnce() -> Fut,
        Fut: Future<Output = T>,
    {
        self.enqueue_managed_inner(room_id, info, None, None, should_run, job)
            .await
    }

    /// 持久化受管入口：尚未开跑的任务写入 `room_queue_jobs`，
    /// 开跑或取消时删除行。`spec.row_id = Some` 表示启动恢复既有行。
    pub async fn enqueue_managed_persisted<F, Fut, T>(
        &self,
        room_id: i64,
        info: QueueJobInfo,
        spec: PersistJobSpec,
        job: F,
    ) -> AppResult<Option<T>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = T>,
    {
        let row_id = self.persist_row(room_id, &info, spec).await?;
        let result = self
            .enqueue_managed_inner(room_id, info, Some(row_id), None, || async { true }, job)
            .await;
        Ok(result)
    }

    /// 启动恢复入口：登记房间锁等待后通过 `waiting` 通知调用方，
    /// 用于恢复驱动按行顺序确定性派发下一条任务。
    pub(crate) async fn enqueue_managed_restored<F, Fut, T>(
        &self,
        room_id: i64,
        info: QueueJobInfo,
        spec: PersistJobSpec,
        waiting: oneshot::Sender<()>,
        job: F,
    ) -> AppResult<Option<T>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = T>,
    {
        let row_id = self.persist_row(room_id, &info, spec).await?;
        let result = self
            .enqueue_managed_inner(
                room_id,
                info,
                Some(row_id),
                Some(waiting),
                || async { true },
                job,
            )
            .await;
        Ok(result)
    }

    /// 取得或登记持久化行：恢复任务直接复用既有行，新入队任务先插入。
    async fn persist_row(
        &self,
        room_id: i64,
        info: &QueueJobInfo,
        spec: PersistJobSpec,
    ) -> AppResult<i64> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| AppError::internal("队列持久化未配置"))?;
        match spec.row_id {
            Some(row_id) => Ok(row_id),
            None => {
                let agent_id = info
                    .agent_id
                    .ok_or_else(|| AppError::internal("持久化队列任务缺少智能体"))?;
                Ok(store
                    .insert(NewQueueJob {
                        room_id,
                        agent_id,
                        label: info.label.clone(),
                        kind: info.kind.as_str().to_string(),
                        instruction: spec.instruction,
                        moderator_trigger_message_id: spec.moderator_trigger_message_id,
                        enqueued_at: now_ms(),
                    })
                    .await?)
            }
        }
    }

    /// 受管任务核心：登记快照 → 等待房间锁与暂停闸门 → 取消/前置检查 →
    /// 删除持久化行 → 执行。持久化行在真正开跑前删除，保证重启只恢复
    /// 从未开跑的任务。
    async fn enqueue_managed_inner<G, FutG, F, Fut, T>(
        &self,
        room_id: i64,
        info: QueueJobInfo,
        persist_row: Option<i64>,
        waiting: Option<oneshot::Sender<()>>,
        should_run: G,
        job: F,
    ) -> Option<T>
    where
        G: Fn() -> FutG,
        FutG: Future<Output = bool>,
        F: FnOnce() -> Fut,
        Fut: Future<Output = T>,
    {
        let slot = self.slot(room_id);
        let id = slot.next_id.fetch_add(1, Ordering::SeqCst);
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut jobs = slot.jobs.lock().unwrap_or_else(|error| error.into_inner());
            jobs.insert(
                id,
                JobEntry {
                    id,
                    room_id,
                    agent_id: info.agent_id,
                    label: info.label,
                    kind: info.kind,
                    cancelable: info.cancelable,
                    state: QueueJobState::Queued,
                    enqueued_at: now_ms(),
                    started_at: None,
                    cancel: cancel.clone(),
                    persist_row,
                },
            );
        }
        // guard 同时负责从快照移除，覆盖完成、失败与 panic 路径。
        let _job = JobGuard {
            slot: slot.clone(),
            id,
        };
        slot.pending.fetch_add(1, Ordering::SeqCst);
        let _pending = PendingGuard(slot.clone());

        // 公平 Mutex 在第一次 poll 时才把当前任务登记进等待队列；恢复任务
        // 在这一步之后通知驱动，才能确定下一行一定排在自己后面。
        let mut lock = std::pin::pin!(slot.lock.lock());
        let _guard = if let Some(waiting) = waiting {
            let mut waiting = Some(waiting);
            std::future::poll_fn(|context| {
                let poll = lock.as_mut().poll(context);
                if let Some(sender) = waiting.take() {
                    let _ = sender.send(());
                }
                poll
            })
            .await
        } else {
            lock.as_mut().await
        };
        // 暂停期间由第一个排到的任务持有房间锁等待，后续任务在公平
        // Mutex 上排队；取消也要能唤醒等待者，避免暂停时无法释放锁。
        while slot.paused.load(Ordering::SeqCst) && !cancel.load(Ordering::SeqCst) {
            slot.resume.notified().await;
        }
        let started = {
            let mut jobs = slot.jobs.lock().unwrap_or_else(|error| error.into_inner());
            match jobs.get_mut(&id) {
                Some(entry) if !entry.cancel.load(Ordering::SeqCst) => {
                    entry.state = QueueJobState::Running;
                    entry.started_at = Some(now_ms());
                    true
                }
                // 已被取消：entry 已被 cancel 移除。
                _ => false,
            }
        };
        if !started || !should_run().await {
            self.delete_persisted(persist_row).await;
            remove_job(&slot, id);
            return None;
        }
        self.delete_persisted(persist_row).await;
        let result = job().await;
        // 先移除条目再释放房间锁，避免下一任务已 running 时本任务仍留在快照。
        remove_job(&slot, id);
        Some(result)
    }

    /// 当前房间的队列快照：运行中优先，其余按入队顺序编号。
    pub fn snapshot(&self, room_id: i64) -> QueueSnapshot {
        let slot = {
            let slots = self.slots.lock().unwrap_or_else(|error| error.into_inner());
            slots.get(&room_id).cloned()
        };
        let Some(slot) = slot else {
            return QueueSnapshot {
                room_id,
                paused: false,
                jobs: Vec::new(),
            };
        };
        let mut entries: Vec<QueueJob> = {
            let jobs = slot.jobs.lock().unwrap_or_else(|error| error.into_inner());
            jobs.values()
                .map(|entry| QueueJob {
                    id: entry.id,
                    room_id: entry.room_id,
                    agent_id: entry.agent_id,
                    label: entry.label.clone(),
                    kind: entry.kind,
                    state: entry.state,
                    position: 0,
                    enqueued_at: entry.enqueued_at,
                    started_at: entry.started_at,
                })
                .collect()
        };
        // id 与入队顺序一致；同一毫秒入队也能稳定排序。
        entries.sort_by_key(|job| (job.state == QueueJobState::Queued, job.id));
        let mut position = 0;
        for job in &mut entries {
            if job.state == QueueJobState::Queued {
                position += 1;
                job.position = position;
            }
        }
        QueueSnapshot {
            room_id,
            paused: slot.paused.load(Ordering::SeqCst),
            jobs: entries,
        }
    }

    /// 取消仍在排队的任务；运行中或不可取消时返回错误，未知任务也是错误。
    pub async fn cancel(&self, room_id: i64, job_id: u64) -> Result<(), QueueCancelError> {
        let slot = {
            let slots = self.slots.lock().unwrap_or_else(|error| error.into_inner());
            slots.get(&room_id).cloned()
        };
        let Some(slot) = slot else {
            return Err(QueueCancelError::NotFound);
        };
        let persist_row = {
            let mut jobs = slot.jobs.lock().unwrap_or_else(|error| error.into_inner());
            let Some(entry) = jobs.get(&job_id) else {
                return Err(QueueCancelError::NotFound);
            };
            match entry.state {
                QueueJobState::Running => return Err(QueueCancelError::Running),
                QueueJobState::Queued if !entry.cancelable => {
                    return Err(QueueCancelError::NotCancelable)
                }
                QueueJobState::Queued => {
                    entry.cancel.store(true, Ordering::SeqCst);
                    jobs.remove(&job_id).and_then(|entry| entry.persist_row)
                }
            }
        };
        // 等待闸门的任务可能正持有房间锁：唤醒后它会看到取消标志并释放锁。
        slot.resume.notify_one();
        self.delete_persisted(persist_row).await;
        Ok(())
    }

    /// 删除待执行行；删除失败只记录日志，不阻断已开始的运行。
    async fn delete_persisted(&self, persist_row: Option<i64>) {
        let (Some(store), Some(row_id)) = (self.store.as_ref(), persist_row) else {
            return;
        };
        if let Err(error) = store.delete(row_id).await {
            tracing::warn!(
                event = "queue_persist_delete_failed",
                row_id,
                error = %error.message,
                "queue persistence row delete failed"
            );
        }
    }

    /// 房间当前是否有排队或运行中的任务。
    pub fn is_busy(&self, room_id: i64) -> bool {
        self.slots
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&room_id)
            .is_some_and(|slot| slot.pending.load(Ordering::SeqCst) > 0)
    }
}

struct PendingGuard(Arc<RoomSlot>);

impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.0.pending.fetch_sub(1, Ordering::SeqCst);
    }
}

struct JobGuard {
    slot: Arc<RoomSlot>,
    id: u64,
}

impl Drop for JobGuard {
    fn drop(&mut self) {
        remove_job(&self.slot, self.id);
    }
}

/// 从快照移除任务；正常路径在释放房间锁前调用，JobGuard 负责 panic 兜底。
fn remove_job(slot: &RoomSlot, id: u64) {
    slot.jobs
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(&id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    fn info(label: &str, cancelable: bool) -> QueueJobInfo {
        QueueJobInfo {
            agent_id: None,
            label: label.to_string(),
            kind: QueueJobKind::Chat,
            cancelable,
        }
    }

    fn job_snapshot(queue: &RoomRunQueue, room_id: i64) -> Vec<QueueJob> {
        queue.snapshot(room_id).jobs
    }

    #[tokio::test]
    async fn serializes_same_room_and_parallelizes_rooms() {
        let queue = Arc::new(RoomRunQueue::new());
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for room in [1_i64, 1, 2] {
            let queue = queue.clone();
            let active = active.clone();
            let max_active = max_active.clone();
            handles.push(tokio::spawn(async move {
                queue
                    .enqueue(room, || async move {
                        let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                        max_active.fetch_max(current, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(30)).await;
                        active.fetch_sub(1, Ordering::SeqCst);
                    })
                    .await;
            }));
        }
        for handle in handles {
            handle.await.unwrap();
        }

        assert_eq!(max_active.load(Ordering::SeqCst), 2, "不同房间可并行，同房间串行");
        assert!(!queue.is_busy(1));
        assert!(!queue.is_busy(2));
    }

    #[tokio::test]
    async fn reports_busy_while_queued() {
        let queue = Arc::new(RoomRunQueue::new());
        let (release, wait) = tokio::sync::oneshot::channel::<()>();

        let first = {
            let queue = queue.clone();
            tokio::spawn(async move {
                queue
                    .enqueue(7, || async {
                        let _ = wait.await;
                    })
                    .await;
            })
        };
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(queue.is_busy(7));

        let second = {
            let queue = queue.clone();
            tokio::spawn(async move {
                queue.enqueue(7, || async {}).await;
            })
        };
        release.send(()).unwrap();
        first.await.unwrap();
        second.await.unwrap();
        assert!(!queue.is_busy(7));
    }

    #[tokio::test]
    async fn managed_jobs_report_positions_and_cleanup() {
        let queue = Arc::new(RoomRunQueue::new());
        let (release, wait) = tokio::sync::oneshot::channel::<()>();
        let wait = Arc::new(tokio::sync::Mutex::new(Some(wait)));

        let first = {
            let queue = queue.clone();
            let wait = wait.clone();
            tokio::spawn(async move {
                queue
                    .enqueue_managed(3, info("甲", true), || async move {
                        let wait = wait.lock().await.take().unwrap();
                        let _ = wait.await;
                    })
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(10)).await;
        let second = {
            let queue = queue.clone();
            tokio::spawn(async move {
                queue
                    .enqueue_managed(3, info("乙", true), || async {})
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(10)).await;
        let third = {
            let queue = queue.clone();
            tokio::spawn(async move {
                queue
                    .enqueue_managed(3, info("丙", true), || async {})
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(10)).await;

        let jobs = job_snapshot(&queue, 3);
        assert_eq!(jobs.len(), 3, "{jobs:?}");
        assert_eq!(jobs[0].label, "甲");
        assert_eq!(jobs[0].state, QueueJobState::Running);
        assert_eq!(jobs[0].position, 0);
        assert_eq!((jobs[1].label.as_str(), jobs[1].position), ("乙", 1));
        assert_eq!((jobs[2].label.as_str(), jobs[2].position), ("丙", 2));
        assert_eq!(jobs[1].state, QueueJobState::Queued);

        release.send(()).unwrap();
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
        third.await.unwrap().unwrap();
        assert!(job_snapshot(&queue, 3).is_empty());
        assert!(!queue.is_busy(3));
    }

    #[tokio::test]
    async fn cancel_queued_job_skips_it_and_runs_next() {
        let queue = Arc::new(RoomRunQueue::new());
        let (release, wait) = tokio::sync::oneshot::channel::<()>();
        let wait = Arc::new(tokio::sync::Mutex::new(Some(wait)));
        let cancelled_ran = Arc::new(AtomicBool::new(false));
        let next_ran = Arc::new(AtomicBool::new(false));

        let first = {
            let queue = queue.clone();
            let wait = wait.clone();
            tokio::spawn(async move {
                queue
                    .enqueue_managed(4, info("甲", true), || async move {
                        let wait = wait.lock().await.take().unwrap();
                        let _ = wait.await;
                    })
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(10)).await;
        let second = {
            let queue = queue.clone();
            let cancelled_ran = cancelled_ran.clone();
            tokio::spawn(async move {
                queue
                    .enqueue_managed(4, info("乙", true), || async move {
                        cancelled_ran.store(true, Ordering::SeqCst);
                    })
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(10)).await;
        let queued_id = job_snapshot(&queue, 4)
            .into_iter()
            .find(|job| job.label == "乙")
            .expect("乙应已入队")
            .id;
        assert_eq!(queue.cancel(4, queued_id).await, Ok(()));
        assert!(job_snapshot(&queue, 4).iter().all(|job| job.label != "乙"));

        let third = {
            let queue = queue.clone();
            let next_ran = next_ran.clone();
            tokio::spawn(async move {
                queue
                    .enqueue_managed(4, info("丙", true), || async move {
                        next_ran.store(true, Ordering::SeqCst);
                    })
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(10)).await;

        release.send(()).unwrap();
        first.await.unwrap().unwrap();
        assert_eq!(second.await.unwrap(), None);
        third.await.unwrap().unwrap();
        assert!(!cancelled_ran.load(Ordering::SeqCst), "已取消任务的 job 不能执行");
        assert!(next_ran.load(Ordering::SeqCst), "取消不能让后续任务卡住");
        assert!(!queue.is_busy(4));
    }

    #[tokio::test]
    async fn cancel_running_or_non_cancelable_is_rejected() {
        let queue = Arc::new(RoomRunQueue::new());
        let (release, wait) = tokio::sync::oneshot::channel::<()>();
        let wait = Arc::new(tokio::sync::Mutex::new(Some(wait)));

        let first = {
            let queue = queue.clone();
            let wait = wait.clone();
            tokio::spawn(async move {
                queue
                    .enqueue_managed(5, info("运行中", true), || async move {
                        let wait = wait.lock().await.take().unwrap();
                        let _ = wait.await;
                    })
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(10)).await;
        let running_id = job_snapshot(&queue, 5)[0].id;
        assert_eq!(
            queue.cancel(5, running_id).await,
            Err(QueueCancelError::Running),
            "运行中不能从队列取消"
        );

        let second = {
            let queue = queue.clone();
            tokio::spawn(async move {
                queue
                    .enqueue_managed(5, info("讨论", false), || async {})
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(10)).await;
        let queued_id = job_snapshot(&queue, 5)
            .into_iter()
            .find(|job| job.label == "讨论")
            .unwrap()
            .id;
        assert_eq!(
            queue.cancel(5, queued_id).await,
            Err(QueueCancelError::NotCancelable)
        );
        assert_eq!(
            queue.cancel(5, 9999).await,
            Err(QueueCancelError::NotFound)
        );
        assert_eq!(queue.cancel(99, 1).await, Err(QueueCancelError::NotFound));

        release.send(()).unwrap();
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
        assert!(job_snapshot(&queue, 5).is_empty());
    }

    #[test]
    fn job_kind_round_trips_and_limits_persistence() {
        for kind in [
            QueueJobKind::Chat,
            QueueJobKind::Moderator,
            QueueJobKind::Discussion,
            QueueJobKind::Collaboration,
            QueueJobKind::Task,
        ] {
            assert_eq!(QueueJobKind::parse(kind.as_str()), Some(kind));
            assert_eq!(
                kind.is_persistable(),
                matches!(kind, QueueJobKind::Chat | QueueJobKind::Moderator)
            );
        }
        assert_eq!(QueueJobKind::parse("unknown"), None);
    }

    #[tokio::test]
    async fn pause_holds_jobs_until_resume() {
        let queue = Arc::new(RoomRunQueue::new());
        queue.set_paused(6, true).await.unwrap();

        let ran = Arc::new(AtomicBool::new(false));
        let job = {
            let queue = queue.clone();
            let ran = ran.clone();
            tokio::spawn(async move {
                queue
                    .enqueue_managed(6, info("甲", true), || async move {
                        ran.store(true, Ordering::SeqCst);
                    })
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;

        assert!(!ran.load(Ordering::SeqCst), "暂停期间不能启动任务");
        let snapshot = queue.snapshot(6);
        assert!(snapshot.paused);
        assert_eq!(snapshot.jobs[0].state, QueueJobState::Queued);
        assert_eq!(snapshot.jobs[0].position, 1);
        assert!(queue.is_busy(6));

        queue.set_paused(6, false).await.unwrap();
        job.await.unwrap().unwrap();
        assert!(ran.load(Ordering::SeqCst));
        assert!(!queue.snapshot(6).paused);
        assert!(job_snapshot(&queue, 6).is_empty());
    }

    #[tokio::test]
    async fn cancel_while_paused_releases_room_lock_for_next_job() {
        let queue = Arc::new(RoomRunQueue::new());
        queue.set_paused(7, true).await.unwrap();

        let first = {
            let queue = queue.clone();
            tokio::spawn(async move {
                queue
                    .enqueue_managed(7, info("甲", true), || async {})
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        let second_ran = Arc::new(AtomicBool::new(false));
        let second = {
            let queue = queue.clone();
            let second_ran = second_ran.clone();
            tokio::spawn(async move {
                queue
                    .enqueue_managed(7, info("乙", true), || async move {
                        second_ran.store(true, Ordering::SeqCst);
                    })
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;

        let first_id = job_snapshot(&queue, 7)
            .into_iter()
            .find(|job| job.label == "甲")
            .expect("甲应持有房间锁等待暂停")
            .id;
        queue.cancel(7, first_id).await.unwrap();
        // 被取消的等待者应释放房间锁，让乙排到闸门前而不是卡死。
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!second_ran.load(Ordering::SeqCst));

        queue.set_paused(7, false).await.unwrap();
        assert_eq!(first.await.unwrap(), None);
        second.await.unwrap().unwrap();
        assert!(second_ran.load(Ordering::SeqCst));
        assert!(job_snapshot(&queue, 7).is_empty());
    }
}
