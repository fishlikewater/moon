//! 房间历史清理。
//!
//! 职责边界：
//! - 只删除「纯聊天」消息：不关联任务、讨论或委派批次；
//! - 运行中（队列忙、运行中讨论/委派/运行）拒绝删除，避免破坏回放；
//! - 归档文件按消息 id 同步清理，失败只影响计数，不影响删除结果。

use std::sync::Arc;

use serde_json::Value;

use crate::domain::event::RoomEvent;
use crate::domain::message::{Message, MessageCleanupResult};
use crate::domain::run::RunStatus;
use crate::error::{AppError, AppResult};
use crate::services::attachments::AttachmentService;
use crate::services::chat::BroadcastFn;
use crate::services::message_artifacts::archive_path;
use crate::services::MAX_SAFE_INTEGER;
use crate::store::{MessageStore, RoomStore, RunStore};

/// `keepRecent` 的允许上限；固定值，不做可配置化。
pub const MAX_KEEP_RECENT: i64 = 5_000;

pub type RoomBusyFn = Arc<dyn Fn(i64) -> bool + Send + Sync>;

pub struct RoomHistoryOptions {
    pub rooms: RoomStore,
    pub messages: MessageStore,
    pub runs: RunStore,
    /// 房间队列是否忙碌；由 ChatService 暴露，确保与聊天运行共用同一个队列。
    pub is_room_busy: RoomBusyFn,
    pub broadcast: BroadcastFn,
    pub attachments: Arc<AttachmentService>,
}

pub struct RoomHistoryService {
    rooms: RoomStore,
    messages: MessageStore,
    runs: RunStore,
    is_room_busy: RoomBusyFn,
    broadcast: BroadcastFn,
    attachments: Arc<AttachmentService>,
}

impl RoomHistoryService {
    pub fn new(options: RoomHistoryOptions) -> Self {
        Self {
            rooms: options.rooms,
            messages: options.messages,
            runs: options.runs,
            is_room_busy: options.is_room_busy,
            broadcast: options.broadcast,
            attachments: options.attachments,
        }
    }

    /// 清理房间聊天窗口历史。
    ///
    /// `keepRecent = 0` 表示隐藏当前全部可见历史；否则保留最近 N 条纯聊天可见。
    /// 纯聊天消息物理删除；任务、讨论与委派消息保留在库中，但旧消息不再出现在聊天窗口。
    pub async fn cleanup(
        &self,
        raw_room_id: &str,
        body: &Value,
    ) -> AppResult<MessageCleanupResult> {
        let room_id = parse_room_id(raw_room_id)?;
        let keep_recent = parse_keep_recent(body.get("keepRecent"))?;
        let Some(room) = self.rooms.get_by_id(room_id).await? else {
            return Err(AppError::not_found(format!("房间不存在：{room_id}")));
        };

        self.assert_idle(room_id).await?;

        let current_floor = self.rooms.history_start_message_id(room_id).await?;
        let history = self.messages.list_all_by_room(room_id).await?;
        // 被委派/讨论外键引用的消息即使没有元数据也不能删。
        let protected_ids = self.messages.protected_message_ids(room_id).await?;
        let eligible: Vec<&Message> = history
            .iter()
            .filter(|message| {
                message.id > current_floor
                    && is_pure_chat_message(message)
                    && !protected_ids.contains(&message.id)
            })
            .collect();
        let keep = usize::try_from(keep_recent)
            .unwrap_or(usize::MAX)
            .min(eligible.len());
        // 窗口从保留消息之前开始：keep=0 时清空当前全部可见历史。
        let floor = if keep == 0 {
            history
                .iter()
                .map(|message| message.id)
                .max()
                .unwrap_or(current_floor)
        } else {
            eligible[eligible.len() - keep].id - 1
        }
        .max(current_floor);
        if floor <= current_floor {
            return Ok(MessageCleanupResult {
                deleted: 0,
                hidden: 0,
                artifacts_deleted: 0,
                attachments_deleted: 0,
            });
        }

        let deletable: Vec<&Message> = history
            .iter()
            .filter(|message| {
                message.id > current_floor
                    && message.id <= floor
                    && is_pure_chat_message(message)
                    && !protected_ids.contains(&message.id)
            })
            .collect();
        let hidden = history
            .iter()
            .filter(|message| message.id > current_floor && message.id <= floor)
            .count()
            .saturating_sub(deletable.len()) as u64;

        let mut deleted = 0;
        let mut artifacts_deleted = 0;
        let mut attachments_deleted = 0;
        if !deletable.is_empty() {
            let ids: Vec<i64> = deletable.iter().map(|message| message.id).collect();
            let attachments = self.attachments.list_for_messages(&ids).await?;
            deleted = self.messages.delete_by_ids(&ids).await?;
            self.attachments.delete_rows_for_messages(&ids).await?;
            attachments_deleted = self.attachments.remove_files(&attachments);
            artifacts_deleted = self.remove_artifacts(&room.workspace_dir, room_id, &deletable);
        }

        // 即使没有可物理删除的消息，也要推进水位隐藏受保护的旧消息。
        self.rooms
            .advance_history_start_message_id(room_id, floor)
            .await?;
        (self.broadcast)(room_id, &RoomEvent::HistoryCleared { room_id });
        Ok(MessageCleanupResult {
            deleted,
            hidden,
            artifacts_deleted,
            attachments_deleted,
        })
    }

