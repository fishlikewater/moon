//! 附件元数据访问层。
//!
//! 磁盘文件由 service 写入/删除；本层只负责 `message_attachments` 行，
//! 并为消息列表提供一次批量查询。

use std::collections::HashMap;

use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::db::Db;
use crate::domain::attachment::{Attachment, CreateAttachmentInput, MessageAttachment};
use crate::domain::now_ms;
use crate::error::{AppError, AppResult};

const ATTACHMENT_COLUMNS: &str =
    "id, room_id, message_id, name, mime_type, size, storage_path, created_at";
const QUERY_CHUNK_SIZE: usize = 500;

#[derive(Debug, Clone)]
pub struct AttachmentStore {
    db: Db,
}

impl AttachmentStore {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    fn pool(&self) -> &SqlitePool {
        self.db.pool()
    }

    pub async fn insert(&self, input: CreateAttachmentInput) -> AppResult<Attachment> {
        let created_at = now_ms();
        let result = sqlx::query(
            "INSERT INTO message_attachments
               (room_id, message_id, name, mime_type, size, storage_path, created_at)
             VALUES (?, NULL, ?, ?, ?, ?, ?)",
        )
        .bind(input.room_id)
        .bind(&input.name)
        .bind(&input.mime_type)
        .bind(input.size)
        .bind(&input.storage_path)
        .bind(created_at)
        .execute(self.pool())
        .await?;

        Ok(Attachment {
            id: result.last_insert_rowid(),
            room_id: input.room_id,
            message_id: None,
            name: input.name,
            mime_type: input.mime_type,
            size: input.size,
            storage_path: input.storage_path,
            created_at,
        })
    }

    pub async fn get_by_id(&self, id: i64) -> AppResult<Option<Attachment>> {
        let row = sqlx::query_as::<_, AttachmentRow>(&format!(
            "SELECT {ATTACHMENT_COLUMNS} FROM message_attachments WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(map_row))
    }

    /// 按消息 id 批量挂载附件；不存在的消息自然没有键。
    pub async fn list_by_messages(
        &self,
        message_ids: &[i64],
    ) -> AppResult<HashMap<i64, Vec<MessageAttachment>>> {
        let mut grouped: HashMap<i64, Vec<MessageAttachment>> = HashMap::new();
        for chunk in message_ids.chunks(QUERY_CHUNK_SIZE) {
            let placeholders = vec!["?"; chunk.len()].join(", ");
            let sql = format!(
                "SELECT {ATTACHMENT_COLUMNS} FROM message_attachments
                  WHERE message_id IN ({placeholders}) ORDER BY id"
            );
            let mut query = sqlx::query_as::<_, AttachmentRow>(&sql);
            for id in chunk {
                query = query.bind(id);
            }
            let rows = query.fetch_all(self.pool()).await?;
            for row in rows {
                let attachment = map_row(row);
                if let Some(message_id) = attachment.message_id {
                    grouped
                        .entry(message_id)
                        .or_default()
                        .push(MessageAttachment::from(&attachment));
                }
            }
        }
        Ok(grouped)
    }

    /// 清理用：带磁盘路径的完整行。
    pub async fn list_full_by_messages(&self, message_ids: &[i64]) -> AppResult<Vec<Attachment>> {
        let mut items = Vec::new();
        for chunk in message_ids.chunks(QUERY_CHUNK_SIZE) {
            let placeholders = vec!["?"; chunk.len()].join(", ");
            let sql = format!(
                "SELECT {ATTACHMENT_COLUMNS} FROM message_attachments
                  WHERE message_id IN ({placeholders}) ORDER BY id"
            );
            let mut query = sqlx::query_as::<_, AttachmentRow>(&sql);
            for id in chunk {
                query = query.bind(id);
            }
            items.extend(query.fetch_all(self.pool()).await?.into_iter().map(map_row));
        }
        Ok(items)
    }

    /// 发送消息前的业务校验：全部存在、属于该房间、尚未绑定。
    pub async fn validate_for_link(&self, room_id: i64, ids: &[i64]) -> AppResult<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let unique: std::collections::HashSet<i64> = ids.iter().copied().collect();
        if unique.len() != ids.len() {
            return Err(AppError::validation("attachmentIds 不能包含重复项"));
        }
        for id in ids {
            let Some(attachment) = self.get_by_id(*id).await? else {
                return Err(AppError::validation(format!("附件不存在：{id}")));
            };
            if attachment.room_id != room_id {
                return Err(AppError::validation(format!("附件不属于当前房间：{id}")));
            }
            if attachment.message_id.is_some() {
                return Err(AppError::validation(format!("附件已绑定其他消息：{id}")));
            }
        }
        Ok(())
    }

    /// 在消息插入事务内绑定；任何一条未命中都让整个事务回滚。
    pub async fn link_in_transaction(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        room_id: i64,
        message_id: i64,
        ids: &[i64],
    ) -> AppResult<()> {
        for id in ids {
            let result = sqlx::query(
                "UPDATE message_attachments SET message_id = ?
                  WHERE id = ? AND room_id = ? AND message_id IS NULL",
            )
            .bind(message_id)
            .bind(id)
            .bind(room_id)
            .execute(&mut **tx)
            .await?;
            if result.rows_affected() != 1 {
                return Err(AppError::validation(format!(
                    "附件无法绑定（不存在、跨房间或已绑定）：{id}"
                )));
            }
        }
        Ok(())
    }

    /// 只允许删除未绑定附件；返回被删除的完整行供 service 删文件。
    pub async fn delete_unbound(&self, id: i64) -> AppResult<Option<Attachment>> {
        let Some(attachment) = self.get_by_id(id).await? else {
            return Ok(None);
        };
        if attachment.message_id.is_some() {
            return Err(AppError::constraint("附件已绑定消息，不能单独删除"));
        }
        let result =
            sqlx::query("DELETE FROM message_attachments WHERE id = ? AND message_id IS NULL")
                .bind(id)
                .execute(self.pool())
                .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::constraint("附件已绑定消息，不能单独删除"));
        }
        Ok(Some(attachment))
    }

    /// 历史清理：显式删除这些消息的附件行（外键级联之外的兜底）。
    pub async fn delete_for_messages(&self, message_ids: &[i64]) -> AppResult<u64> {
        let mut deleted = 0_u64;
        for chunk in message_ids.chunks(QUERY_CHUNK_SIZE) {
            let placeholders = vec!["?"; chunk.len()].join(", ");
            let sql =
                format!("DELETE FROM message_attachments WHERE message_id IN ({placeholders})");
            let mut query = sqlx::query(&sql);
            for id in chunk {
                query = query.bind(id);
            }
            deleted += query.execute(self.pool()).await?.rows_affected();
        }
        Ok(deleted)
    }
}

#[derive(sqlx::FromRow)]
struct AttachmentRow {
    id: i64,
    room_id: i64,
    message_id: Option<i64>,
    name: String,
    mime_type: String,
    size: i64,
    storage_path: String,
    created_at: i64,
}

fn map_row(row: AttachmentRow) -> Attachment {
    Attachment {
        id: row.id,
        room_id: row.room_id,
        message_id: row.message_id,
        name: row.name,
        mime_type: row.mime_type,
        size: row.size,
        storage_path: row.storage_path,
        created_at: row.created_at,
    }
}
