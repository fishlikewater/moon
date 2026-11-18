//! 房间实时事件（WebSocket `/ws?roomId=<id>` 推送）。
//!
//! 服务端只向订阅了该房间的连接广播；所有事件都带 `roomId`。

use serde::Serialize;

use crate::domain::collaboration::CollaborationBatch;
use crate::domain::discussion::Discussion;
use crate::domain::message::{Message, MessageReaction};
use crate::domain::run::RunActivity;
use crate::domain::task::Task;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum RoomEvent {
    RunStarted {
        room_id: i64,
        run_id: i64,
        agent_id: i64,
        message_id: i64,
        /// 已落库的 streaming 占位消息。
        message: Message,
    },
    /// 同一运行进入新阶段：旧阶段已落定为 progress。
    MessageStarted {
        room_id: i64,
        run_id: i64,
        message: Message,
    },
    MessageDelta {
        room_id: i64,
        message_id: i64,
        text: String,
    },
    MessageFinal {
        room_id: i64,
        message: Message,
    },
    /// 某条消息的表情反应已切换；客户端只替换该消息的 reactions。
    MessageReactionsChanged {
        room_id: i64,
        message_id: i64,
        reactions: Vec<MessageReaction>,
    },
    /// 房间状态快照已提交；客户端按 `state.unit` 分派并整份替换本地快照。
    ///
    /// 快照一律携带完整领域对象，不使用半量 delta：断线补齐与重同步后
    /// 客户端都能靠单条事件自愈。
    StateChanged {
        room_id: i64,
        state: RoomState,
    },
    RunActivity {
        room_id: i64,
        activity: RunActivity,
    },
    /// 纯聊天历史已清理；客户端应重新拉取消息列表。
    HistoryCleared {
        room_id: i64,
    },
}

/// 房间状态快照单元：`unit` 决定领域对象类型，客户端据此分派。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "unit", content = "snapshot", rename_all = "snake_case")]
pub enum RoomState {
    Task(Task),
    Discussion(Discussion),
    Collaboration(CollaborationBatch),
}

/// 房间事件帧：`epoch`/`seq` 与领域事件字段平铺在同一个 JSON 对象里。
///
/// - `epoch` 标识产生该序号的进程，进程重启后取值不同；
/// - `seq` 由 `RoomSocketHub` 按房间统一分配，领域事件本身不感知回放语义。
///
/// 客户端重连时带回这两个值，服务端据此判断补发范围或要求重新同步。
#[derive(Debug, Clone, Serialize)]
pub struct RoomEventFrame<'a> {
    pub epoch: u64,
    pub seq: u64,
    #[serde(flatten)]
    pub event: &'a RoomEvent,
}
