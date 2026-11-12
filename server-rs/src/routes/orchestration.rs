//! 编排控制薄路由：任务激活与房间级暂停开关。

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::domain::task::Task;
use crate::error::{AppError, AppResult};
use crate::services::parse_id_label;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/tasks/{id}/activate", post(activate_task))
        .route("/api/rooms/{id}/orchestration", get(get_state))
        .route("/api/rooms/{id}/orchestration/pause", post(pause))
        .route("/api/rooms/{id}/orchestration/resume", post(resume))
}

async fn activate_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<(StatusCode, Json<Task>)> {
    let (task, _started_now) = state.task_service.activate(&id).await?;
    if state.orchestrator.is_running(task.id) {
        return Err(AppError::constraint(format!(
            "任务 #{} 的编排已在运行中",
            task.id
        )));
    }
    state.orchestrator.start(task.id);
    Ok((StatusCode::ACCEPTED, Json(task)))
}

async fn get_state(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let room_id = require_room(&state, &id).await?;
    Ok(Json(json!({
        "roomId": room_id,
        "paused": state.pause_control.is_paused(room_id),
    })))
}

async fn pause(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let room_id = require_room(&state, &id).await?;
    state.pause_control.pause(room_id);
    Ok(Json(json!({ "roomId": room_id, "paused": true })))
}

async fn resume(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let room_id = require_room(&state, &id).await?;
    state.pause_control.resume(room_id);
    Ok(Json(json!({ "roomId": room_id, "paused": false })))
}

async fn require_room(state: &AppState, raw: &str) -> AppResult<i64> {
    let room_id = parse_id_label(raw, "房间")?;
    if state.room_store.get_by_id(room_id).await?.is_none() {
        return Err(AppError::not_found(format!("房间不存在：{room_id}")));
    }
    Ok(room_id)
}
