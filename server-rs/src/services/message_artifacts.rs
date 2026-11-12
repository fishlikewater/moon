//! 长回复归档与提示词压缩。
//!
//! 把智能体的完整回复写入房间工作区，作为可追溯的全文；为提示词生成
//! 「摘要 + 绝对路径」，避免历史消息把上下文无限撑大。归档失败时调用方
//! 保持原文即可，不影响消息本身。

use std::path::{Path, PathBuf};

use chrono::{DateTime, SecondsFormat, Utc};

use crate::domain::attachment::MessageAttachment;
use crate::domain::message::{Message, MessageKind};
use crate::domain::room::Room;

/// 超过该字符数的回复才归档。
pub const ARCHIVE_MIN_CHARS: usize = 8_000;
/// 转录里保留的摘要长度（字符）。
pub const ARCHIVE_EXCERPT_CHARS: usize = 400;
/// 归档文件的单行上限；超过则软换行，保证 Read 工具能完整读取。
pub const ARCHIVE_LINE_LIMIT: usize = 1_000;

pub struct ArtifactInput<'a> {
    pub workspace_dir: &'a str,
    pub room_id: i64,
    pub message_id: i64,
    pub room_name: &'a str,
    pub sender_label: &'a str,
    pub message_seq: i64,
    pub content: &'a str,
    pub created_at: i64,
}

/// 归档文件的固定位置：房间工作区下的 `.moon/artifacts/room-<id>/`。
pub fn archive_path(workspace_dir: &str, room_id: i64, message_id: i64) -> PathBuf {
    Path::new(workspace_dir)
        .join(".moon")
        .join("artifacts")
        .join(format!("room-{room_id}"))
        .join(format!("message-{message_id}.md"))
}

/// 写入归档文件；失败返回 None，由调用方决定降级方式。
pub fn save_message_artifact(input: &ArtifactInput<'_>) -> Option<PathBuf> {
    let file_path = archive_path(input.workspace_dir, input.room_id, input.message_id);
    let directory = file_path.parent()?;
    if std::fs::create_dir_all(directory).is_err() {
        return None;
    }
    // 显式 UTF-8：Rust 字符串本身即 UTF-8，按字节写入。
    std::fs::write(&file_path, format_artifact(input)).ok()?;
    Some(file_path)
}

/// 已存在则直接返回路径，缺失时补写；用于历史消息的按需归档。
pub fn ensure_message_artifact(input: &ArtifactInput<'_>) -> Option<PathBuf> {
    let file_path = archive_path(input.workspace_dir, input.room_id, input.message_id);
    if file_path.exists() {
        return Some(file_path);
    }
    save_message_artifact(input)
}

/// 阶段总结不进入模型上下文，只服务展示。
pub fn context_messages(messages: &[Message]) -> Vec<Message> {
    messages
        .iter()
        .filter(|message| message.kind != MessageKind::Progress)
        .cloned()
        .collect()
}

/// 生成一条转录行；长消息替换为摘要 + 路径，归档失败时退回原文。
pub fn compact_message_line(room: &Room, message: &Message, label: &str) -> String {
    let prefix = format!("#{} {}：", message.seq, label);
    let attachments = attachment_prompt_lines(&message.attachments);
    if message.content.chars().count() < ARCHIVE_MIN_CHARS {
        return format!("{prefix}{}{attachments}", message.content);
    }

    let file_path = ensure_message_artifact(&ArtifactInput {
        workspace_dir: &room.workspace_dir,
        room_id: room.id,
        message_id: message.id,
        room_name: &room.name,
        sender_label: label,
        message_seq: message.seq,
        content: &message.content,
        created_at: message.created_at,
    });
    let Some(file_path) = file_path else {
        return format!("{prefix}{}{attachments}", message.content);
    };

    let excerpt: String = message.content.chars().take(ARCHIVE_EXCERPT_CHARS).collect();
    format!(
        "{prefix}[长回复已归档，全文 {} 字] {excerpt}…\n（完整内容：{}，可用 Read 工具查看）{attachments}",
        message.content.chars().count(),
        file_path.display()
    )
}

