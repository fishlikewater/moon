//! WebSocket 实时流。
//!
//! 客户端通过 `/ws?roomId=<id>` 订阅单个房间；`broadcast` 只把事件发给
//! 订阅该房间的连接，其他房间的连接收不到任何内容。
//!
//! 每个房间的事件帧都带 `epoch`（进程标识）与房间内单调递增的 `seq`。
//! 客户端重连时带回 `&afterEpoch=<epoch>&afterSeq=<已收到的最大 seq>`：
//! 同一进程内按窗口补发；epoch 不同（进程重启）或水位早于窗口时下发
//! `resync_required`，由客户端重拉权威快照。

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use crate::domain::event::{RoomEvent, RoomEventFrame};
use crate::AppState;

/// WebSocket 升级路径；Vite 代理与前端 hook 共用该常量对应值。
pub const WS_PATH: &str = "/ws";

const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

/// 单房间回放窗口的事件条数上限；约一分钟的流式增量。
const MAX_REPLAY_EVENTS: usize = 1024;

/// 单房间回放窗口的字节上限；超长工具输出不会挤占其它房间。
const MAX_REPLAY_BYTES: usize = 2 * 1024 * 1024;

/// 进程级 epoch：用于区分"同一房间的 `seq` 来自哪个服务端进程"。
///
/// 只承担可比性，不做安全用途。取值必须落在 JS 安全整数（2^53）以内，
/// 否则前端无法精确回传；微秒时间戳约 1.7e15，安全且足以区分重启。
fn process_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_micros() as u64)
        .unwrap_or(0)
}

/// 单房间事件流：序号水位与有界回放缓冲。
///
/// 缓冲必须在房间没有订阅者时也保留，否则断线期间的事件无法补齐；
/// 因此条目不会随订阅消失，只在超出窗口时丢弃最旧的事件。
#[derive(Default)]
struct RoomStream {
    seq: u64,
    buffered_bytes: usize,
    buffer: VecDeque<(u64, String)>,
}

#[derive(Default)]
struct HubState {
    next_id: u64,
    rooms: HashMap<i64, Vec<(u64, UnboundedSender<String>)>>,
    streams: HashMap<i64, RoomStream>,
}

/// 房间订阅中心；可在多个请求间共享。
#[derive(Clone)]
pub struct RoomSocketHub {
    inner: Arc<Mutex<HubState>>,
    /// 进程级标识：`seq` 只在同一 epoch 内可比。
    epoch: u64,
}

impl Default for RoomSocketHub {
    fn default() -> Self {
        Self {
            inner: Arc::default(),
            epoch: process_epoch(),
        }
    }
}

impl RoomSocketHub {
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前进程的 epoch；客户端用它判断水位是否来自同一个服务端进程。
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// 注册订阅并返回需要先发送的补发帧。
    ///
    /// 注册与补发计算在同一把锁内完成：锁释放后的广播只会进入 channel，
    /// 而补发帧在进入会话循环前先写 socket，因此顺序不会倒置。
    fn subscribe(
        &self,
        room_id: i64,
        watermark: Option<(u64, u64)>,
    ) -> (u64, UnboundedReceiver<String>, Vec<String>) {
        let (sender, receiver) = unbounded_channel();
        let mut state = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        state.next_id += 1;
        let id = state.next_id;
        state.rooms.entry(room_id).or_default().push((id, sender));

        let immediate = {
            let stream = state.streams.entry(room_id).or_default();
            match watermark {
                // 首次连接由客户端自行拉取快照，不需要补发历史。
                None => Vec::new(),
                // 水位来自上一个服务端进程：内存缓冲与序号都不可比。
                Some((epoch, _)) if epoch != self.epoch => vec![resync_frame(room_id)],
                // 水位与当前一致：没有遗漏。
                Some((_, after)) if after == stream.seq => Vec::new(),
                // 同进程却超前于服务端序号：状态不一致，按重同步处理。
                Some((_, after)) if after > stream.seq => vec![resync_frame(room_id)],
                Some((_, after)) => match stream.buffer.front() {
                    // 窗口覆盖 (after, seq] 时逐帧补发。
                    Some((oldest, _)) if after + 1 >= *oldest => stream
                        .buffer
                        .iter()
                        .filter(|(seq, _)| *seq > after)
                        .map(|(_, payload)| payload.clone())
                        .collect(),
                    // 水位已早于窗口，只能要求客户端重新同步。
                    _ => vec![resync_frame(room_id)],
                },
            }
        };
        (id, receiver, immediate)
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

    /// 把事件广播给订阅指定房间的连接；无订阅者时仍然写入回放缓冲。
    pub fn broadcast(&self, room_id: i64, event: &RoomEvent) {
        let mut state = self.inner.lock().unwrap_or_else(|error| error.into_inner());

        let payload = {
            let stream = state.streams.entry(room_id).or_default();
            let seq = stream.seq + 1;
            let Ok(payload) = serde_json::to_string(&RoomEventFrame {
                epoch: self.epoch,
                seq,
                event,
            }) else {
                // 序列化失败不占用序号，避免把空洞写进回放窗口。
                return;
            };
            stream.seq = seq;
            stream.buffered_bytes += payload.len();
            stream.buffer.push_back((seq, payload.clone()));
            while stream.buffer.len() > MAX_REPLAY_EVENTS
                || stream.buffered_bytes > MAX_REPLAY_BYTES
            {
                let Some((_, dropped)) = stream.buffer.pop_front() else {
                    break;
                };
                stream.buffered_bytes = stream.buffered_bytes.saturating_sub(dropped.len());
            }
            payload
        };

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

/// 从查询参数解析 `afterSeq`：缺失表示首次连接，不补发。
///
/// 出现但非法的值返回 Err，由调用方按 1008 拒绝订阅，避免把客户端 bug
/// 静默当成首次连接处理。
pub fn parse_after_seq(raw: Option<&String>) -> Result<Option<u64>, ()> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(());
    }
    raw.parse::<u64>()
        .ok()
        .filter(|value| *value <= MAX_SAFE_INTEGER as u64)
        .map(Some)
        .ok_or(())
}

/// 从查询参数解析 `afterEpoch`；规则与 `afterSeq` 一致。
pub fn parse_after_epoch(raw: Option<&String>) -> Result<Option<u64>, ()> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(());
    }
    raw.parse::<u64>()
        .ok()
        .filter(|value| *value <= MAX_SAFE_INTEGER as u64)
        .map(Some)
        .ok_or(())
}

