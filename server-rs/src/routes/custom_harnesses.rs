//! 自定义 ACP agent 注册表路由：瘦转发层。

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};

use crate::domain::harness::{AcpProbeResponse, CustomHarness};
use crate::error::AppResult;
use crate::routes::parse_json_body;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/custom-harnesses", get(list).post(create))
        .route(
            "/api/custom-harnesses/{id}",
            get(get_one).patch(update).delete(remove),
        )
        .route("/api/custom-harnesses/{id}/probe", post(probe))
}

async fn list(State(state): State<AppState>) -> AppResult<Json<Vec<CustomHarness>>> {
    Ok(Json(state.custom_harness_service.list().await?))
}

async fn get_one(
    State(state): State<AppState>,
    Path(raw_id): Path<String>,
) -> AppResult<Json<CustomHarness>> {
    let id = crate::services::parse_id(&raw_id)?;
    Ok(Json(state.custom_harness_service.get(id).await?))
}

async fn probe(
    State(state): State<AppState>,
    Path(raw_id): Path<String>,
) -> AppResult<Json<AcpProbeResponse>> {
    let id = crate::services::parse_id(&raw_id)?;
    Ok(Json(state.custom_harness_service.probe(id).await?))
}

async fn create(
    State(state): State<AppState>,
    body: Bytes,
) -> AppResult<(StatusCode, Json<CustomHarness>)> {
    let created = state
        .custom_harness_service
        .create(require_body_map(&body)?)
        .await?;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn update(
    State(state): State<AppState>,
    Path(raw_id): Path<String>,
    body: Bytes
) -> AppResult<Json<CustomHarness>> {
    let id = crate::services::parse_id(&raw_id)?;
    let updated = state
        .custom_harness_service
        .update(id, require_body_map(&body)?)
        .await?;
    Ok(Json(updated))
}

async fn remove(
    State(state): State<AppState>,
    Path(raw_id): Path<String>,
) -> AppResult<StatusCode> {
    let id = crate::services::parse_id(&raw_id)?;
    state.custom_harness_service.remove(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// 空请求体按历史契约落到空对象。
fn require_body_map(body: &Bytes) -> AppResult<serde_json::Map<String, serde_json::Value>> {
    crate::services::require_body(parse_json_body(body)?)
}
