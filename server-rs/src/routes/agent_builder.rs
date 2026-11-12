//! 创建助手薄路由：只做参数转发，业务规则在 `AgentBuilderService`。

use axum::body::Bytes;
use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};

use crate::domain::agent_builder::AgentBuilderChatResponse;
use crate::error::AppResult;
use crate::routes::parse_json_body;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/agent-builder/chat", post(chat))
}

async fn chat(
    State(state): State<AppState>,
    body: Bytes,
) -> AppResult<Json<AgentBuilderChatResponse>> {
    Ok(Json(
        state
            .agent_builder_service
            .chat(parse_json_body(&body)?)
            .await?,
    ))
}
