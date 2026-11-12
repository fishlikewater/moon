//! ZCode provider 配置读取与运行期模型覆盖。
//!
//! 配置来源与 zcode 自身保持一致：
//! 1. 环境变量 `ZCODE_PERSONAL_PROVIDER_CONFIG_FILE`
//! 2. `${ZCODE_DATA_BASE_DIR || home}/.zcode/v2/provider_config.json`
//!
//! 该文件包含 API 密钥等敏感字段。目录接口只向外暴露模型展示字段；
//! 运行期覆盖写入系统临时目录的独占副本，并在运行结束后清理。
//! 配置文件一律按 UTF-8 读取，不依赖系统默认编码。

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde_json::{Map, Value};

use crate::adapters::codex_config::{model_key, non_empty};
use crate::adapters::{unique_temp_dir, Cleanup};
use crate::domain::agent::ModelConfig;
use crate::domain::harness::HarnessModelOption;

#[derive(Debug, Clone)]
pub struct ZcodeConfigError(pub String);

impl fmt::Display for ZcodeConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ZcodeConfigError {}

type ConfigResult<T> = Result<T, ZcodeConfigError>;

/// 按 zcode 的优先级解析个人 provider 配置文件路径。
pub fn resolve_provider_config_path() -> PathBuf {
    if let Some(explicit) = std::env::var("ZCODE_PERSONAL_PROVIDER_CONFIG_FILE")
        .ok()
        .and_then(|value| non_empty(&value))
    {
        return PathBuf::from(explicit);
    }
    let base = std::env::var("ZCODE_DATA_BASE_DIR")
        .ok()
        .and_then(|value| non_empty(&value))
        .map(PathBuf::from)
        .unwrap_or_else(crate::adapters::codex_config::home_dir);
    base.join(".zcode").join("v2").join("provider_config.json")
}

/// ZCode 安装目录中的内置模型规则路径；缺失时返回 `None`。
///
/// 新版 ZCode 不再把内置 provider 放在安装目录，而是按版本生成到
/// `${ZCODE_DATA_BASE_DIR || home}/.zcode/v2/runtime/provider/` 下，并由
/// `ZCODE_BUILTIN_PROVIDER_CONFIG_FILE` 指向。桌面端从快捷方式启动时继承不到
/// 该变量，因此这里按同样规则自行定位，供运行与模型目录共用。
pub fn resolve_zcode_builtin_config_path() -> Option<PathBuf> {
    if let Some(override_path) = std::env::var("MOON_ZCODE_BUILTIN_CONFIG")
        .ok()
        .and_then(|value| non_empty(&value))
    {
        return Some(PathBuf::from(override_path));
    }
    if let Some(existing) = std::env::var("ZCODE_BUILTIN_PROVIDER_CONFIG_FILE")
        .ok()
        .and_then(|value| non_empty(&value))
        .map(PathBuf::from)
        .filter(|path| path.is_file())
    {
        return Some(existing);
    }

    let resolved = crate::adapters::zcode::resolve_zcode_command(None);
    let mut candidates: Vec<PathBuf> = resolved
        .args_prefix
        .iter()
        .map(PathBuf::from)
        .collect();
    candidates.push(PathBuf::from(&resolved.command));

    for candidate in candidates {
        if let Some(root) = candidate.parent().and_then(Path::parent) {
            let builtin = root.join("config").join("provider").join("zcode-builtin.json");
            if builtin.is_file() {
                return Some(builtin);
            }
        }
        // 原生 ZCode.exe：安装根目录下的 resources/config/provider。
        if let Some(root) = candidate.parent() {
            let builtin = root
                .join("resources")
                .join("config")
                .join("provider")
                .join("zcode-builtin.json");
            if builtin.is_file() {
                return Some(builtin);
            }
        }
    }

    let base = std::env::var("ZCODE_DATA_BASE_DIR")
        .ok()
        .and_then(|value| non_empty(&value))
        .map(PathBuf::from)
        .unwrap_or_else(crate::adapters::codex_config::home_dir);
    select_runtime_builtin(
        &base
            .join(".zcode")
            .join("v2")
            .join("runtime")
            .join("provider"),
    )
}

