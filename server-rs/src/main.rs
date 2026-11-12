//! Moon Rust 后端可执行入口。

use moon_server::config::Config;
use moon_server::db::Db;
use moon_server::{build_app, AppState};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = Config::from_env()?;
    let db = Db::open(&config.db_path).await?;
    tracing::info!(
        db = %config.db_path.display(),
        storage = %config.storage_root.display(),
        "database ready"
    );
    match config.run_timeout {
        Some(limit) => tracing::info!(run_timeout_secs = limit.as_secs(), "run timeout enabled"),
        None => tracing::info!("run timeout disabled"),
    }
    let state = AppState::new(db, config.storage_root.clone()).await?;

    let listener = TcpListener::bind((config.host.as_str(), config.port)).await?;
    tracing::info!("Moon Rust API listening on http://{}:{}", config.host, config.port);
    axum::serve(listener, build_app(state)).await?;
    Ok(())
}
