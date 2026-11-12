//! 会话总览路由：跨房间只读聚合。

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

use crate::domain::session::SessionOverview;
use crate::error::AppResult;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/sessions", get(list_sessions))
}

async fn list_sessions(State(state): State<AppState>) -> AppResult<Json<Vec<SessionOverview>>> {
    Ok(Json(state.session_service.list_all().await?))
}
