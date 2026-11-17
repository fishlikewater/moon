//! ACP 供应商桥接。
//!
//! Moon 的模型供应商默认只注入 `OPENAI_*` / `ANTHROPIC_*` 环境变量，这对
//! 只读自身配置的 ACP agent 不够：agent 不会把供应商模型宣告给协议层，
//! Moon 的模型偏好就永远只能回退到 agent 默认值。
//!
//! 本模块识别三类可桥接的 ACP agent，在**隔离目录**里写入它们各自的配置：
//!
//! - Kimi Code（`@moonshot-ai/kimi-code`）：`KIMI_CODE_HOME/config.toml`，
//!   把供应商模型注册成 `moon-{providerId}/{modelId}` 并设为默认模型；
//! - Codex ACP（`@zed-industries/codex-acp`）：`CODEX_HOME/config.toml`，
//!   写根级 `model` / `model_provider`，密钥经 `env_key` 传递；
//! - Claude Code ACP（`@zed-industries/claude-code-acp`）：不写配置文件，
//!   用 `CLAUDE_CONFIG_DIR` 隔离会话目录并注入 `ANTHROPIC_*` 与 `ANTHROPIC_MODEL`。
//!
//! 不识别或未选择供应商模型时返回 `None`，调用方保持原有 env 注入行为。

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::domain::agent::ModelConfig;
use crate::domain::harness::CustomHarness;
use crate::domain::provider::{ApiProvider, ProviderModel, ProviderProtocol};
use crate::domain::session::RunSessionContext;
use crate::error::{AppError, AppResult, ErrorCode};

/// 可桥接的 ACP agent 类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeKind {
    Kimi,
    Codex,
    Claude,
}

impl BridgeKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            BridgeKind::Kimi => "Kimi Code",
            BridgeKind::Codex => "Codex ACP",
            BridgeKind::Claude => "Claude Code ACP",
        }
    }
}

/// Codex 桥接的密钥环境变量；配置文件只引用键名，不落盘密钥。
pub const CODEX_PROVIDER_KEY_ENV: &str = "MOON_ACP_PROVIDER_KEY";

/// 桥接计划：env 注入内容、需下发的模型 id，以及临时目录清理守卫。
#[derive(Debug)]
pub struct ProviderBridgePlan {
    pub kind: BridgeKind,
    pub env: Vec<(String, String)>,
    /// 协议层应下发的模型 id；Kimi 为注册后的 `moon-{providerId}/{modelId}`。
    pub effective_model: Option<String>,
    _cleanup: Cleanup,
}

/// 临时目录守卫：会话目录不在其中，永不删除。
#[derive(Debug)]
struct Cleanup(Option<PathBuf>);

impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = fs::remove_dir_all(path);
        }
    }
}

/// 纯函数：按命令与参数识别可桥接的 ACP agent。
///
/// 先看完整包名（`@moonshot-ai/kimi-code`、`@zed-industries/codex-acp`），
/// 再退回可执行文件名；不匹配普通 `npx` / `node`，避免误伤其他 agent。
pub fn detect_bridge_kind(command: &str, args: &[String]) -> Option<BridgeKind> {
    let command_name = Path::new(command)
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let tokens: Vec<String> = std::iter::once(command_name.clone())
        .chain(args.iter().map(|arg| arg.to_lowercase()))
        .collect();
    let hits = |needle: &str| tokens.iter().any(|token| token.contains(needle));

    if hits("claude-code-acp") {
        return Some(BridgeKind::Claude);
    }
    if hits("codex-acp") {
        return Some(BridgeKind::Codex);
    }
    if hits("kimi-code") || hits("kimi_code") || command_name.contains("kimi") {
        return Some(BridgeKind::Kimi);
    }
    None
}

