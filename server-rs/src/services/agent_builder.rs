//! 智能体创建助手。
//!
//! 职责边界：
//! - 内置助手不落库，只借用 zcode harness 生成对话回复；
//! - 模型输出先视为不可信文本，只有 `moon-agent-draft` 代码块通过解析与
//!   模型目录校验后才会成为草稿；
//! - 本服务只读 agent 名称与模型目录，不写库、不广播、不记录提示词或配置。

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use futures_util::StreamExt;
use serde_json::{Map, Value};

use crate::adapters::{AdapterFactory, RunInput};
use crate::domain::agent::{Agent, HarnessKind, ModelConfig};
use crate::domain::agent_builder::{
    AgentBuilderChatResponse, AgentBuilderMessage, AgentBuilderRunInfo, AgentDraft,
};
use crate::domain::harness::HarnessModelCatalog;
use crate::domain::now_ms;
use crate::domain::run_event::RunEvent;
use crate::error::{AppError, AppResult};
use crate::services::harness_models::get_harness_model_catalog;
use crate::services::moderator::{scan_fence_blocks, strip_blocks};
use crate::services::require_body;
use crate::services::settings::SettingsService;
use crate::store::AgentStore;

pub const AGENT_BUILDER_NAME: &str = "智能体创建助手";
pub const AGENT_BUILDER_HARNESS: HarnessKind = HarnessKind::Zcode;

/// 单次请求允许携带的对话上限；固定值，不做可配置化。
const MAX_MESSAGES: usize = 24;
const MAX_MESSAGE_CHARS: usize = 8_000;
const MAX_TOTAL_CHARS: usize = 32_000;
const DRAFT_BLOCK_INFO: &str = "moon-agent-draft";

const BUILDER_PERSONA: &str =
    "你是 Moon 的智能体创建助手。通过对话把用户的想法整理成可直接创建的智能体定义，并在每轮回复末尾输出完整草稿。";

pub type CatalogReader = Arc<dyn Fn(HarnessKind) -> HarnessModelCatalog + Send + Sync>;

pub struct AgentBuilderServiceOptions {
    pub agents: AgentStore,
    pub adapter_factory: AdapterFactory,
    /// harness 运行目录；生产位于 userData 下。
    pub workspace_dir: PathBuf,
    /// 默认读取真实 harness 模型目录；测试注入固定目录。
    pub catalog_reader: Option<CatalogReader>,
    /// 运行总时长上限来源；与聊天路径共用同一设置。
    pub settings: Arc<SettingsService>,
}

pub struct AgentBuilderService {
    agents: AgentStore,
    adapter_factory: AdapterFactory,
    workspace_dir: PathBuf,
    catalog_reader: CatalogReader,
    settings: Arc<SettingsService>,
}

#[derive(Debug, Default)]
pub struct AgentDraftExtraction {
    /// 移除所有草稿块后的展示文本。
    pub reply: String,
    /// 最后一个草稿块解析出的 JSON 对象；没有或损坏时为 `None`。
    pub raw: Option<Map<String, Value>>,
    /// 存在草稿块但 JSON 无法解析。
    pub malformed: bool,
}

pub struct AgentDraftResolution {
    pub draft: AgentDraft,
    pub notes: Vec<String>,
}

impl AgentBuilderService {
    pub fn new(options: AgentBuilderServiceOptions) -> Self {
        Self {
            agents: options.agents,
            adapter_factory: options.adapter_factory,
            workspace_dir: options.workspace_dir,
            catalog_reader: options
                .catalog_reader
                .unwrap_or_else(|| Arc::new(get_harness_model_catalog)),
            settings: options.settings,
        }
    }

