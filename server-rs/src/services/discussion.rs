//! 房间讨论：协议解析、提示词与串行轮次调度。
//!
//! 关键不变量：
//! - 同一房间最多一个 running 讨论（服务检查 + 数据库部分唯一索引）；
//! - 参与者每人每轮最多一次发言，总发言不超过 参与人数 × 讨论 maxRounds；
//! - 所有运行经 ChatService 复用房间串行队列；
//! - `start()` 只创建并启动异步循环，绝不等待首轮运行，避免主持人 after 死锁；
//! - 结束/停止使用条件更新，任何失败都留下可见系统消息。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::domain::agent::Agent;
use crate::domain::discussion::{
    Discussion, DiscussionStatus, DiscussionStopReason, ModeratorDiscussionDraft,
    DEFAULT_DISCUSSION_ROUNDS, DISCUSSION_JUDGE_TRANSCRIPT_LIMIT, DISCUSSION_MESSAGE_CHARS,
    DISCUSSION_REASON_CHARS, DISCUSSION_SUMMARY_CHARS, DISCUSSION_TOPIC_CHARS,
    DISCUSSION_TRANSCRIPT_LIMIT, MAX_DISCUSSION_PARTICIPANTS, MAX_DISCUSSION_ROUNDS,
    MIN_DISCUSSION_PARTICIPANTS,
};
use crate::domain::event::RoomEvent;
use crate::domain::message::{
    DiscussionStance, DiscussionStopVote, Message, MessageDiscussion, MessageSenderType,
    MessageStatus,
};
use crate::domain::room::Room;
use crate::error::{AppError, AppResult};
use crate::services::chat::{
    BroadcastFn, ChatService, DiscussionMetadataPatch, FinalizeFn, FinalizedContent,
    RunAgentOptions,
};
use crate::services::moderator::{extract_last_block, strip_blocks, CONCISE_REPLY_RULE};
use crate::store::discussions::{CreateDiscussionInput, FinishDiscussionInput};
use crate::store::{AgentStore, DiscussionStore, MessageStore, RoomStore};

#[derive(Debug, Clone)]
pub struct ParsedDiscussionTurn {
    pub display: String,
    pub stance: DiscussionStance,
    pub reply_to_agent_id: Option<i64>,
    pub stop_vote: DiscussionStopVote,
    pub stop_reason: Option<String>,
    /// 协议缺失/降级原因，由服务层转成系统提示。
    pub notes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ParsedDiscussionJudge {
    pub display: String,
    pub decision: String,
    pub reason: String,
    pub summary: Option<String>,
    pub consensus: Vec<String>,
    pub disagreements: Vec<String>,
    pub notes: Vec<String>,
}

pub struct DiscussionService {
    rooms: RoomStore,
    agents: AgentStore,
    messages: MessageStore,
    discussions: DiscussionStore,
    chat: Arc<ChatService>,
    broadcast: BroadcastFn,
}

impl DiscussionService {
    pub fn new(
        rooms: RoomStore,
        agents: AgentStore,
        messages: MessageStore,
        discussions: DiscussionStore,
        chat: Arc<ChatService>,
        broadcast: BroadcastFn,
    ) -> Self {
        Self {
            rooms,
            agents,
            messages,
            discussions,
            chat,
            broadcast,
        }
    }

    pub async fn get_latest(&self, raw_room_id: &str) -> AppResult<Option<Discussion>> {
        let room_id = parse_id(raw_room_id, "roomId")?;
        self.require_room(room_id).await?;
        self.discussions.latest_by_room(room_id).await
    }

    /// 创建讨论并立即返回；讨论循环在后台入队运行。
    ///
    /// 调用方通常是主持人运行的 after 回调；如果这里 await 首轮运行，
    /// 首轮会排在主持人运行之后，而主持人运行又在等待 after，形成死锁。
    pub async fn start(
        self: &Arc<Self>,
        room: &Room,
        moderator: &Agent,
        draft: ModeratorDiscussionDraft,
    ) -> AppResult<Discussion> {
        let room = self.require_room(room.id).await?;
        let moderator = self.require_moderator(&room, moderator).await?;
        let members = self.rooms.list_members(room.id).await?;
        let member_ids: HashSet<i64> = members.iter().map(|member| member.id).collect();

        let topic = draft.topic.trim().to_string();
        if topic.is_empty() || topic.chars().count() > DISCUSSION_TOPIC_CHARS {
            return Err(AppError::validation(format!(
                "讨论主题必须是非空字符串且不超过 {DISCUSSION_TOPIC_CHARS} 字符"
            )));
        }
        // 房间设置是轮数上限；主持人省略轮数时取默认 3 与上限的较小值。
        let room_limit = room.discussion_max_rounds;
        let max_rounds = draft
            .max_rounds
            .unwrap_or_else(|| DEFAULT_DISCUSSION_ROUNDS.min(room_limit));
        if max_rounds < 1 || max_rounds > room_limit {
            return Err(AppError::validation(format!(
                "讨论轮次必须是 1..{room_limit} 的整数"
            )));
        }

        let participant_ids = draft.participant_ids;
        if participant_ids.len() < MIN_DISCUSSION_PARTICIPANTS
            || participant_ids.len() > MAX_DISCUSSION_PARTICIPANTS
        {
            return Err(AppError::validation(format!(
                "讨论参与者必须是 {MIN_DISCUSSION_PARTICIPANTS}..{MAX_DISCUSSION_PARTICIPANTS} 名成员"
            )));
        }

        let mut seen = HashSet::new();
        let mut member_by_id = HashMap::new();
        for member in &members {
            member_by_id.insert(member.id, member.clone());
        }
        for agent_id in &participant_ids {
            if *agent_id == moderator.id {
                return Err(AppError::validation("主持人不能作为讨论参与者"));
            }
            if !member_ids.contains(agent_id) {
                return Err(AppError::validation(format!(
                    "讨论参与者不是房间成员：{agent_id}"
                )));
            }
            if !seen.insert(*agent_id) {
                return Err(AppError::validation(format!("讨论参与者重复：{agent_id}")));
            }
        }

        if self
            .discussions
            .find_running_by_room(room.id)
            .await?
            .is_some()
        {
            return Err(AppError::constraint(
                "房间已有进行中的讨论，请先停止当前讨论",
            ));
        }

        let discussion = match self
            .discussions
            .create(CreateDiscussionInput {
                room_id: room.id,
                topic,
                max_rounds,
                participant_ids: participant_ids.clone(),
                created_by_agent_id: Some(moderator.id),
            })
            .await
        {
            Ok(discussion) => discussion,
            Err(error) if error.message.contains("UNIQUE constraint failed: discussions.room_id") => {
                return Err(AppError::with_source(
                    crate::error::ErrorCode::ConstraintViolation,
                    "房间已有进行中的讨论，请先停止当前讨论",
                    std::io::Error::new(std::io::ErrorKind::Other, error.to_string()),
                ));
            }
            Err(error) => return Err(error),
        };

        let participant_names = participant_ids
            .iter()
            .map(|agent_id| {
                member_by_id
                    .get(agent_id)
                    .map(|member| member.name.clone())
                    .unwrap_or_else(|| format!("#{agent_id}"))
            })
            .collect::<Vec<_>>()
            .join("、");
        let start_message = format!(
            "讨论已开始：{}\n参与者：{}\n最多 {} 轮；每位参与者请给出立场、回复对象和 stop/continue 投票。",
            discussion.topic, participant_names, discussion.max_rounds
        );
        self.post_discussion_system_message(&discussion, &start_message)
            .await?;
        self.broadcast_changed(&discussion);
        Ok(discussion)
    }

    /// 启动讨论循环；调用方在 `start()` 返回后立即调用，不等待首轮运行。
    pub fn start_loop(self: &Arc<Self>, discussion_id: i64) {
        let service = Arc::clone(self);
        tokio::spawn(async move {
            service.run_discussion(discussion_id).await;
        });
    }