/// 从 `runtime/provider/<platform>/<version>/<endpoint>/zcode-builtin.json`
/// 中选择语义版本最高的配置；同版本取修改时间最新者。
fn select_runtime_builtin(provider_root: &Path) -> Option<PathBuf> {
    let mut best: Option<((u64, u64, u64), std::time::SystemTime, PathBuf)> = None;
    for platform in read_directories(provider_root) {
        for version in read_directories(&platform) {
            let Some(version_key) = version
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(parse_version)
            else {
                continue;
            };
            for endpoint in read_directories(&version) {
                let candidate = endpoint.join("zcode-builtin.json");
                if !candidate.is_file() {
                    continue;
                }
                let modified = std::fs::metadata(&candidate)
                    .and_then(|metadata| metadata.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                let is_newer = match &best {
                    Some((best_version, best_modified, _)) => {
                        (version_key, modified) > (*best_version, *best_modified)
                    }
                    None => true,
                };
                if is_newer {
                    best = Some((version_key, modified, candidate));
                }
            }
        }
    }
    best.map(|(_, _, path)| path)
}

fn read_directories(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| {
            entry
                .file_type()
                .map(|file_type| file_type.is_dir())
                .unwrap_or(false)
        })
        .map(|entry| entry.path())
        .collect()
}