/// 读取表单里的模型 id；空值视为未选择。
fn requested_model(config: Option<&ModelConfig>) -> Option<&str> {
    config
        .and_then(|config| config.get("model"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn requested_reasoning(config: Option<&ModelConfig>) -> Option<&str> {
    config
        .and_then(|config| config.get("reasoningLevel"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// 准备桥接计划；不识别、未选模型或校验失败都在 spawn 前返回。
pub fn prepare_provider_bridge(
    harness: &CustomHarness,
    provider: &ApiProvider,
    config: Option<&ModelConfig>,
    session: Option<&RunSessionContext>,
) -> AppResult<Option<ProviderBridgePlan>> {
    let Some(kind) = detect_bridge_kind(&harness.command, &harness.args) else {
        return Ok(None);
    };
    let Some(model_id) = requested_model(config) else {
        return Ok(None);
    };
    let model = provider
        .models
        .iter()
        .find(|model| model.id == model_id)
        .ok_or_else(|| {
            AppError::validation(format!(
                "模型「{model_id}」不在供应商「{}」的模型列表中，请编辑智能体重新选择",
                provider.name
            ))
        })?;

    match (kind, provider.protocol) {
        (BridgeKind::Codex, ProviderProtocol::Anthropic) => {
            return Err(AppError::validation(
                "Codex ACP 只支持 OpenAI 兼容供应商（responses 协议）；请把供应商协议改为 OpenAI，或为该智能体更换 ACP agent",
            ));
        }
        (BridgeKind::Claude, ProviderProtocol::Openai) => {
            return Err(AppError::validation(
                "Claude Code ACP 只支持 Anthropic 兼容供应商；请把供应商协议改为 Anthropic，或为该智能体更换 ACP agent",
            ));
        }
        _ => {}
    }

    let (home, cleanup) = match session {
        Some(session) => (
            Path::new(&session.storage_dir).join("acp-provider-home"),
            Cleanup(None),
        ),
        None => {
            let home = super::unique_temp_dir("moon-acp-provider-").map_err(|error| {
                AppError::with_source(
                    ErrorCode::HarnessRunFailed,
                    "无法创建 ACP 供应商配置目录",
                    error,
                )
            })?;
            (home.clone(), Cleanup(Some(home)))
        }
    };
    fs::create_dir_all(&home).map_err(|error| {
        AppError::with_source(
            ErrorCode::HarnessRunFailed,
            "无法创建 ACP 供应商配置目录",
            error,
        )
    })?;

    let reasoning = requested_reasoning(config);
    let (env, effective_model) = match kind {
        BridgeKind::Kimi => {
            write_kimi_config(&home, provider, model, reasoning)?;
            (
                vec![(
                    "KIMI_CODE_HOME".to_string(),
                    home.to_string_lossy().to_string(),
                )],
                Some(kimi_model_id(provider.id, &model.id)),
            )
        }
        BridgeKind::Codex => {
            write_codex_config(&home, provider, model)?;
            let mut env = vec![("CODEX_HOME".to_string(), home.to_string_lossy().to_string())];
            if !provider.api_key.trim().is_empty() {
                env.push((CODEX_PROVIDER_KEY_ENV.to_string(), provider.api_key.clone()));
            }
            (env, Some(model.id.clone()))
        }
        BridgeKind::Claude => (
            claude_env(&home, provider, &model.id),
            Some(model.id.clone()),
        ),
    };

    Ok(Some(ProviderBridgePlan {
        kind,
        env,
        effective_model,
        _cleanup: cleanup,
    }))
}

/// Kimi 注册后的模型 id；协议层使用它精确命中 `session/set_config_option`。
fn kimi_model_id(provider_id: i64, model_id: &str) -> String {
    format!("moon-{provider_id}/{model_id}")
}

fn write_kimi_config(
    home: &Path,
    provider: &ApiProvider,
    model: &ProviderModel,
    reasoning: Option<&str>,
) -> AppResult<()> {
    let provider_key = format!("moon-{}", provider.id);
    let model_key = kimi_model_id(provider.id, &model.id);

    let mut lines = vec![
        format!("default_model = {}", toml_string(&model_key)),
        "telemetry = false".to_string(),
        String::new(),
        format!("[providers.{}]", toml_string(&provider_key)),
        format!("base_url = {}", toml_string(&provider.base_url)),
        format!("type = {}", toml_string(provider.protocol.as_str())),
    ];
    if !provider.api_key.trim().is_empty() {
        lines.push(format!("api_key = {}", toml_string(&provider.api_key)));
    }

    lines.push(String::new());
    lines.push(format!("[models.{}]", toml_string(&model_key)));
    lines.push(format!("provider = {}", toml_string(&provider_key)));
    lines.push(format!("model = {}", toml_string(&model.id)));
    lines.push(format!("display_name = {}", toml_string(&model.name)));
    lines.push(format!(
        "max_context_size = {}",
        model.context_window.unwrap_or(1_000_000)
    ));
    lines.push(format!(
        "capabilities = [{}]",
        kimi_capabilities(model)
            .iter()
            .map(|value| toml_string(value))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    if !model.reasoning_levels.is_empty() {
        lines.push(format!(
            "support_efforts = [{}]",
            model
                .reasoning_levels
                .iter()
                .map(|value| toml_string(value))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        if let Some(level) =
            reasoning.filter(|level| model.reasoning_levels.iter().any(|v| v == *level))
        {
            lines.push(format!("default_effort = {}", toml_string(level)));
        }
    }

    write_config(home, "Kimi Code", lines.join("\n") + "\n")
}

fn kimi_capabilities(model: &ProviderModel) -> Vec<&'static str> {
    let mut capabilities = vec!["tool_use"];
    if !model.reasoning_levels.is_empty() {
        capabilities.push("thinking");
    }
    if model.input_types.iter().any(|kind| kind == "image") {
        capabilities.push("image_in");
    }
    if model.input_types.iter().any(|kind| kind == "video") {
        capabilities.push("video_in");
    }
    capabilities
}

fn write_codex_config(home: &Path, provider: &ApiProvider, model: &ProviderModel) -> AppResult<()> {
    let mut lines = vec![
        format!("model = {}", toml_string(&model.id)),
        "model_provider = \"moon\"".to_string(),
        String::new(),
        "[model_providers.moon]".to_string(),
        format!("name = {}", toml_string(&format!("Moon {}", provider.name))),
        format!("base_url = {}", toml_string(&provider.base_url)),
        "wire_api = \"responses\"".to_string(),
    ];
    if !provider.api_key.trim().is_empty() {
        lines.push(format!("env_key = {}", toml_string(CODEX_PROVIDER_KEY_ENV)));
    }
    write_config(home, "Codex ACP", lines.join("\n") + "\n")
}

/// Claude Code 的 Anthropic SDK 会自行追加 `/v1/messages`，base 末尾的
/// `/v1` 必须去掉，否则网关会收到 `/v1/v1/messages`。
fn anthropic_base_url(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches('/');
    trimmed.strip_suffix("/v1").unwrap_or(trimmed).to_string()
}

fn claude_env(home: &Path, provider: &ApiProvider, model_id: &str) -> Vec<(String, String)> {
    let mut env = vec![
        (
            "CLAUDE_CONFIG_DIR".to_string(),
            home.to_string_lossy().to_string(),
        ),
        (
            "ANTHROPIC_BASE_URL".to_string(),
            anthropic_base_url(&provider.base_url),
        ),
        ("ANTHROPIC_MODEL".to_string(), model_id.to_string()),
    ];
    if !provider.api_key.trim().is_empty() {
        env.push(("ANTHROPIC_AUTH_TOKEN".to_string(), provider.api_key.clone()));
    }
    env
}

/// 字符串字面量转义：只处理 TOML 基本字符串需要的反斜杠与双引号。
fn toml_string(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn write_config(home: &Path, agent: &str, content: String) -> AppResult<()> {
    let path = home.join("config.toml");
    fs::write(&path, content).map_err(|error| {
        AppError::with_source(
            ErrorCode::HarnessRunFailed,
            format!("无法写入 {agent} 的供应商配置：{}", path.display()),
            error,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::provider::ProviderModel;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn harness(name: &str, command: &str, args: &[&str]) -> CustomHarness {
        CustomHarness {
            id: 1,
            name: name.to_string(),
            command: command.to_string(),
            args: args.iter().map(|arg| arg.to_string()).collect(),
            env: BTreeMap::new(),
            created_at: 0,
            agent_count: 0,
            capabilities: None,
            probed_at: None,
            probe_error: None,
            probe_failed_at: None,
        }
    }

    fn provider(protocol: ProviderProtocol, api_key: &str) -> ApiProvider {
        ApiProvider {
            id: 7,
            name: "Moon Provider".to_string(),
            protocol,
            base_url: "https://opencode.ai/zen/go/v1".to_string(),
            api_key: api_key.to_string(),
            enabled: true,
            models: vec![ProviderModel {
                id: "deepseek-v4.1-flash".to_string(),
                name: "DeepSeek V4.1 Flash".to_string(),
                context_window: Some(400_000),
                reasoning_levels: vec!["high".to_string(), "max".to_string()],
                input_types: vec!["text".to_string(), "image".to_string()],
            }],
            agent_count: 0,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn config(model: &str, reasoning: Option<&str>) -> ModelConfig {
        let mut config = ModelConfig::new();
        if !model.is_empty() {
            config.insert("model".to_string(), json!(model));
        }
        if let Some(level) = reasoning {
            config.insert("reasoningLevel".to_string(), json!(level));
        }
        config
    }

    fn temp_base(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "moon-acp-provider-test-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn env_value<'a>(plan: &'a ProviderBridgePlan, key: &str) -> Option<&'a str> {
        plan.env
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    #[test]
    fn detects_supported_agents_by_package_and_command() {
        assert_eq!(
            detect_bridge_kind(
                "npx.cmd",
                &[
                    "-y".to_string(),
                    "@moonshot-ai/kimi-code@2.6.2".to_string(),
                    "acp".to_string()
                ]
            ),
            Some(BridgeKind::Kimi)
        );
        assert_eq!(
            detect_bridge_kind(
                "node",
                &[r"C:\cache\@zed-industries\codex-acp\bin\codex-acp.js".to_string()]
            ),
            Some(BridgeKind::Codex)
        );
        assert_eq!(
            detect_bridge_kind("claude-code-acp", &[]),
            Some(BridgeKind::Claude)
        );
        assert_eq!(
            detect_bridge_kind("npx", &["-y".to_string(), "some-agent".to_string()]),
            None
        );
    }

    #[test]
    fn kimi_config_registers_provider_model_and_default() {
        let base = temp_base("kimi");
        let h = harness(
            "Kimi",
            "npx.cmd",
            &["-y", "@moonshot-ai/kimi-code@2.6.2", "acp"],
        );
        let plan = prepare_provider_bridge(
            &h,
            &provider(ProviderProtocol::Openai, "secret"),
            Some(&config("deepseek-v4.1-flash", Some("max"))),
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(plan.kind, BridgeKind::Kimi);
        assert_eq!(
            plan.effective_model.as_deref(),
            Some("moon-7/deepseek-v4.1-flash")
        );
        let home = env_value(&plan, "KIMI_CODE_HOME").unwrap().to_string();
        let text = fs::read_to_string(Path::new(&home).join("config.toml")).unwrap();
        assert!(text.contains("default_model = \"moon-7/deepseek-v4.1-flash\""));
        assert!(text.contains("[providers.\"moon-7\"]"));
        assert!(text.contains("type = \"openai\""));
        assert!(text.contains("api_key = \"secret\""));
        assert!(text.contains("[models.\"moon-7/deepseek-v4.1-flash\"]"));
        assert!(text.contains("max_context_size = 400000"));
        assert!(text.contains("\"thinking\""));
        assert!(text.contains("\"image_in\""));
        assert!(text.contains("default_effort = \"max\""));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn codex_config_uses_env_key_and_never_writes_secret() {
        let h = harness("Codex", "npx.cmd", &["-y", "@zed-industries/codex-acp"]);
        let plan = prepare_provider_bridge(
            &h,
            &provider(ProviderProtocol::Openai, "secret"),
            Some(&config("deepseek-v4.1-flash", None)),
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(plan.kind, BridgeKind::Codex);
        let home = env_value(&plan, "CODEX_HOME").unwrap().to_string();
        let text = fs::read_to_string(Path::new(&home).join("config.toml")).unwrap();
        assert!(text.contains("model = \"deepseek-v4.1-flash\""));
        assert!(text.contains("model_provider = \"moon\""));
        assert!(text.contains("wire_api = \"responses\""));
        assert!(text.contains("env_key = \"MOON_ACP_PROVIDER_KEY\""));
        assert!(!text.contains("secret"));
        assert_eq!(env_value(&plan, CODEX_PROVIDER_KEY_ENV), Some("secret"));
        assert_eq!(plan.effective_model.as_deref(), Some("deepseek-v4.1-flash"));
    }

    #[test]
    fn claude_env_isolates_config_and_normalizes_v1_base() {
        let h = harness(
            "Claude",
            "npx.cmd",
            &["-y", "@zed-industries/claude-code-acp"],
        );
        let plan = prepare_provider_bridge(
            &h,
            &provider(ProviderProtocol::Anthropic, "secret"),
            Some(&config("deepseek-v4.1-flash", None)),
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(plan.kind, BridgeKind::Claude);
        assert_eq!(
            env_value(&plan, "ANTHROPIC_BASE_URL"),
            Some("https://opencode.ai/zen/go")
        );
        assert_eq!(
            env_value(&plan, "ANTHROPIC_MODEL"),
            Some("deepseek-v4.1-flash")
        );
        assert_eq!(env_value(&plan, "ANTHROPIC_AUTH_TOKEN"), Some("secret"));
        let home = env_value(&plan, "CLAUDE_CONFIG_DIR").unwrap();
        assert!(Path::new(home).is_dir());
    }

    #[test]
    fn missing_model_or_unknown_agent_keeps_env_only() {
        let provider = provider(ProviderProtocol::Openai, "secret");
        let kimi = harness("Kimi", "npx.cmd", &["@moonshot-ai/kimi-code"]);
        assert!(
            prepare_provider_bridge(&kimi, &provider, Some(&config("", None)), None)
                .unwrap()
                .is_none()
        );
        let other = harness("Other", "npx.cmd", &["some-acp-agent"]);
        assert!(prepare_provider_bridge(
            &other,
            &provider,
            Some(&config("deepseek-v4.1-flash", None)),
            None
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn protocol_mismatch_fails_before_spawn_with_actionable_message() {
        let codex = harness("Codex", "npx.cmd", &["@zed-industries/codex-acp"]);
        let error = prepare_provider_bridge(
            &codex,
            &provider(ProviderProtocol::Anthropic, "secret"),
            Some(&config("deepseek-v4.1-flash", None)),
            None,
        )
        .unwrap_err();
        assert!(error.message.contains("OpenAI 兼容"));

        let claude = harness("Claude", "npx.cmd", &["@zed-industries/claude-code-acp"]);
        let error = prepare_provider_bridge(
            &claude,
            &provider(ProviderProtocol::Openai, "secret"),
            Some(&config("deepseek-v4.1-flash", None)),
            None,
        )
        .unwrap_err();
        assert!(error.message.contains("Anthropic 兼容"));
    }

    #[test]
    fn session_home_persists_and_temp_home_is_removed() {
        let storage = temp_base("session");
        let session = RunSessionContext {
            harness_session_id: None,
            storage_dir: storage.to_string_lossy().to_string(),
            last_context_message_id: 0,
        };
        let h = harness("Kimi", "npx.cmd", &["@moonshot-ai/kimi-code"]);
        let plan = prepare_provider_bridge(
            &h,
            &provider(ProviderProtocol::Openai, "secret"),
            Some(&config("deepseek-v4.1-flash", None)),
            Some(&session),
        )
        .unwrap()
        .unwrap();
        let home = env_value(&plan, "KIMI_CODE_HOME").unwrap().to_string();
        assert!(Path::new(&home).join("config.toml").exists());
        drop(plan);
        assert!(Path::new(&home).join("config.toml").exists());

        let temp = prepare_provider_bridge(
            &h,
            &provider(ProviderProtocol::Openai, "secret"),
            Some(&config("deepseek-v4.1-flash", None)),
            None,
        )
        .unwrap()
        .unwrap();
        let temp_home = env_value(&temp, "KIMI_CODE_HOME").unwrap().to_string();
        assert!(Path::new(&temp_home).exists());
        drop(temp);
        assert!(!Path::new(&temp_home).exists());
        let _ = fs::remove_dir_all(storage);
    }
}
