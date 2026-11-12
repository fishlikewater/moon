//! 房间文件清单与下载路由。

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use crate::error::{AppError, AppResult};
use crate::services::files::{self, RoomFiles};
use crate::services::parse_id;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/rooms/{id}/files", get(list_files))
        .route("/api/rooms/{id}/files/download", get(download_file))
}

#[derive(Debug, Deserialize)]
struct DownloadQuery {
    root: Option<String>,
    path: Option<String>,
}

async fn list_files(
    State(state): State<AppState>,
    Path(raw_room_id): Path<String>,
) -> AppResult<Json<RoomFiles>> {
    let room_id = parse_id(&raw_room_id)?;
    Ok(Json(files::list_room_files(&state.room_store, room_id).await?))
}

async fn download_file(
    State(state): State<AppState>,
    Path(raw_room_id): Path<String>,
    Query(query): Query<DownloadQuery>,
) -> AppResult<Response> {
    let room_id = parse_id(&raw_room_id)?;
    let root = query.root.unwrap_or_default();
    let path = query.path.unwrap_or_default();
    if root.trim().is_empty() {
        return Err(AppError::validation("缺少 root 参数"));
    }
    let file = files::read_room_file(&state.room_store, room_id, &root, &path).await?;

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!(
            "attachment; filename=\"{}\"",
            file.file_name
        ))
        .map_err(|_| AppError::internal("无法生成文件下载响应头"))?,
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Ok((headers, Bytes::from(file.bytes)).into_response())
}
