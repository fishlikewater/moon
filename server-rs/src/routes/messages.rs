//! 消息路由：发送返回 202，智能体回复经 WebSocket 推送。
//!
//! `POST /api/rooms/{id}/messages/cleanup` 同步清理纯聊天历史，
//! 返回实际删除计数；任务/讨论/委派消息永不进入删除集合。

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;

use crate::domain::message::{Message, MessageCleanupResult, MessageReaction};
use crate::error::AppResult;
use crate::routes::parse_json_body;
use crate::services::chat::HandleUserMessageResult;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/rooms/{id}/messages",
            get(list_messages).post(post_message),
        )
        .route("/api/rooms/{id}/messages/cleanup", post(cleanup_history))
        .route(
            "/api/rooms/{id}/messages/{message_id}/reactions",
            post(toggle_reaction),
        )
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReactionListResponse {
    reactions: Vec<MessageReaction>,
}

async fn list_messages(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Vec<Message>>> {
    Ok(Json(state.chat.list_messages(&id).await?))
}

async fn post_message(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> AppResult<(StatusCode, Json<HandleUserMessageResult>)> {
    let result = state
        .chat
        .handle_user_message(&id, parse_json_body(&body)?)
        .await?;
    Ok((StatusCode::ACCEPTED, Json(result)))
}

async fn cleanup_history(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> AppResult<Json<MessageCleanupResult>> {
    Ok(Json(
        state
            .history_service
            .cleanup(&id, &parse_json_body(&body)?)
            .await?,
    ))
}

async fn toggle_reaction(
    State(state): State<AppState>,
    Path((id, message_id)): Path<(String, String)>,
    body: Bytes,
) -> AppResult<Json<ReactionListResponse>> {
    let reactions = state
        .reaction_service
        .toggle(&id, &message_id, &parse_json_body(&body)?)
        .await?;
    Ok(Json(ReactionListResponse { reactions }))
}
