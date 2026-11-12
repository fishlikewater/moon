//! 房间附件上传、下载与磁盘清理。
//!
//! 文件固定保存在房间首个工作目录的 `.moon/attachments/room-<id>/`；
//! 客户端不能指定磁盘路径，原始文件名只进入元数据。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::domain::attachment::{Attachment, CreateAttachmentInput, MessageAttachment};
use crate::error::{AppError, AppResult};
use crate::services::settings::SettingsService;
use crate::store::{AttachmentStore, RoomStore};

const MAX_FILE_NAME_CHARS: usize = 120;
static FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// 路由已按上限流式读取后的上传载荷。
pub struct AttachmentUpload {
    pub name: String,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

pub struct AttachmentService {
    store: AttachmentStore,
    rooms: RoomStore,
    settings: Arc<SettingsService>,
}

impl AttachmentService {
    pub fn new(store: AttachmentStore, rooms: RoomStore, settings: Arc<SettingsService>) -> Self {
        Self {
            store,
            rooms,
            settings,
        }
    }

    /// 单文件上限（字节）；路由读取 multipart 前先取该值。
    pub async fn max_file_bytes(&self) -> AppResult<i64> {
        Ok(self
            .settings
            .get()
            .await?
            .settings
            .attachment_max_file_bytes)
    }

    pub async fn upload(
        &self,
        room_id: i64,
        upload: AttachmentUpload,
    ) -> AppResult<MessageAttachment> {
        let max_bytes = self.max_file_bytes().await?;
        if upload.bytes.is_empty() {
            return Err(AppError::validation("附件不能为空"));
        }
        if upload.bytes.len() as i64 > max_bytes {
            return Err(AppError::validation(format!(
                "附件超过单文件上限（{} 字节）",
                max_bytes
            )));
        }

        let room = self
            .rooms
            .get_by_id(room_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("房间不存在：{room_id}")))?;
        let workspace = PathBuf::from(&room.workspace_dir);
        if !workspace.is_dir() {
            return Err(AppError::constraint(format!(
                "房间工作目录不可用，无法保存附件：{}",
                room.workspace_dir
            )));
        }
        let workspace = workspace
            .canonicalize()
            .map_err(|error| AppError::constraint(format!("房间工作目录不可解析：{error}")))?;

        let directory = workspace
            .join(".moon")
            .join("attachments")
            .join(format!("room-{room_id}"));
        tokio::fs::create_dir_all(&directory)
            .await
            .map_err(|error| AppError::constraint(format!("无法创建附件目录：{error}")))?;
        let directory = directory
            .canonicalize()
            .map_err(|error| AppError::constraint(format!("附件目录不可解析：{error}")))?;
        if !directory.starts_with(&workspace) {
            return Err(AppError::constraint("附件目录超出房间工作区"));
        }

        let file_name = generated_file_name(&upload.name);
        let target = directory.join(&file_name);
        if target.parent() != Some(directory.as_path()) {
            return Err(AppError::validation("附件文件名不合法"));
        }
        let size = upload.bytes.len() as i64;
        if let Err(error) = tokio::fs::write(&target, &upload.bytes).await {
            let _ = tokio::fs::remove_file(&target).await;
            return Err(AppError::constraint(format!("写入附件失败：{error}")));
        }

        let attachment = self
            .store
            .insert(CreateAttachmentInput {
                room_id,
                name: cleaned_display_name(&upload.name),
                mime_type: normalize_mime(&upload.mime_type),
                size,
                storage_path: target.to_string_lossy().into_owned(),
            })
            .await;
        match attachment {
            Ok(attachment) => Ok(MessageAttachment::from(&attachment)),
            Err(error) => {
                let _ = tokio::fs::remove_file(&target).await;
                Err(error)
            }
        }
    }

