//! 模型供应商注册表路由：瘦转发层。

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};

use crate::domain::provider::ApiProvider;
use crate::error::AppResult;
use crate::routes::parse_json_body;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/api-providers", get(list).post(create))
        .route(
            "/api/api-providers/{id}",
            get(get_one).patch(update).delete(remove),
        )
}

async fn list(State(state): State<AppState>) -> AppResult<Json<Vec<ApiProvider>>> {
    Ok(Json(state.api_provider_service.list().await?))
}

async fn get_one(
    State(state): State<AppState>,
    Path(raw_id): Path<String>,
) -> AppResult<Json<ApiProvider>> {
    let id = crate::services::parse_id(&raw_id)?;
    Ok(Json(state.api_provider_service.get(id).await?))
}

async fn create(
    State(state): State<AppState>,
    body: Bytes,
) -> AppResult<(StatusCode, Json<ApiProvider>)> {
    let created = state
        .api_provider_service
        .create(require_body_map(&body)?)
        .await?;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn update(
    State(state): State<AppState>,
    Path(raw_id): Path<String>,
    body: Bytes,
) -> AppResult<Json<ApiProvider>> {
    let id = crate::services::parse_id(&raw_id)?;
    let updated = state
        .api_provider_service
        .update(id, require_body_map(&body)?)
        .await?;
    Ok(Json(updated))
}

async fn remove(
    State(state): State<AppState>,
    Path(raw_id): Path<String>,
) -> AppResult<StatusCode> {
    let id = crate::services::parse_id(&raw_id)?;
    state.api_provider_service.remove(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// 空请求体按历史契约落到空对象。
fn require_body_map(body: &Bytes) -> AppResult<serde_json::Map<String, serde_json::Value>> {
    crate::services::require_body(parse_json_body(body)?)
}
