//! 智能体路由。

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, patch};
use axum::{Json, Router};

use crate::domain::agent::Agent;
use crate::domain::room::Room;
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
        .route(
            "/api/agents/{id}/direct-chat",
            get(get_direct_chat).post(create_direct_chat),
        )
}

/// 只读查询：智能体已有直聊时返回房间，否则 404（渲染路径不产生写操作）。
async fn get_direct_chat(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Room>> {
    let Some(room) = state.room_service.get_direct_chat(&id).await? else {
        return Err(crate::error::AppError::not_found("该智能体还没有直聊"));
    };
    Ok(Json(room))
}

/// 幂等创建：已存在返回 200，新建返回 201。
async fn create_direct_chat(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<(StatusCode, Json<Room>)> {
    let (room, created) = state.room_service.ensure_direct_chat(&id).await?;
    Ok((
        if created {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(room),
    ))
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
