//! 消息访问层：追加、流式更新、结构化元数据与历史清理。

use std::collections::HashMap;

use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::db::Db;
use crate::domain::message::{
    CollaborationMessageRole, DiscussionStance, DiscussionStopVote, Message,
    MessageCollaboration, MessageDiscussion, MessageKind, MessageReaction, MessageSenderType,
    MessageStatus, LOCAL_REACTION_ACTOR,
};
use crate::domain::now_ms;
use crate::error::{AppError, AppResult};
use crate::store::AttachmentStore;

/// 单条 DELETE 的绑定变量上限远低于 SQLite 的 32,766，按 500 一段拆分。
const DELETE_CHUNK_SIZE: usize = 500;

const MESSAGE_COLUMNS: &str = "id, room_id, seq, sender_type, sender_agent_id, status, kind, run_id, \
     content, task_id, reply_to_message_id, discussion_id, discussion_round, discussion_stance, \
     discussion_reply_to_message_id, discussion_stop_vote, discussion_stop_reason, \
     collaboration_batch_id, collaboration_round, collaboration_role, created_at";

#[derive(Debug, Clone)]
pub struct AppendMessageInput {
    pub room_id: i64,
    pub sender_type: MessageSenderType,
    pub sender_agent_id: Option<i64>,
    pub content: String,
    pub status: MessageStatus,
    pub kind: MessageKind,
    pub run_id: Option<i64>,
    pub task_id: Option<i64>,
    /// 普通消息的回复目标；讨论回复仍写入 `discussion`。
    pub reply_to_message_id: Option<i64>,
    pub discussion: Option<MessageDiscussion>,
    pub collaboration: Option<MessageCollaboration>,
}

impl AppendMessageInput {
    /// 普通最终消息（用户/系统消息、非流式回复）。
    pub fn final_message(
        room_id: i64,
        sender_type: MessageSenderType,
        sender_agent_id: Option<i64>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            room_id,
            sender_type,
            sender_agent_id,
            content: content.into(),
            status: MessageStatus::Final,
            kind: MessageKind::Reply,
            run_id: None,
            task_id: None,
            reply_to_message_id: None,
            discussion: None,
            collaboration: None,
        }
    }

    pub fn with_task(mut self, task_id: Option<i64>) -> Self {
        self.task_id = task_id;
        self
    }

    pub fn with_reply_to(mut self, message_id: Option<i64>) -> Self {
        self.reply_to_message_id = message_id;
        self
    }

    pub fn with_discussion(mut self, discussion: Option<MessageDiscussion>) -> Self {
        self.discussion = discussion;
        self
    }

    pub fn with_collaboration(mut self, collaboration: Option<MessageCollaboration>) -> Self {
        self.collaboration = collaboration;
        self
    }
}

#[derive(Debug, Clone, Default)]
pub struct UpdateMessageInput {
    pub content: Option<String>,
    pub status: Option<MessageStatus>,
    pub kind: Option<MessageKind>,
}

/// 讨论消息的结构化字段补丁；discussionId/round 在创建时固定。
#[derive(Debug, Clone, Default)]
pub struct UpdateMessageDiscussionInput {
    pub stance: Option<Option<DiscussionStance>>,
    pub reply_to_message_id: Option<Option<i64>>,
    pub stop_vote: Option<Option<DiscussionStopVote>>,
    pub stop_reason: Option<Option<String>>,
}

#[derive(Debug, Clone)]
pub struct MessageStore {
    db: Db,
    attachments: AttachmentStore,
}

impl MessageStore {
    pub fn new(db: Db) -> Self {
        Self {
            attachments: AttachmentStore::new(db.clone()),
            db,
        }
    }

    fn pool(&self) -> &SqlitePool {
        self.db.pool()
    }

    /// 追加消息并分配房间内序号。
    ///
    /// `seq` 由「同房间当前最大 seq + 1」得出，取值与插入在同一事务内完成，
    /// 因此同房间严格递增、跨房间各自从 1 开始计数。
    pub async fn append(&self, input: AppendMessageInput) -> AppResult<Message> {
        self.append_internal(input, &[]).await
    }

