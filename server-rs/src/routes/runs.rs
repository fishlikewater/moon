//! 运行查询路由：最近运行列表与单次运行详情。

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};

use crate::domain::run::{AgentRun, RunDetailResponse};
use crate::error::AppResult;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/rooms/{id}/runs", get(list_runs))
        .route("/api/rooms/{id}/runs/{run_id}", get(get_run_detail))
}

async fn list_runs(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> AppResult<Json<Vec<AgentRun>>> {
    Ok(Json(
        state
            .chat
            .list_runs(&id, query.get("limit").map(String::as_str))
            .await?,
    ))
}

async fn get_run_detail(
    State(state): State<AppState>,
    Path((id, run_id)): Path<(String, String)>,
) -> AppResult<Json<RunDetailResponse>> {
    Ok(Json(state.chat.get_run_detail(&id, &run_id).await?))
}
