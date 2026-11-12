//! 讨论薄路由：最新讨论查询、用户停止与失败续跑。
//!
//! 创建讨论只由主持人的自然语言动作触发，不提供创建接口。

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};

use crate::domain::discussion::Discussion;
use crate::error::AppResult;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/rooms/{room_id}/discussions/latest",
            get(get_latest),
        )
        .route(
            "/api/rooms/{room_id}/discussions/{discussion_id}/stop",
            post(stop_discussion),
        )
        .route(
            "/api/rooms/{room_id}/discussions/{discussion_id}/resume",
            post(resume_discussion),
        )
}

async fn get_latest(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> AppResult<Json<Option<Discussion>>> {
    Ok(Json(
        state.discussion_service.get_latest(&room_id).await?,
    ))
}

async fn stop_discussion(
    State(state): State<AppState>,
    Path((room_id, discussion_id)): Path<(String, String)>,
) -> AppResult<Json<Discussion>> {
    Ok(Json(
        state
            .discussion_service
            .stop(&room_id, &discussion_id)
            .await?,
    ))
}

async fn resume_discussion(
    State(state): State<AppState>,
    Path((room_id, discussion_id)): Path<(String, String)>,
) -> AppResult<Json<Discussion>> {
    Ok(Json(
        state
            .discussion_service
            .resume(&room_id, &discussion_id)
            .await?,
    ))
}
