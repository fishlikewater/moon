//! Moon 桌面端入口：同进程启动 axum 后端，再创建 Tauri 窗口。
//!
//! 不启动 Node sidecar；数据库与会话存储位于用户本地数据目录。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use moon_server::adapters::{create_adapter, AdapterFactory, EventStream, HarnessAdapter};
use moon_server::db::Db;
use moon_server::domain::agent::{Agent, HarnessKind};
use moon_server::domain::run_event::{RunEvent, RunInput};
use moon_server::{build_app, AppState, AppStateOptions};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

/// 开发调试用 fake harness：产出成功回复与会话 ID，便于在无模型额度时
/// 走通桌面端「运行 → 会话面板归档/删除」流程。
struct DesktopFakeAdapter {
    counter: Arc<AtomicU64>,
}

impl HarnessAdapter for DesktopFakeAdapter {
    fn kind(&self) -> HarnessKind {
        HarnessKind::Fake
    }

    fn run(&self, input: RunInput) -> EventStream {
        let session_id = input
            .session
            .and_then(|session| session.harness_session_id)
            .unwrap_or_else(|| {
                format!(
                    "fake-session-{}",
                    self.counter.fetch_add(1, Ordering::SeqCst) + 1
                )
            });
        let text = "fake harness 已完成本次运行。".to_string();
        let events = vec![
            Ok(RunEvent::Session(session_id)),
            Ok(RunEvent::TextDelta(text.clone())),
            Ok(RunEvent::Final(text)),
        ];
        Box::pin(tokio_stream::iter(events))
    }
}

/// 仅在 `MOON_FAKE_HARNESS=1` 时接管 fake 智能体；其余 harness 行为不变。
fn desktop_adapter_factory() -> Option<AdapterFactory> {
    if std::env::var("MOON_FAKE_HARNESS").ok().as_deref() != Some("1") {
        return None;
    }
    let counter = Arc::new(AtomicU64::new(0));
    Some(Arc::new(move |agent: &Agent| {
        if agent.harness == HarnessKind::Fake {
            return Ok(Arc::new(DesktopFakeAdapter {
                counter: Arc::clone(&counter),
            }) as Arc<dyn HarnessAdapter>);
        }
        create_adapter(agent.harness)
    }))
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let data_dir = app.path().app_local_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let db_path = data_dir.join("moon.db");
            let storage_root = data_dir.join("sessions");

            let db = tauri::async_runtime::block_on(Db::open(&db_path))?;
            let state = tauri::async_runtime::block_on(AppState::new_with_options(
                db,
                storage_root,
                AppStateOptions {
                    adapter_factory: desktop_adapter_factory(),
                    hub: None,
                },
            ))?;
            let listener = tauri::async_runtime::block_on(TcpListener::bind(("127.0.0.1", 0)))?;
            let port = listener.local_addr()?.port();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = axum::serve(listener, build_app(state)).await {
                    eprintln!("Moon 后端已停止：{error}");
                }
            });
            tracing::info!(port, db = %db_path.display(), "Moon desktop backend ready");

            // 页面脚本先于前端代码执行；client/ws 从这里读取本机 API 地址。
            let script = format!(
                "window.__MOON_API_BASE__ = \"http://127.0.0.1:{port}\";"
            );
            WebviewWindowBuilder::new(
                app.handle(),
                "main",
                WebviewUrl::App("index.html".into()),
            )
            .title("Moon 开发工作台")
            .inner_size(1440.0, 900.0)
            .min_inner_size(1100.0, 700.0)
            .decorations(false)
            .initialization_script(&script)
            .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Moon 桌面端启动失败");
}