    pub async fn chat(&self, input: Value) -> AppResult<AgentBuilderChatResponse> {
        let body = require_body(input)?;
        let messages = parse_messages(body.get("messages"))?;
        let assistant_model_config = parse_model_config(body.get("assistantModelConfig"))?;

        let catalog = (self.catalog_reader)(AGENT_BUILDER_HARNESS);
        let mut existing_names = Vec::new();
        let mut existing_name_set = HashSet::new();
        for agent in self.agents.list().await? {
            if existing_name_set.insert(agent.name.clone()) {
                existing_names.push(agent.name);
            }
        }
        let prompt = build_agent_builder_prompt(&messages, &catalog, &existing_names);

        // harness 需要真实目录；只在真正运行时创建，避免测试或只读启动产生副作用。
        std::fs::create_dir_all(&self.workspace_dir).map_err(|error| {
            AppError::with_source(
                crate::error::ErrorCode::InternalError,
                "无法创建创建助手工作目录",
                error,
            )
        })?;

        let agent = Agent {
            id: 0,
            name: AGENT_BUILDER_NAME.to_string(),
            persona: BUILDER_PERSONA.to_string(),
            harness: AGENT_BUILDER_HARNESS,
            model_config: assistant_model_config.clone(),
            builtin_role: None,
            custom_harness_id: None,
            workspace_dir: None,
            created_at: now_ms(),
        };

        let (text, run) = self.run(agent, prompt, assistant_model_config).await?;
        let extracted = extract_agent_draft_block(&text);
        let mut notes = Vec::new();
        if extracted.malformed {
            notes.push(
                "助手返回的草稿格式无法解析，请继续对话或手动填写右侧草稿。".to_string(),
            );
        }

        let draft = extracted.raw.as_ref().map(|raw| {
            let resolved = resolve_agent_draft(raw, &catalog, &existing_name_set);
            notes.extend(resolved.notes);
            resolved.draft
        });

        let mut reply = extracted.reply;
        if reply.is_empty() {
            if draft.is_some() {
                reply = "已更新右侧草稿，请确认或继续补充要求。".to_string();
            }
        }

        Ok(AgentBuilderChatResponse {
            reply,
            draft,
            notes,
            run,
        })
    }

    async fn run(
        &self,
        agent: Agent,
        prompt: String,
        config: Option<ModelConfig>,
    ) -> AppResult<(String, AgentBuilderRunInfo)> {
        let adapter = (self.adapter_factory)(&agent)?;
        // 先取出模型名，config 随后 move 进 RunInput。
        let model = config
            .as_ref()
            .and_then(|value| value.get("model"))
            .and_then(Value::as_str)
            .map(str::to_string);
        // 用户覆盖的执行文件同样作用于助手运行，避免自动检测失败。
        let config = self
            .settings
            .with_harness_bin_override(agent.harness, config)
            .await?;
        let mut stream = adapter.run(RunInput {
            prompt,
            workspace_dir: self.workspace_dir.to_string_lossy().into_owned(),
            additional_directories: Vec::new(),
            config,
            session: None,
            permission_mode: None,
            timeout: self.settings.run_timeout().await?,
        });

        let started = Instant::now();
        let mut accumulated = String::new();
        let mut final_text = String::new();
        let mut failure: Option<String> = None;
        let mut token_count: Option<i64> = None;
        let mut context_size: Option<i64> = None;
        let mut tool_call_count: Option<i64> = None;
        let mut usage_duration_ms: Option<i64> = None;

        while let Some(item) = stream.next().await {
            match item {
                Ok(RunEvent::TextDelta(text)) => accumulated.push_str(&text),
                Ok(RunEvent::Final(text)) => final_text = text,
                Ok(RunEvent::Usage {
                    token_count: tokens,
                    context_size: context,
                    tool_call_count: tools,
                    duration_ms,
                }) => {
                    if tokens.is_some() {
                        token_count = tokens;
                    }
                    if context.is_some() {
                        context_size = context;
                    }
                    if let Some(tools) = tools {
                        tool_call_count = Some(tools);
                    }
                    if duration_ms.is_some() {
                        usage_duration_ms = duration_ms;
                    }
                }
                Ok(RunEvent::ToolStart { .. }) => {
                    tool_call_count = Some(tool_call_count.unwrap_or(0) + 1);
                }
                Ok(RunEvent::Error(message)) => {
                    failure = Some(message);
                    break;
                }
                // 推理与状态事件只服务于运行详情，不影响创建助手的回复文本。
                Ok(_) => {}
                Err(error) => return Err(error),
            }
        }

        if let Some(failure) = failure {
            return Err(AppError::harness_run_failed(format!(
                "创建助手运行失败：{failure}"
            )));
        }

        let content = if final_text.trim().is_empty() {
            accumulated
        } else {
            final_text
        };
        if content.trim().is_empty() {
            return Err(AppError::harness_run_failed("创建助手未返回任何内容"));
        }
        let run = AgentBuilderRunInfo {
            status: "completed".to_string(),
            model,
            duration_ms: Some(
                usage_duration_ms.unwrap_or_else(|| started.elapsed().as_millis() as i64),
            ),
            token_count,
            tool_call_count,
            context_size,
        };
        Ok((content, run))
    }
}

