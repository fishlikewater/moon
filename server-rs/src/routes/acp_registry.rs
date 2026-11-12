//! ACP registry 目录路由：瘦转发层，业务降级逻辑在服务内。

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

use crate::domain::harness::AcpRegistryPayload;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/acp-registry", get(list))
}

async fn list(State(state): State<AppState>) -> Json<AcpRegistryPayload> {
    Json(state.acp_registry_service.list().await)
}