    /// 用户消息与附件在同一事务内提交；任一条附件无法绑定则整体回滚。
    pub async fn append_with_attachments(
        &self,
        input: AppendMessageInput,
        attachment_ids: &[i64],
    ) -> AppResult<Message> {
        self.append_internal(input, attachment_ids).await
    }

    async fn append_internal(
        &self,
        input: AppendMessageInput,
        attachment_ids: &[i64],
    ) -> AppResult<Message> {
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let (message_id, seq, created_at) = insert_message(&mut tx, &input).await?;
        if !attachment_ids.is_empty() {
            self.attachments
                .link_in_transaction(&mut tx, input.room_id, message_id, attachment_ids)
                .await?;
        }
        tx.commit().await?;

        if attachment_ids.is_empty() {
            return Ok(message_from(input, message_id, seq, created_at));
        }
        self.get_by_id(message_id).await?.ok_or_else(|| {
            AppError::internal(format!("附件绑定后消息不存在：{message_id}"))
        })
    }

    pub async fn get_by_id(&self, id: i64) -> AppResult<Option<Message>> {
        let row = sqlx::query_as::<_, MessageRow>(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(self.pool())
        .await?;
        let Some(message) = row.map(map_message_row).transpose()? else {
            return Ok(None);
        };
        Ok(self.hydrate(vec![message]).await?.pop())
    }

    /// 房间消息按 seq 升序，即讨论的原始顺序。
    pub async fn list_by_room(&self, room_id: i64) -> AppResult<Vec<Message>> {
        let rows = sqlx::query_as::<_, MessageRow>(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages WHERE room_id = ? ORDER BY seq"
        ))
        .bind(room_id)
        .fetch_all(self.pool())
        .await?;
        self.hydrate_rows(rows).await
    }

    /// 房间当前最大消息 id；无消息时返回 0。用作上下文水位提交上界。
    pub async fn max_id(&self, room_id: i64) -> AppResult<i64> {
        let id = sqlx::query_scalar(
            "SELECT COALESCE(MAX(id), 0) FROM messages WHERE room_id = ?",
        )
        .bind(room_id)
        .fetch_one(self.pool())
        .await?;
        Ok(id)
    }

