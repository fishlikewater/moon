//! 委派批次薄路由：最新批次查询与用户停止。

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};

use crate::domain::collaboration::CollaborationBatch;
use crate::error::AppResult;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/rooms/{room_id}/collaborations/latest",
            get(get_latest),
        )
        .route(
            "/api/rooms/{room_id}/collaborations/{batch_id}/stop",
            post(stop_batch),
        )
}

async fn get_latest(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<Option<CollaborationBatch>>> {
    Ok(Json(
        state.collaboration_service.get_latest(&room_id).await?,
    ))
}

async fn stop_batch(
    State(state): State<AppState>,
    Path((room_id, batch_id)): Path<(String, String)>,
) -> AppResult<Json<CollaborationBatch>> {
    Ok(Json(
        state
            .collaboration_service
            .stop(&room_id, &batch_id)
            .await?,
    ))
}
