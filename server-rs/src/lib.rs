//! Moon Rust 后端库入口。
//!
//! 路由、服务与存储按阶段逐步迁入；当前提供配置、数据库迁移、统一错误
//! 契约、健康检查、智能体/房间/会员/会话 API，以及消息运行与 WS 事件流。

pub mod adapters;
pub mod config;
pub mod db;
pub mod domain;
pub mod error;
pub mod queue;
pub mod routes;
pub mod services;
pub mod store;
pub mod ws;

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::Request;
use axum::http::HeaderValue;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};
use tower_http::cors::{Any, CorsLayer};

use crate::adapters::AdapterFactory;
use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::queue::RoomRunQueue;
use crate::services::acp_registry::AcpRegistryService;
use crate::services::agent_builder::{AgentBuilderService, AgentBuilderServiceOptions};
use crate::services::agents::AgentService;
use crate::services::attachments::AttachmentService;
use crate::services::builtin_moderator::BuiltinModeratorService;
use crate::services::chat::{BroadcastFn, ChatService, ChatServiceOptions};
use crate::services::collaborations::CollaborationService;
use crate::services::custom_harnesses::CustomHarnessService;
use crate::services::discussion::DiscussionService;
use crate::services::orchestrator::{OrchestratorEngine, PauseControl};
use crate::services::reactions::ReactionService;
use crate::services::recovery;
use crate::services::room_history::{RoomBusyFn, RoomHistoryOptions, RoomHistoryService};
use crate::services::rooms::RoomService;
use crate::services::sessions::SessionService;
use crate::services::settings::SettingsService;
use crate::services::task_scheduler::{spawn_loop, TaskScheduler};
use crate::services::tasks::{TaskService, TaskServiceOptions};
use crate::services::timeline::TimelineService;
use crate::store::{
    AgentStore, AttachmentStore, CollaborationStore, CustomHarnessStore, DiscussionStore,
    MessageStore, QueueStore, RoomStore, RunStore, SessionStore, SettingsStore, TaskStore,
};
use crate::ws::RoomSocketHub;

/// 进程内共享状态。
#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub agent_service: Arc<AgentService>,
    pub room_service: Arc<RoomService>,
    pub session_service: Arc<SessionService>,
    pub chat: Arc<ChatService>,
    pub task_service: Arc<TaskService>,
    /// 定时任务调度器；生产启动时由 `spawn_loop` 驱动，测试可直接 `tick`。
    pub task_scheduler: Arc<TaskScheduler>,
    pub timeline_service: Arc<TimelineService>,
    pub history_service: Arc<RoomHistoryService>,
    pub reaction_service: Arc<ReactionService>,
    pub discussion_service: Arc<DiscussionService>,
    pub collaboration_service: Arc<CollaborationService>,
    pub orchestrator: Arc<OrchestratorEngine>,
    pub pause_control: Arc<PauseControl>,
    pub agent_builder_service: Arc<AgentBuilderService>,
    pub room_store: RoomStore,
    pub queue: Arc<RoomRunQueue>,
    pub custom_harness_store: CustomHarnessStore,
    pub custom_harness_service: Arc<CustomHarnessService>,
    /// ACP registry 目录代理：CDN 拉取 + TTL 缓存 + 快照回退。
    pub acp_registry_service: Arc<AcpRegistryService>,
    pub settings_store: SettingsStore,
    pub settings_service: Arc<SettingsService>,
    pub attachment_service: Arc<AttachmentService>,
    pub ws_hub: RoomSocketHub,
}

#[derive(Default)]
pub struct AppStateOptions {
    /// 测试注入脚本化适配器；生产使用默认工厂。
    pub adapter_factory: Option<AdapterFactory>,
    /// 外部持有同一个 hub 时可注入；默认新建。
    pub hub: Option<RoomSocketHub>,
}

impl AppState {
    /// 装配 store/service，并幂等确保内置主持人存在、回填历史房间。
    pub async fn new(db: Db, storage_root: impl Into<PathBuf>) -> AppResult<Self> {
        Self::new_with_options(db, storage_root, AppStateOptions::default()).await
    }