    /// 取水位之后、快照上界之前的消息，按全局 id 升序。
    ///
    /// 清理历史后 `seq` 可能回退或复用，因此增量只认 `id`。
    pub async fn list_after_id(
        &self,
        room_id: i64,
        after_id: i64,
        up_to_id: i64,
    ) -> AppResult<Vec<Message>> {
        if up_to_id <= after_id {
            return Ok(Vec::new());
        }
        let rows = sqlx::query_as::<_, MessageRow>(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages
              WHERE room_id = ? AND id > ? AND id <= ?
              ORDER BY id"
        ))
        .bind(room_id)
        .bind(after_id)
        .bind(up_to_id)
        .fetch_all(self.pool())
        .await?;
        self.hydrate_rows(rows).await
    }

    /// 运行产生的阶段总结与最终回复按 seq 升序。
    pub async fn list_by_run(&self, run_id: i64) -> AppResult<Vec<Message>> {
        let rows = sqlx::query_as::<_, MessageRow>(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages WHERE run_id = ? ORDER BY seq"
        ))
        .bind(run_id)
        .fetch_all(self.pool())
        .await?;
        self.hydrate_rows(rows).await
    }

    /// 任务关联消息按 id 升序；只返回最终回复，阶段总结不进入时间线。
    pub async fn list_by_task(&self, task_id: i64) -> AppResult<Vec<Message>> {
        let rows = sqlx::query_as::<_, MessageRow>(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages
              WHERE task_id = ? AND kind = 'reply'
              ORDER BY id"
        ))
        .bind(task_id)
        .fetch_all(self.pool())
        .await?;
        self.hydrate_rows(rows).await
    }

    pub async fn list_by_discussion(&self, discussion_id: i64) -> AppResult<Vec<Message>> {
        let rows = sqlx::query_as::<_, MessageRow>(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages
              WHERE discussion_id = ? ORDER BY seq"
        ))
        .bind(discussion_id)
        .fetch_all(self.pool())
        .await?;
        self.hydrate_rows(rows).await
    }

    pub async fn list_by_collaboration(&self, batch_id: i64) -> AppResult<Vec<Message>> {
        let rows = sqlx::query_as::<_, MessageRow>(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages
              WHERE collaboration_batch_id = ? ORDER BY seq"
        ))
        .bind(batch_id)
        .fetch_all(self.pool())
        .await?;
        self.hydrate_rows(rows).await
    }

    /// 把消息从讨论/委派结构中摘除，只保留房间聊天归属。
    pub async fn clear_structured_metadata(&self, id: i64) -> AppResult<Option<Message>> {
        let result = sqlx::query(
            "UPDATE messages
                SET discussion_id = NULL, discussion_round = NULL,
                    discussion_stance = NULL, discussion_reply_to_message_id = NULL,
                    discussion_stop_vote = NULL, discussion_stop_reason = NULL,
                    collaboration_batch_id = NULL, collaboration_round = NULL,
                    collaboration_role = NULL
              WHERE id = ?",
        )
        .bind(id)
        .execute(self.pool())
        .await?;
        if result.rows_affected() == 0 {
            return Ok(None);
        }
        self.get_by_id(id).await
    }

    /// 启动恢复：把所有残留的 `streaming` 消息标记为 `failed`。
    pub async fn fail_streaming(&self) -> AppResult<u64> {
        let result = sqlx::query("UPDATE messages SET status = 'failed' WHERE status = 'streaming'")
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected())
    }

    /// 被其它表外键引用的消息 id：委派触发消息、指派产出与讨论回复目标。
    pub async fn protected_message_ids(&self, room_id: i64) -> AppResult<Vec<i64>> {
        let rows = sqlx::query_scalar::<_, i64>(
            "SELECT user_message_id AS id
               FROM collaboration_batches
              WHERE room_id = ? AND user_message_id IS NOT NULL
             UNION
             SELECT a.message_id AS id
               FROM collaboration_assignments AS a
               JOIN collaboration_batches AS b ON b.id = a.batch_id
              WHERE b.room_id = ? AND a.message_id IS NOT NULL
             UNION
             SELECT discussion_reply_to_message_id AS id
               FROM messages
              WHERE room_id = ? AND discussion_reply_to_message_id IS NOT NULL",
        )
        .bind(room_id)
        .bind(room_id)
        .bind(room_id)
        .fetch_all(self.pool())
        .await?;
        Ok(rows)
    }

    /// 按 id 物理删除消息（历史清理专用），返回实际删除行数。
    pub async fn delete_by_ids(&self, ids: &[i64]) -> AppResult<u64> {
        if ids.is_empty() {
            return Ok(0);
        }
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let mut deleted = 0_u64;
        for chunk in ids.chunks(DELETE_CHUNK_SIZE) {
            let placeholders = vec!["?"; chunk.len()].join(", ");
            // 显式删除反应：不依赖连接级外键开关，也让清理结果可直接断言。
            let reaction_sql =
                format!("DELETE FROM message_reactions WHERE message_id IN ({placeholders})");
            let mut reaction_query = sqlx::query(&reaction_sql);
            for id in chunk {
                reaction_query = reaction_query.bind(id);
            }
            reaction_query.execute(&mut *tx).await?;

            let sql = format!("DELETE FROM messages WHERE id IN ({placeholders})");
            let mut query = sqlx::query(&sql);
            for id in chunk {
                query = query.bind(id);
            }
            deleted += query.execute(&mut *tx).await?.rows_affected();
        }
        tx.commit().await?;
        Ok(deleted)
    }

    /// 流式消息占位/落定：只更新传入的字段，未传入的保持原值。
    pub async fn update(
        &self,
        id: i64,
        patch: UpdateMessageInput,
    ) -> AppResult<Option<Message>> {
        let Some(existing) = self.get_by_id(id).await? else {
            return Ok(None);
        };

        let content = patch.content.unwrap_or_else(|| existing.content.clone());
        let status = patch.status.unwrap_or(existing.status);
        let kind = patch.kind.unwrap_or(existing.kind);

        sqlx::query("UPDATE messages SET content = ?, status = ?, kind = ? WHERE id = ?")
            .bind(&content)
            .bind(status.as_str())
            .bind(kind.as_str())
            .bind(id)
            .execute(self.pool())
            .await?;

        Ok(Some(Message {
            content,
            status,
            kind,
            ..existing
        }))
    }

    /// 写入讨论轮次的结构化字段；非讨论消息不会被临时升级为讨论消息。
    pub async fn update_discussion(
        &self,
        id: i64,
        patch: UpdateMessageDiscussionInput,
    ) -> AppResult<Option<Message>> {
        let Some(existing) = self.get_by_id(id).await? else {
            return Ok(None);
        };
        let Some(current) = existing.discussion.clone() else {
            return Ok(Some(existing));
        };

        let next = MessageDiscussion {
            discussion_id: current.discussion_id,
            round: current.round,
            stance: patch.stance.unwrap_or(current.stance),
            reply_to_message_id: patch
                .reply_to_message_id
                .unwrap_or(current.reply_to_message_id),
            stop_vote: patch.stop_vote.unwrap_or(current.stop_vote),
            stop_reason: patch.stop_reason.unwrap_or(current.stop_reason),
        };

        sqlx::query(
            "UPDATE messages
                SET discussion_stance = ?,
                    discussion_reply_to_message_id = ?,
                    discussion_stop_vote = ?,
                    discussion_stop_reason = ?
              WHERE id = ?",
        )
        .bind(next.stance.map(DiscussionStance::as_str))
        .bind(next.reply_to_message_id)
        .bind(next.stop_vote.map(DiscussionStopVote::as_str))
        .bind(&next.stop_reason)
        .bind(id)
        .execute(self.pool())
        .await?;

        Ok(Some(Message {
            discussion: Some(next),
            ..existing
        }))
    }

    async fn hydrate_rows(&self, rows: Vec<MessageRow>) -> AppResult<Vec<Message>> {
        let messages = rows
            .into_iter()
            .map(map_message_row)
            .collect::<AppResult<Vec<_>>>()?;
        self.hydrate(messages).await
    }

    async fn hydrate(&self, mut messages: Vec<Message>) -> AppResult<Vec<Message>> {
        if messages.is_empty() {
            return Ok(messages);
        }
        let ids: Vec<i64> = messages.iter().map(|message| message.id).collect();
        let mut grouped = self.attachments.list_by_messages(&ids).await?;
        let mut reactions = self
            .list_reactions_by_messages(&ids, LOCAL_REACTION_ACTOR)
            .await?;
        for message in &mut messages {
            message.attachments = grouped.remove(&message.id).unwrap_or_default();
            message.reactions = reactions.remove(&message.id).unwrap_or_default();
        }
        Ok(messages)
    }

    /// 批量读取消息反应，避免 hydrate 时逐条查询。
    pub async fn list_reactions_by_messages(
        &self,
        message_ids: &[i64],
        actor: &str,
    ) -> AppResult<HashMap<i64, Vec<MessageReaction>>> {
        let mut grouped: HashMap<i64, Vec<MessageReaction>> = HashMap::new();
        if message_ids.is_empty() {
            return Ok(grouped);
        }
        let placeholders = vec!["?"; message_ids.len()].join(", ");
        let sql = format!(
            "SELECT message_id, emoji, COUNT(*) AS count,
                    MAX(CASE WHEN actor = ? THEN 1 ELSE 0 END) AS reacted_by_me
               FROM message_reactions
              WHERE message_id IN ({placeholders})
              GROUP BY message_id, emoji
              ORDER BY MIN(id)"
        );
        let mut query = sqlx::query_as::<_, ReactionRow>(&sql).bind(actor);
        for id in message_ids {
            query = query.bind(id);
        }
        for row in query.fetch_all(self.pool()).await? {
            grouped
                .entry(row.message_id)
                .or_default()
                .push(MessageReaction {
                    emoji: row.emoji,
                    count: row.count,
                    reacted_by_me: row.reacted_by_me != 0,
                });
        }
        Ok(grouped)
    }

    /// 切换单个消息的单个 emoji 反应，返回该消息的最新反应。
    pub async fn toggle_reaction(
        &self,
        message_id: i64,
        emoji: &str,
        actor: &str,
    ) -> AppResult<Vec<MessageReaction>> {
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let existing: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM message_reactions
              WHERE message_id = ? AND emoji = ? AND actor = ?",
        )
        .bind(message_id)
        .bind(emoji)
        .bind(actor)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(id) = existing {
            sqlx::query("DELETE FROM message_reactions WHERE id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        } else {
            sqlx::query(
                "INSERT INTO message_reactions (message_id, emoji, actor, created_at)
                 VALUES (?, ?, ?, ?)",
            )
            .bind(message_id)
            .bind(emoji)
            .bind(actor)
            .bind(now_ms())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;

        let mut grouped = self
            .list_reactions_by_messages(&[message_id], actor)
            .await?;
        Ok(grouped.remove(&message_id).unwrap_or_default())
    }
}