    /// 续跑失败讨论：沿用原 topic/参与者/轮数/当前轮，跳过本轮已完成的发言。
    ///
    /// 只接受 `failed` 讨论；恢复后仍是同一 discussion 记录，不新建讨论。
    pub async fn resume(
        self: &Arc<Self>,
        raw_room_id: &str,
        raw_discussion_id: &str,
    ) -> AppResult<Discussion> {
        let room_id = parse_id(raw_room_id, "roomId")?;
        let discussion_id = parse_id(raw_discussion_id, "discussionId")?;
        let room = self.require_room(room_id).await?;
        let existing = self.require_discussion(discussion_id).await?;
        if existing.room_id != room_id {
            return Err(AppError::not_found(format!(
                "房间 {room_id} 中不存在讨论 {discussion_id}"
            )));
        }
        if existing.status != DiscussionStatus::Failed {
            return Err(AppError::constraint("讨论当前状态不可继续"));
        }
        self.require_moderator_by_id(&room).await?;
        let participants = self.load_participants(&existing).await?;
        if participants.len() != existing.participant_ids.len() {
            return Err(AppError::constraint("讨论参与者已变更，无法继续"));
        }
        if self
            .discussions
            .find_running_by_room(room_id)
            .await?
            .is_some()
        {
            return Err(AppError::constraint(
                "房间已有进行中的讨论，请先停止当前讨论",
            ));
        }

        let history = self.messages.list_by_discussion(discussion_id).await?;
        let spoken = spoken_agent_ids(&history, existing.current_round);
        let next = existing
            .participant_ids
            .iter()
            .find(|agent_id| !spoken.contains(agent_id));
        let reopened = match self.discussions.reopen(discussion_id).await? {
            Some(reopened) if reopened.status == DiscussionStatus::Running => reopened,
            _ => return Err(AppError::constraint("讨论当前状态不可继续")),
        };
        let continue_message = match next {
            Some(agent_id) => {
                let name = participants
                    .iter()
                    .find(|participant| participant.id == *agent_id)
                    .map(|participant| participant.name.clone())
                    .unwrap_or_else(|| format!("#{agent_id}"));
                format!(
                    "讨论继续：第 {} 轮，从「{name}」继续",
                    reopened.current_round
                )
            }
            None => format!("讨论继续：第 {} 轮裁决", reopened.current_round),
        };
        self.post_discussion_system_message(&reopened, &continue_message)
            .await?;
        self.broadcast_changed(&reopened);
        self.start_loop(reopened.id);
        Ok(reopened)
    }

    /// 用户手动停止；幂等，已结束讨论原样返回。
    pub async fn stop(
        &self,
        raw_room_id: &str,
        raw_discussion_id: &str,
    ) -> AppResult<Discussion> {
        let room_id = parse_id(raw_room_id, "roomId")?;
        let discussion_id = parse_id(raw_discussion_id, "discussionId")?;
        self.require_room(room_id).await?;

        let existing = self.require_discussion(discussion_id).await?;
        if existing.room_id != room_id {
            return Err(AppError::not_found(format!(
                "房间 {room_id} 中不存在讨论 {discussion_id}"
            )));
        }
        if existing.status != DiscussionStatus::Running {
            return Ok(existing);
        }

        let stopped = self
            .discussions
            .finish(
                existing.id,
                FinishDiscussionInput {
                    status: DiscussionStatus::Stopped,
                    stop_reason: DiscussionStopReason::UserStop,
                    summary: None,
                },
            )
            .await?;
        let Some(stopped) = stopped else {
            return self.require_discussion(discussion_id).await;
        };
        self.post_discussion_system_message(
            &stopped,
            &format!("讨论已停止：{}", stopped.topic),
        )
        .await?;
        self.broadcast_changed(&stopped);
        Ok(stopped)
    }

    async fn run_discussion(self: Arc<Self>, discussion_id: i64) {
        if let Err(error) = self.run_discussion_inner(discussion_id).await {
            tracing::error!(
                discussion_id,
                error = %error.message,
                "discussion loop failed"
            );
            if let Ok(Some(current)) = self.discussions.get_by_id(discussion_id).await {
                if current.status == DiscussionStatus::Running {
                    let _ = self
                        .fail_discussion(
                            &current,
                            DiscussionStopReason::InternalError,
                            &error.message,
                        )
                        .await;
                }
            }
        }
    }

