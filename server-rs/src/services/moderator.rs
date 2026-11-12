//! 主持人普通聊天协议：提示词组装与结构化动作提取。
//!
//! 主持人是普通房间成员，运行仍走 `ChatService`；本模块只做纯逻辑：
//! - 把成员、最近对话与用户请求组装成指令；
//! - 从模型原始文本中提取 `moon-task` / `moon-discussion` / `moon-delegation`
//!   代码块，剥离后返回展示文本；非法块只降级为可见提示，不抛给 HTTP 层。

use serde_json::Value;

use crate::domain::agent::Agent;
use crate::domain::discussion::{
    ModeratorDiscussionDraft, DEFAULT_DISCUSSION_ROUNDS, DISCUSSION_TOPIC_CHARS,
    MIN_DISCUSSION_PARTICIPANTS,
};
use crate::domain::message::Message;
use crate::domain::room::Room;
use crate::domain::task::TaskPlan;
use crate::services::context_compaction::{
    compact_transcript, CompactionOptions, LabeledMessage,
};
use crate::services::discussion::parse_moderator_discussion_draft;
use crate::services::message_artifacts::{compact_message_line, context_messages};
use crate::services::task_plan::parse_task_plan;

/// 一次普通聊天回合最多委派的成员数；固定值，不做可配置化。
pub const MAX_DELEGATION_ASSIGNMENTS: usize = 5;
/// 注入主持人普通聊天提示词的最近消息条数。
pub const MODERATOR_CHAT_CONTEXT_LIMIT: usize = 20;

pub const MAX_INSTRUCTION_CHARS: usize = 4_000;
pub const MAX_SUMMARY_CHARS: usize = 2_000;
pub const MAX_TASK_TITLE_CHARS: usize = 200;
pub const MAX_TASK_DESCRIPTION_CHARS: usize = 8_000;

/// 面向执行型智能体的通用回复约束。
pub const CONCISE_REPLY_RULE: &str = "回复保持精简：先给结论，通常不超过 800 字，只保留关键证据与下一步；用户明确要求详细报告或完整方案时才展开，并把超出部分写入工作区文件，在回复中给出文件路径。";

#[derive(Debug, Clone, PartialEq)]
pub struct ChatAssignment {
    pub agent_id: i64,
    pub instruction: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModeratorDelegation {
    pub summary: String,
    pub assignments: Vec<ChatAssignment>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModeratorTaskDraft {
    pub title: String,
    pub description: String,
    pub plan: TaskPlan,
    pub auto_start: bool,
}

#[derive(Debug, Clone)]
pub enum ModeratorChatAction {
    None,
    Delegate(ModeratorDelegation),
    Discussion(ModeratorDiscussionDraft),
    Task(ModeratorTaskDraft),
}

#[derive(Debug, Clone)]
pub struct ModeratorChatExtraction {
    /// 剥离所有结构化块后的展示文本。
    pub text: String,
    pub action: ModeratorChatAction,
    /// 需要以系统消息展示的降级/忽略原因。
    pub notes: Vec<String>,
}

/// 扫描 ``` 围栏代码块，保留位置以便整体剥离。
#[derive(Debug, Clone)]
pub(crate) struct FenceBlock {
    pub info: String,
    pub body: String,
    pub start: usize,
    pub end: usize,
}

pub(crate) fn scan_fence_blocks(raw: &str) -> Vec<FenceBlock> {
    let mut blocks = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = raw[cursor..].find("```") {
        let start = cursor + relative;
        let after = start + 3;
        let Some(info_end_relative) = raw[after..].find('\n') else {
            break;
        };
        let info_end = after + info_end_relative;
        let info = raw[after..info_end]
            .trim_end_matches('\r')
            .trim()
            .to_lowercase();
        let body_start = info_end + 1;
        let Some(close_relative) = raw[body_start..].find("```") else {
            break;
        };
        let end = body_start + close_relative + 3;
        blocks.push(FenceBlock {
            info,
            body: raw[body_start..body_start + close_relative].to_string(),
            start,
            end,
        });
        cursor = end;
    }
    blocks
}

pub(crate) fn extract_last_block(raw: &str, kind: &str) -> Option<String> {
    scan_fence_blocks(raw)
        .into_iter()
        .rev()
        .find(|block| block.info == kind)
        .map(|block| block.body)
}

/// 删除指定类型的围栏块，其余文本 trim 后返回。
pub(crate) fn strip_blocks(raw: &str, kinds: &[&str]) -> String {
    let blocks = scan_fence_blocks(raw);
    let mut output = String::with_capacity(raw.len());
    let mut cursor = 0;
    for block in blocks {
        if !kinds.contains(&block.info.as_str()) {
            continue;
        }
        output.push_str(&raw[cursor..block.start]);
        cursor = block.end;
    }
    output.push_str(&raw[cursor..]);
    output.trim().to_string()
}

fn is_positive_id(value: &Value) -> Option<i64> {
    if let Some(id) = value.as_i64() {
        return (id > 0).then_some(id);
    }
    value
        .as_f64()
        .filter(|number| number.is_finite() && number.fract() == 0.0 && *number > 0.0)
        .filter(|number| *number <= 9_007_199_254_740_991_f64)
        .map(|number| number as i64)
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

fn parse_delegation(
    body: &str,
    moderator: &Agent,
    members: &[Agent],
) -> Result<ModeratorDelegation, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|_| "moon-delegation 块不是合法 JSON".to_string())?;
    let Value::Object(map) = value else {
        return Err("moon-delegation 块必须是 JSON 对象".to_string());
    };

    let Some(Value::Array(raw_assignments)) = map.get("assignments") else {
        return Err(format!(
            "moon-delegation.assignments 必须包含 1..{MAX_DELEGATION_ASSIGNMENTS} 条"
        ));
    };
    if raw_assignments.is_empty() || raw_assignments.len() > MAX_DELEGATION_ASSIGNMENTS {
        return Err(format!(
            "moon-delegation.assignments 必须包含 1..{MAX_DELEGATION_ASSIGNMENTS} 条"
        ));
    }

    let mut seen = std::collections::HashSet::new();
    let mut assignments = Vec::with_capacity(raw_assignments.len());
    for (index, item) in raw_assignments.iter().enumerate() {
        let Value::Object(item) = item else {
            return Err(format!("assignments[{index}] 必须是对象"));
        };
        let Some(agent_id) = item.get("agentId").and_then(is_positive_id) else {
            return Err(format!("assignments[{index}].agentId 必须是成员 id"));
        };
        if agent_id == moderator.id {
            return Err("主持人不能指派自己".to_string());
        }
        if !members.iter().any(|member| member.id == agent_id) {
            return Err(format!("assignments[{index}].agentId 不是房间成员"));
        }
        if !seen.insert(agent_id) {
            return Err(format!("assignments[{index}].agentId 重复"));
        }
        assignments.push(ChatAssignment {
            agent_id,
            instruction: read_trimmed_string(
                item.get("instruction"),
                &format!("assignments[{index}].instruction"),
                MAX_INSTRUCTION_CHARS,
            )?,
        });
    }

    let summary = match map.get("summary") {
        None => String::new(),
        Some(value) => read_trimmed_string(Some(value), "summary", MAX_SUMMARY_CHARS)?,
    };
    Ok(ModeratorDelegation {
        summary,
        assignments,
    })
}

fn parse_discussion(
    body: &str,
    moderator: &Agent,
    members: &[Agent],
) -> Result<ModeratorDiscussionDraft, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|_| "moon-discussion 块不是合法 JSON".to_string())?;
    parse_moderator_discussion_draft(&value, moderator, members)
}