/// 附件清单：每条消息列出名称、MIME、大小与本机绝对路径。
pub fn attachment_prompt_lines(attachments: &[MessageAttachment]) -> String {
    if attachments.is_empty() {
        return String::new();
    }
    let lines = attachments
        .iter()
        .map(|attachment| {
            format!(
                "附件：{}（{}，{} B）→ {}",
                attachment.name, attachment.mime_type, attachment.size, attachment.storage_path
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("\n{lines}")
}

/// 当前用户指令：正文为空时给出附件说明，非空时原样保留正文。
pub fn instruction_with_attachments(
    content: &str,
    attachments: &[MessageAttachment],
) -> String {
    let suffix = attachment_prompt_lines(attachments);
    if suffix.is_empty() {
        return content.to_string();
    }
    if content.trim().is_empty() {
        format!("（用户发送了附件，未附加文字说明）{suffix}")
    } else {
        format!("{content}{suffix}")
    }
}

fn format_artifact(input: &ArtifactInput<'_>) -> String {
    let timestamp = DateTime::<Utc>::from_timestamp_millis(input.created_at)
        .map(|value| value.to_rfc3339_opts(SecondsFormat::Millis, true))
        .unwrap_or_else(|| "未知".to_string());
    let header = [
        "# 智能体回复归档".to_string(),
        String::new(),
        format!("- 房间：{}（#{}）", input.room_name, input.room_id),
        format!("- 智能体：{}", input.sender_label),
        format!(
            "- 消息：#{}（id={}）",
            input.message_seq, input.message_id
        ),
        format!("- 时间：{timestamp}"),
        format!("- 字数：{}", input.content.chars().count()),
        String::new(),
        "---".to_string(),
        String::new(),
    ]
    .join("\n");
    format!(
        "{header}{}\n",
        wrap_long_lines(input.content, ARCHIVE_LINE_LIMIT)
    )
}

/// 把超长单行软换行到 limit 以内；只插入换行符，不改写原字符。
pub fn wrap_long_lines(text: &str, limit: usize) -> String {
    if limit == 0 {
        return text.to_string();
    }
    text.split('\n')
        .map(|line| {
            let chars: Vec<char> = line.chars().collect();
            if chars.len() <= limit {
                return line.to_string();
            }
            chars
                .chunks(limit)
                .map(|chunk| chunk.iter().collect::<String>())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::message::{MessageSenderType, MessageStatus};
    use crate::domain::room::{RoomPermissionMode, RoomReplyMode};

    #[test]
    fn archived_message_keeps_attachment_paths_in_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().to_string_lossy().into_owned();
        let room = Room {
            id: 7,
            name: "附件房间".to_string(),
            workspace_dir: workspace.clone(),
            workspace_dirs: vec![workspace],
            moderator_agent_id: None,
            member_ids: Vec::new(),
            reply_mode: RoomReplyMode::Mentions,
            discussion_max_rounds: 5,
            permission_mode: RoomPermissionMode::Auto,
            created_at: 1,
            last_activity_at: None,
        };
        let attachment = MessageAttachment {
            id: 2,
            name: "图.png".to_string(),
            mime_type: "image/png".to_string(),
            size: 42,
            created_at: 1,
            storage_path: "C:/ws/.moon/attachments/room-7/a.png".to_string(),
        };
        let message = Message {
            id: 11,
            room_id: 7,
            seq: 3,
            sender_type: MessageSenderType::User,
            sender_agent_id: None,
            status: MessageStatus::Final,
            kind: MessageKind::Reply,
            run_id: None,
            content: "长".repeat(ARCHIVE_MIN_CHARS),
            task_id: None,
            reply_to_message_id: None,
            discussion: None,
            collaboration: None,
            attachments: vec![attachment.clone()],
            reactions: Vec::new(),
            created_at: 1,
        };

        let line = compact_message_line(&room, &message, "我");
        assert!(line.contains("长回复已归档"), "{line}");
        assert!(line.contains("图.png"), "{line}");
        assert!(line.contains("image/png"), "{line}");
        assert!(line.contains("C:/ws/.moon/attachments/room-7/a.png"), "{line}");

        let instruction = instruction_with_attachments("看看附件", &[attachment]);
        assert!(instruction.starts_with("看看附件"), "{instruction}");
        assert!(instruction.contains("附件：图.png"), "{instruction}");
        assert!(
            instruction.contains("C:/ws/.moon/attachments/room-7/a.png"),
            "{instruction}"
        );
    }

    #[test]
    fn wraps_long_lines_without_losing_characters() {
        let text = "a".repeat(2500);
        let wrapped = wrap_long_lines(&text, 1000);
        assert_eq!(wrapped.replace('\n', ""), text);
        assert_eq!(wrapped.split('\n').count(), 3);
    }

    #[test]
    fn keeps_short_texts_intact() {
        assert_eq!(wrap_long_lines("abc", 10), "abc");
        assert_eq!(wrap_long_lines("abc", 0), "abc");
    }
}