    async fn run_discussion_inner(&self, discussion_id: i64) -> AppResult<()> {
        let Some(mut discussion) = self.discussions.get_by_id(discussion_id).await? else {
            return Ok(());
        };
        if discussion.status != DiscussionStatus::Running {
            return Ok(());
        }

        let room = self.require_room(discussion.room_id).await?;
        let moderator = self.require_moderator_by_id(&room).await?;
        let participants = self.load_participants(&discussion).await?;
        if participants.len() != discussion.participant_ids.len() {
            self.fail_discussion(
                &discussion,
                DiscussionStopReason::ParticipantFailed,
                "部分讨论参与者已不存在",
            )
            .await?;
            return Ok(());
        }

        let mut total_turns = 0_usize;
        // 发言上限跟随讨论快照：参与人数 × maxRounds。
        let max_turns = participants.len() * discussion.max_rounds as usize;

        for round in discussion.current_round..=discussion.max_rounds {
            let Some(current) = self.discussions.get_by_id(discussion_id).await? else {
                return Ok(());
            };
            if current.status != DiscussionStatus::Running {
                return Ok(());
            }
            discussion = current;

            let mut remaining = discussion.participant_ids.clone();
            let mut votes: HashMap<i64, DiscussionStopVote> = HashMap::new();

            // 续跑：本轮已完成的发言不再重跑，并用其 stop 票重建本轮票型。
            let round_history = self.messages.list_by_discussion(discussion_id).await?;
            for message in &round_history {
                if message.sender_type != MessageSenderType::Agent {
                    continue;
                }
                let Some(meta) = message.discussion.as_ref() else {
                    continue;
                };
                if meta.round != round || meta.stance.is_none() {
                    continue;
                }
                let Some(agent_id) = message.sender_agent_id else {
                    continue;
                };
                remaining.retain(|id| *id != agent_id);
                if let Some(vote) = meta.stop_vote {
                    votes.insert(agent_id, vote);
                }
            }

            while !remaining.is_empty() {
                let Some(before_turn) = self.discussions.get_by_id(discussion_id).await? else {
                    return Ok(());
                };
                if before_turn.status != DiscussionStatus::Running {
                    return Ok(());
                }
                discussion = before_turn;

                let agent_id = remaining.remove(0);
                let Some(member) = self.member_of(&room, agent_id).await? else {
                    self.fail_discussion(
                        &discussion,
                        DiscussionStopReason::ParticipantFailed,
                        &format!("讨论参与者 #{agent_id} 已不在房间中"),
                    )
                    .await?;
                    return Ok(());
                };

                total_turns += 1;
                if total_turns > max_turns {
                    self.fail_discussion(
                        &discussion,
                        DiscussionStopReason::InternalError,
                        &format!("讨论发言数超过上限 {max_turns}"),
                    )
                    .await?;
                    return Ok(());
                }

                let history = self.messages.list_by_discussion(discussion_id).await?;
                let previous_turns: Vec<Message> = history
                    .iter()
                    .filter(|message| {
                        message.sender_type == MessageSenderType::Agent
                            && message
                                .discussion
                                .as_ref()
                                .and_then(|meta| meta.stance)
                                .is_some()
                    })
                    .cloned()
                    .collect();
                // 复用会话且本人已在本场讨论发过言：完整人设与历史已在
                // harness 上下文中，只发上次发言之后的增量，避免每轮重复。
                let compact_prompt = spoke_before(&history, member.id)
                    && self
                        .chat
                        .has_resumable_session(room.id, &member)
                        .await?;
                let instruction = build_discussion_turn_instruction(
                    &room,
                    &member,
                    &discussion.topic,
                    &participants,
                    round,
                    discussion.max_rounds,
                    &history,
                    compact_prompt,
                );

                let parsed: Arc<Mutex<Option<ParsedDiscussionTurn>>> = Arc::new(Mutex::new(None));
                let finalize = turn_finalize(
                    member.clone(),
                    participants.clone(),
                    previous_turns,
                    parsed.clone(),
                );
                let discussions = self.discussions.clone();
                let guard_id = discussion_id;
                let run = self
                    .chat
                    .run_agent_queued_checked(
                        room.id,
                        &member,
                        &instruction,
                        None,
                        RunAgentOptions {
                            stream: Some(true),
                            skip_room_context: true,
                            discussion: Some(crate::services::chat::DiscussionRef {
                                discussion_id,
                                round,
                            }),
                            finalize: Some(finalize),
                            ..RunAgentOptions::default()
                        },
                        move || {
                            let discussions = discussions.clone();
                            async move {
                                matches!(
                                    discussions.get_by_id(guard_id).await,
                                    Ok(Some(current)) if current.status == DiscussionStatus::Running
                                )
                            }
                        },
                    )
                    .await?;

                // 排队期间讨论被停止：出队时守卫已跳过本次发言。
                let Some(run) = run else {
                    return Ok(());
                };
                let Some(after_run) = self.discussions.get_by_id(discussion_id).await? else {
                    return Ok(());
                };
                if after_run.status != DiscussionStatus::Running {
                    return Ok(());
                }
                discussion = after_run;

                let parsed_turn = parsed.lock().unwrap_or_else(|e| e.into_inner()).take();
                if run.status != crate::domain::run::RunStatus::Succeeded {
                    self.fail_discussion(
                        &discussion,
                        DiscussionStopReason::ParticipantFailed,
                        &format!(
                            "参与者「{}」运行失败：{}",
                            member.name,
                            run.error.clone().unwrap_or_else(|| run.status.as_str().to_string())
                        ),
                    )
                    .await?;
                    return Ok(());
                }
                let Some(turn) = parsed_turn else {
                    self.fail_discussion(
                        &discussion,
                        DiscussionStopReason::ParticipantFailed,
                        &format!("参与者「{}」运行失败：输出无法解析", member.name),
                    )
                    .await?;
                    return Ok(());
                };

                votes.insert(member.id, turn.stop_vote);
                if !turn.notes.is_empty() {
                    self.post_discussion_system_message(
                        &discussion,
                        &format!("讨论提示（{}）：{}", member.name, turn.notes.join("；")),
                    )
                    .await?;
                }
                if let Some(reply_to) = turn.reply_to_agent_id {
                    if let Some(index) = remaining.iter().position(|id| *id == reply_to) {
                        let id = remaining.remove(index);
                        remaining.insert(0, id);
                    }
                }
            }

            let Some(after_round) = self.discussions.get_by_id(discussion_id).await? else {
                return Ok(());
            };
            if after_round.status != DiscussionStatus::Running {
                return Ok(());
            }
            discussion = after_round;

            let all_voted_stop = participants
                .iter()
                .all(|participant| votes.get(&participant.id) == Some(&DiscussionStopVote::Stop));
            let reached_max = round >= discussion.max_rounds;

            if all_voted_stop || reached_max {
                let forced = if all_voted_stop {
                    DiscussionStopReason::AllVotedStop
                } else {
                    DiscussionStopReason::MaxRounds
                };
                let judged = self
                    .judge(&discussion, &room, &moderator, &participants, round, Some(forced))
                    .await?;
                let summary = judged
                    .as_ref()
                    .and_then(|judge| judge.summary.clone())
                    .unwrap_or_else(|| {
                        if all_voted_stop {
                            "所有参与者投票结束讨论。".to_string()
                        } else {
                            "讨论达到最大轮次，已停止。".to_string()
                        }
                    });
                let _ = self
                    .finish_discussion(&discussion, DiscussionStatus::Completed, forced, Some(summary))
                    .await?;
                return Ok(());
            }

            let Some(judged) = self
                .judge(&discussion, &room, &moderator, &participants, round, None)
                .await?
            else {
                self.fail_discussion(
                    &discussion,
                    DiscussionStopReason::ModeratorFailed,
                    "主持人裁决连续失败，讨论已停止",
                )
                .await?;
                return Ok(());
            };

            if judged.decision == "stop" {
                let summary = judged
                    .summary
                    .clone()
                    .unwrap_or_else(|| judged.reason.clone());
                let _ = self
                    .finish_discussion(
                        &discussion,
                        DiscussionStatus::Completed,
                        DiscussionStopReason::ModeratorDecision,
                        Some(summary),
                    )
                    .await?;
                return Ok(());
            }

            let Some(next) = self
                .discussions
                .advance_round(discussion_id, round + 1)
                .await?
            else {
                return Ok(());
            };
            if next.status != DiscussionStatus::Running {
                return Ok(());
            }
            discussion = next;
            self.broadcast_changed(&discussion);
        }

        // 防御：正常路径在最大轮次已收尾；此处只处理异常循环退出。
        if let Some(current) = self.discussions.get_by_id(discussion_id).await? {
            if current.status == DiscussionStatus::Running {
                let _ = self
                    .finish_discussion(
                        &current,
                        DiscussionStatus::Completed,
                        DiscussionStopReason::MaxRounds,
                        Some("讨论达到最大轮次，已停止。".to_string()),
                    )
                    .await?;
            }
        }
        Ok(())
    }

    async fn judge(
        &self,
        discussion: &Discussion,
        room: &Room,
        moderator: &Agent,
        participants: &[Agent],
        round: i64,
        forced_stop: Option<DiscussionStopReason>,
    ) -> AppResult<Option<ParsedDiscussionJudge>> {
        for _ in 0..2 {
            let history = self.messages.list_by_discussion(discussion.id).await?;
            let compact_prompt = spoke_before(&history, moderator.id)
                && self
                    .chat
                    .has_resumable_session(room.id, moderator)
                    .await?;
            let instruction = build_discussion_judge_instruction(
                room,
                moderator,
                &discussion.topic,
                participants,
                round,
                discussion.max_rounds,
                &history,
                forced_stop,
                compact_prompt,
            );
            let parsed: Arc<Mutex<Option<ParsedDiscussionJudge>>> = Arc::new(Mutex::new(None));
            let finalize = judge_finalize(parsed.clone());
            let discussions = self.discussions.clone();
            let guard_id = discussion.id;
            let run = self
                .chat
                .run_agent_queued_checked(
                    room.id,
                    moderator,
                    &instruction,
                    None,
                    RunAgentOptions {
                        stream: Some(false),
                        skip_room_context: true,
                        discussion: Some(crate::services::chat::DiscussionRef {
                            discussion_id: discussion.id,
                            round,
                        }),
                        finalize: Some(finalize),
                        ..RunAgentOptions::default()
                    },
                    move || {
                        let discussions = discussions.clone();
                        async move {
                            matches!(
                                discussions.get_by_id(guard_id).await,
                                Ok(Some(current)) if current.status == DiscussionStatus::Running
                            )
                        }
                    },
                )
                .await?;

            let Some(run) = run else {
                return Ok(None);
            };
            if run.status == crate::domain::run::RunStatus::Succeeded {
                let judge = parsed
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .take();
                if let Some(judge) = judge {
                    // 裁决运行期间讨论可能已被用户停止；此时不再追加裁决提示。
                    let Some(current) = self.discussions.get_by_id(discussion.id).await? else {
                        return Ok(None);
                    };
                    if current.status != DiscussionStatus::Running {
                        return Ok(None);
                    }
                    if !judge.notes.is_empty() {
                        self.post_discussion_system_message(
                            discussion,
                            &format!("主持人裁决提示：{}", judge.notes.join("；")),
                        )
                        .await?;
                    }
                    return Ok(Some(judge));
                }
            }
        }
        Ok(None)
    }

