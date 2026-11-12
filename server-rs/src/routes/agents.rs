//! 智能体路由。

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, patch};
use axum::{Json, Router};

use crate::domain::agent::Agent;
use crate::error::AppResult;
use crate::routes::parse_json_body;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/agents", get(list_agents).post(create_agent))
        .route(
            "/api/agents/{id}",
            patch(update_agent).delete(delete_agent),
        )
}

async fn list_agents(State(state): State<AppState>) -> AppResult<Json<Vec<Agent>>> {
    Ok(Json(state.agent_service.list().await?))
}

async fn create_agent(
    State(state): State<AppState>,
    body: Bytes,
) -> AppResult<(StatusCode, Json<Agent>)> {
    let agent = state
        .agent_service
        .create(parse_json_body(&body)?)
        .await?;
    Ok((StatusCode::CREATED, Json(agent)))
}

async fn update_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> AppResult<Json<Agent>> {
    let agent = state
        .agent_service
        .update(&id, parse_json_body(&body)?)
        .await?;
    Ok(Json(agent))
}

async fn delete_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    state.agent_service.delete(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}