fn parse_version(name: &str) -> Option<(u64, u64, u64)> {
    let mut parts = name.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

#[derive(Debug, Clone)]
struct ProviderRule {
    provider_id: String,
    provider_name: String,
    enabled: bool,
    template_id: Option<String>,
    config: Map<String, Value>,
}

#[derive(Debug, Clone, Default)]
struct ProviderModelInfo {
    reasoning_levels: Vec<String>,
    default_reasoning_level: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuleKind {
    Model,
    ModelApi,
    ProviderSite,
    TemplateModel,
    ProviderModel,
    ManualProviderModel,
}

#[derive(Debug, Clone)]
struct EffectiveModelRule {
    kind: RuleKind,
    provider_id: Option<String>,
    model_id: Option<String>,
    template_id: Option<String>,
    model_match: Option<String>,
    api_type_match: Option<String>,
    base_url_match: Option<String>,
    config: Map<String, Value>,
}

#[derive(Debug, Clone, Default)]
struct ProviderMeta {
    template_id: Option<String>,
    api_type: Option<String>,
    base_url: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct BuiltinRules {
    rules: Vec<EffectiveModelRule>,
    providers: HashMap<String, ProviderMeta>,
    templates: HashMap<String, ProviderMeta>,
}

#[derive(Debug, Clone, Default)]
struct ResolvedProviderMeta {
    template_id: Option<String>,
    api_type: Option<String>,
    base_url: Option<String>,
}

/// 读取 ZCode 配置中的可选模型。
///
/// 只包含启用 provider；provider 顺序优先跟随 `providerOrder`。
/// 个人配置里有 `personalModelIds` 时以用户的模型列表为准，
/// 否则使用内置 provider 的 `providerModelRules`。
pub fn read_zcode_model_catalog(
    builtin_config_path: Option<&Path>,
) -> ConfigResult<Vec<HarnessModelOption>> {
    let config = read_config_document(&resolve_provider_config_path())?;
    let config = config
        .get("config")
        .and_then(Value::as_object)
        .ok_or_else(|| ZcodeConfigError("ZCode 模型配置结构不完整".to_string()))?;

    let provider_rules = read_provider_rules(config);
    let models_by_provider = read_provider_models(config);
    let builtin = read_builtin_rules(builtin_config_path);
    let personal_rules = read_personal_model_rules(config);
    let provider_order = read_string_list(config.get("providerOrder"));
    let provider_index: HashMap<&str, usize> = provider_rules
        .iter()
        .enumerate()
        .map(|(index, rule)| (rule.provider_id.as_str(), index))
        .collect();
    let order_index: HashMap<&str, usize> = provider_order
        .iter()
        .enumerate()
        .map(|(index, provider_id)| (provider_id.as_str(), index))
        .collect();

    let mut providers: Vec<&ProviderRule> = provider_rules
        .iter()
        .filter(|rule| rule.enabled)
        .collect();
    providers.sort_by_key(|rule| {
        (
            order_index
                .get(rule.provider_id.as_str())
                .copied()
                .unwrap_or(usize::MAX),
            provider_index
                .get(rule.provider_id.as_str())
                .copied()
                .unwrap_or(usize::MAX),
        )
    });

    let mut catalog = Vec::new();
    for provider in providers {
        let provider_models = models_by_provider.get(&provider.provider_id);
        let personal_model_ids = read_optional_string_list(provider.config.get("personalModelIds"));
        let model_ids = match personal_model_ids {
            None => provider_models
                .map(|models| models.iter().map(|(model_id, _)| model_id.clone()).collect())
                .unwrap_or_default(),
            Some(personal_model_ids) => unique_strings(
                read_string_list(provider.config.get("modelOrder"))
                    .into_iter()
                    .chain(personal_model_ids)
                    .collect(),
            ),
        };

        for model_id in model_ids {
            let info = provider_models.and_then(|models| {
                models
                    .iter()
                    .find(|(candidate, _)| candidate == &model_id)
                    .map(|(_, info)| info)
            });
            let reasoning = resolve_reasoning_levels(
                provider,
                &model_id,
                builtin.as_ref(),
                &personal_rules,
                info,
            );
            catalog.push(HarnessModelOption {
                key: model_key(&provider.provider_id, &model_id),
                provider_id: provider.provider_id.clone(),
                provider_name: provider.provider_name.clone(),
                model_id,
                reasoning_levels: reasoning.reasoning_levels,
                default_reasoning_level: reasoning.default_reasoning_level,
            });
        }
    }
    Ok(catalog)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZcodeModelSelection {
    pub provider_id: String,
    pub model_id: String,
    pub reasoning_level: Option<String>,
}

/// 只有 providerId 与 model 同时存在时才形成可应用的模型选择。
pub fn read_zcode_model_selection(config: Option<&ModelConfig>) -> Option<ZcodeModelSelection> {
    let config = config?;
    let provider_id = config.get("providerId").and_then(Value::as_str).and_then(non_empty)?;
    let model_id = config.get("model").and_then(Value::as_str).and_then(non_empty)?;
    Some(ZcodeModelSelection {
        provider_id,
        model_id,
        reasoning_level: config
            .get("reasoningLevel")
            .and_then(Value::as_str)
            .and_then(non_empty),
    })
}

/// 运行期模型覆盖：临时配置文件与清理载体。
pub struct ZcodeModelOverride {
    pub env: Vec<(String, String)>,
    _cleanup: Cleanup,
}

/// 复制个人 provider 配置，写入 `config.defaultModelSelection` 后供单次运行使用。
pub fn create_zcode_model_override(
    selection: &ZcodeModelSelection,
) -> ConfigResult<ZcodeModelOverride> {
    let source_path = resolve_provider_config_path();
    let mut document = read_config_document(&source_path)?;
    let config = document
        .get_mut("config")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| ZcodeConfigError("ZCode 模型配置结构不完整".to_string()))?;
    let mut selection_value = Map::new();
    selection_value.insert(
        "providerId".to_string(),
        Value::String(selection.provider_id.clone()),
    );
    selection_value.insert(
        "modelId".to_string(),
        Value::String(selection.model_id.clone()),
    );
    if let Some(level) = &selection.reasoning_level {
        let mut options = Map::new();
        options.insert("reasoningLevel".to_string(), Value::String(level.clone()));
        selection_value.insert("options".to_string(), Value::Object(options));
    }
    config.insert(
        "defaultModelSelection".to_string(),
        Value::Object(selection_value),
    );

    let dir = unique_temp_dir("moon-zcode-model-")
        .map_err(|_| ZcodeConfigError("无法创建模型配置临时目录".to_string()))?;
    let cleanup = Cleanup::directory(dir.clone());
    let file_path = dir.join("provider_config.json");
    let serialized = serde_json::to_string(&document)
        .map_err(|_| ZcodeConfigError("无法序列化模型配置".to_string()))?;
    // 显式按 UTF-8 写入，与 zcode 自身读取编码一致。
    std::fs::write(&file_path, format!("{serialized}\n").as_bytes())
        .map_err(|_| ZcodeConfigError("无法写入模型配置临时文件".to_string()))?;

    Ok(ZcodeModelOverride {
        env: vec![(
            "ZCODE_PERSONAL_PROVIDER_CONFIG_FILE".to_string(),
            file_path.to_string_lossy().into_owned(),
        )],
        _cleanup: cleanup,
    })
}

fn read_config_document(path: &Path) -> ConfigResult<Value> {
    let raw = std::fs::read_to_string(path).map_err(|error| {
        ZcodeConfigError(if error.kind() == std::io::ErrorKind::NotFound {
            "未找到 ZCode 模型配置".to_string()
        } else {
            "无法读取 ZCode 模型配置".to_string()
        })
    })?;
    let parsed: Value = serde_json::from_str(&raw)
        .map_err(|_| ZcodeConfigError("ZCode 模型配置不是合法 JSON".to_string()))?;
    if !parsed.is_object() || !parsed.get("config").is_some_and(Value::is_object) {
        return Err(ZcodeConfigError("ZCode 模型配置结构不完整".to_string()));
    }
    Ok(parsed)
}

fn read_provider_rules(config: &Map<String, Value>) -> Vec<ProviderRule> {
    let Some(raw_rules) = config
        .get("providerConfigRules")
        .and_then(Value::as_object)
        .and_then(|container| container.get("providerRules"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };

    let mut rules = Vec::new();
    for raw in raw_rules {
        let Some(raw) = raw.as_object() else {
            continue;
        };
        let Some(provider_id) = raw.get("providerId").and_then(Value::as_str).and_then(non_empty)
        else {
            continue;
        };
        let provider_name = raw
            .get("providerName")
            .and_then(Value::as_str)
            .and_then(non_empty)
            .or_else(|| {
                raw.get("templateId")
                    .and_then(Value::as_str)
                    .and_then(non_empty)
            })
            .unwrap_or_else(|| provider_id.clone());
        rules.push(ProviderRule {
            provider_id,
            provider_name,
            enabled: raw.get("enabled").and_then(Value::as_bool) != Some(false),
            template_id: raw.get("templateId").and_then(Value::as_str).and_then(non_empty),
            config: raw
                .get("config")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default(),
        });
    }
    rules
}

fn read_provider_models(
    config: &Map<String, Value>,
) -> HashMap<String, Vec<(String, ProviderModelInfo)>> {
    let mut models: HashMap<String, Vec<(String, ProviderModelInfo)>> = HashMap::new();
    let Some(raw_rules) = config
        .get("modelConfigRules")
        .and_then(Value::as_object)
        .and_then(|container| container.get("providerModelRules"))
        .and_then(Value::as_array)
    else {
        return models;
    };

    for raw in raw_rules {
        let Some(raw) = raw.as_object() else {
            continue;
        };
        let Some(provider_id) = raw.get("providerId").and_then(Value::as_str).and_then(non_empty)
        else {
            continue;
        };
        let Some(model_id) = raw.get("modelId").and_then(Value::as_str).and_then(non_empty) else {
            continue;
        };
        let provider_models = models.entry(provider_id).or_default();
        if provider_models
            .iter()
            .any(|(candidate, _)| candidate == &model_id)
        {
            continue;
        }
        let option_specs = raw
            .get("config")
            .and_then(Value::as_object)
            .and_then(|config| config.get("optionSpecs"))
            .and_then(Value::as_object);
        let reasoning_spec = option_specs
            .and_then(|specs| specs.get("reasoningLevel"))
            .and_then(Value::as_object);
        provider_models.push((
            model_id,
            ProviderModelInfo {
                reasoning_levels: reasoning_spec
                    .map(|spec| read_string_list(spec.get("values")))
                    .unwrap_or_default(),
                default_reasoning_level: reasoning_spec
                    .and_then(|spec| spec.get("default"))
                    .and_then(Value::as_str)
                    .and_then(non_empty),
            },
        ));
    }
    models
}

fn read_builtin_rules(path: Option<&Path>) -> Option<BuiltinRules> {
    let path = path?;
    let text = std::fs::read_to_string(path).ok()?;
    let document: Value = serde_json::from_str(&text).ok()?;
    let config = document.get("config").and_then(Value::as_object)?;
    let model_config = config.get("modelConfigRules").and_then(Value::as_object)?;

    let mut rules = Vec::new();
    append_builtin_rules(model_config.get("modelRules"), RuleKind::Model, &mut rules);
    append_builtin_rules(
        model_config.get("modelApiRules"),
        RuleKind::ModelApi,
        &mut rules,
    );
    append_builtin_rules(
        model_config.get("providerSiteRules"),
        RuleKind::ProviderSite,
        &mut rules,
    );
    append_builtin_rules(
        model_config.get("templateModelRules"),
        RuleKind::TemplateModel,
        &mut rules,
    );
    append_builtin_rules(
        model_config.get("builtinProviderModelRules"),
        RuleKind::ProviderModel,
        &mut rules,
    );

    let provider_config = config
        .get("providerConfigRules")
        .and_then(Value::as_object);
    Some(BuiltinRules {
        rules,
        providers: read_provider_meta_map(
            provider_config.and_then(|container| container.get("providerRules")),
            "providerId",
        ),
        templates: read_provider_meta_map(
            provider_config.and_then(|container| container.get("templateRules")),
            "templateId",
        ),
    })
}

fn append_builtin_rules(value: Option<&Value>, kind: RuleKind, rules: &mut Vec<EffectiveModelRule>) {
    let Some(items) = value.and_then(Value::as_array) else {
        return;
    };
    for raw in items {
        let Some(raw) = raw.as_object() else {
            continue;
        };
        rules.push(EffectiveModelRule {
            kind,
            provider_id: raw.get("providerId").and_then(Value::as_str).and_then(non_empty),
            model_id: raw.get("modelId").and_then(Value::as_str).and_then(non_empty),
            template_id: raw.get("templateId").and_then(Value::as_str).and_then(non_empty),
            model_match: raw.get("modelMatch").and_then(Value::as_str).and_then(non_empty),
            api_type_match: raw
                .get("apiTypeMatch")
                .and_then(Value::as_str)
                .and_then(non_empty),
            base_url_match: raw
                .get("baseUrlMatch")
                .and_then(Value::as_str)
                .and_then(non_empty),
            config: raw
                .get("config")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default(),
        });
    }
}

fn read_provider_meta_map(value: Option<&Value>, id_key: &str) -> HashMap<String, ProviderMeta> {
    let mut result = HashMap::new();
    let Some(items) = value.and_then(Value::as_array) else {
        return result;
    };
    for raw in items {
        let Some(raw) = raw.as_object() else {
            continue;
        };
        let Some(id) = raw.get(id_key).and_then(Value::as_str).and_then(non_empty) else {
            continue;
        };
        let config = raw.get("config").and_then(Value::as_object);
        let api = config
            .and_then(|config| config.get("api"))
            .and_then(Value::as_object);
        result.insert(
            id,
            ProviderMeta {
                template_id: raw.get("templateId").and_then(Value::as_str).and_then(non_empty),
                api_type: api
                    .and_then(|api| api.get("type"))
                    .and_then(Value::as_str)
                    .and_then(non_empty),
                base_url: api
                    .and_then(|api| api.get("baseUrl"))
                    .and_then(Value::as_str)
                    .and_then(non_empty),
            },
        );
    }
    result
}

fn read_personal_model_rules(config: &Map<String, Value>) -> Vec<EffectiveModelRule> {
    let Some(container) = config.get("modelConfigRules").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut rules = Vec::new();
    append_exact_rules(
        container.get("providerModelRules"),
        RuleKind::ProviderModel,
        &mut rules,
    );
    append_exact_rules(
        container.get("manualProviderModelRules"),
        RuleKind::ManualProviderModel,
        &mut rules,
    );
    rules
}

fn append_exact_rules(value: Option<&Value>, kind: RuleKind, rules: &mut Vec<EffectiveModelRule>) {
    let Some(items) = value.and_then(Value::as_array) else {
        return;
    };
    for raw in items {
        let Some(raw) = raw.as_object() else {
            continue;
        };
        let Some(provider_id) = raw.get("providerId").and_then(Value::as_str).and_then(non_empty)
        else {
            continue;
        };
        let Some(model_id) = raw.get("modelId").and_then(Value::as_str).and_then(non_empty) else {
            continue;
        };
        rules.push(EffectiveModelRule {
            kind,
            provider_id: Some(provider_id),
            model_id: Some(model_id),
            template_id: None,
            model_match: None,
            api_type_match: None,
            base_url_match: None,
            config: raw
                .get("config")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default(),
        });
    }
}

/// 按 ZCode 的规则顺序求有效配置：内置匹配规则 → 内置模板/精确规则 → 个人精确规则。
fn resolve_reasoning_levels(
    provider: &ProviderRule,
    model_id: &str,
    builtin: Option<&BuiltinRules>,
    personal_rules: &[EffectiveModelRule],
    personal: Option<&ProviderModelInfo>,
) -> ProviderModelInfo {
    let Some(builtin) = builtin else {
        return personal.cloned().unwrap_or_default();
    };

    let provider_meta = resolve_provider_meta(provider, builtin);
    let mut config = Map::new();
    for rule in builtin.rules.iter().chain(personal_rules.iter()) {
        if !matches_model_rule(rule, provider, model_id, &provider_meta) {
            continue;
        }
        config = overlay_config(config, &rule.config);
    }

    let reasoning_spec = config
        .get("optionSpecs")
        .and_then(Value::as_object)
        .and_then(|specs| specs.get("reasoningLevel"))
        .and_then(Value::as_object);
    let Some(reasoning_spec) = reasoning_spec else {
        return personal.cloned().unwrap_or_default();
    };

    ProviderModelInfo {
        reasoning_levels: read_string_list(reasoning_spec.get("values")),
        default_reasoning_level: reasoning_spec
            .get("default")
            .and_then(Value::as_str)
            .and_then(non_empty)
            .or_else(|| personal.and_then(|info| info.default_reasoning_level.clone())),
    }
}

fn resolve_provider_meta(provider: &ProviderRule, builtin: &BuiltinRules) -> ResolvedProviderMeta {
    let api = provider.config.get("api").and_then(Value::as_object);
    let provider_meta = builtin.providers.get(&provider.provider_id);
    let template_id = provider
        .template_id
        .clone()
        .or_else(|| provider_meta.and_then(|meta| meta.template_id.clone()));
    let template_meta = template_id
        .as_ref()
        .and_then(|template_id| builtin.templates.get(template_id));
    ResolvedProviderMeta {
        template_id,
        api_type: api
            .and_then(|api| api.get("type"))
            .and_then(Value::as_str)
            .and_then(non_empty)
            .or_else(|| provider_meta.and_then(|meta| meta.api_type.clone()))
            .or_else(|| template_meta.and_then(|meta| meta.api_type.clone())),
        base_url: api
            .and_then(|api| api.get("baseUrl"))
            .and_then(Value::as_str)
            .and_then(non_empty)
            .or_else(|| provider_meta.and_then(|meta| meta.base_url.clone()))
            .or_else(|| template_meta.and_then(|meta| meta.base_url.clone())),
    }
}

fn matches_model_rule(
    rule: &EffectiveModelRule,
    provider: &ProviderRule,
    model_id: &str,
    provider_meta: &ResolvedProviderMeta,
) -> bool {
    if matches!(rule.kind, RuleKind::ProviderModel | RuleKind::ManualProviderModel) {
        return rule.provider_id.as_deref() == Some(provider.provider_id.as_str())
            && rule.model_id.as_deref() == Some(model_id);
    }
    if rule.kind == RuleKind::TemplateModel {
        return provider_meta.template_id.is_some()
            && rule.template_id.as_deref() == provider_meta.template_id.as_deref()
            && rule.model_id.as_deref() == Some(model_id);
    }
    let Some(model_match) = rule.model_match.as_deref() else {
        return false;
    };
    if !matches_regex(model_match, model_id) {
        return false;
    }
    if matches!(rule.kind, RuleKind::ModelApi | RuleKind::ProviderSite) {
        if let Some(api_type_match) = rule.api_type_match.as_deref() {
            let Some(api_type) = provider_meta.api_type.as_deref() else {
                return false;
            };
            if !matches_regex(api_type_match, api_type) {
                return false;
            }
        }
    }
    if rule.kind == RuleKind::ProviderSite {
        if let Some(base_url_match) = rule.base_url_match.as_deref() {
            let Some(base_url) = provider_meta.base_url.as_deref() else {
                return false;
            };
            if !matches_regex(base_url_match, &normalize_rule_base_url(base_url)) {
                return false;
            }
        }
    }
    true
}

fn matches_regex(pattern: &str, value: &str) -> bool {
    Regex::new(&format!("(?i)^(?:{pattern})$"))
        .map(|regex| regex.is_match(value))
        .unwrap_or(false)
}

fn normalize_rule_base_url(value: &str) -> String {
    let (base, tail) = match value.find(|character| character == '?' || character == '#') {
        Some(index) => (&value[..index], &value[index..]),
        None => (value, ""),
    };
    format!("{}{}", base.trim_end_matches('/'), tail)
}

fn overlay_config(base: Map<String, Value>, next: &Map<String, Value>) -> Map<String, Value> {
    let mut result = base;
    for (key, value) in next {
        let merged = match (result.get(key), value) {
            (Some(Value::Object(current)), Value::Object(next)) => {
                Value::Object(overlay_config(current.clone(), next))
            }
            _ => value.clone(),
        };
        result.insert(key.clone(), merged);
    }
    result
}

fn read_optional_string_list(value: Option<&Value>) -> Option<Vec<String>> {
    value.and_then(Value::as_array).map(|_| read_string_list(value))
}

fn read_string_list(value: Option<&Value>) -> Vec<String> {
    let Some(items) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    unique_strings(
        items
            .iter()
            .filter_map(Value::as_str)
            .filter_map(non_empty)
            .collect(),
    )
}

fn unique_strings(values: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "moon-zcode-config-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn write_builtin(root: &Path, version: &str, endpoint: &str) -> PathBuf {
        let dir = root
            .join("windows-x86_64")
            .join(version)
            .join(endpoint);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("zcode-builtin.json");
        std::fs::write(&path, "{}").unwrap();
        path
    }

    #[test]
    fn selects_highest_version_by_number_not_lexicographic_order() {
        let root = temp_root("version");
        write_builtin(&root, "3.9.9", "endpoint-a");
        let newest = write_builtin(&root, "3.17.2", "endpoint-b");
        write_builtin(&root, "3.16.3", "endpoint-c");

        assert_eq!(select_runtime_builtin(&root), Some(newest));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn skips_version_directories_without_builtin_config() {
        let root = temp_root("missing");
        write_builtin(&root, "3.16.3", "endpoint-a");
        std::fs::create_dir_all(root.join("windows-x86_64").join("3.18.0").join("endpoint-b"))
            .unwrap();

        assert_eq!(
            select_runtime_builtin(&root),
            Some(
                root.join("windows-x86_64")
                    .join("3.16.3")
                    .join("endpoint-a")
                    .join("zcode-builtin.json")
            )
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn returns_none_when_runtime_directory_is_absent() {
        let root = temp_root("absent");
        assert_eq!(select_runtime_builtin(&root), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