fn parse_task(body: &str) -> Result<ModeratorTaskDraft, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|_| "moon-task 块不是合法 JSON".to_string())?;
    let Value::Object(map) = value else {
        return Err("moon-task 块必须是 JSON 对象".to_string());
    };

    let title = read_trimmed_string(map.get("title"), "title", MAX_TASK_TITLE_CHARS)?;
    if let Some(description) = map.get("description") {
        if !description.is_string() {
            return Err("description 必须是字符串".to_string());
        }
    }
    let description = map
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    if description.chars().count() > MAX_TASK_DESCRIPTION_CHARS {
        return Err(format!(
            "description 不能超过 {MAX_TASK_DESCRIPTION_CHARS} 字符"
        ));
    }
    let plan = parse_task_plan(map.get("plan").unwrap_or(&Value::Null))?;

    Ok(ModeratorTaskDraft {
        title,
        description,
        plan,
        auto_start: map.get("autoStart").and_then(Value::as_bool) != Some(false),
    })
}

/// 解析主持人普通聊天输出中的结构化动作。
///
/// 所有块都从展示文本剥离；同时出现多个动作时按任务 > 讨论 > 委派处理，
/// 被忽略的动作留下 note。非法块只降级，不抛错给调用方。
pub fn extract_moderator_action(
    raw: &str,
    moderator: &Agent,
    members: &[Agent],
) -> ModeratorChatExtraction {
    let text = strip_blocks(raw, &["moon-delegation", "moon-task", "moon-discussion"]);
    let blocks = scan_fence_blocks(raw);
    let mut notes = Vec::new();

    let task_block = blocks
        .iter()
        .rev()
        .find(|block| block.info == "moon-task");
    let discussion_block = blocks
        .iter()
        .rev()
        .find(|block| block.info == "moon-discussion");
    let delegation_block = blocks
        .iter()
        .rev()
        .find(|block| block.info == "moon-delegation");

    if task_block.is_none() && discussion_block.is_none() && delegation_block.is_none() {
        return ModeratorChatExtraction {
            text,
            action: ModeratorChatAction::None,
            notes,
        };
    }

    let other_members: Vec<&Agent> = members
        .iter()
        .filter(|member| member.id != moderator.id)
        .collect();

    if let Some(task_block) = task_block {
        if other_members.is_empty() {
            return ModeratorChatExtraction {
                text: if text.is_empty() {
                    "房间没有其他成员，无法创建需要协作的任务。".to_string()
                } else {
                    text
                },
                action: ModeratorChatAction::None,
                notes: vec!["房间没有其他成员，moon-task 块未执行".to_string()],
            };
        }

        match parse_task(&task_block.body) {
            Ok(task) => {
                if discussion_block.is_some() {
                    notes.push("同一回复中同时出现 moon-task 与 moon-discussion，已按任务处理".to_string());
                }
                if delegation_block.is_some() {
                    notes.push("同一回复中同时出现 moon-task 与 moon-delegation，已按任务处理".to_string());
                }
                return ModeratorChatExtraction {
                    text,
                    action: ModeratorChatAction::Task(task),
                    notes,
                };
            }
            Err(message) => {
                notes.push(message);
                return ModeratorChatExtraction {
                    text: if text.is_empty() {
                        "主持人返回了无法解析的任务指令，未执行。".to_string()
                    } else {
                        text
                    },
                    action: ModeratorChatAction::None,
                    notes,
                };
            }
        }
    }

    if let Some(discussion_block) = discussion_block {
        if other_members.len() < MIN_DISCUSSION_PARTICIPANTS {
            return ModeratorChatExtraction {
                text: if text.is_empty() {
                    format!(
                        "房间至少需要 {MIN_DISCUSSION_PARTICIPANTS} 名非主持人成员才能发起讨论。"
                    )
                } else {
                    text
                },
                action: ModeratorChatAction::None,
                notes: vec!["房间成员不足，moon-discussion 块未执行".to_string()],
            };
        }

        match parse_discussion(&discussion_block.body, moderator, members) {
            Ok(discussion) => {
                if delegation_block.is_some() {
                    notes.push(
                        "同一回复中同时出现 moon-discussion 与 moon-delegation，已按讨论处理"
                            .to_string(),
                    );
                }
                return ModeratorChatExtraction {
                    text,
                    action: ModeratorChatAction::Discussion(discussion),
                    notes,
                };
            }
            Err(message) => {
                notes.push(message);
                return ModeratorChatExtraction {
                    text: if text.is_empty() {
                        "主持人返回了无法解析的讨论指令，未执行。".to_string()
                    } else {
                        text
                    },
                    action: ModeratorChatAction::None,
                    notes,
                };
            }
        }
    }

    let Some(delegation_block) = delegation_block else {
        return ModeratorChatExtraction {
            text,
            action: ModeratorChatAction::None,
            notes,
        };
    };

    match parse_delegation(&delegation_block.body, moderator, members) {
        Ok(delegation) => ModeratorChatExtraction {
            text,
            action: ModeratorChatAction::Delegate(delegation),
            notes,
        },
        Err(message) => {
            notes.push(message);
            ModeratorChatExtraction {
                text: if text.is_empty() {
                    "主持人返回了无法解析的委派指令，未执行。".to_string()
                } else {
                    text
                },
                action: ModeratorChatAction::None,
                notes,
            }
        }
    }
}