    pub async fn download(&self, id: i64) -> AppResult<(Attachment, Vec<u8>)> {
        let attachment = self
            .store
            .get_by_id(id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("附件不存在：{id}")))?;
        let bytes = tokio::fs::read(&attachment.storage_path)
            .await
            .map_err(|_| AppError::not_found(format!("附件文件不存在：{id}")))?;
        Ok((attachment, bytes))
    }

    /// 只允许删除未绑定附件；返回是否确实删除。
    pub async fn delete_unbound(&self, id: i64) -> AppResult<()> {
        let Some(attachment) = self.store.delete_unbound(id).await? else {
            return Err(AppError::not_found(format!("附件不存在：{id}")));
        };
        let _ = tokio::fs::remove_file(&attachment.storage_path).await;
        Ok(())
    }

    /// 发送消息前校验数量、归属与绑定状态。
    pub async fn validate_for_message(&self, room_id: i64, ids: &[i64]) -> AppResult<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let max_files = self
            .settings
            .get()
            .await?
            .settings
            .attachment_max_files_per_message;
        if ids.len() as i64 > max_files {
            return Err(AppError::validation(format!(
                "单条消息最多 {max_files} 个附件"
            )));
        }
        self.store.validate_for_link(room_id, ids).await
    }

    pub async fn list_for_messages(&self, message_ids: &[i64]) -> AppResult<Vec<Attachment>> {
        self.store.list_full_by_messages(message_ids).await
    }

    pub async fn delete_rows_for_messages(&self, message_ids: &[i64]) -> AppResult<u64> {
        self.store.delete_for_messages(message_ids).await
    }

    /// 尽力删除磁盘文件；返回成功删除数，缺失文件不计。
    pub fn remove_files(&self, attachments: &[Attachment]) -> u64 {
        attachments
            .iter()
            .filter(|attachment| remove_file_if_exists(Path::new(&attachment.storage_path)))
            .count() as u64
    }
}

fn remove_file_if_exists(path: &Path) -> bool {
    match std::fs::remove_file(path) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => false,
    }
}

/// 展示名：去掉路径分隔符与控制字符，保留可读字符。
fn cleaned_display_name(raw: &str) -> String {
    let name = raw
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .trim()
        .chars()
        .filter(|character| !character.is_control())
        .collect::<String>();
    if name.is_empty() {
        "file".to_string()
    } else {
        name.chars().take(MAX_FILE_NAME_CHARS).collect()
    }
}

/// 磁盘名：只保留字母数字、`.`、`-`、`_`，其余替换为 `_`。
fn generated_file_name(raw: &str) -> String {
    let base = cleaned_display_name(raw);
    let sanitized: String = base
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect();
    let sanitized = sanitized.trim_matches('.').trim_matches('_');
    let sanitized = if sanitized.is_empty() {
        "file"
    } else {
        sanitized
    };
    let counter = FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(
        "{}-{}-{}",
        crate::domain::now_ms(),
        counter,
        sanitized
            .chars()
            .take(MAX_FILE_NAME_CHARS)
            .collect::<String>()
    )
}

fn normalize_mime(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.len() > 200
        || !trimmed
            .chars()
            .all(|character| !character.is_control() && character != ' ')
    {
        "application/octet-stream".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_disk_names_and_rejects_path_components() {
        let name = generated_file_name("../evil name.txt");
        assert!(!name.contains(".."));
        assert!(!name.contains('/'));
        assert!(!name.contains('\\'));
        assert!(name.ends_with("evil_name.txt"));

        let empty = generated_file_name("");
        assert!(empty.ends_with("file"));
    }

    #[test]
    fn keeps_unicode_display_but_not_control_characters() {
        assert_eq!(cleaned_display_name("报告\u{0000}.pdf"), "报告.pdf");
        assert_eq!(cleaned_display_name("C:\\tmp\\a.txt"), "a.txt");
        assert_eq!(cleaned_display_name("   "), "file");
    }

    #[test]
    fn normalizes_invalid_mime() {
        assert_eq!(normalize_mime("image/png"), "image/png");
        assert_eq!(normalize_mime(""), "application/octet-stream");
        assert_eq!(
            normalize_mime("text/plain\r\nX-Evil: 1"),
            "application/octet-stream"
        );
    }
}