fn parse_messages(value: Option<&Value>) -> AppResult<Vec<AgentBuilderMessage>> {
    let Some(items) = value.and_then(Value::as_array) else {
        return Err(AppError::validation(
            "messages 必须是至少包含一条消息的数组",
        ));
    };
    if items.is_empty() {
        return Err(AppError::validation(
            "messages 必须是至少包含一条消息的数组",
        ));
    }
    if items.len() > MAX_MESSAGES {
        return Err(AppError::validation(format!(
            "messages 最多允许 {MAX_MESSAGES} 条"
        )));
    }

    let mut messages = Vec::with_capacity(items.len());
    let mut total = 0;
    for item in items {
        let Some(item) = item.as_object() else {
            return Err(AppError::validation("messages 中的每一项都必须是对象"));
        };
        let role = item.get("role").and_then(Value::as_str);
        if role != Some("user") && role != Some("assistant") {
            return Err(AppError::validation(
                "messages.role 必须是 user 或 assistant",
            ));
        }
        let Some(content) = item.get("content").and_then(Value::as_str) else {
            return Err(AppError::validation("messages.content 必须是非空字符串"));
        };
        if content.trim().is_empty() {
            return Err(AppError::validation("messages.content 必须是非空字符串"));
        }
        if content.chars().count() > MAX_MESSAGE_CHARS {
            return Err(AppError::validation(format!(
                "单条消息不能超过 {MAX_MESSAGE_CHARS} 个字符"
            )));
        }
        total += content.chars().count();
        messages.push(AgentBuilderMessage {
            role: role.unwrap_or_default().to_string(),
            content: content.to_string(),
        });
    }

    if total > MAX_TOTAL_CHARS {
        return Err(AppError::validation(format!(
            "对话总长度不能超过 {MAX_TOTAL_CHARS} 个字符"
        )));
    }
    if messages.last().map(|message| message.role.as_str()) != Some("user") {
        return Err(AppError::validation("最后一条消息必须来自用户"));
    }
    Ok(messages)
}

fn parse_model_config(value: Option<&Value>) -> AppResult<Option<ModelConfig>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(map)) => Ok(Some(map.clone())),
        Some(_) => Err(AppError::validation(
            "assistantModelConfig 必须是对象或 null",
        )),
    }
}

/// 提取回复中的草稿块。
///
/// 草稿块协议固定为：
///
/// ```moon-agent-draft
/// { ... }
/// ```
///
/// 取最后一个块作为当前草稿；所有块都从展示文本中移除。JSON 损坏时不抛错，
/// 由调用方降级为“无草稿 + 提示”。
pub fn extract_agent_draft_block(text: &str) -> AgentDraftExtraction {
    let blocks = scan_fence_blocks(text);
    let draft_blocks: Vec<_> = blocks
        .iter()
        .filter(|block| block.info == DRAFT_BLOCK_INFO)
        .collect();

    let mut raw = None;
    let mut malformed = false;
    if let Some(last) = draft_blocks.last() {
        match serde_json::from_str::<Value>(&last.body) {
            Ok(Value::Object(map)) => raw = Some(map),
            _ => malformed = true,
        }
    }

    AgentDraftExtraction {
        reply: strip_draft_blocks(text),
        raw,
        malformed,
    }
}

fn strip_draft_blocks(text: &str) -> String {
    let stripped = strip_blocks(text, &[DRAFT_BLOCK_INFO]);
    let mut output = String::with_capacity(stripped.len());
    let mut newlines = 0;
    for character in stripped.chars() {
        if character == '\n' {
            newlines += 1;
            if newlines > 2 {
                continue;
            }
        } else {
            newlines = 0;
        }
        output.push(character);
    }
    output.trim().to_string()
}

