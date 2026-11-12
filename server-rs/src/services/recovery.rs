//! 启动恢复：把上次进程退出时残留的活跃痕迹一致化标记为终态。
//!
//! 不变量：
//! - 只做标记，不做猜测性续跑（不重建运行、不重新派发）；
//! - 全部写入在同一事务内完成，失败时整体回滚；
//! - 幂等：第二次执行时已无 `streaming` / `running` 残留，任务也已是
//!   `blocked`，因此不会产生新的状态变化、事件或系统消息。

use std::collections::HashSet;

use serde::Serialize;
use serde_json::json;

use crate::db::Db;
use crate::domain::event::RoomEvent;
use crate::domain::message::{
    CollaborationMessageRole, Message, MessageCollaboration, MessageDiscussion, MessageKind,
    MessageSenderType, MessageStatus,
};
use crate::domain::now_ms;
use crate::domain::task::TaskStatus;
use crate::error::AppResult;
use crate::services::chat::BroadcastFn;
use crate::store::{CollaborationStore, DiscussionStore};

/// 恢复时写入的系统消息正文；测试与前端据此识别恢复痕迹。
pub const RECOVERY_SYSTEM_MESSAGE: &str = "服务重启：编排中断，任务已标记为阻塞";
pub const RECOVERY_DISCUSSION_MESSAGE: &str = "服务重启：讨论中断，已标记为失败";
pub const RECOVERY_COLLABORATION_MESSAGE: &str = "服务重启：委派批次中断，已标记为失败";

/// 可观测计数，便于启动日志与幂等断言。
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryResult {
    pub streaming_messages: u64,
    pub running_runs: u64,
    pub blocked_tasks: u64,
    pub interrupted_discussions: u64,
    pub interrupted_collaborations: u64,
}

struct InterruptedDiscussion {
    discussion_id: i64,
    room_id: i64,
    message: Message,
}

struct InterruptedCollaboration {
    batch_id: i64,
    room_id: i64,
    message: Message,
}

struct BlockedTask {
    task_id: i64,
    room_id: i64,
    status: TaskStatus,
    message: Message,
}