    pub async fn new_with_options(
        db: Db,
        storage_root: impl Into<PathBuf>,
        options: AppStateOptions,
    ) -> AppResult<Self> {
        let storage_root = storage_root.into();
        let agent_store = AgentStore::new(db.clone());
        let room_store = RoomStore::new(db.clone());
        let message_store = MessageStore::new(db.clone());
        let run_store = RunStore::new(db.clone());
        let session_store = SessionStore::new(db.clone());
        let task_store = TaskStore::new(db.clone());
        let discussion_store = DiscussionStore::new(db.clone());
        let collaboration_store = CollaborationStore::new(db.clone());
        let custom_harness_store = CustomHarnessStore::new(db.clone());
        let custom_harness_service = Arc::new(CustomHarnessService::new(custom_harness_store.clone()));
        let acp_registry_service = Arc::new(AcpRegistryService::new(include_str!(
            "assets/acp_registry_snapshot.json"
        )));
        let settings_store = SettingsStore::new(db.clone());
        let queue_store = QueueStore::new(db.clone());
        let settings_service = Arc::new(SettingsService::new(settings_store.clone()));
        let attachment_store = AttachmentStore::new(db.clone());
        let attachment_service = Arc::new(AttachmentService::new(
            attachment_store,
            room_store.clone(),
            settings_service.clone(),
        ));
        let queue = Arc::new(RoomRunQueue::with_store(queue_store.clone()));
        // 恢复上次进程退出时仍暂停的房间；没有任务的暂停房间也预建 slot。
        queue.seed_paused(&queue_store.list_paused_rooms().await?);
        let hub = options.hub.unwrap_or_default();

        let session_service = Arc::new(SessionService::new(
            session_store,
            room_store.clone(),
            storage_root.clone(),
        ));
        let builtin_moderator =
            BuiltinModeratorService::new(agent_store.clone(), room_store.clone());
        builtin_moderator.ensure_and_backfill().await?;

        let agent_service = Arc::new(AgentService::new(
            agent_store.clone(),
            session_service.clone(),
            custom_harness_store.clone(),
        ));
        let room_service = Arc::new(RoomService::new(
            room_store.clone(),
            agent_store.clone(),
            builtin_moderator,
            session_service.clone(),
        ));

        let broadcast: BroadcastFn = {
            let hub = hub.clone();
            Arc::new(move |room_id, event| hub.broadcast(room_id, event))
        };
        let reaction_service = Arc::new(ReactionService::new(
            message_store.clone(),
            room_store.clone(),
            broadcast.clone(),
        ));
        let adapter_factory: AdapterFactory = {
            let custom_harness_store = custom_harness_store.clone();
            options
                .adapter_factory
                .clone()
                .unwrap_or_else(|| crate::adapters::default_adapter_factory(custom_harness_store))
        };
        let chat = Arc::new(ChatService::new(ChatServiceOptions {
            rooms: room_store.clone(),
            messages: message_store.clone(),
            runs: run_store.clone(),
            agents: agent_store.clone(),
            queue: queue.clone(),
            broadcast: broadcast.clone(),
            adapter_factory: Some(adapter_factory.clone()),
            session_service: Some(session_service.clone()),
            attachments: attachment_service.clone(),
            settings: settings_service.clone(),
        }));

        // 会话归档/删除与历史清理需要知道房间是否仍有排队或运行中的任务。
        let is_room_busy: RoomBusyFn = {
            let chat_weak = Arc::downgrade(&chat);
            Arc::new(move |room_id| {
                chat_weak
                    .upgrade()
                    .is_some_and(|service| service.is_room_busy(room_id))
            })
        };
        session_service.set_busy_checker(is_room_busy.clone());

        let task_service = Arc::new(TaskService::new(TaskServiceOptions {
            rooms: room_store.clone(),
            tasks: task_store.clone(),
            broadcast: broadcast.clone(),
        }));
        let task_scheduler = Arc::new(TaskScheduler::new(task_store.clone(), task_service.clone()));
        let collaboration_service = Arc::new(CollaborationService::new(
            room_store.clone(),
            collaboration_store.clone(),
            message_store.clone(),
            broadcast.clone(),
        ));
        chat.set_collaboration_service(collaboration_service.clone());
        let pause_control = Arc::new(PauseControl::new());
        let orchestrator = Arc::new(OrchestratorEngine::new(
            room_store.clone(),
            task_store.clone(),
            agent_store.clone(),
            message_store.clone(),
            chat.clone(),
            task_service.clone(),
            broadcast.clone(),
            pause_control.clone(),
        ));
        chat.set_orchestrator(Arc::downgrade(&orchestrator));
        let discussion_service = Arc::new(DiscussionService::new(
            room_store.clone(),
            agent_store.clone(),
            message_store.clone(),
            discussion_store.clone(),
            chat.clone(),
            broadcast.clone(),
        ));
        chat.set_discussion_service(Arc::downgrade(&discussion_service));
        chat.set_task_service(Arc::downgrade(&task_service));
        let agent_builder_service = Arc::new(AgentBuilderService::new(AgentBuilderServiceOptions {
            agents: agent_store,
            adapter_factory,
            workspace_dir: storage_root.join("agent-builder"),
            catalog_reader: None,
            settings: settings_service.clone(),
        }));
        let timeline_service = Arc::new(TimelineService::new(
            task_store,
            message_store.clone(),
            run_store.clone(),
        ));
        let history_service = Arc::new(RoomHistoryService::new(RoomHistoryOptions {
            rooms: room_store.clone(),
            messages: message_store,
            runs: run_store,
            is_room_busy,
            broadcast: broadcast.clone(),
            attachments: attachment_service.clone(),
        }));

        // 启动时把上次残留的 streaming/running/讨论一致化为终态；重复执行无副作用。
        let recovery = recovery::recover_interrupted(
            &db,
            Some(&discussion_store),
            Some(&collaboration_store),
            Some(&broadcast),
        )
        .await?;
        tracing::info!(
            streaming_messages = recovery.streaming_messages,
            running_runs = recovery.running_runs,
            blocked_tasks = recovery.blocked_tasks,
            interrupted_discussions = recovery.interrupted_discussions,
            interrupted_collaborations = recovery.interrupted_collaborations,
            "startup recovery"
        );

        // 恢复上次退出时尚未开跑的聊天/主持人任务；running 痕迹已由上面的
        // 恢复标记为 interrupted，不会重复执行。
        chat.restore_persisted_queue().await?;

        // 定时任务调度：首轮延迟一个周期，避免与启动恢复抢跑。
        spawn_loop(task_scheduler.clone(), std::time::Duration::from_secs(15));

        Ok(Self {
            db,
            agent_service,
            room_service,
            session_service,
            chat,
            task_service,
            task_scheduler,
            timeline_service,
            history_service,
            reaction_service,
            discussion_service,
            collaboration_service,
            orchestrator,
            pause_control,
            agent_builder_service,
            room_store,
            queue,
            custom_harness_store,
            custom_harness_service,
            acp_registry_service,
            settings_store,
            settings_service,
            attachment_service,
            ws_hub: hub,
        })
    }
}