    async fn load_participants(&self, discussion: &Discussion) -> AppResult<Vec<Agent>> {
        let mut participants = Vec::with_capacity(discussion.participant_ids.len());
        for agent_id in &discussion.participant_ids {
            if let Some(agent) = self.agents.get_by_id(*agent_id).await? {
                participants.push(agent);
            }
        }
        Ok(participants)
    }

    async fn member_of(&self, room: &Room, agent_id: i64) -> AppResult<Option<Agent>> {
        Ok(self
            .rooms
            .list_members(room.id)
            .await?
            .into_iter()
            .find(|member| member.id == agent_id))
    }

    async fn finish_discussion(
        &self,
        discussion: &Discussion,
        status: DiscussionStatus,
        stop_reason: DiscussionStopReason,
        summary: Option<String>,
    ) -> AppResult<Option<Discussion>> {
        let finished = self
            .discussions
            .finish(
                discussion.id,
                FinishDiscussionInput {
                    status,
                    stop_reason,
                    summary,
                },
            )
            .await?;
        let Some(finished) = finished else {
            return Ok(None);
        };
        self.post_discussion_system_message(
            &finished,
            &format!("讨论已结束：{}", stop_reason.label()),
        )
        .await?;
        self.broadcast_changed(&finished);
        Ok(Some(finished))
    }

    async fn fail_discussion(
        &self,
        discussion: &Discussion,
        stop_reason: DiscussionStopReason,
        message: &str,
    ) -> AppResult<()> {
        let failed = self
            .discussions
            .finish(
                discussion.id,
                FinishDiscussionInput {
                    status: DiscussionStatus::Failed,
                    stop_reason,
                    summary: None,
                },
            )
            .await?;
        let Some(failed) = failed else {
            return Ok(());
        };
        self.post_discussion_system_message(&failed, &format!("讨论失败：{message}"))
            .await?;
        self.broadcast_changed(&failed);
        Ok(())
    }

    async fn post_discussion_system_message(
        &self,
        discussion: &Discussion,
        content: &str,
    ) -> AppResult<Message> {
        self.chat
            .post_system_message(
                discussion.room_id,
                content,
                None,
                Some(discussion_meta(discussion, discussion.current_round)),
                None,
            )
            .await
    }

    fn broadcast_changed(&self, discussion: &Discussion) {
        (self.broadcast)(
            discussion.room_id,
            &RoomEvent::DiscussionChanged {
                room_id: discussion.room_id,
                discussion: discussion.clone(),
            },
        );
    }

    async fn require_room(&self, room_id: i64) -> AppResult<Room> {
        self.rooms
            .get_by_id(room_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("房间不存在：{room_id}")))
    }

    async fn require_discussion(&self, discussion_id: i64) -> AppResult<Discussion> {
        self.discussions
            .get_by_id(discussion_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("讨论不存在：{discussion_id}")))
    }

    async fn require_moderator(&self, room: &Room, candidate: &Agent) -> AppResult<Agent> {
        if candidate.builtin_role != Some(crate::domain::agent::BuiltinAgentRole::Moderator) {
            return Err(AppError::constraint("只有内置主持人可以发起讨论"));
        }
        self.require_moderator_by_id(room).await
    }

    async fn require_moderator_by_id(&self, room: &Room) -> AppResult<Agent> {
        let Some(moderator_agent_id) = room.moderator_agent_id else {
            return Err(AppError::constraint("房间未设置主持人，无法发起讨论"));
        };
        if !room.member_ids.contains(&moderator_agent_id) {
            return Err(AppError::constraint(
                "房间主持人不是房间成员，无法发起讨论",
            ));
        }
        match self.agents.get_by_id(moderator_agent_id).await? {
            Some(moderator)
                if moderator.builtin_role
                    == Some(crate::domain::agent::BuiltinAgentRole::Moderator) =>
            {
                Ok(moderator)
            }
            _ => Err(AppError::constraint("房间内置主持人不存在或身份非法")),
        }
    }
}

fn discussion_meta(discussion: &Discussion, round: i64) -> MessageDiscussion {
    MessageDiscussion {
        discussion_id: discussion.id,
        round,
        stance: None,
        reply_to_message_id: None,
        stop_vote: None,
        stop_reason: None,
    }
}

fn turn_finalize(
    agent: Agent,
    participants: Vec<Agent>,
    previous_turns: Vec<Message>,
    parsed: Arc<Mutex<Option<ParsedDiscussionTurn>>>,
) -> FinalizeFn {
    Arc::new(move |raw| {
        let turn = parse_discussion_turn(raw, &agent, &participants);
        let reply_to_message_id = resolve_reply_target(turn.reply_to_agent_id, &previous_turns);
        let patch = DiscussionMetadataPatch {
            stance: Some(Some(turn.stance)),
            reply_to_message_id: Some(reply_to_message_id),
            stop_vote: Some(Some(turn.stop_vote)),
            stop_reason: Some(turn.stop_reason.clone()),
        };
        let content = turn.display.clone();
        *parsed.lock().unwrap_or_else(|error| error.into_inner()) = Some(turn);
        Ok(FinalizedContent {
            content,
            discussion: Some(patch),
        })
    })
}

fn judge_finalize(parsed: Arc<Mutex<Option<ParsedDiscussionJudge>>>) -> FinalizeFn {
    Arc::new(move |raw| {
        let judge = parse_discussion_judge(raw);
        let content = judge.display.clone();
        *parsed.lock().unwrap_or_else(|error| error.into_inner()) = Some(judge);
        Ok(FinalizedContent {
            content,
            discussion: Some(DiscussionMetadataPatch {
                stance: Some(None),
                reply_to_message_id: Some(None),
                stop_vote: Some(None),
                stop_reason: Some(None),
            }),
        })
    })
}

/// 把 replyToAgentId 解析为该参与者在讨论中的最后一条发言 id。
fn resolve_reply_target(reply_to_agent_id: Option<i64>, previous_turns: &[Message]) -> Option<i64> {
    let reply_to_agent_id = reply_to_agent_id?;
    previous_turns
        .iter()
        .rev()
        .find(|message| message.sender_agent_id == Some(reply_to_agent_id))
        .map(|message| message.id)
}

/// 本轮已经产出带立场发言的参与者 id；续跑用它跳过已完成的发言。
fn spoken_agent_ids(messages: &[Message], round: i64) -> HashSet<i64> {
    messages
        .iter()
        .filter_map(|message| {
            if message.sender_type != MessageSenderType::Agent {
                return None;
            }
            let meta = message.discussion.as_ref()?;
            if meta.round != round || meta.stance.is_none() {
                return None;
            }
            message.sender_agent_id
        })
        .collect()
}

fn parse_id(raw: &str, field: &str) -> AppResult<i64> {
    let parsed = if !raw.is_empty() && raw.bytes().all(|byte| byte.is_ascii_digit()) {
        raw.parse::<i64>()
            .ok()
            .filter(|id| *id > 0 && *id <= 9_007_199_254_740_991)
    } else {
        None
    };
    parsed.ok_or_else(|| AppError::validation(format!("非法的 {field}：{raw}")))
}

fn clip(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut clipped: String = value.chars().take(max_chars.saturating_sub(1)).collect();
    clipped.push('…');
    clipped
}

fn read_trimmed_string(value: Option<&Value>, field: &str, max_chars: usize) -> Result<String, String> {
    let Some(text) = value.and_then(Value::as_str) else {
        return Err(format!("{field} 必须是非空字符串"));
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(format!("{field} 必须是非空字符串"));
    }
    if trimmed.chars().count() > max_chars {
        return Err(format!("{field} 不能超过 {max_chars} 字符"));
    }
    Ok(trimmed.to_string())
}

