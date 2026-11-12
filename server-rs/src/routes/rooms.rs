//! 房间、成员与会话路由。

use std::collections::HashMap;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, patch, post, put};
use axum::{Json, Router};

use crate::domain::room::Room;
use crate::domain::run::ContextPreview;
use crate::domain::session::{AgentSession, RoomMemberSessionSettings, RoomSessionsResponse};
use crate::error::{AppError, AppResult};
use crate::queue::{QueueCancelError, QueueSnapshot};
use crate::routes::parse_json_body;
use crate::services::{parse_id_label, value_to_id_string};
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/rooms", get(list_rooms).post(create_room))
        .route("/api/rooms/{id}", patch(update_room).delete(delete_room))
        .route("/api/rooms/{id}/members", post(add_member))
        .route(
            "/api/rooms/{id}/members/{agent_id}",
            delete(remove_member),
        )
        .route("/api/rooms/{id}/sessions", get(list_sessions))
        .route("/api/rooms/{id}/context", get(context_preview))
        .route("/api/rooms/{id}/queue", get(get_queue))
        .route(
            "/api/rooms/{id}/queue/pause",
            put(set_queue_paused),
        )
        .route(
            "/api/rooms/{id}/queue/{job_id}",
            delete(cancel_queue_job),
        )
        .route(
            "/api/rooms/{id}/members/{agent_id}/session-settings",
            put(set_reuse_session),
        )
        .route(
            "/api/rooms/{id}/sessions/{session_id}/archive",
            post(archive_session),
        )
        .route(
            "/api/rooms/{id}/sessions/{session_id}",
            delete(remove_session),
        )
}

async fn list_rooms(State(state): State<AppState>) -> AppResult<Json<Vec<Room>>> {
    Ok(Json(state.room_service.list().await?))
}

async fn create_room(
    State(state): State<AppState>,
    body: Bytes,
) -> AppResult<(StatusCode, Json<Room>)> {
    let room = state
        .room_service
        .create(parse_json_body(&body)?)
        .await?;
    Ok((StatusCode::CREATED, Json(room)))
}

async fn update_room(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> AppResult<Json<Room>> {
    let room = state
        .room_service
        .update(&id, parse_json_body(&body)?)
        .await?;
    Ok(Json(room))
}

async fn delete_room(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    state.room_service.remove(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn add_member(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> AppResult<(StatusCode, Json<Room>)> {
    let payload = parse_json_body(&body)?;
    let agent_id = value_to_id_string(payload.get("agentId"));
    let room = state.room_service.add_member(&id, &agent_id).await?;
    Ok((StatusCode::CREATED, Json(room)))
}

async fn remove_member(
    State(state): State<AppState>,
    Path((id, agent_id)): Path<(String, String)>,
) -> AppResult<Json<Room>> {
    let room = state.room_service.remove_member(&id, &agent_id).await?;
    Ok(Json(room))
}

async fn list_sessions(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<RoomSessionsResponse>> {
    Ok(Json(state.session_service.list(&id).await?))
}

async fn context_preview(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> AppResult<Json<ContextPreview>> {
    Ok(Json(state.chat.context_preview(&id, &query).await?))
}

async fn get_queue(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<QueueSnapshot>> {
    let room_id = parse_id_label(&id, "房间")?;
    if state.room_store.get_by_id(room_id).await?.is_none() {
        return Err(AppError::not_found(format!("房间不存在：{room_id}")));
    }
    Ok(Json(state.queue.snapshot(room_id)))
}

async fn set_queue_paused(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> AppResult<Json<QueueSnapshot>> {
    let room_id = parse_id_label(&id, "房间")?;
    if state.room_store.get_by_id(room_id).await?.is_none() {
        return Err(AppError::not_found(format!("房间不存在：{room_id}")));
    }
    let body = parse_json_body(&body)?;
    let paused = body
        .get("paused")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| AppError::validation("paused 必须是布尔值"))?;
    Ok(Json(state.queue.set_paused(room_id, paused).await?))
}

async fn cancel_queue_job(
    State(state): State<AppState>,
    Path((id, job_id)): Path<(String, String)>,
) -> AppResult<StatusCode> {
    let room_id = parse_id_label(&id, "房间")?;
    if state.room_store.get_by_id(room_id).await?.is_none() {
        return Err(AppError::not_found(format!("房间不存在：{room_id}")));
    }
    let job_id = parse_id_label(&job_id, "队列任务")? as u64;
    match state.queue.cancel(room_id, job_id).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(QueueCancelError::NotFound) => Err(AppError::not_found(format!(
            "队列任务不存在：{job_id}"
        ))),
        Err(QueueCancelError::Running) => {
            Err(AppError::constraint("该任务已开始运行，不能从队列取消"))
        }
        Err(QueueCancelError::NotCancelable) => Err(AppError::constraint(
            "该任务类型不支持队列取消，请使用对应的停止入口",
        )),
    }
}

async fn set_reuse_session(
    State(state): State<AppState>,
    Path((id, agent_id)): Path<(String, String)>,
    body: Bytes,
) -> AppResult<Json<RoomMemberSessionSettings>> {
    let settings = state
        .session_service
        .set_reuse_session(&id, &agent_id, parse_json_body(&body)?)
        .await?;
    Ok(Json(settings))
}

async fn archive_session(
    State(state): State<AppState>,
    Path((id, session_id)): Path<(String, String)>,
) -> AppResult<Json<AgentSession>> {
    Ok(Json(
        state.session_service.archive(&id, &session_id).await?,
    ))
}

async fn remove_session(
    State(state): State<AppState>,
    Path((id, session_id)): Path<(String, String)>,
) -> AppResult<StatusCode> {
    state.session_service.remove(&id, &session_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