/// 把模型给出的草稿 JSON 校验为可展示草稿。
///
/// 名称/人设允许为空，前端据此提示补齐；harness 越界时回退 zcode；
/// 内置 harness 的模型与思考等级必须能在目录中找到，否则清空并给出 note；
/// acp 没有全局模型目录，模型原样保留、由创建时选择的注册表条目决定。
pub fn resolve_agent_draft(
    raw: &Map<String, Value>,
    catalog: &HarnessModelCatalog,
    existing_names: &HashSet<String>,
) -> AgentDraftResolution {
    let mut notes = Vec::new();
    let name = read_string(raw.get("name"));
    let persona = read_string(raw.get("persona"));
    let requested_harness = read_string(raw.get("harness"));
    let harness = HarnessKind::ALL
        .iter()
        .copied()
        .find(|kind| kind.as_str() == requested_harness)
        .unwrap_or(AGENT_BUILDER_HARNESS);

    if !requested_harness.is_empty() && harness.as_str() != requested_harness {
        notes.push(format!(
            "harness「{requested_harness}」不合法，已回退为 zcode。"
        ));
    }
    if name.is_empty() {
        notes.push("草稿还缺名称。".to_string());
    }
    if persona.is_empty() {
        notes.push("草稿还缺人设。".to_string());
    }
    if !name.is_empty() && existing_names.contains(&name) {
        notes.push(format!("名称「{name}」已存在，请修改后再创建。"));
    }
    if harness == HarnessKind::KimiCode {
        notes.push(format!(
            "{} 目前只能创建，运行适配器尚未实现。",
            harness.as_str()
        ));
    }
    if harness == HarnessKind::Fake {
        notes.push("fake 仅用于测试，无法在真实房间工作。".to_string());
    }

    let model = read_string(raw.get("model"));
    let provider_id = read_string(raw.get("providerId"));
    let reasoning_level = read_string(raw.get("reasoningLevel"));
    let mut model_config: Option<ModelConfig> = None;

    // ACP 没有全局模型目录：模型由创建时选择的注册表条目决定，这里原样保留，
    // 不拿 zcode 目录校验，也避免把创建助手自己的目录语义带进 acp 智能体。
    if harness == HarnessKind::Acp {
        if !model.is_empty() {
            let mut config = Map::new();
            config.insert("model".to_string(), Value::String(model));
            if !provider_id.is_empty() {
                config.insert("providerId".to_string(), Value::String(provider_id));
            }
            if !reasoning_level.is_empty() {
                config.insert(
                    "reasoningLevel".to_string(),
                    Value::String(reasoning_level),
                );
            }
            model_config = Some(config);
            notes.push(
                "acp 模型由创建时选择的 ACP agent 候选决定，草稿建议未做全局校验。"
                    .to_string(),
            );
        }
        return AgentDraftResolution {
            draft: AgentDraft {
                name,
                persona,
                harness,
                model_config,
            },
            notes,
        };
    }

    if !model.is_empty() {
        if !catalog.available || catalog.models.is_empty() {
            let mut config = Map::new();
            config.insert("model".to_string(), Value::String(model));
            if !provider_id.is_empty() {
                config.insert("providerId".to_string(), Value::String(provider_id));
            }
            if !reasoning_level.is_empty() {
                config.insert(
                    "reasoningLevel".to_string(),
                    Value::String(reasoning_level),
                );
            }
            model_config = Some(config);
            notes.push(catalog.message.clone().unwrap_or_else(|| {
                format!(
                    "当前无法读取 {} 模型目录，助手建议的模型未被校验。",
                    harness.as_str()
                )
            }));
        } else {
            let exact = if provider_id.is_empty() {
                None
            } else {
                catalog.models.iter().find(|option| {
                    option.provider_id == provider_id && option.model_id == model
                })
            };
            let by_model: Vec<_> = catalog
                .models
                .iter()
                .filter(|option| option.model_id == model)
                .collect();
            let option = exact.or_else(|| by_model.first().copied());

            let Some(option) = option else {
                notes.push(format!(
                    "助手建议的模型「{model}」不在 {} 模型目录中，已留空。",
                    harness.as_str()
                ));
                return AgentDraftResolution {
                    draft: AgentDraft {
                        name,
                        persona,
                        harness,
                        model_config,
                    },
                    notes,
                };
            };

            if provider_id.is_empty() && by_model.len() > 1 {
                notes.push(format!(
                    "模型「{model}」存在于多个 provider，已选择「{}」。",
                    option.provider_name
                ));
            } else if !provider_id.is_empty() && provider_id != option.provider_id {
                notes.push(format!(
                    "模型「{model}」属于 provider「{}」，已按目录纠正。",
                    option.provider_name
                ));
            }

            let mut config = Map::new();
            config.insert("model".to_string(), Value::String(option.model_id.clone()));
            config.insert(
                "providerId".to_string(),
                Value::String(option.provider_id.clone()),
            );
            if !reasoning_level.is_empty() {
                if option.reasoning_levels.contains(&reasoning_level) {
                    config.insert(
                        "reasoningLevel".to_string(),
                        Value::String(reasoning_level),
                    );
                } else {
                    notes.push(format!(
                        "模型「{}」不支持思考等级「{reasoning_level}」，已留空。",
                        option.model_id
                    ));
                }
            }
            model_config = Some(config);
        }
    }

    AgentDraftResolution {
        draft: AgentDraft {
            name,
            persona,
            harness,
            model_config,
        },
        notes,
    }
}