/// 在已开启的事务内插入消息并分配房间内序号。
async fn insert_message(
    tx: &mut Transaction<'_, Sqlite>,
    input: &AppendMessageInput,
) -> AppResult<(i64, i64, i64)> {
    let seq: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM messages WHERE room_id = ?",
    )
    .bind(input.room_id)
    .fetch_one(&mut **tx)
    .await?;

    let created_at = now_ms();
    let discussion_id = input.discussion.as_ref().map(|value| value.discussion_id);
    let discussion_round = input.discussion.as_ref().map(|value| value.round);
    let discussion_stance = input
        .discussion
        .as_ref()
        .and_then(|value| value.stance)
        .map(DiscussionStance::as_str);
    let discussion_reply_to = input
        .discussion
        .as_ref()
        .and_then(|value| value.reply_to_message_id);
    let discussion_stop_vote = input
        .discussion
        .as_ref()
        .and_then(|value| value.stop_vote)
        .map(DiscussionStopVote::as_str);
    let discussion_stop_reason = input
        .discussion
        .as_ref()
        .and_then(|value| value.stop_reason.as_deref());
    let collaboration_batch_id = input.collaboration.as_ref().map(|value| value.batch_id);
    let collaboration_round = input.collaboration.as_ref().map(|value| value.round);
    let collaboration_role = input
        .collaboration
        .as_ref()
        .map(|value| value.role.as_str());

    let result = sqlx::query(
        "INSERT INTO messages (
           room_id, seq, sender_type, sender_agent_id, status, kind, run_id,
           content, task_id, reply_to_message_id,
           discussion_id, discussion_round, discussion_stance,
           discussion_reply_to_message_id, discussion_stop_vote,
           discussion_stop_reason, collaboration_batch_id,
           collaboration_round, collaboration_role, created_at
         )
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(input.room_id)
    .bind(seq)
    .bind(input.sender_type.as_str())
    .bind(input.sender_agent_id)
    .bind(input.status.as_str())
    .bind(input.kind.as_str())
    .bind(input.run_id)
    .bind(&input.content)
    .bind(input.task_id)
    .bind(input.reply_to_message_id)
    .bind(discussion_id)
    .bind(discussion_round)
    .bind(discussion_stance)
    .bind(discussion_reply_to)
    .bind(discussion_stop_vote)
    .bind(discussion_stop_reason)
    .bind(collaboration_batch_id)
    .bind(collaboration_round)
    .bind(collaboration_role)
    .bind(created_at)
    .execute(&mut **tx)
    .await?;
    Ok((result.last_insert_rowid(), seq, created_at))
}

