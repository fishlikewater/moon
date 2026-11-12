//! 消息附件领域类型。

use serde::Serialize;

/// 附件元数据；`storage_path` 只用于服务端定位磁盘文件，不返回前端。
#[derive(Debug, Clone, PartialEq)]
pub struct Attachment {
    pub id: i64,
    pub room_id: i64,
    pub message_id: Option<i64>,
    pub name: String,
    pub mime_type: String,
    pub size: i64,
    pub storage_path: String,
    pub created_at: i64,
}

/// 消息与上传响应共用的客户端视图。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageAttachment {
    pub id: i64,
    pub name: String,
    pub mime_type: String,
    pub size: i64,
    pub created_at: i64,
    /// 绝对路径；只服务提示词与清理，序列化时跳过。
    #[serde(skip_serializing, skip_deserializing, default)]
    pub storage_path: String,
}

impl From<&Attachment> for MessageAttachment {
    fn from(attachment: &Attachment) -> Self {
        Self {
            id: attachment.id,
            name: attachment.name.clone(),
            mime_type: attachment.mime_type.clone(),
            size: attachment.size,
            created_at: attachment.created_at,
            storage_path: attachment.storage_path.clone(),
        }
    }
}

/// store 层创建参数。
#[derive(Debug, Clone)]
pub struct CreateAttachmentInput {
    pub room_id: i64,
    pub name: String,
    pub mime_type: String,
    pub size: i64,
    pub storage_path: String,
}
