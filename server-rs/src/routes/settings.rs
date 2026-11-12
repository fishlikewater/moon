//! 应用设置路由。

use axum::body::Bytes;
use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

use crate::domain::settings::{AppSettings, AppSettingsResponse};
use crate::error::AppResult;
use crate::routes::parse_json_body;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/settings", get(get_settings).patch(update_settings))
}

async fn get_settings(State(state): State<AppState>) -> AppResult<Json<AppSettingsResponse>> {
    Ok(Json(state.settings_service.get().await?))
}

async fn update_settings(
    State(state): State<AppState>,
    body: Bytes,
) -> AppResult<Json<AppSettings>> {
    Ok(Json(
        state
            .settings_service
            .update(&parse_json_body(&body)?)
            .await?,
    ))
}