/// 解析主持人 `moon-discussion` 块；不合法时由调用方降级为可见提示。
pub fn parse_moderator_discussion_draft(
    value: &Value,
    moderator: &Agent,
    members: &[Agent],
) -> Result<ModeratorDiscussionDraft, String> {
    let Value::Object(map) = value else {
        return Err("moon-discussion 块必须是 JSON 对象".to_string());
    };
    let topic = read_trimmed_string(
        map.get("topic"),
        "moon-discussion.topic",
        DISCUSSION_TOPIC_CHARS,
    )?;

    let Some(Value::Array(raw_ids)) = map.get("participantIds") else {
        return Err(format!(
            "moon-discussion.participantIds 必须包含 {MIN_DISCUSSION_PARTICIPANTS}..{MAX_DISCUSSION_PARTICIPANTS} 位成员"
        ));
    };
    if raw_ids.len() < MIN_DISCUSSION_PARTICIPANTS || raw_ids.len() > MAX_DISCUSSION_PARTICIPANTS {
        return Err(format!(
            "moon-discussion.participantIds 必须包含 {MIN_DISCUSSION_PARTICIPANTS}..{MAX_DISCUSSION_PARTICIPANTS} 位成员"
        ));
    }

    let member_ids: HashSet<i64> = members.iter().map(|member| member.id).collect();
    let mut seen = HashSet::new();
    let mut participant_ids = Vec::with_capacity(raw_ids.len());
    for (index, item) in raw_ids.iter().enumerate() {
        let Some(agent_id) = item.as_i64().filter(|id| *id > 0) else {
            return Err(format!(
                "moon-discussion.participantIds[{index}] 必须是成员 id"
            ));
        };
        if agent_id == moderator.id {
            return Err("主持人不能作为讨论参与者".to_string());
        }
        if !member_ids.contains(&agent_id) {
            return Err(format!(
                "moon-discussion.participantIds[{index}] 不是房间成员"
            ));
        }
        if !seen.insert(agent_id) {
            return Err("moon-discussion.participantIds 不能重复".to_string());
        }
        participant_ids.push(agent_id);
    }

    let max_rounds = match map.get("maxRounds") {
        None => None,
        Some(value) => {
            let Some(rounds) = value.as_i64() else {
                return Err(format!(
                    "moon-discussion.maxRounds 必须是 1..{MAX_DISCUSSION_ROUNDS} 的整数"
                ));
            };
            if !(1..=MAX_DISCUSSION_ROUNDS).contains(&rounds) {
                return Err(format!(
                    "moon-discussion.maxRounds 必须是 1..{MAX_DISCUSSION_ROUNDS} 的整数"
                ));
            }
            Some(rounds)
        }
    };

    Ok(ModeratorDiscussionDraft {
        topic,
        participant_ids,
        max_rounds,
    })
}

/// 解析参与者轮次块；协议缺失或字段非法时降级，而不是让整次运行失败。
pub fn parse_discussion_turn(
    raw: &str,
    agent: &Agent,
    participants: &[Agent],
) -> ParsedDiscussionTurn {
    let display_raw = strip_blocks(raw, &["moon-discussion-turn"]);
    let display = if display_raw.is_empty() {
        "（本轮未提供可见发言）".to_string()
    } else {
        display_raw
    };
    let mut notes = Vec::new();

    let Some(body) = extract_last_block(raw, "moon-discussion-turn") else {
        return ParsedDiscussionTurn {
            display,
            stance: DiscussionStance::Propose,
            reply_to_agent_id: None,
            stop_vote: DiscussionStopVote::Continue,
            stop_reason: None,
            notes: vec!["未输出 moon-discussion-turn 协议块，按继续处理".to_string()],
        };
    };

    let value: Value = match serde_json::from_str(&body) {
        Ok(value) => value,
        Err(_) => {
            return ParsedDiscussionTurn {
                display,
                stance: DiscussionStance::Propose,
                reply_to_agent_id: None,
                stop_vote: DiscussionStopVote::Continue,
                stop_reason: None,
                notes: vec!["moon-discussion-turn 块不是合法 JSON，按继续处理".to_string()],
            };
        }
    };
    let Value::Object(value) = value else {
        return ParsedDiscussionTurn {
            display,
            stance: DiscussionStance::Propose,
            reply_to_agent_id: None,
            stop_vote: DiscussionStopVote::Continue,
            stop_reason: None,
            notes: vec!["moon-discussion-turn 块必须是 JSON 对象，按继续处理".to_string()],
        };
    };

    let stance = match value.get("stance") {
        Some(Value::String(raw)) => match DiscussionStance::parse(raw) {
            Some(stance) => stance,
            None => {
                notes.push("stance 非法，按 propose 处理".to_string());
                DiscussionStance::Propose
            }
        },
        Some(_) => {
            notes.push("stance 非法，按 propose 处理".to_string());
            DiscussionStance::Propose
        }
        None => DiscussionStance::Propose,
    };

    let participant_ids: HashSet<i64> = participants.iter().map(|member| member.id).collect();
    let reply_to_agent_id = match value.get("replyToAgentId") {
        None | Some(Value::Null) => None,
        Some(raw) => {
            let candidate = raw.as_i64().filter(|id| *id > 0);
            match candidate {
                Some(id) if id != agent.id && participant_ids.contains(&id) => Some(id),
                _ => {
                    notes.push("replyToAgentId 不是其他讨论参与者，已忽略".to_string());
                    None
                }
            }
        }
    };

    let stop_vote = match value.get("stopVote") {
        Some(Value::String(raw)) => match DiscussionStopVote::parse(raw) {
            Some(vote) => vote,
            None => {
                notes.push("stopVote 非法，按 continue 处理".to_string());
                DiscussionStopVote::Continue
            }
        },
        Some(_) => {
            notes.push("stopVote 非法，按 continue 处理".to_string());
            DiscussionStopVote::Continue
        }
        None => DiscussionStopVote::Continue,
    };

    let stop_reason = match value.get("stopReason") {
        None | Some(Value::Null) => None,
        Some(Value::String(raw)) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(clip(trimmed, DISCUSSION_REASON_CHARS))
            }
        }
        Some(_) => {
            notes.push("stopReason 非法，已忽略".to_string());
            None
        }
    };

    ParsedDiscussionTurn {
        display,
        stance,
        reply_to_agent_id,
        stop_vote,
        stop_reason,
        notes,
    }
}

fn read_string_array(
    value: Option<&Value>,
    field: &str,
    max_items: usize,
    max_chars: usize,
) -> Result<Vec<String>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let Value::Array(items) = value else {
        return Err(format!("{field} 必须是字符串数组"));
    };
    let mut values = Vec::with_capacity(items.len().min(max_items));
    for (index, item) in items.iter().take(max_items).enumerate() {
        let Some(text) = item.as_str() else {
            return Err(format!("{field}[{index}] 必须是非空字符串"));
        };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err(format!("{field}[{index}] 必须是非空字符串"));
        }
        values.push(clip(trimmed, max_chars));
    }
    Ok(values)
}

