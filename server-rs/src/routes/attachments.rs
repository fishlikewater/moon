//! 附件上传/下载/删除路由。
//!
//! multipart 解析与大小上限在此收口；路径、文件名与磁盘写入在 service 层。

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Multipart, Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};

use crate::domain::attachment::MessageAttachment;
use crate::error::{AppError, AppResult};
use crate::services::attachments::AttachmentUpload;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/rooms/{id}/attachments",
            post(upload).layer(DefaultBodyLimit::disable()),
        )
        .route("/api/attachments/{id}", get(download).delete(remove))
}

async fn upload(
    State(state): State<AppState>,
    Path(raw_room_id): Path<String>,
    mut multipart: Multipart,
) -> AppResult<(StatusCode, Json<MessageAttachment>)> {
    let room_id = crate::services::parse_id(&raw_room_id)?;
    let max_bytes = state.attachment_service.max_file_bytes().await?;

    let mut found = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| AppError::validation(format!("multipart 解析失败：{error}")))?
    {
        if field.name() != Some("file") {
            continue;
        }
        let name = field.file_name().unwrap_or("file").to_string();
        let mime_type = field
            .content_type()
            .unwrap_or("application/octet-stream")
            .to_string();
        let mut bytes: Vec<u8> = Vec::new();
        let mut field = field;
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|error| AppError::validation(format!("读取上传内容失败：{error}")))?
        {
            if bytes.len() as i64 + chunk.len() as i64 > max_bytes {
                return Err(AppError::validation(format!(
                    "附件超过单文件上限（{max_bytes} 字节）"
                )));
            }
            bytes.extend_from_slice(&chunk);
        }
        found = Some(AttachmentUpload {
            name,
            mime_type,
            bytes,
        });
        break;
    }

    let Some(upload) = found else {
        return Err(AppError::validation("multipart 必须包含 file 字段"));
    };
    let attachment = state.attachment_service.upload(room_id, upload).await?;
    Ok((StatusCode::CREATED, Json(attachment)))
}

async fn download(
    State(state): State<AppState>,
    Path(raw_id): Path<String>,
) -> AppResult<Response> {
    let id = crate::services::parse_id(&raw_id)?;
    let (attachment, bytes) = state.attachment_service.download(id).await?;

    let mut headers = HeaderMap::new();
    let dangerous = is_dangerous_mime(&attachment.mime_type);
    let content_type = if dangerous {
        "application/octet-stream".to_string()
    } else {
        attachment.mime_type.clone()
    };
    let content_type = HeaderValue::from_str(&content_type)
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));
    headers.insert(header::CONTENT_TYPE, content_type);
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&if dangerous {
            format!("attachment; filename=\"attachment-{id}\"")
        } else {
            format!("inline; filename=\"attachment-{id}\"")
        })
        .map_err(|_| AppError::internal("无法生成附件下载响应头"))?,
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    let body = Bytes::from(bytes);
    Ok((headers, body).into_response())
}

/// 可被浏览器直接执行或渲染脚本的 MIME 一律降级为下载。
fn is_dangerous_mime(mime_type: &str) -> bool {
    let mime_type = mime_type.to_ascii_lowercase();
    mime_type.starts_with("text/html")
        || mime_type.starts_with("application/xhtml")
        || mime_type.starts_with("image/svg")
        || mime_type.starts_with("text/xml")
        || mime_type.starts_with("application/xml")
}

async fn remove(
    State(state): State<AppState>,
    Path(raw_id): Path<String>,
) -> AppResult<StatusCode> {
    let id = crate::services::parse_id(&raw_id)?;
    state.attachment_service.delete_unbound(id).await?;
    Ok(StatusCode::NO_CONTENT)
}
