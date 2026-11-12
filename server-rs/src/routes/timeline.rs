//! 任务时间线薄路由：`GET /api/tasks/{id}/timeline`。

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};

use crate::domain::timeline::TaskTimelineResponse;
use crate::error::AppResult;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/tasks/{id}/timeline", get(get_timeline))
}

async fn get_timeline(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<TaskTimelineResponse>> {
    Ok(Json(state.timeline_service.get_timeline(&id).await?))
}