/// 构建创建助手提示词。
///
/// 只拼入模型目录中的安全字段与现有名称，不读取 API key、baseUrl 或完整配置。
pub fn build_agent_builder_prompt(
    messages: &[AgentBuilderMessage],
    catalog: &HarnessModelCatalog,
    existing_names: &[String],
) -> String {
    let names = if existing_names.is_empty() {
        "（暂无）".to_string()
    } else {
        existing_names
            .iter()
            .take(100)
            .cloned()
            .collect::<Vec<_>>()
            .join("、")
    };

    let model_lines = if catalog.available && !catalog.models.is_empty() {
        catalog
            .models
            .iter()
            .map(|model| {
                let levels = if model.reasoning_levels.is_empty() {
                    "未声明".to_string()
                } else {
                    model.reasoning_levels.join(", ")
                };
                format!(
                    "- {} / {} / {} / 思考等级：{}",
                    model.provider_name, model.provider_id, model.model_id, levels
                )
            })
            .collect()
    } else {
        vec![format!(
            "（当前无法读取 zcode 模型目录：{}）",
            catalog.message.as_deref().unwrap_or("未知原因")
        )]
    };

    let transcript = messages
        .iter()
        .map(|message| {
            let label = if message.role == "user" { "用户" } else { "助手" };
            let content = strip_draft_blocks(&message.content);
            let shown = if content.is_empty() {
                "（无文字回复）".to_string()
            } else {
                content
            };
            format!("{label}：{shown}")
        })
        .collect::<Vec<_>>()
        .join("\n");

    let previous_draft = messages
        .iter()
        .rev()
        .filter(|message| message.role == "assistant")
        .find_map(|message| {
            extract_agent_draft_block(&message.content)
                .raw
                .map(|raw| serde_json::to_string(&raw).unwrap_or_default())
        });

    let mut lines = vec![
        "你是 Moon 的「智能体创建助手」。你的职责是通过对话，把用户的想法整理成一个可以直接创建的 Moon 智能体。".to_string(),
        String::new(),
        "工作规则：".to_string(),
        "1. 回复使用简体中文，控制在 120 字以内；不要输出思考过程，不使用 Markdown 表格。".to_string(),
        "2. 信息足够时直接给出整理结果；缺少关键信息且无法合理推断时，最多问一个澄清问题。".to_string(),
        "3. 每次回复的最后必须输出一个完整的草稿块，不是增量补丁。格式固定为：".to_string(),
        "```moon-agent-draft".to_string(),
        "{\"name\":\"\",\"persona\":\"\",\"harness\":\"zcode\",\"model\":\"\",\"providerId\":\"\",\"reasoningLevel\":\"\"}".to_string(),
        "```".to_string(),
        "4. name 是 2-8 个汉字或英文短名；persona 写清职责、擅长领域、工作方式与输出偏好，80-300 字。".to_string(),
        "5. harness 只能从 zcode/codex/kimi-code/fake/acp 中选择，默认 zcode。".to_string(),
        "6. model 与 reasoningLevel 只能从下方模型目录中选择；harness 为 acp 时 model 留空（由创建时选择的注册表 agent 决定）；拿不准就留空字符串，禁止编造。".to_string(),
        "7. 即使用户还在补充信息，也要输出当前最佳草稿；未知字符串字段留空。".to_string(),
        String::new(),
        format!("现有智能体名称（新名称不能重复）：{names}"),
        "可用 harness：zcode/codex（可运行）、kimi-code（仅创建，运行适配器未实现）、fake（仅测试）、acp（需在创建时选择已注册的 ACP agent，模型由该 agent 决定）。".to_string(),
        "zcode 模型目录（provider 名称 / providerId / modelId / 思考等级）：".to_string(),
    ];
    lines.extend(model_lines);
    lines.push(String::new());
    lines.push(match previous_draft {
        None => "上一版草稿：（暂无）".to_string(),
        Some(draft) => format!("上一版草稿：{draft}"),
    });
    lines.push(String::new());
    lines.push("以下是对话记录（按时间顺序）：".to_string());
    lines.push(transcript);
    lines.push(String::new());
    lines.push("请回复用户，并在回复末尾输出最新完整草稿块。".to_string());
    lines.join("\n")
}

fn read_string(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .map(|text| text.trim().to_string())
        .unwrap_or_default()
}
