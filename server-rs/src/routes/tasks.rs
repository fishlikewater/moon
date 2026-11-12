//! 任务薄路由：房间任务列表/创建与受控状态流转。

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, patch};
use axum::{Json, Router};

use crate::domain::task::Task;
use crate::error::AppResult;
use crate::routes::parse_json_body;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/rooms/{id}/tasks", get(list_tasks).post(create_task))
        .route("/api/tasks/{id}", patch(update_task))
        .route("/api/tasks/{id}/status", patch(transition_task))
}

async fn list_tasks(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Vec<Task>>> {
    Ok(Json(state.task_service.list_by_room(&id).await?))
}

async fn create_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> AppResult<(StatusCode, Json<Task>)> {
    let task = state
        .task_service
        .create(&id, parse_json_body(&body)?)
        .await?;
    Ok((StatusCode::CREATED, Json(task)))
}

async fn transition_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> AppResult<Json<Task>> {
    Ok(Json(
        state
            .task_service
            .transition(&id, parse_json_body(&body)?)
            .await?,
    ))
}

async fn update_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> AppResult<Json<Task>> {
    Ok(Json(
        state
            .task_service
            .update(&id, parse_json_body(&body)?)
            .await?,
    ))
}