fn message_from(input: AppendMessageInput, id: i64, seq: i64, created_at: i64) -> Message {
    Message {
        id,
        room_id: input.room_id,
        seq,
        sender_type: input.sender_type,
        sender_agent_id: input.sender_agent_id,
        status: input.status,
        kind: input.kind,
        run_id: input.run_id,
        content: input.content,
        task_id: input.task_id,
        reply_to_message_id: input.reply_to_message_id,
        discussion: input.discussion,
        collaboration: input.collaboration,
        attachments: Vec::new(),
        reactions: Vec::new(),
        created_at,
    }
}

#[derive(sqlx::FromRow)]
struct ReactionRow {
    message_id: i64,
    emoji: String,
    count: i64,
    reacted_by_me: i64,
}

#[derive(sqlx::FromRow)]
struct MessageRow {
    id: i64,
    room_id: i64,
    seq: i64,
    sender_type: String,
    sender_agent_id: Option<i64>,
    status: String,
    kind: String,
    run_id: Option<i64>,
    content: String,
    task_id: Option<i64>,
    reply_to_message_id: Option<i64>,
    discussion_id: Option<i64>,
    discussion_round: Option<i64>,
    discussion_stance: Option<String>,
    discussion_reply_to_message_id: Option<i64>,
    discussion_stop_vote: Option<String>,
    discussion_stop_reason: Option<String>,
    collaboration_batch_id: Option<i64>,
    collaboration_round: Option<i64>,
    collaboration_role: Option<String>,
    created_at: i64,
}