/// 解析主持人裁决块；缺失或非法时按 continue 降级，最大轮次仍会兜底。
pub fn parse_discussion_judge(raw: &str) -> ParsedDiscussionJudge {
    let display_raw = strip_blocks(raw, &["moon-discussion-judge"]);
    let fallback_display = if display_raw.is_empty() {
        "主持人未给出可解析的裁决，讨论继续。".to_string()
    } else {
        display_raw.clone()
    };

    let Some(body) = extract_last_block(raw, "moon-discussion-judge") else {
        return ParsedDiscussionJudge {
            display: fallback_display,
            decision: "continue".to_string(),
            reason: "主持人未输出 moon-discussion-judge 协议块".to_string(),
            summary: None,
            consensus: Vec::new(),
            disagreements: Vec::new(),
            notes: vec!["未输出 moon-discussion-judge 协议块，按继续处理".to_string()],
        };
    };
    let value: Value = match serde_json::from_str(&body) {
        Ok(value) => value,
        Err(_) => {
            return ParsedDiscussionJudge {
                display: fallback_display,
                decision: "continue".to_string(),
                reason: "主持人裁决块不是合法 JSON".to_string(),
                summary: None,
                consensus: Vec::new(),
                disagreements: Vec::new(),
                notes: vec!["moon-discussion-judge 块不是合法 JSON，按继续处理".to_string()],
            };
        }
    };
    let Value::Object(value) = value else {
        return ParsedDiscussionJudge {
            display: fallback_display,
            decision: "continue".to_string(),
            reason: "主持人裁决块不是 JSON 对象".to_string(),
            summary: None,
            consensus: Vec::new(),
            disagreements: Vec::new(),
            notes: vec!["moon-discussion-judge 块必须是 JSON 对象，按继续处理".to_string()],
        };
    };

    let mut notes = Vec::new();
    let decision = match value.get("decision").and_then(Value::as_str) {
        Some("stop") => Some("stop".to_string()),
        Some("continue") => Some("continue".to_string()),
        _ => {
            notes.push("decision 非法，按 continue 处理".to_string());
            None
        }
    };

    let reason = match value.get("reason") {
        Some(Value::String(raw)) if !raw.trim().is_empty() => {
            clip(raw.trim(), DISCUSSION_REASON_CHARS)
        }
        Some(_) => {
            notes.push("reason 非法，已使用默认理由".to_string());
            "主持人未提供裁决理由".to_string()
        }
        None => "主持人未提供裁决理由".to_string(),
    };

    let summary = match value.get("summary") {
        None | Some(Value::Null) => None,
        Some(Value::String(raw)) if !raw.trim().is_empty() => {
            Some(clip(raw.trim(), DISCUSSION_SUMMARY_CHARS))
        }
        Some(_) => {
            notes.push("summary 非法，已忽略".to_string());
            None
        }
    };

    let consensus = match read_string_array(value.get("consensus"), "consensus", 10, 500) {
        Ok(values) => values,
        Err(message) => {
            notes.push(message);
            Vec::new()
        }
    };
    let disagreements =
        match read_string_array(value.get("disagreements"), "disagreements", 10, 500) {
            Ok(values) => values,
            Err(message) => {
                notes.push(message);
                Vec::new()
            }
        };

    let final_decision = decision.unwrap_or_else(|| "continue".to_string());
    let display = if final_decision == "stop" && summary.is_some() {
        compose_conclusion_display(
            summary.as_deref().unwrap_or_default(),
            &consensus,
            &disagreements,
        )
    } else if display_raw.is_empty() {
        format!(
            "主持人裁决：{}——{}",
            if final_decision == "stop" { "停止" } else { "继续" },
            reason
        )
    } else {
        display_raw
    };

    ParsedDiscussionJudge {
        display,
        decision: final_decision,
        reason,
        summary,
        consensus,
        disagreements,
        notes,
    }
}

/// 讨论结论展示文本：summary 作为首段，共识/分歧转成 Markdown 列表。
///
/// 旧版只把 summary 拼成一行，长结论在消息气泡里是一整块文字墙；结构化
/// 输出让前端 Markdown 渲染出清晰层级，同时去掉 summary 里重复的「结论：」。
fn compose_conclusion_display(
    summary: &str,
    consensus: &[String],
    disagreements: &[String],
) -> String {
    let body = summary.trim();
    let body = body.strip_prefix("结论：").unwrap_or(body);
    let mut output = format!("讨论结论：{body}");
    if !consensus.is_empty() {
        output.push_str("\n\n**共识**");
        for item in consensus {
            output.push_str(&format!("\n- {item}"));
        }
    }
    if !disagreements.is_empty() {
        output.push_str("\n\n**分歧**");
        for item in disagreements {
            output.push_str(&format!("\n- {item}"));
        }
    }
    output
}

fn stance_label(stance: DiscussionStance) -> &'static str {
    match stance {
        DiscussionStance::Propose => "提出",
        DiscussionStance::Support => "支持",
        DiscussionStance::Oppose => "反对",
        DiscussionStance::Question => "质疑",
    }
}

/// 只包含参与者轮次（stance 非空），不把裁决/系统消息混入辩论上下文。
pub fn format_discussion_transcript(
    history: &[Message],
    member_names: &HashMap<i64, String>,
    limit: usize,
) -> String {
    format_discussion_transcript_after(
        history,
        member_names,
        None,
        limit,
        "（暂无讨论发言）",
    )
}

/// 只格式化 `after_seq` 之后（严格大于）的参与者发言。
fn format_discussion_transcript_after(
    history: &[Message],
    member_names: &HashMap<i64, String>,
    after_seq: Option<i64>,
    limit: usize,
    empty_hint: &str,
) -> String {
    let turns: Vec<&Message> = history
        .iter()
        .filter(|message| {
            message.sender_type == MessageSenderType::Agent
                && message
                    .discussion
                    .as_ref()
                    .and_then(|meta| meta.stance)
                    .is_some()
                && after_seq.map_or(true, |seq| message.seq > seq)
        })
        .collect();
    let recent = &turns[turns.len().saturating_sub(limit)..];
    if recent.is_empty() {
        return empty_hint.to_string();
    }

    let by_id: HashMap<i64, &Message> = history.iter().map(|m| (m.id, m)).collect();
    let mut lines = Vec::with_capacity(recent.len());
    for message in recent {
        let Some(meta) = &message.discussion else {
            continue;
        };
        let name = message
            .sender_agent_id
            .map(|id| {
                member_names
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| format!("智能体#{id}"))
            })
            .unwrap_or_else(|| "智能体".to_string());
        let reply_name = meta
            .reply_to_message_id
            .and_then(|id| by_id.get(&id).copied())
            .and_then(|target| target.sender_agent_id)
            .map(|id| {
                member_names
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| format!("智能体#{id}"))
            });
        let reply = reply_name
            .map(|name| format!("，回复 @{name}"))
            .unwrap_or_default();
        let vote = if meta.stop_vote == Some(DiscussionStopVote::Stop) {
            "，投票停止"
        } else {
            "，投票继续"
        };
        let stop_reason = if meta.stop_vote == Some(DiscussionStopVote::Stop) {
            meta.stop_reason
                .as_ref()
                .map(|reason| format!("（理由：{}）", clip(reason, 200)))
                .unwrap_or_default()
        } else {
            String::new()
        };
        lines.push(format!(
            "#{} 第{}轮 {}（{}{}{}{}）：{}",
            message.seq,
            meta.round,
            name,
            stance_label(meta.stance.unwrap_or(DiscussionStance::Propose)),
            reply,
            vote,
            stop_reason,
            clip(&message.content, DISCUSSION_MESSAGE_CHARS)
        ));
    }
    lines.join("\n")
}

