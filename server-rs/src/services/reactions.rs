//! 消息表情反应：校验房间归属与固定调色板，切换后广播最新反应。

use serde_json::Value;

use crate::domain::event::RoomEvent;
use crate::domain::message::{MessageReaction, LOCAL_REACTION_ACTOR, REACTION_EMOJIS};
use crate::error::{AppError, AppResult};
use crate::services::chat::BroadcastFn;
use crate::services::parse_id_label;
use crate::store::{MessageStore, RoomStore};

pub struct ReactionService {
    messages: MessageStore,
    rooms: RoomStore,
    broadcast: BroadcastFn,
}

impl ReactionService {
    pub fn new(messages: MessageStore, rooms: RoomStore, broadcast: BroadcastFn) -> Self {
        Self {
            messages,
            rooms,
            broadcast,
        }
    }

    /// 切换一条消息上的一个 emoji 反应，并返回该消息的最新反应列表。
    pub async fn toggle(
        &self,
        raw_room_id: &str,
        raw_message_id: &str,
        body: &Value,
    ) -> AppResult<Vec<MessageReaction>> {
        let room_id = parse_id_label(raw_room_id, "房间")?;
        let message_id = parse_id_label(raw_message_id, "消息")?;
        if self.rooms.get_by_id(room_id).await?.is_none() {
            return Err(AppError::not_found("房间不存在"));
        }
        let message = self
            .messages
            .get_by_id(message_id)
            .await?
            .filter(|message| message.room_id == room_id)
            .ok_or_else(|| AppError::not_found("消息不存在"))?;

        let emoji = body.get("emoji").and_then(Value::as_str).unwrap_or("");
        if !REACTION_EMOJIS.contains(&emoji) {
            return Err(AppError::validation("不支持的表情反应"));
        }

        let reactions = self
            .messages
            .toggle_reaction(message.id, emoji, LOCAL_REACTION_ACTOR)
            .await?;
        let event = RoomEvent::MessageReactionsChanged {
            room_id,
            message_id: message.id,
            reactions: reactions.clone(),
        };
        (self.broadcast)(room_id, &event);
        Ok(reactions)
    }
}
