//! 确定性上下文压缩。
//!
//! 只做字符预算与裁剪，不调用模型：最近若干条保留全文，更早的消息压成
//! 一行摘要；预算仍不足时从最旧开始省略，并在转录前标注省略条数。
//! 长回复归档仍由调用方传入的 `full_line` 负责，本模块只决定保留哪一条。

use crate::domain::message::Message;
use crate::services::message_artifacts::attachment_prompt_lines;

/// 固定上限：最多考虑最近 20 条 reply 消息。
pub const CONTEXT_MAX_MESSAGES: usize = 20;
/// 最近 6 条保留全文。
pub const CONTEXT_FULL_RECENT: usize = 6;
/// 较早消息摘要的最大字符数。
pub const CONTEXT_EXCERPT_CHARS: usize = 160;
/// 默认预算，与设置默认值一致。
pub const DEFAULT_CONTEXT_BUDGET_CHARS: usize = 24_000;

/// 展示用 token 估算：固定按 4 字符 ≈ 1 token，向上取整。
pub fn estimate_tokens(chars: usize) -> usize {
    chars.div_ceil(4)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionOptions {
    pub enabled: bool,
    pub budget_chars: usize,
    pub max_messages: usize,
    pub full_recent: usize,
    pub excerpt_chars: usize,
}

impl CompactionOptions {
    pub fn new(enabled: bool, budget_chars: i64) -> Self {
        Self {
            enabled,
            budget_chars: usize::try_from(budget_chars).unwrap_or(0),
            ..Self::default()
        }
    }
}

impl Default for CompactionOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            budget_chars: DEFAULT_CONTEXT_BUDGET_CHARS,
            max_messages: CONTEXT_MAX_MESSAGES,
            full_recent: CONTEXT_FULL_RECENT,
            excerpt_chars: CONTEXT_EXCERPT_CHARS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionResult {
    pub transcript: String,
    pub included: usize,
    pub compressed: usize,
    pub omitted: usize,
    pub raw_chars: usize,
    pub effective_chars: usize,
}

/// 已解析出发送者显示名的消息；压缩算法不依赖 store 或 agent 查询。
pub struct LabeledMessage<'a> {
    pub label: &'a str,
    pub message: &'a Message,
}

struct Entry {
    line: String,
    compressed: bool,
}

/// 按预算构建转录；`full_line` 提供与真实提示词完全一致的单条全文行。
pub fn compact_transcript<F>(
    messages: &[LabeledMessage<'_>],
    full_line: F,
    options: &CompactionOptions,
) -> CompactionResult
where
    F: Fn(&Message, &str) -> String,
{
    let considered = if messages.len() > options.max_messages {
        &messages[messages.len() - options.max_messages..]
    } else {
        messages
    };
    let raw_chars = considered
        .iter()
        .map(|item| {
            item.message.content.chars().count()
                + attachment_prompt_lines(&item.message.attachments).chars().count()
        })
        .sum();

    let entries: Vec<Entry> = considered
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let from_end = considered.len() - index;
            let compressed = options.enabled && from_end > options.full_recent;
            let line = if compressed {
                excerpt_line(item.message, item.label, options.excerpt_chars)
            } else {
                full_line(item.message, item.label)
            };
            Entry { line, compressed }
        })
        .collect();

    if !options.enabled || entries.is_empty() {
        let transcript = join_entries(&entries, 0);
        let effective_chars = transcript.chars().count();
        return CompactionResult {
            transcript,
            included: entries.len(),
            compressed: 0,
            omitted: 0,
            raw_chars,
            effective_chars,
        };
    }

    let mut remaining = entries;
    let mut omitted = 0;
    // 至少保留最新一条；预算再小也不清空上下文。
    while remaining.len() > 1
        && transcript_chars(&remaining, omitted) > options.budget_chars
    {
        remaining.remove(0);
        omitted += 1;
    }

    let transcript = join_entries(&remaining, omitted);
    let effective_chars = transcript.chars().count();
    CompactionResult {
        transcript,
        included: remaining.len(),
        compressed: remaining.iter().filter(|entry| entry.compressed).count(),
        omitted,
        raw_chars,
        effective_chars,
    }
}

fn transcript_chars(entries: &[Entry], omitted: usize) -> usize {
    join_entries(entries, omitted).chars().count()
}

fn join_entries(entries: &[Entry], omitted: usize) -> String {
    let mut lines: Vec<&str> = Vec::with_capacity(entries.len() + 1);
    let marker;
    if omitted > 0 {
        marker = format!("（更早 {omitted} 条已省略）");
        lines.push(&marker);
    }
    lines.extend(entries.iter().map(|entry| entry.line.as_str()));
    if lines.is_empty() {
        "（暂无历史消息）".to_string()
    } else {
        lines.join("\n")
    }
}