fn participant_lines(participants: &[Agent]) -> String {
    if participants.is_empty() {
        return "（无参与者）".to_string();
    }
    participants
        .iter()
        .map(|participant| {
            format!(
                "- id={} 名称：{}｜人设：{}",
                participant.id, participant.name, participant.persona
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 增量提示词只需要 id 与称呼：人设已在会话上下文中，重复会挤占有效上下文。
fn participant_brief_lines(participants: &[Agent], exclude_id: i64) -> String {
    let lines: Vec<String> = participants
        .iter()
        .filter(|participant| participant.id != exclude_id)
        .map(|participant| format!("- id={} 名称：{}", participant.id, participant.name))
        .collect();
    if lines.is_empty() {
        return "（无其他参与者）".to_string();
    }
    lines.join("\n")
}

/// 本场讨论中该智能体是否已有完成的消息（发言或裁决）。
///
/// 有则说明完整提示词至少进过一次同一 harness 会话，后续可改发增量。
fn spoke_before(history: &[Message], agent_id: i64) -> bool {
    history.iter().any(|message| {
        message.sender_type == MessageSenderType::Agent
            && message.sender_agent_id == Some(agent_id)
            && message.status == MessageStatus::Final
    })
}

fn last_final_seq(history: &[Message], agent_id: i64) -> Option<i64> {
    history
        .iter()
        .rev()
        .find(|message| {
            message.sender_type == MessageSenderType::Agent
                && message.sender_agent_id == Some(agent_id)
                && message.status == MessageStatus::Final
        })
        .map(|message| message.seq)
}

fn member_names_of(participants: &[Agent], extra: &[Agent]) -> HashMap<i64, String> {
    participants
        .iter()
        .chain(extra.iter())
        .map(|agent| (agent.id, agent.name.clone()))
        .collect()
}

fn discussion_turn_protocol() -> String {
    [
        "本轮要求：".to_string(),
        "- 针对主题给出你的判断，可以提出方案、支持、反对或质疑。".to_string(),
        "- 如果反对或质疑，请明确指向某位参与者，并说明你不同意哪一点。".to_string(),
        "- 不要重复已有观点；补充新论据、约束、风险或可验证的下一步。".to_string(),
        "- 如果认为讨论已经可以结束，或无法继续推进，将 stopVote 设为 stop 并写清理由。".to_string(),
        String::new(),
        "输出格式：先写自然语言正文，再追加一个 ```moon-discussion-turn 代码块：".to_string(),
        "{\"stance\":\"propose|support|oppose|question\",\"replyToAgentId\":2,\"stopVote\":\"continue|stop\",\"stopReason\":\"不超过 1000 字\"}".to_string(),
        String::new(),
        "规则：".to_string(),
        "- stance 必须是 propose / support / oppose / question 之一。".to_string(),
        "- replyToAgentId 可省略；只能指向其他讨论参与者，且只能回复本轮之前已发言的人。".to_string(),
        "- stopVote 必须是 continue / stop；stopReason 在 stop 时必填。".to_string(),
        "- 不要创建任务、不要委派、不要发起新讨论、不要输出思考过程。".to_string(),
        "- 只输出本轮发言正文与一个协议块，不要复述讨论记录。".to_string(),
        format!("- {CONCISE_REPLY_RULE}"),
    ]
    .join("\n")
}

/// 构造单个参与者的讨论发言提示词；不包含 provider/modelConfig。
///
/// `compact` 为真时假定本场讨论的完整背景已在复用会话中，只发增量：
/// 主题、轮次、其他参与者 id/称呼、上次发言之后的新发言与协议要求。
pub fn build_discussion_turn_instruction(
    room: &Room,
    agent: &Agent,
    topic: &str,
    participants: &[Agent],
    round: i64,
    max_rounds: i64,
    history: &[Message],
    compact: bool,
) -> String {
    let names = member_names_of(participants, std::slice::from_ref(agent));

    if compact {
        let transcript = format_discussion_transcript_after(
            history,
            &names,
            last_final_seq(history, agent.id),
            DISCUSSION_TRANSCRIPT_LIMIT,
            "（你上次发言之后暂无新发言）",
        );
        return [
            format!(
                "你是房间「{}」讨论参与者「{}」，本场讨论继续。",
                room.name, agent.name
            ),
            format!("讨论主题：{topic}"),
            format!("当前轮次：第 {round} / {max_rounds} 轮"),
            String::new(),
            "其他参与者（replyToAgentId 只能指向他们）：".to_string(),
            participant_brief_lines(participants, agent.id),
            String::new(),
            "你上次发言之后的新发言（完整人设与此前记录已在会话上下文中）：".to_string(),
            transcript,
            String::new(),
            discussion_turn_protocol(),
        ]
        .join("\n");
    }

    let transcript = format_discussion_transcript(history, &names, DISCUSSION_TRANSCRIPT_LIMIT);
    [
        format!(
            "你是房间「{}」中的讨论参与者「{}」。",
            room.name, agent.name
        ),
        format!("人设：{}", agent.persona),
        String::new(),
        format!("讨论主题：{topic}"),
        format!("当前轮次：第 {round} / {max_rounds} 轮"),
        String::new(),
        "讨论参与者（只能与他们讨论，不得创建新讨论）：".to_string(),
        participant_lines(participants),
        String::new(),
        format!(
            "已有讨论发言（按时间顺序，最多 {DISCUSSION_TRANSCRIPT_LIMIT} 条）："
        ),
        transcript,
        String::new(),
        discussion_turn_protocol(),
    ]
    .join("\n")
}

fn discussion_judge_protocol(forced: &str) -> String {
    [
        "裁决规则：".to_string(),
        "- 如果已有明确共识、关键分歧无法通过继续讨论收敛，或继续只会重复观点，输出 stop。".to_string(),
        "- 如果仍存在未被回答的问题，且下一轮可能产生新信息，输出 continue。".to_string(),
        format!("- {forced}"),
        String::new(),
        "输出格式：先写一句给用户看的简短裁决，再追加一个 ```moon-discussion-judge 代码块：".to_string(),
        "{\"decision\":\"continue|stop\",\"reason\":\"裁决理由\",\"summary\":\"停止时的结论\",\"consensus\":[\"共识\"],\"disagreements\":[\"分歧\"]}".to_string(),
        String::new(),
        "规则：".to_string(),
        "- decision 必须是 continue / stop；reason 必须是非空字符串。".to_string(),
        "- decision 为 stop 时 summary 必须给出可执行结论：用 Markdown，首句写结论，随后按要点分段；需要列点时每行以 `- ` 开头，不要写成一整段。".to_string(),
        "- consensus / disagreements 是字符串数组，各最多 10 条；没有时传空数组。".to_string(),
        "- 不要编造未在讨论中出现的观点，不要输出思考过程。".to_string(),
    ]
    .join("\n")
}

/// 构造主持人裁决提示词；forced_stop 非空时要求必须停止并总结。
///
/// `compact` 与参与者发言同理：复用会话且已裁决过时，只发上次裁决之后
/// 的新发言与裁决要求。
pub fn build_discussion_judge_instruction(
    room: &Room,
    moderator: &Agent,
    topic: &str,
    participants: &[Agent],
    round: i64,
    max_rounds: i64,
    history: &[Message],
    forced_stop: Option<DiscussionStopReason>,
    compact: bool,
) -> String {
    let names = member_names_of(participants, std::slice::from_ref(moderator));
    let forced = match forced_stop {
        Some(DiscussionStopReason::AllVotedStop) => {
            "本轮所有参与者都投票停止，你必须在裁决中同意停止并给出总结。"
        }
        Some(DiscussionStopReason::MaxRounds) => {
            "讨论已达到最大轮次，你必须在裁决中同意停止并给出总结。"
        }
        _ => "请判断是否已经形成足够结论，或继续讨论是否能产生新信息。",
    };
    let protocol = discussion_judge_protocol(forced);

    if compact {
        let transcript = format_discussion_transcript_after(
            history,
            &names,
            last_final_seq(history, moderator.id),
            DISCUSSION_JUDGE_TRANSCRIPT_LIMIT,
            "（你上次裁决之后暂无新发言）",
        );
        return [
            format!(
                "你是房间「{}」的中立主持人「{}」，继续裁决本场讨论。",
                room.name, moderator.name
            ),
            "你不参与辩论，只负责裁决讨论是否继续，并在停止时总结共识与分歧。".to_string(),
            String::new(),
            format!("讨论主题：{topic}"),
            format!("当前轮次：第 {round} / {max_rounds} 轮"),
            String::new(),
            "参与者（不含你自己）：".to_string(),
            participant_brief_lines(participants, moderator.id),
            String::new(),
            "你上次裁决之后的新发言（完整主题与此前记录已在会话上下文中）：".to_string(),
            transcript,
            String::new(),
            protocol,
        ]
        .join("\n");
    }

    let transcript =
        format_discussion_transcript(history, &names, DISCUSSION_JUDGE_TRANSCRIPT_LIMIT);
    [
        format!(
            "你是房间「{}」的中立主持人「{}」。",
            room.name, moderator.name
        ),
        "你不参与辩论，只负责裁决讨论是否继续，并在停止时总结共识与分歧。".to_string(),
        String::new(),
        format!("讨论主题：{topic}"),
        format!("当前轮次：第 {round} / {max_rounds} 轮"),
        String::new(),
        "参与者（不含你自己）：".to_string(),
        participant_lines(participants),
        String::new(),
        format!(
            "本轮及此前的讨论发言（最多 {DISCUSSION_JUDGE_TRANSCRIPT_LIMIT} 条）："
        ),
        transcript,
        String::new(),
        protocol,
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::{BuiltinAgentRole, HarnessKind};

    fn agent(id: i64, name: &str, role: Option<BuiltinAgentRole>) -> Agent {
        Agent {
            id,
            name: name.to_string(),
            persona: format!("{name} 的人设"),
            harness: HarnessKind::Fake,
            model_config: None,
            builtin_role: role,
            custom_harness_id: None,
            workspace_dir: None,
            created_at: 1,
        }
    }

    fn room() -> Room {
        Room {
            id: 1,
            name: "讨论房间".to_string(),
            workspace_dir: ".".to_string(),
            workspace_dirs: vec![".".to_string()],
            moderator_agent_id: None,
            member_ids: vec![2, 3],
            reply_mode: crate::domain::room::RoomReplyMode::Moderator,
            discussion_max_rounds: 2,
            permission_mode: crate::domain::room::RoomPermissionMode::Auto,
            created_at: 1,
            last_activity_at: None,
        }
    }

    fn turn_message(seq: i64, agent_id: i64, content: &str) -> Message {
        Message {
            id: seq,
            room_id: 1,
            seq,
            sender_type: MessageSenderType::Agent,
            sender_agent_id: Some(agent_id),
            status: MessageStatus::Final,
            kind: crate::domain::message::MessageKind::Reply,
            run_id: None,
            content: content.to_string(),
            task_id: None,
            reply_to_message_id: None,
            discussion: Some(MessageDiscussion {
                discussion_id: 1,
                round: 1,
                stance: Some(DiscussionStance::Propose),
                reply_to_message_id: None,
                stop_vote: Some(DiscussionStopVote::Continue),
                stop_reason: None,
            }),
            collaboration: None,
            attachments: Vec::new(),
            reactions: Vec::new(),
            created_at: 1,
        }
    }

    fn judge_message(seq: i64, agent_id: i64, content: &str) -> Message {
        let mut message = turn_message(seq, agent_id, content);
        if let Some(discussion) = &mut message.discussion {
            discussion.stance = None;
        }
        message
    }

    #[test]
    fn turn_protocol_degrades_and_preserves_fields() {
        let participant = agent(2, "成员", None);
        let parsed = parse_discussion_turn(
            "正文\n```moon-discussion-turn\n{\"stance\":\"oppose\",\"replyToAgentId\":3,\"stopVote\":\"stop\",\"stopReason\":\"没有新信息\"}\n```",
            &participant,
            &[participant.clone(), agent(3, "另一位", None)],
        );
        assert_eq!(parsed.display, "正文");
        assert_eq!(parsed.stance, DiscussionStance::Oppose);
        assert_eq!(parsed.reply_to_agent_id, Some(3));
        assert_eq!(parsed.stop_vote, DiscussionStopVote::Stop);
        assert_eq!(parsed.stop_reason.as_deref(), Some("没有新信息"));

        let degraded = parse_discussion_turn("只有正文", &participant, &[participant.clone()]);
        assert_eq!(degraded.stance, DiscussionStance::Propose);
        assert_eq!(degraded.stop_vote, DiscussionStopVote::Continue);
        assert_eq!(degraded.display, "只有正文");
        assert_eq!(degraded.notes.len(), 1);
    }

    #[test]
    fn judge_protocol_detects_stop_and_summary() {
        let judge = parse_discussion_judge(
            "裁决\n```moon-discussion-judge\n{\"decision\":\"stop\",\"reason\":\"已收敛\",\"summary\":\"结论：采用方案 A\",\"consensus\":[\"先小流量\"],\"disagreements\":[\"灰度范围待定\"]}\n```",
        );
        assert_eq!(judge.decision, "stop");
        assert_eq!(
            judge.display,
            "讨论结论：采用方案 A\n\n**共识**\n- 先小流量\n\n**分歧**\n- 灰度范围待定"
        );
        assert_eq!(judge.summary.as_deref(), Some("结论：采用方案 A"));
        assert_eq!(judge.consensus, vec!["先小流量"]);

        let degraded = parse_discussion_judge("没有协议块");
        assert_eq!(degraded.decision, "continue");
        assert!(degraded.notes[0].contains("未输出"));
    }

    #[test]
    fn compact_turn_instruction_only_sends_new_turns() {
        let first = agent(2, "甲", None);
        let second = agent(3, "乙", None);
        let participants = [first.clone(), second.clone()];
        let history = [
            turn_message(1, first.id, "甲的旧发言"),
            turn_message(2, second.id, "乙的新发言"),
        ];

        let compact = build_discussion_turn_instruction(
            &room(),
            &first,
            "是否上线",
            &participants,
            2,
            2,
            &history,
            true,
        );
        assert!(compact.contains("本场讨论继续"));
        assert!(compact.contains("你上次发言之后的新发言"));
        assert!(compact.contains("乙的新发言"));
        assert!(!compact.contains("甲的旧发言"));
        assert!(!compact.contains("甲 的人设"), "增量提示词不应重复人设");
        assert!(compact.contains("id=3 名称：乙"));
        assert!(compact.contains("moon-discussion-turn"));

        let full = build_discussion_turn_instruction(
            &room(),
            &first,
            "是否上线",
            &participants,
            2,
            2,
            &history,
            false,
        );
        assert!(full.contains("甲 的人设"));
        assert!(full.contains("甲的旧发言"));
        assert!(full.contains("乙的新发言"));
        assert!(full.len() > compact.len());
    }

    #[test]
    fn compact_judge_instruction_only_sends_turns_after_last_judge() {
        let first = agent(2, "甲", None);
        let second = agent(3, "乙", None);
        let moderator = agent(1, "主持人", None);
        let participants = [first.clone(), second.clone()];
        let history = [
            turn_message(1, first.id, "甲的旧发言"),
            judge_message(2, moderator.id, "上轮裁决"),
            turn_message(3, second.id, "乙的新发言"),
        ];

        let compact = build_discussion_judge_instruction(
            &room(),
            &moderator,
            "是否上线",
            &participants,
            2,
            2,
            &history,
            None,
            true,
        );
        assert!(compact.contains("继续裁决本场讨论"));
        assert!(compact.contains("你上次裁决之后的新发言"));
        assert!(compact.contains("乙的新发言"));
        assert!(!compact.contains("甲的旧发言"));
        assert!(!compact.contains("甲 的人设"), "增量提示词不应重复人设");
        assert!(compact.contains("moon-discussion-judge"));

        let full = build_discussion_judge_instruction(
            &room(),
            &moderator,
            "是否上线",
            &participants,
            2,
            2,
            &history,
            None,
            false,
        );
        assert!(full.contains("甲 的人设"));
        assert!(full.len() > compact.len());
    }
}