    /// 运行中或等待中的房间不允许清理，避免删除正在流式写入的消息。
    async fn assert_idle(&self, room_id: i64) -> AppResult<()> {
        let running = self
            .runs
            .list_by_status(RunStatus::Running)
            .await?
            .iter()
            .any(|run| run.room_id == room_id);
        let busy = (self.is_room_busy)(room_id)
            || running
            || self.rooms.has_running_discussion(room_id).await?
            || self.rooms.has_running_collaboration(room_id).await?;
        if busy {
            return Err(AppError::constraint(
                "房间有运行中的任务、讨论或委派，请先停止后再清理历史",
            ));
        }
        Ok(())
    }

    fn remove_artifacts(&self, workspace_dir: &str, room_id: i64, messages: &[&Message]) -> u64 {
        let mut removed = 0;
        for message in messages {
            let file_path = archive_path(workspace_dir, room_id, message.id);
            if !file_path.exists() {
                continue;
            }
            if std::fs::remove_file(&file_path).is_ok() {
                removed += 1;
            }
        }
        removed
    }
}

/// 纯聊天消息：没有任务、讨论或委派关联。
pub fn is_pure_chat_message(message: &Message) -> bool {
    message.task_id.is_none() && message.discussion.is_none() && message.collaboration.is_none()
}

/// 路径参数里的房间 id：接受数字字符串，拒绝空值与非正安全整数。
fn parse_room_id(raw: &str) -> AppResult<i64> {
    let trimmed = raw.trim();
    let parsed = if trimmed.is_empty() {
        None
    } else {
        trimmed
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite() && value.fract() == 0.0)
            .filter(|value| *value > 0.0 && *value <= MAX_SAFE_INTEGER as f64)
            .map(|value| value as i64)
    };
    parsed.ok_or_else(|| AppError::validation(format!("房间 id 不合法：{raw}")))
}

fn parse_keep_recent(value: Option<&Value>) -> AppResult<i64> {
    let Some(value) = value else {
        return Ok(0);
    };
    let number = match value {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| {
                number
                    .as_f64()
                    .filter(|value| value.fract() == 0.0 && *value >= i64::MIN as f64)
                    .map(|value| value as i64)
            }),
        _ => None,
    };
    let Some(number) = number else {
        return Err(AppError::validation(
            "keepRecent 必须是 0 到 5000 之间的整数",
        ));
    };
    if !(0..=MAX_KEEP_RECENT).contains(&number) {
        return Err(AppError::validation(format!(
            "keepRecent 必须在 0 到 {MAX_KEEP_RECENT} 之间"
        )));
    }
    Ok(number)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keep_recent_defaults_and_rejects_out_of_range() {
        assert_eq!(parse_keep_recent(None).unwrap(), 0);
        assert_eq!(parse_keep_recent(Some(&serde_json::json!(7))).unwrap(), 7);
        assert!(parse_keep_recent(Some(&serde_json::json!("7"))).is_err());
        assert!(parse_keep_recent(Some(&serde_json::json!(-1))).is_err());
        assert!(parse_keep_recent(Some(&serde_json::json!(5001))).is_err());
    }
}