fn excerpt_line(message: &Message, label: &str, limit: usize) -> String {
    let total = message.content.chars().count();
    let body: String = message.content.chars().take(limit).collect();
    let ellipsis = if total > limit { "…" } else { "" };
    format!(
        "#{} {}：{}{}{}",
        message.seq,
        label,
        body,
        ellipsis,
        attachment_prompt_lines(&message.attachments)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::attachment::MessageAttachment;
    use crate::domain::message::{MessageKind, MessageSenderType, MessageStatus};

    fn message(id: i64, seq: i64, content: &str) -> Message {
        Message {
            id,
            room_id: 1,
            seq,
            sender_type: MessageSenderType::User,
            sender_agent_id: None,
            status: MessageStatus::Final,
            kind: MessageKind::Reply,
            run_id: None,
            content: content.to_string(),
            task_id: None,
            reply_to_message_id: None,
            discussion: None,
            collaboration: None,
            attachments: Vec::new(),
            reactions: Vec::new(),
            created_at: 1,
        }
    }

    fn labeled<'a>(label: &'a str, messages: &'a [Message]) -> Vec<LabeledMessage<'a>> {
        messages
            .iter()
            .map(|message| LabeledMessage { label, message })
            .collect()
    }

    fn full_line(message: &Message, label: &str) -> String {
        format!("#{} {}：{}", message.seq, label, message.content)
    }

    #[test]
    fn keeps_recent_full_and_compresses_older_without_reordering() {
        let messages: Vec<Message> = (0..10)
            .map(|index| message(index, index + 1, &format!("消息{index}")))
            .collect();
        let options = CompactionOptions {
            full_recent: 3,
            ..CompactionOptions::default()
        };

        let result = compact_transcript(&labeled("用户", &messages), full_line, &options);

        let lines: Vec<&str> = result.transcript.lines().collect();
        assert_eq!(lines.len(), 10);
        assert_eq!(result.included, 10);
        assert_eq!(result.compressed, 7);
        assert_eq!(result.omitted, 0);
        assert_eq!(lines[0], "#1 用户：消息0");
        assert_eq!(lines[6], "#7 用户：消息6");
        assert_eq!(lines[7], "#8 用户：消息7");
        assert_eq!(lines[9], "#10 用户：消息9");
    }

    #[test]
    fn omits_oldest_until_budget_and_keeps_newest() {
        let messages: Vec<Message> = (0..10)
            .map(|index| message(index, index + 1, &format!("第{index}条消息内容")))
            .collect();
        let options = CompactionOptions {
            full_recent: 3,
            budget_chars: 40,
            ..CompactionOptions::default()
        };

        let result = compact_transcript(&labeled("用户", &messages), full_line, &options);

        assert!(result.omitted > 0, "{result:?}");
        assert!(result.transcript.contains("更早"));
        assert!(result.transcript.contains("第9条消息内容"));
        assert!(!result.transcript.contains("第0条消息内容"));
        assert_eq!(result.included + result.omitted, 10);
        assert_eq!(result.compressed, 0);
    }

    #[test]
    fn excerpt_keeps_sender_attachment_path_and_limit() {
        let mut with_attachment = message(1, 1, &"长".repeat(200));
        with_attachment.attachments.push(MessageAttachment {
            id: 9,
            name: "图.png".to_string(),
            mime_type: "image/png".to_string(),
            size: 42,
            created_at: 1,
            storage_path: "C:/ws/.moon/attachments/room-1/a.png".to_string(),
        });
        let messages = vec![with_attachment, message(2, 2, "最新")];
        let options = CompactionOptions {
            full_recent: 1,
            ..CompactionOptions::default()
        };

        let result = compact_transcript(&labeled("用户", &messages), full_line, &options);

        let first = result.transcript.lines().next().unwrap();
        assert!(first.starts_with("#1 用户："));
        assert_eq!(
            first.chars().count(),
            "#1 用户：".chars().count() + 160 + 1
        );
        assert!(result.transcript.contains("图.png"));
        assert!(result
            .transcript
            .contains("C:/ws/.moon/attachments/room-1/a.png"));
    }

    #[test]
    fn disabled_keeps_full_lines_and_existing_limit() {
        let messages: Vec<Message> = (0..25)
            .map(|index| message(index, index + 1, &format!("消息{index}")))
            .collect();
        let options = CompactionOptions {
            enabled: false,
            ..CompactionOptions::default()
        };

        let result = compact_transcript(&labeled("用户", &messages), full_line, &options);

        let lines: Vec<&str> = result.transcript.lines().collect();
        assert_eq!(lines.len(), 20);
        assert_eq!(lines[0], "#6 用户：消息5");
        assert_eq!(lines[19], "#25 用户：消息24");
        assert_eq!(result.compressed, 0);
        assert_eq!(result.omitted, 0);
    }

    #[test]
    fn empty_history_uses_placeholder() {
        let options = CompactionOptions::default();
        let result = compact_transcript(&[], full_line, &options);
        assert_eq!(result.transcript, "（暂无历史消息）");
        assert_eq!(result.included, 0);
        assert_eq!(result.effective_chars, "（暂无历史消息）".chars().count());
    }
}