/// 恢复规则：
/// - `streaming` 消息 → `failed`（保留已流出的内容）；
/// - `running` 运行 → `interrupted`（写 `finished_at`）；
/// - 恢复前存在未终结运行且任务状态为 `in_progress` → 转 `blocked`，
///   并追加一条关联该任务的系统消息。
pub async fn recover_interrupted(
    db: &Db,
    discussions_store: Option<&DiscussionStore>,
    collaborations_store: Option<&CollaborationStore>,
    broadcast: Option<&BroadcastFn>,
) -> AppResult<RecoveryResult> {
    let mut tx = db.pool().begin().await?;

    let runs = sqlx::query_as::<_, (i64, Option<i64>)>(
        "SELECT id, task_id FROM agent_runs WHERE status = 'running' ORDER BY id",
    )
    .fetch_all(&mut *tx)
    .await?;
    let mut task_ids = HashSet::new();
    for (_, task_id) in &runs {
        if let Some(task_id) = task_id {
            task_ids.insert(*task_id);
        }
    }

    let now = now_ms();
    let interrupted_runs = if runs.is_empty() {
        0
    } else {
        let result = sqlx::query(
            "UPDATE agent_runs SET status = 'interrupted', finished_at = ? WHERE status = 'running'",
        )
        .bind(now)
        .execute(&mut *tx)
        .await?;
        result.rows_affected()
    };

    let streaming_messages = sqlx::query("UPDATE messages SET status = 'failed' WHERE status = 'streaming'")
        .execute(&mut *tx)
        .await?
        .rows_affected();

    let mut blocked = Vec::new();
    for task_id in task_ids {
        let row = sqlx::query_as::<_, (i64, i64, String)>(
            "SELECT id, room_id, status FROM tasks WHERE id = ?",
        )
        .bind(task_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((task_id, room_id, status)) = row else {
            continue;
        };
        if status != TaskStatus::InProgress.as_str() {
            continue;
        }

        sqlx::query("UPDATE tasks SET status = 'blocked', updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(task_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO task_events (task_id, type, payload_json, created_at)
             VALUES (?, 'status_changed', ?, ?)",
        )
        .bind(task_id)
        .bind(serde_json::to_string(&json!({
            "from": TaskStatus::InProgress.as_str(),
            "to": TaskStatus::Blocked.as_str(),
        }))?)
        .bind(now)
        .execute(&mut *tx)
        .await?;

        let seq: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM messages WHERE room_id = ?",
        )
        .bind(room_id)
        .fetch_one(&mut *tx)
        .await?;
        let result = sqlx::query(
            "INSERT INTO messages (
               room_id, seq, sender_type, sender_agent_id, status, kind, run_id,
               content, task_id, created_at
             )
             VALUES (?, ?, 'system', NULL, 'final', 'reply', NULL, ?, ?, ?)",
        )
        .bind(room_id)
        .bind(seq)
        .bind(RECOVERY_SYSTEM_MESSAGE)
        .bind(task_id)
        .bind(now)
        .execute(&mut *tx)
        .await?;

        blocked.push(BlockedTask {
            task_id,
            room_id,
            status: TaskStatus::Blocked,
            message: Message {
                id: result.last_insert_rowid(),
                room_id,
                seq,
                sender_type: MessageSenderType::System,
                sender_agent_id: None,
                status: MessageStatus::Final,
                kind: MessageKind::Reply,
                run_id: None,
                content: RECOVERY_SYSTEM_MESSAGE.to_string(),
                task_id: Some(task_id),
                reply_to_message_id: None,
                discussion: None,
                collaboration: None,
                attachments: Vec::new(),
                reactions: Vec::new(),
                created_at: now,
            },
        });
    }

    let mut interrupted_collaborations = Vec::new();
    if collaborations_store.is_some() {
        let running = sqlx::query_as::<_, (i64, i64, i64)>(
            "SELECT id, room_id, current_round FROM collaboration_batches
              WHERE status = 'running' ORDER BY id",
        )
        .fetch_all(&mut *tx)
        .await?;
        if !running.is_empty() {
            sqlx::query(
                "UPDATE collaboration_batches
                    SET status = 'failed', stop_reason = 'interrupted', updated_at = ?
                  WHERE status = 'running'",
            )
            .bind(now)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE collaboration_assignments
                    SET status = 'failed', error = 'interrupted', finished_at = ?
                  WHERE status IN ('queued', 'running')",
            )
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        for (batch_id, room_id, current_round) in running {
            let seq: i64 = sqlx::query_scalar(
                "SELECT COALESCE(MAX(seq), 0) + 1 FROM messages WHERE room_id = ?",
            )
            .bind(room_id)
            .fetch_one(&mut *tx)
            .await?;
            let result = sqlx::query(
                "INSERT INTO messages (
                   room_id, seq, sender_type, sender_agent_id, status, kind, run_id,
                   content, task_id, collaboration_batch_id, collaboration_round,
                   collaboration_role, created_at
                 )
                 VALUES (?, ?, 'system', NULL, 'final', 'reply', NULL, ?, NULL, ?, ?,
                         'system', ?)",
            )
            .bind(room_id)
            .bind(seq)
            .bind(RECOVERY_COLLABORATION_MESSAGE)
            .bind(batch_id)
            .bind(current_round)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            interrupted_collaborations.push(InterruptedCollaboration {
                batch_id,
                room_id,
                message: Message {
                    id: result.last_insert_rowid(),
                    room_id,
                    seq,
                    sender_type: MessageSenderType::System,
                    sender_agent_id: None,
                    status: MessageStatus::Final,
                    kind: MessageKind::Reply,
                    run_id: None,
                    content: RECOVERY_COLLABORATION_MESSAGE.to_string(),
                    task_id: None,
                    reply_to_message_id: None,
                    discussion: None,
                    collaboration: Some(MessageCollaboration {
                        batch_id,
                        round: current_round,
                        role: CollaborationMessageRole::System,
                    }),
                    attachments: Vec::new(),
                    reactions: Vec::new(),
                    created_at: now,
                },
            });
        }
    }

    let mut interrupted_discussions = Vec::new();
    if discussions_store.is_some() {
        let running = sqlx::query_as::<_, (i64, i64, i64)>(
            "SELECT id, room_id, current_round FROM discussions
              WHERE status = 'running' ORDER BY id",
        )
        .fetch_all(&mut *tx)
        .await?;
        if !running.is_empty() {
            sqlx::query(
                "UPDATE discussions SET status = 'failed', stop_reason = 'interrupted', updated_at = ?
                  WHERE status = 'running'",
            )
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        for (discussion_id, room_id, current_round) in running {
            let seq: i64 = sqlx::query_scalar(
                "SELECT COALESCE(MAX(seq), 0) + 1 FROM messages WHERE room_id = ?",
            )
            .bind(room_id)
            .fetch_one(&mut *tx)
            .await?;
            let result = sqlx::query(
                "INSERT INTO messages (
                   room_id, seq, sender_type, sender_agent_id, status, kind, run_id,
                   content, task_id, discussion_id, discussion_round, created_at
                 )
                 VALUES (?, ?, 'system', NULL, 'final', 'reply', NULL, ?, NULL, ?, ?, ?)",
            )
            .bind(room_id)
            .bind(seq)
            .bind(RECOVERY_DISCUSSION_MESSAGE)
            .bind(discussion_id)
            .bind(current_round)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            interrupted_discussions.push(InterruptedDiscussion {
                discussion_id,
                room_id,
                message: Message {
                    id: result.last_insert_rowid(),
                    room_id,
                    seq,
                    sender_type: MessageSenderType::System,
                    sender_agent_id: None,
                    status: MessageStatus::Final,
                    kind: MessageKind::Reply,
                    run_id: None,
                    content: RECOVERY_DISCUSSION_MESSAGE.to_string(),
                    task_id: None,
                    reply_to_message_id: None,
                    discussion: Some(MessageDiscussion {
                        discussion_id,
                        round: current_round,
                        stance: None,
                        reply_to_message_id: None,
                        stop_vote: None,
                        stop_reason: None,
                    }),
                    collaboration: None,
                    attachments: Vec::new(),
                    reactions: Vec::new(),
                    created_at: now,
                },
            });
        }
    }

    tx.commit().await?;

    if let Some(broadcast) = broadcast {
        if let Some(collaborations_store) = collaborations_store {
            for entry in &interrupted_collaborations {
                if let Ok(Some(batch)) = collaborations_store.get_by_id(entry.batch_id).await {
                    broadcast(
                        entry.room_id,
                        &RoomEvent::CollaborationChanged {
                            room_id: entry.room_id,
                            batch,
                        },
                    );
                }
                broadcast(
                    entry.room_id,
                    &RoomEvent::MessageFinal {
                        room_id: entry.room_id,
                        message: entry.message.clone(),
                    },
                );
            }
        }
        if let Some(discussions_store) = discussions_store {
            for entry in &interrupted_discussions {
                if let Ok(Some(discussion)) =
                    discussions_store.get_by_id(entry.discussion_id).await
                {
                    broadcast(
                        entry.room_id,
                        &RoomEvent::DiscussionChanged {
                            room_id: entry.room_id,
                            discussion,
                        },
                    );
                }
                broadcast(
                    entry.room_id,
                    &RoomEvent::MessageFinal {
                        room_id: entry.room_id,
                        message: entry.message.clone(),
                    },
                );
            }
        }
        // 事务提交后再广播，保证订阅方看到的都是已落库状态。
        for entry in &blocked {
            broadcast(
                entry.room_id,
                &RoomEvent::TaskChanged {
                    room_id: entry.room_id,
                    task_id: entry.task_id,
                    status: entry.status,
                },
            );
            broadcast(
                entry.room_id,
                &RoomEvent::MessageFinal {
                    room_id: entry.room_id,
                    message: entry.message.clone(),
                },
            );
        }
    }

    Ok(RecoveryResult {
        streaming_messages,
        running_runs: interrupted_runs,
        blocked_tasks: blocked.len() as u64,
        interrupted_discussions: interrupted_discussions.len() as u64,
        interrupted_collaborations: interrupted_collaborations.len() as u64,
    })
}
