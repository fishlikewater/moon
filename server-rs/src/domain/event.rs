//! 房间实时事件（WebSocket `/ws?roomId=<id>` 推送）。
//!
//! 服务端只向订阅了该房间的连接广播；所有事件都带 `roomId`。

use serde::Serialize;

use crate::domain::collaboration::CollaborationBatch;
use crate::domain::discussion::Discussion;
use crate::domain::message::{Message, MessageReaction};
use crate::domain::run::RunActivity;
use crate::domain::task::TaskStatus;

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
    /// 任务状态已在事务内提交；客户端据此实时更新看板。
    TaskChanged {
        room_id: i64,
        task_id: i64,
        status: TaskStatus,
    },
    /// 讨论状态或轮次已提交；客户端据此更新讨论状态条。
    DiscussionChanged {
        room_id: i64,
        discussion: Discussion,
    },
    /// 委派批次或指派状态已提交；客户端据此更新批次卡片。
    CollaborationChanged {
        room_id: i64,
        batch: CollaborationBatch,
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