/// 客户端水位（epoch + seq）；两者必须同时出现，缺一视为非法。
pub fn parse_watermark(
    raw_epoch: Option<&String>,
    raw_seq: Option<&String>,
) -> Result<Option<(u64, u64)>, ()> {
    match (parse_after_epoch(raw_epoch)?, parse_after_seq(raw_seq)?) {
        (None, None) => Ok(None),
        (Some(epoch), Some(seq)) => Ok(Some((epoch, seq))),
        _ => Err(()),
    }
}

/// 客户端水位早于回放窗口时的重同步信号；不属于领域事件，因此不带 `seq`。
fn resync_frame(room_id: i64) -> String {
    serde_json::json!({ "type": "resync_required", "roomId": room_id }).to_string()
}

/// 1008 = policy violation：参数不合法就不接受订阅。
async fn close_with_reason(socket: &mut WebSocket, reason: &str) {
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code: 1008,
            reason: reason.into(),
        })))
        .await;
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
    let watermark = parse_watermark(query.get("afterEpoch"), query.get("afterSeq"));
    upgrade.on_upgrade(move |socket| handle_socket(socket, room_id, watermark, state.ws_hub.clone()))
}

async fn handle_socket(
    mut socket: WebSocket,
    room_id: Option<i64>,
    watermark: Result<Option<(u64, u64)>, ()>,
    hub: RoomSocketHub,
) {
    let Some(room_id) = room_id else {
        close_with_reason(&mut socket, "缺少或非法的 roomId").await;
        return;
    };
    let Ok(watermark) = watermark else {
        close_with_reason(&mut socket, "缺少或非法的 afterSeq 水位").await;
        return;
    };

    let (subscriber_id, mut receiver, immediate) = hub.subscribe(room_id, watermark);
    for frame in immediate {
        if socket.send(Message::Text(frame.into())).await.is_err() {
            hub.unsubscribe(room_id, subscriber_id);
            return;
        }
    }

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
    use serde_json::Value;

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

    fn parse(payload: &str) -> Value {
        serde_json::from_str(payload).expect("frame json")
    }

    #[test]
    fn parses_room_id_strictly() {
        assert_eq!(parse_room_id(Some(&"12".to_string())), Some(12));
        assert_eq!(parse_room_id(Some(&"0".to_string())), None);
        assert_eq!(parse_room_id(Some(&"abc".to_string())), None);
        assert_eq!(parse_room_id(None), None);
    }

    #[test]
    fn parses_after_seq_strictly() {
        assert_eq!(parse_after_seq(None), Ok(None));
        assert_eq!(parse_after_seq(Some(&"0".to_string())), Ok(Some(0)));
        assert_eq!(parse_after_seq(Some(&"42".to_string())), Ok(Some(42)));
        assert_eq!(parse_after_seq(Some(&"".to_string())), Err(()));
        assert_eq!(parse_after_seq(Some(&"-1".to_string())), Err(()));
        assert_eq!(parse_after_seq(Some(&"1.5".to_string())), Err(()));
        assert_eq!(parse_after_seq(Some(&"abc".to_string())), Err(()));
        assert_eq!(
            parse_after_seq(Some(&(MAX_SAFE_INTEGER as u64 + 1).to_string())),
            Err(())
        );
    }

    #[test]
    fn broadcasts_only_to_subscribed_room() {
        let hub = RoomSocketHub::new();
        let (_, mut first, _) = hub.subscribe(1, None);
        let (_, mut second, _) = hub.subscribe(2, None);
        assert_eq!(hub.subscriber_count(1), 1);

        hub.broadcast(1, &sample_event());
        let payload = first.blocking_recv().expect("event");
        assert_eq!(parse(&payload)["type"], "message_final");
        assert!(second.try_recv().is_err());
    }

    #[test]
    fn assigns_monotonic_seq_per_room() {
        let hub = RoomSocketHub::new();
        let (_, mut receiver, _) = hub.subscribe(7, None);

        hub.broadcast(7, &sample_event());
        hub.broadcast(7, &sample_event());

        assert_eq!(parse(&receiver.blocking_recv().unwrap())["seq"], 1);
        assert_eq!(parse(&receiver.blocking_recv().unwrap())["seq"], 2);
    }

    #[test]
    fn buffers_events_without_subscribers() {
        let hub = RoomSocketHub::new();
        hub.broadcast(9, &sample_event());
        hub.broadcast(9, &sample_event());

        let (_, _, immediate) = hub.subscribe(9, Some((hub.epoch(), 1)));
        assert_eq!(immediate.len(), 1);
        assert_eq!(parse(&immediate[0])["seq"], 2);
    }

    #[test]
    fn skips_replay_without_watermark_or_when_current() {
        let hub = RoomSocketHub::new();
        hub.broadcast(9, &sample_event());

        assert!(hub.subscribe(9, None).2.is_empty());
        assert!(hub.subscribe(9, Some((hub.epoch(), 1))).2.is_empty());
    }

    #[test]
    fn requests_resync_when_watermark_precedes_window() {
        let hub = RoomSocketHub::new();
        for _ in 0..MAX_REPLAY_EVENTS + 3 {
            hub.broadcast(9, &sample_event());
        }

        let (_, _, immediate) = hub.subscribe(9, Some((hub.epoch(), 1)));
        assert_eq!(immediate.len(), 1);
        let frame = parse(&immediate[0]);
        assert_eq!(frame["type"], "resync_required");
        assert_eq!(frame["roomId"], 9);
        assert!(frame.get("seq").is_none());
    }

    #[test]
    fn requests_resync_when_watermark_exceeds_server_seq() {
        // 进程重启：服务端序号从 0 重新开始，客户端水位更高。
        let hub = RoomSocketHub::new();
        hub.broadcast(9, &sample_event());

        let (_, _, immediate) = hub.subscribe(9, Some((hub.epoch(), 99)));
        assert_eq!(immediate.len(), 1);
        assert_eq!(parse(&immediate[0])["type"], "resync_required");
    }

    #[test]
    fn requests_resync_when_epoch_differs_even_if_seq_matches() {
        let hub = RoomSocketHub::new();
        hub.broadcast(9, &sample_event());

        // 序号一致但 epoch 不同：水位来自上一个服务端进程，不能当作已同步。
        let (_, _, immediate) = hub.subscribe(9, Some((hub.epoch() + 1, 1)));
        assert_eq!(immediate.len(), 1);
        assert_eq!(parse(&immediate[0])["type"], "resync_required");
    }

    #[test]
    fn frames_carry_process_epoch() {
        let hub = RoomSocketHub::new();
        let (_, mut receiver, _) = hub.subscribe(7, None);
        hub.broadcast(7, &sample_event());

        let frame = parse(&receiver.blocking_recv().unwrap());
        assert_eq!(frame["epoch"], hub.epoch());
        assert_eq!(frame["seq"], 1);
    }

    #[test]
    fn parses_watermark_pair_strictly() {
        let epoch = Some("7".to_string());
        let seq = Some("9".to_string());
        assert_eq!(parse_watermark(None, None), Ok(None));
        assert_eq!(parse_watermark(epoch.as_ref(), seq.as_ref()), Ok(Some((7, 9))));
        assert_eq!(parse_watermark(epoch.as_ref(), None), Err(()));
        assert_eq!(parse_watermark(None, seq.as_ref()), Err(()));
    }

    #[test]
    fn drops_oldest_events_beyond_window() {
        let hub = RoomSocketHub::new();
        for _ in 0..MAX_REPLAY_EVENTS + 5 {
            hub.broadcast(9, &sample_event());
        }

        let (_, _, immediate) = hub.subscribe(9, Some((hub.epoch(), 5)));
        assert_eq!(immediate.len(), MAX_REPLAY_EVENTS);
        assert_eq!(parse(&immediate[0])["seq"], 6);
    }
}