pub fn build_app(state: AppState) -> Router {
    // Tauri 桌面端窗口从 `tauri.localhost` 加载页面，HTTP 请求跨到本机 API；
    // 只允许 Tauri 自身与本地 Vite 开发页面的来源。
    let cors = CorsLayer::new()
        .allow_origin([
            HeaderValue::from_static("http://tauri.localhost"),
            HeaderValue::from_static("tauri://localhost"),
            HeaderValue::from_static("http://localhost:5173"),
        ])
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .merge(routes::agents::router())
        .merge(routes::acp_registry::router())
        .merge(routes::custom_harnesses::router())
        .merge(routes::rooms::router())
        .merge(routes::messages::router())
        .merge(routes::attachments::router())
        .merge(routes::files::router())
        .merge(routes::runs::router())
        .merge(routes::sessions::router())
        .merge(routes::tasks::router())
        .merge(routes::timeline::router())
        .merge(routes::discussions::router())
        .merge(routes::collaborations::router())
        .merge(routes::orchestration::router())
        .merge(routes::agent_builder::router())
        .merge(routes::harnesses::router())
        .merge(routes::settings::router())
        .merge(ws::router())
        .route("/api/health", get(health))
        .fallback(fallback)
        .layer(cors)
        .with_state(state)
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true }))
}

async fn fallback(request: Request) -> AppError {
    AppError::not_found(format!(
        "接口不存在：{} {}",
        request.method(),
        request.uri().path()
    ))
}