fn sender_label(message: &Message, member_names: &std::collections::HashMap<i64, String>) -> String {
    match message.sender_type {
        crate::domain::message::MessageSenderType::System => "系统".to_string(),
        crate::domain::message::MessageSenderType::User => "用户".to_string(),
        crate::domain::message::MessageSenderType::Agent => match message.sender_agent_id {
            None => "智能体".to_string(),
            Some(agent_id) => member_names
                .get(&agent_id)
                .cloned()
                .unwrap_or_else(|| format!("智能体#{agent_id}")),
        },
    }
}

/// 构建主持人普通聊天提示词。
///
/// 只注入成员 id/名称/人设与最近对话；不读取 provider 配置或密钥。
pub fn build_moderator_chat_instruction(
    room: &Room,
    moderator: &Agent,
    members: &[Agent],
    history: &[Message],
    instruction: &str,
    compaction: &CompactionOptions,
) -> String {
    let others: Vec<&Agent> = members
        .iter()
        .filter(|member| member.id != moderator.id)
        .collect();
    let member_names: std::collections::HashMap<i64, String> = members
        .iter()
        .map(|member| (member.id, member.name.clone()))
        .collect();

    let member_lines = if others.is_empty() {
        "（房间没有其他成员，只能直接回答）".to_string()
    } else {
        others
            .iter()
            .map(|member| {
                format!(
                    "- id={} 名称：{}｜人设：{}",
                    member.id, member.name, member.persona
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    let recent = context_messages(history);
    let recent = &recent[recent.len().saturating_sub(MODERATOR_CHAT_CONTEXT_LIMIT)..];
    let labeled_owned: Vec<(String, &Message)> = recent
        .iter()
        .map(|message| (sender_label(message, &member_names), message))
        .collect();
    let labeled: Vec<LabeledMessage<'_>> = labeled_owned
        .iter()
        .map(|(label, message)| LabeledMessage {
            label: label.as_str(),
            message,
        })
        .collect();
    let context = compact_transcript(
        &labeled,
        |message, label| compact_message_line(room, message, label),
        compaction,
    );
    let transcript = context.transcript;

    let default_rounds = DEFAULT_DISCUSSION_ROUNDS.min(room.discussion_max_rounds);
    let lines = vec![
        format!(
            "你是房间「{}」的固定内置主持人「{}」。",
            room.name, moderator.name
        ),
        "你负责判断用户请求：简单问题直接回答；需要成员交换意见、互相质疑并形成结论时发起讨论；需要成员执行边界清晰的明确工作时委派；多步骤、需要验收的工作创建任务并启动编排。".to_string(),
        "讨论由成员发言，你只负责发起、裁决和收尾，不代替成员辩论。你不代替成员执行具体工作，也不自动把任务标记完成。".to_string(),
        "委派由系统执行成有界批次：成员完成本轮后，系统会自动让你汇总并决定继续下一轮或停止，最多 3 轮；你不需要在回复中承诺等待或手动催办。".to_string(),
        String::new(),
        "房间其他成员（只能使用下列 id；不得指派自己）：".to_string(),
        member_lines,
        String::new(),
        format!(
            "最近对话（按时间顺序，最多 {MODERATOR_CHAT_CONTEXT_LIMIT} 条）："
        ),
        transcript,
        String::new(),
        "用户当前请求：".to_string(),
        instruction.to_string(),
        String::new(),
        "请四选一：".to_string(),
        "1. 直接回答：只输出自然语言回复。".to_string(),
        "2. 发起讨论：输出一个 ```moon-discussion 代码块，格式：".to_string(),
        format!(
            "{{\"topic\":\"需要成员讨论的问题\",\"participantIds\":[2,3],\"maxRounds\":{default_rounds}}}"
        ),
        format!(
            "   规则：topic 不超过 {DISCUSSION_TOPIC_CHARS} 字，用一句话概括；{MIN_DISCUSSION_PARTICIPANTS}..5 位非主持人成员；agentId 必须来自成员清单；不得包含自己；不得重复；maxRounds 为 1..{}，省略时默认 {default_rounds}。",
            room.discussion_max_rounds
        ),
        "3. 委派：输出一个 ```moon-delegation 代码块，格式：".to_string(),
        "{\"summary\":\"给用户的简短说明\",\"assignments\":[{\"agentId\":2,\"instruction\":\"具体、可执行、可验收的成员任务\"}]}".to_string(),
        format!(
            "   规则：summary 不超过 {MAX_SUMMARY_CHARS} 字；instruction 不超过 {MAX_INSTRUCTION_CHARS} 字；1..5 位成员；agentId 必须来自成员清单；不得指派自己；不得重复。委派只负责启动第一轮，系统会自动收集结果并安排主持人的后续汇总与轮次判断，不要在回复里假装成员已经完成。"
        ),
        "4. 创建任务：输出一个 ```moon-task 代码块，格式：".to_string(),
        "{\"title\":\"任务标题\",\"description\":\"背景与约束\",\"plan\":{\"goal\":\"可观察目标\",\"acceptanceCriteria\":[\"AC-1 ...\"],\"verification\":[\"如何验证，例如运行某测试\"],\"scope\":[\"范围边界\"]},\"autoStart\":true}".to_string(),
        format!(
            "   规则：title 不超过 {MAX_TASK_TITLE_CHARS} 字；description 不超过 {MAX_TASK_DESCRIPTION_CHARS} 字；只在请求需要多步骤协作与验收时创建；autoStart 默认 true，系统会启动编排。"
        ),
        String::new(),
        "动作选择：".to_string(),
        "- 用户要求成员讨论、比较方案、互相质疑或形成结论时，优先发起讨论。".to_string(),
        "- 用户要求“分析、评估、计划、方案、任务拆分”等需要收敛与多方视角的请求时，优先讨论或创建任务，不要用一次性委派代替。".to_string(),
        "- 用户要求完成边界清晰、单个成员可独立交付的工作时，委派成员。".to_string(),
        "- 用户要求多步骤交付、验收与验证时，创建任务。".to_string(),
        String::new(),
        "输出规则：".to_string(),
        "- 结构化块必须是合法 JSON，且最多出现一个；同时出现时按 任务 > 讨论 > 委派 处理；".to_string(),
        "- 块外可以写一段给用户看的简短说明，但不要复述块内容；".to_string(),
        "- 房间没有其他成员时只能直接回答；".to_string(),
        "- 不要编造成员 id、验收结果或命令输出；".to_string(),
        "- 只输出回复正文，不要输出思考过程。".to_string(),
        format!("- {CONCISE_REPLY_RULE}"),
    ];
    lines.join("\n")
}

/// 把委派单包装成成员可执行的指令；成员不会继续派发。
pub fn build_delegated_instruction(assignment: &ChatAssignment, summary: &str) -> String {
    let mut lines = Vec::new();
    if !summary.trim().is_empty() {
        lines.push(format!("主持人概述：{}", summary.trim()));
    }
    lines.push(format!("主持人安排你执行：{}", assignment.instruction));
    lines.push(
        "请直接完成该安排并给出可核对的结果，不要继续派发任务或创建新任务。".to_string(),
    );
    lines.push(CONCISE_REPLY_RULE.to_string());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::{BuiltinAgentRole, HarnessKind};
    use crate::domain::discussion::DEFAULT_DISCUSSION_ROUNDS;

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

    fn room(mutator: impl FnOnce(&mut Room)) -> Room {
        let mut room = Room {
            id: 1,
            name: "协议房间".to_string(),
            workspace_dir: "C:/tmp".to_string(),
            workspace_dirs: vec!["C:/tmp".to_string()],
            moderator_agent_id: Some(1),
            member_ids: vec![1, 2, 3],
            reply_mode: crate::domain::room::RoomReplyMode::Moderator,
            discussion_max_rounds: 5,
            permission_mode: crate::domain::room::RoomPermissionMode::Auto,
            created_at: 1,
            last_activity_at: None,
        };
        mutator(&mut room);
        room
    }

    #[test]
    fn extracts_task_and_strips_every_block() {
        let moderator = agent(1, "主持人", Some(BuiltinAgentRole::Moderator));
        let members = vec![moderator.clone(), agent(2, "成员", None)];
        let raw = concat!(
            "我来创建任务。\n",
            "```moon-task\n",
            "{\"title\":\"实现功能\",\"plan\":{\"goal\":\"g\",\"acceptanceCriteria\":[\"a\"],\"verification\":[\"v\"]}}\n",
            "```\n",
            "```moon-delegation\n",
            "{\"assignments\":[{\"agentId\":2,\"instruction\":\"x\"}]}\n",
            "```"
        );
        let extraction = extract_moderator_action(raw, &moderator, &members);
        assert_eq!(extraction.text, "我来创建任务。");
        assert!(matches!(
            extraction.action,
            ModeratorChatAction::Task(ModeratorTaskDraft { auto_start: true, .. })
        ));
        assert!(extraction
            .notes
            .iter()
            .any(|note| note.contains("已按任务处理")));
    }

    #[test]
    fn extracts_discussion_and_prefers_task() {
        let moderator = agent(1, "主持人", Some(BuiltinAgentRole::Moderator));
        let members = vec![
            moderator.clone(),
            agent(2, "甲", None),
            agent(3, "乙", None),
        ];
        let discussion = concat!(
            "开始讨论。\n",
            "```moon-discussion\n",
            "{\"topic\":\"是否采用方案 A\",\"participantIds\":[2,3]}\n",
            "```"
        );
        let extraction = extract_moderator_action(discussion, &moderator, &members);
        assert_eq!(extraction.text, "开始讨论。");
        match extraction.action {
            ModeratorChatAction::Discussion(draft) => {
                assert_eq!(draft.topic, "是否采用方案 A");
                assert_eq!(draft.participant_ids, vec![2, 3]);
                assert_eq!(draft.max_rounds, None);
            }
            other => panic!("expected discussion, got {other:?}"),
        }

        let both = format!(
            "{discussion}\n```moon-task\n{{\"title\":\"t\",\"plan\":{{\"goal\":\"g\",\"acceptanceCriteria\":[\"a\"],\"verification\":[\"v\"]}}}}\n```"
        );
        assert!(matches!(
            extract_moderator_action(&both, &moderator, &members).action,
            ModeratorChatAction::Task(_)
        ));
    }

    #[test]
    fn malformed_blocks_degrade_to_none_with_notes() {
        let moderator = agent(1, "主持人", Some(BuiltinAgentRole::Moderator));
        let members = vec![moderator.clone(), agent(2, "成员", None)];
        let extraction = extract_moderator_action(
            "说明\n```moon-delegation\n不是 JSON\n```",
            &moderator,
            &members,
        );
        assert!(matches!(extraction.action, ModeratorChatAction::None));
        assert!(extraction
            .notes
            .iter()
            .any(|note| note.contains("不是合法 JSON")));
        assert_eq!(extraction.text, "说明");

        // 没有其他成员时任务块不执行。
        let solo = extract_moderator_action(
            "```moon-task\n{\"title\":\"t\",\"plan\":{\"goal\":\"g\",\"acceptanceCriteria\":[\"a\"],\"verification\":[\"v\"]}}\n```",
            &moderator,
            &[moderator.clone()],
        );
        assert!(matches!(solo.action, ModeratorChatAction::None));
        assert!(solo.text.contains("没有其他成员"));
    }

    #[test]
    fn instruction_contains_member_ids_and_room_round_ceiling() {
        let moderator = agent(1, "主持人", Some(BuiltinAgentRole::Moderator));
        let members = vec![moderator.clone(), agent(2, "成员", None)];
        let room = room(|room| room.discussion_max_rounds = 2);
        let instruction = build_moderator_chat_instruction(
            &room,
            &moderator,
            &members,
            &[],
            "讨论一下",
            &CompactionOptions::default(),
        );
        assert!(instruction.contains("id=2 名称：成员"));
        assert!(instruction.contains("maxRounds 为 1..2"));
        assert!(instruction.contains("省略时默认 2"));
        assert!(instruction.contains("用户当前请求：\n讨论一下"));
        assert!(instruction.contains("moon-discussion"));
        assert!(instruction.contains("moon-task"));
        assert!(instruction.contains("moon-delegation"));
        assert!(!instruction.contains("modelConfig"));
        assert_eq!(DEFAULT_DISCUSSION_ROUNDS, 3);
    }

    #[test]
    fn chat_instruction_applies_context_compaction_budget() {
        use crate::domain::message::{MessageKind, MessageSenderType, MessageStatus};

        let moderator = agent(1, "主持人", Some(BuiltinAgentRole::Moderator));
        let members = vec![moderator.clone(), agent(2, "成员", None)];
        let room = room(|_| {});
        let history: Vec<Message> = (0..10)
            .map(|index| Message {
                id: index,
                room_id: 1,
                seq: index + 1,
                sender_type: MessageSenderType::User,
                sender_agent_id: None,
                status: MessageStatus::Final,
                kind: MessageKind::Reply,
                run_id: None,
                content: format!("第{index}条历史内容"),
                task_id: None,
                reply_to_message_id: None,
                discussion: None,
                collaboration: None,
                attachments: Vec::new(),
                reactions: Vec::new(),
                created_at: 1,
            })
            .collect();
        let compaction = CompactionOptions {
            budget_chars: 40,
            ..CompactionOptions::default()
        };

        let instruction = build_moderator_chat_instruction(
            &room,
            &moderator,
            &members,
            &history,
            "继续",
            &compaction,
        );

        assert!(instruction.contains("更早"), "{instruction}");
        assert!(instruction.contains("第9条历史内容"));
        assert!(!instruction.contains("第0条历史内容"));
    }
}

// ===== 编排主持人的结构化计划协议 =====

/// 注入编排提示词的最近消息条数；固定值。
pub const MODERATOR_CONTEXT_MESSAGE_LIMIT: usize = 20;
/// 通过数据校验后，一次 assign 最多可包含的分配单数量。
pub const MAX_ASSIGNMENTS: usize = 5;
const MAX_PLAN_INSTRUCTION_CHARS: usize = 4_000;
const MAX_EXPECTED_OUTPUT_CHARS: usize = 2_000;
const MAX_PLAN_ACCEPTANCE_CRITERIA: usize = 10;
const MAX_PLAN_ACCEPTANCE_CRITERION_CHARS: usize = 500;
const MAX_PLAN_SUMMARY_CHARS: usize = 2_000;
const MAX_PLAN_VERIFICATION_ITEMS: usize = 5;
const MAX_PLAN_VERIFICATION_CHARS: usize = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignmentKind {
    Work,
    Review,
}

impl AssignmentKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Work => "work",
            Self::Review => "review",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AssignInstruction {
    pub agent_id: i64,
    pub instruction: String,
    /// 缺省视为 work；review 表示复核产出而不是继续实现。
    pub kind: AssignmentKind,
    pub expected_output: Option<String>,
    pub acceptance_criteria: Option<Vec<String>>,
}

/// 主持人约定输出：继续指派成员，或宣布收敛完成并给出验收证据。
#[derive(Debug, Clone, PartialEq)]
pub enum ModeratorPlan {
    Assign { assignments: Vec<AssignInstruction> },
    Done { summary: String, verification: Vec<String> },
}

/// 主持人输出不符合约定 JSON；引擎只对此错误做一次重试。
#[derive(Debug, Clone)]
pub struct PlanParseError(pub String);

impl std::fmt::Display for PlanParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for PlanParseError {}

fn plan_error<T>(message: impl Into<String>) -> Result<T, PlanParseError> {
    Err(PlanParseError(message.into()))
}

fn read_plan_string(
    value: Option<&Value>,
    field: &str,
    max_chars: usize,
) -> Result<String, PlanParseError> {
    let Some(text) = value.and_then(Value::as_str) else {
        return plan_error(format!("{field} 必须是非空字符串"));
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return plan_error(format!("{field} 必须是非空字符串"));
    }
    if trimmed.chars().count() > max_chars {
        return plan_error(format!("{field} 不能超过 {max_chars} 字符"));
    }
    Ok(trimmed.to_string())
}

fn read_optional_plan_string(
    value: Option<&Value>,
    field: &str,
    max_chars: usize,
) -> Result<Option<String>, PlanParseError> {
    match value {
        None => Ok(None),
        Some(value) => read_plan_string(Some(value), field, max_chars).map(Some),
    }
}

fn read_plan_string_list(
    value: Option<&Value>,
    field: &str,
    min_items: usize,
    max_items: usize,
    max_chars: usize,
) -> Result<Vec<String>, PlanParseError> {
    let Some(Value::Array(items)) = value else {
        return plan_error(format!(
            "{field} 必须包含 {min_items}..{max_items} 条非空字符串"
        ));
    };
    if items.len() < min_items || items.len() > max_items {
        return plan_error(format!(
            "{field} 必须包含 {min_items}..{max_items} 条非空字符串"
        ));
    }
    let mut values = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        values.push(read_plan_string(
            Some(item),
            &format!("{field}[{index}]"),
            max_chars,
        )?);
    }
    Ok(values)
}

fn extract_plan_candidate(raw: &str) -> Result<String, PlanParseError> {
    if let Some(body) = scan_fence_blocks(raw)
        .into_iter()
        .find(|block| (block.info.is_empty() || block.info == "json") && !block.body.trim().is_empty())
    {
        return Ok(body.body.trim().to_string());
    }

    let start = raw.find('{');
    let end = raw.rfind('}');
    match (start, end) {
        (Some(start), Some(end)) if end > start => Ok(raw[start..=end].to_string()),
        _ => plan_error("主持人输出中找不到 JSON 对象"),
    }
}

/// 解析主持人输出；容忍代码块与前后说明文字。
pub fn parse_plan(raw: &str) -> Result<ModeratorPlan, PlanParseError> {
    let candidate = extract_plan_candidate(raw)?;
    let value: Value = serde_json::from_str(&candidate)
        .map_err(|_| PlanParseError("主持人输出不是合法 JSON".to_string()))?;
    let Value::Object(map) = value else {
        return plan_error("主持人输出必须是 JSON 对象");
    };

    match map.get("type").and_then(Value::as_str) {
        Some("assign") => {
            let Some(Value::Array(raw_assignments)) = map.get("assignments") else {
                return plan_error(format!(
                    "assign 计划必须包含 1..{MAX_ASSIGNMENTS} 条 assignments"
                ));
            };
            if raw_assignments.is_empty() || raw_assignments.len() > MAX_ASSIGNMENTS {
                return plan_error(format!(
                    "assign 计划必须包含 1..{MAX_ASSIGNMENTS} 条 assignments"
                ));
            }

            let mut seen = std::collections::HashSet::new();
            let mut assignments = Vec::with_capacity(raw_assignments.len());
            for (index, item) in raw_assignments.iter().enumerate() {
                let Value::Object(item) = item else {
                    return plan_error(format!("assignments[{index}] 必须是对象"));
                };
                let Some(agent_id) = item.get("agentId").and_then(Value::as_i64).filter(|id| *id > 0)
                else {
                    return plan_error(format!("assignments[{index}].agentId 必须是成员 id"));
                };
                if !seen.insert(agent_id) {
                    return plan_error(format!(
                        "assignments[{index}].agentId 与前面的分配单重复"
                    ));
                }
                let kind = match item.get("kind") {
                    None => AssignmentKind::Work,
                    Some(Value::String(value)) if value == "work" => AssignmentKind::Work,
                    Some(Value::String(value)) if value == "review" => AssignmentKind::Review,
                    Some(_) => {
                        return plan_error(format!(
                            "assignments[{index}].kind 必须是 work 或 review"
                        ))
                    }
                };
                assignments.push(AssignInstruction {
                    agent_id,
                    instruction: read_plan_string(
                        item.get("instruction"),
                        &format!("assignments[{index}].instruction"),
                        MAX_PLAN_INSTRUCTION_CHARS,
                    )?,
                    kind,
                    expected_output: read_optional_plan_string(
                        item.get("expectedOutput"),
                        &format!("assignments[{index}].expectedOutput"),
                        MAX_EXPECTED_OUTPUT_CHARS,
                    )?,
                    acceptance_criteria: match item.get("acceptanceCriteria") {
                        None => None,
                        Some(value) => Some(read_plan_string_list(
                            Some(value),
                            &format!("assignments[{index}].acceptanceCriteria"),
                            1,
                            MAX_PLAN_ACCEPTANCE_CRITERIA,
                            MAX_PLAN_ACCEPTANCE_CRITERION_CHARS,
                        )?),
                    },
                });
            }
            Ok(ModeratorPlan::Assign { assignments })
        }
        Some("done") => {
            let summary = read_plan_string(map.get("summary"), "done.summary", MAX_PLAN_SUMMARY_CHARS)?;
            let verification = read_plan_string_list(
                map.get("verification"),
                "done.verification",
                1,
                MAX_PLAN_VERIFICATION_ITEMS,
                MAX_PLAN_VERIFICATION_CHARS,
            )?;
            Ok(ModeratorPlan::Done {
                summary,
                verification,
            })
        }
        other => plan_error(format!(
            "未知的计划类型：{}",
            other.unwrap_or("undefined")
        )),
    }
}

/// 组装主持人指令：任务目标、成员清单（含 id 与人设）、最近对话、当前轮次。
pub fn build_moderator_instruction(
    task: &crate::domain::task::Task,
    room: &Room,
    members: &[Agent],
    history: &[Message],
    round: i64,
    max_rounds: i64,
) -> String {
    let member_names: std::collections::HashMap<i64, String> = members
        .iter()
        .map(|member| (member.id, member.name.clone()))
        .collect();
    let moderator = members
        .iter()
        .find(|member| Some(member.id) == room.moderator_agent_id);

    let member_lines = if members.is_empty() {
        "（房间暂无成员）".to_string()
    } else {
        members
            .iter()
            .map(|member| {
                format!(
                    "- id={} 名称：{}｜人设：{}",
                    member.id, member.name, member.persona
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    let recent = context_messages(history);
    let recent = &recent[recent.len().saturating_sub(MODERATOR_CONTEXT_MESSAGE_LIMIT)..];
    let transcript = if recent.is_empty() {
        "（暂无对话）".to_string()
    } else {
        recent
            .iter()
            .map(|message| {
                format!(
                    "#{} {}：{}",
                    message.seq,
                    sender_label(message, &member_names),
                    message.content
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    let description = if task.description.trim().is_empty() {
        "（无描述）"
    } else {
        task.description.as_str()
    };

    let mut lines = vec![
        format!(
            "你是房间「{}」的主持人「{}」，负责把任务编排成成员可执行的子任务并评审结果。",
            room.name,
            moderator.map(|agent| agent.name.as_str()).unwrap_or("主持人")
        ),
        "你是会议的组织者：分解任务、逐轮指派房间成员执行、根据结果决定继续或收敛。".to_string(),
        format!("当前轮次：第 {round} 轮（最多 {max_rounds} 轮；超出将被系统熔断）。"),
        String::new(),
        format!("任务 #{}：{}", task.id, task.title),
        format!("任务描述：{description}"),
        String::new(),
    ];
    match &task.plan {
        None => lines.push(
            "任务未提供结构化计划：请先根据标题与描述明确可观察目标与验收标准，再拆分工作。"
                .to_string(),
        ),
        Some(plan) => {
            lines.push("任务结构化计划：".to_string());
            lines.push(format!("- 目标：{}", plan.goal));
            lines.push("- 验收标准：".to_string());
            for item in &plan.acceptance_criteria {
                lines.push(format!("  - {item}"));
            }
            lines.push("- 验证方式：".to_string());
            for item in &plan.verification {
                lines.push(format!("  - {item}"));
            }
            lines.push("- 范围边界：".to_string());
            if plan.scope.is_empty() {
                lines.push("  - （无显式范围）".to_string());
            } else {
                for item in &plan.scope {
                    lines.push(format!("  - {item}"));
                }
            }
        }
    }
    lines.extend([
        String::new(),
        "房间成员（assignments.agentId 必须严格使用下列 id）：".to_string(),
        member_lines,
        String::new(),
        format!(
            "最近对话（按时间顺序，最多 {MODERATOR_CONTEXT_MESSAGE_LIMIT} 条）："
        ),
        transcript,
        String::new(),
        "请只输出一个 JSON 对象，不要输出解释、复述或思考过程。两种格式二选一：".to_string(),
        "1. 继续指派：{\"type\":\"assign\",\"assignments\":[{\"agentId\":1,\"instruction\":\"具体、可执行的子任务\",\"kind\":\"work\",\"expectedOutput\":\"成员应交付的可核对产出\",\"acceptanceCriteria\":[\"对应任务计划中的验收标准\"]}]}".to_string(),
        "2. 收敛完成：{\"type\":\"done\",\"summary\":\"本轮协作的结论与产出摘要\",\"verification\":[\"实际执行的测试、命令或可核对的验收证据\"]}".to_string(),
        "规则：".to_string(),
        "- assignments 最多 5 条；agentId 必须是上述成员清单中的 id，不得指派自己，同一轮不得重复指派同一成员；".to_string(),
        "- kind 缺省为 work，只允许 work 或 review；review 用于核对产出与验收标准；".to_string(),
        "- 每个分配单都应写清 expectedOutput 与 acceptanceCriteria，方便成员自检和后续复核；".to_string(),
        "- 只要安排过 work，就必须在宣布 done 之前至少完成一次 kind:\"review\" 的复核；".to_string(),
        "- done 必须携带非空 verification，引用真实测试、命令、AC 或可核对产出；不得编造结果；".to_string(),
        "- 任务目标已达成时才输出 done；系统只把任务推进到 review，最终由人验收。".to_string(),
    ]);
    lines.join("\n")
}

/// 把分配单格式化为成员指令：保留目标、预期产出与验收标准。
pub fn format_assignment_instruction(assignment: &AssignInstruction) -> String {
    let mut lines = vec![if assignment.kind == AssignmentKind::Review {
        format!("主持人安排你复核：{}", assignment.instruction)
    } else {
        format!("主持人安排你执行：{}", assignment.instruction)
    }];
    if let Some(expected_output) = &assignment.expected_output {
        lines.push(format!("预期产出：{expected_output}"));
    }
    if let Some(criteria) = &assignment.acceptance_criteria {
        lines.push("验收标准：".to_string());
        lines.extend(criteria.iter().map(|item| format!("- {item}")));
    }
    lines.push(if assignment.kind == AssignmentKind::Review {
        "这是复核任务：请对照验收标准给出明确的通过/不通过结论与证据。".to_string()
    } else {
        "这是执行任务：请完成工作并给出可核对的结果。".to_string()
    });
    lines.push("不要继续派发任务或创建新任务，也不要把任务标记完成。".to_string());
    lines.join("\n")
}

#[cfg(test)]
mod plan_tests {
    use super::*;

    #[test]
    fn parses_assign_and_done_plans() {
        let assign = parse_plan(
            "说明\n```json\n{\"type\":\"assign\",\"assignments\":[{\"agentId\":2,\"instruction\":\"实现\",\"kind\":\"review\",\"expectedOutput\":\"报告\",\"acceptanceCriteria\":[\"AC-1\"]}]}\n```",
        )
        .unwrap();
        match assign {
            ModeratorPlan::Assign { assignments } => {
                assert_eq!(assignments[0].agent_id, 2);
                assert_eq!(assignments[0].kind, AssignmentKind::Review);
                assert_eq!(
                    assignments[0].acceptance_criteria.as_deref(),
                    Some(&["AC-1".to_string()][..])
                );
            }
            _ => panic!("expected assign"),
        }

        let done = parse_plan(
            "{\"type\":\"done\",\"summary\":\"完成\",\"verification\":[\"cargo test\"]}",
        )
        .unwrap();
        assert!(matches!(done, ModeratorPlan::Done { .. }));
    }

    #[test]
    fn rejects_invalid_plans() {
        assert!(parse_plan("没有 JSON").is_err());
        assert!(parse_plan("{\"type\":\"unknown\"}").is_err());
        assert!(parse_plan("{\"type\":\"assign\",\"assignments\":[]}").is_err());
        assert!(parse_plan("{\"type\":\"done\",\"summary\":\"x\"}").is_err());
        assert!(parse_plan(
            "{\"type\":\"assign\",\"assignments\":[{\"agentId\":1,\"instruction\":\"a\"},{\"agentId\":1,\"instruction\":\"b\"}]}"
        )
        .is_err());
    }
}