fn map_message_row(row: MessageRow) -> AppResult<Message> {
    let sender_type = match row.sender_type.as_str() {
        "user" => MessageSenderType::User,
        "agent" => MessageSenderType::Agent,
        "system" => MessageSenderType::System,
        other => {
            return Err(AppError::internal(format!(
                "messages.sender_type 非法：{other}"
            )))
        }
    };
    let status = match row.status.as_str() {
        "final" => MessageStatus::Final,
        "streaming" => MessageStatus::Streaming,
        "failed" => MessageStatus::Failed,
        other => {
            return Err(AppError::internal(format!(
                "messages.status 非法：{other}"
            )))
        }
    };
    let kind = if row.kind == "progress" {
        MessageKind::Progress
    } else {
        MessageKind::Reply
    };

    let discussion = row.discussion_id.map(|discussion_id| MessageDiscussion {
        discussion_id,
        round: row.discussion_round.unwrap_or(0),
        stance: row
            .discussion_stance
            .as_deref()
            .and_then(DiscussionStance::parse),
        reply_to_message_id: row.discussion_reply_to_message_id,
        stop_vote: row
            .discussion_stop_vote
            .as_deref()
            .and_then(DiscussionStopVote::parse),
        stop_reason: row.discussion_stop_reason,
    });

    let collaboration =
        row.collaboration_batch_id
            .map(|batch_id| MessageCollaboration {
                batch_id,
                round: row.collaboration_round.unwrap_or(0),
                role: row
                    .collaboration_role
                    .as_deref()
                    .and_then(CollaborationMessageRole::parse)
                    .unwrap_or(CollaborationMessageRole::System),
            });

    Ok(Message {
        id: row.id,
        room_id: row.room_id,
        seq: row.seq,
        sender_type,
        sender_agent_id: row.sender_agent_id,
        status,
        kind,
        run_id: row.run_id,
        content: row.content,
        task_id: row.task_id,
        reply_to_message_id: row.reply_to_message_id,
        discussion,
        collaboration,
        attachments: Vec::new(),
        reactions: Vec::new(),
        created_at: row.created_at,
    })
}
