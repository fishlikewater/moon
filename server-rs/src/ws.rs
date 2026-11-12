//! WebSocket 实时流。
//!
//! 客户端通过 `/ws?roomId=<id>` 订阅单个房间；`broadcast` 只把事件发给
//! 订阅该房间的连接，其他房间的连接收不到任何内容。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use crate::domain::event::RoomEvent;
use crate::AppState;

/// WebSocket 升级路径；Vite 代理与前端 hook 共用该常量对应值。
pub const WS_PATH: &str = "/ws";

const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

#[derive(Default)]
struct HubState {
    next_id: u64,
    rooms: HashMap<i64, Vec<(u64, UnboundedSender<String>)>>,
}

/// 房间订阅中心；可在多个请求间共享。
#[derive(Clone, Default)]
pub struct RoomSocketHub {
    inner: Arc<Mutex<HubState>>,
}

impl RoomSocketHub {
    pub fn new() -> Self {
        Self::default()
    }

    fn subscribe(&self, room_id: i64) -> (u64, UnboundedReceiver<String>) {
        let (sender, receiver) = unbounded_channel();
        let mut state = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        state.next_id += 1;
        let id = state.next_id;
        state.rooms.entry(room_id).or_default().push((id, sender));
        (id, receiver)
    }

    fn unsubscribe(&self, room_id: i64, subscriber_id: u64) {
        let mut state = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(list) = state.rooms.get_mut(&room_id) {
            list.retain(|(id, _)| *id != subscriber_id);
            if list.is_empty() {
                state.rooms.remove(&room_id);
            }
        }
    }

    /// 把事件广播给订阅指定房间的连接；无订阅者时静默忽略。
    pub fn broadcast(&self, room_id: i64, event: &RoomEvent) {
        let Ok(payload) = serde_json::to_string(event) else {
            return;
        };
        let mut state = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        let Some(list) = state.rooms.get_mut(&room_id) else {
            return;
        };
        list.retain(|(_, sender)| sender.send(payload.clone()).is_ok());
        if list.is_empty() {
            state.rooms.remove(&room_id);
        }
    }

    /// 订阅该房间的活跃连接数（测试与诊断用）。
    pub fn subscriber_count(&self, room_id: i64) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .rooms
            .get(&room_id)
            .map(Vec::len)
            .unwrap_or(0)
    }
}

/// 从查询参数解析 `roomId`；缺失或非法返回 None。
pub fn parse_room_id(raw: Option<&String>) -> Option<i64> {
    let raw = raw?;
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    raw.parse::<i64>()
        .ok()
        .filter(|id| *id > 0 && *id <= MAX_SAFE_INTEGER)
}

pub fn router() -> Router<AppState> {
    Router::new().route(WS_PATH, get(ws_handler))
}

async fn ws_handler(
    upgrade: WebSocketUpgrade,
    Query(query): Query<HashMap<String, String>>,
    State(state): State<AppState>,
) -> Response {
    let room_id = parse_room_id(query.get("roomId"));
    upgrade.on_upgrade(move |socket| handle_socket(socket, room_id, state.ws_hub.clone()))
}

async fn handle_socket(mut socket: WebSocket, room_id: Option<i64>, hub: RoomSocketHub) {
    let Some(room_id) = room_id else {
        // 1008 = policy violation：没有房间上下文就不接受订阅。
        let _ = socket
            .send(Message::Close(Some(CloseFrame {
                code: 1008,
                reason: "缺少或非法的 roomId".into(),
            })))
            .await;
        return;
    };

    let (subscriber_id, mut receiver) = hub.subscribe(room_id);
    loop {
        tokio::select! {
            outgoing = receiver.recv() => {
                match outgoing {
                    Some(text) => {
                        if socket.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    None => break,
                    Some(Err(_)) => break,
                    Some(Ok(Message::Close(_))) => break,
                    Some(Ok(_)) => {}
                }
            }
        }
    }
    hub.unsubscribe(room_id, subscriber_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::message::{Message, MessageKind, MessageSenderType, MessageStatus};

    fn sample_event() -> RoomEvent {
        RoomEvent::MessageFinal {
            room_id: 3,
            message: Message {
                id: 1,
                room_id: 3,
                seq: 1,
                sender_type: MessageSenderType::System,
                sender_agent_id: None,
                status: MessageStatus::Final,
                kind: MessageKind::Reply,
                run_id: None,
                content: "hi".to_string(),
                task_id: None,
                reply_to_message_id: None,
                discussion: None,
                collaboration: None,
                attachments: Vec::new(),
                reactions: Vec::new(),
                created_at: 1,
            },
        }
    }

    #[test]
    fn parses_room_id_strictly() {
        assert_eq!(parse_room_id(Some(&"12".to_string())), Some(12));
        assert_eq!(parse_room_id(Some(&"0".to_string())), None);
        assert_eq!(parse_room_id(Some(&"abc".to_string())), None);
        assert_eq!(parse_room_id(None), None);
    }

    #[tokio::test]
    async fn broadcasts_only_to_subscribed_room() {
        let hub = RoomSocketHub::new();
        let (_, mut first) = hub.subscribe(1);
        let (_, mut second) = hub.subscribe(2);
        assert_eq!(hub.subscriber_count(1), 1);

        hub.broadcast(1, &sample_event());
        let payload = first.recv().await.expect("event");
        assert!(payload.contains("\"type\":\"message_final\""));
        assert!(second.try_recv().is_err());
    }
}
