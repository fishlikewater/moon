//! Codex 本地模型目录读取。
//!
//! `~/.codex/config.toml` 通过根级 `model_catalog_json` 指向模型目录 JSON；
//! 目录项包含 `slug`、`supported_reasoning_levels` 与 `default_reasoning_level`。
//! 这里只读取模型展示所需字段，不输出配置文件中的其他内容。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::adapters::harness_config_error::{HarnessConfigError, HarnessConfigResult};
use crate::domain::harness::HarnessModelOption;

pub struct ReadCodexModelCatalogOptions {
    pub home_dir: Option<PathBuf>,
}

/// 读取 Codex 模型目录；失败只返回安全文案。
pub fn read_codex_model_catalog(
    options: ReadCodexModelCatalogOptions,
) -> HarnessConfigResult<Vec<HarnessModelOption>> {
    let home_dir = std::env::var("CODEX_HOME")
        .ok()
        .and_then(|value| non_empty(&value))
        .map(PathBuf::from)
        .or(options.home_dir)
        .unwrap_or_else(|| home_dir().join(".codex"));

    let config_path = home_dir.join("config.toml");
    let config_text = read_text(&config_path, "未找到 Codex 配置", "无法读取 Codex 配置")?;
    let provider_id = read_root_string(&config_text, "model_provider").unwrap_or_else(|| "codex".to_string());
    let catalog_path = locate_catalog(
        &home_dir,
        read_root_string(&config_text, "model_catalog_json").as_deref(),
    )?;

    let raw = read_text(&catalog_path, "未找到 Codex 模型目录", "无法读取 Codex 模型目录")?;
    let document: Value = serde_json::from_str(&raw)
        .map_err(|_| HarnessConfigError("Codex 模型目录不是合法 JSON".to_string()))?;
    let Some(raw_models) = document.get("models").and_then(Value::as_array) else {
        return Err(HarnessConfigError("Codex 模型目录结构不完整".to_string()));
    };

    let mut models = Vec::new();
    let mut seen = HashSet::new();
    for raw_model in raw_models {
        let Some(model) = raw_model.as_object() else {
            continue;
        };
        if model.get("supported_in_api").and_then(Value::as_bool) == Some(false) {
            continue;
        }
        if let Some(visibility) = model.get("visibility").and_then(Value::as_str) {
            if visibility != "list" {
                continue;
            }
        }
        let Some(model_id) = model.get("slug").and_then(Value::as_str).and_then(non_empty) else {
            continue;
        };
        let key = model_key(&provider_id, &model_id);
        if !seen.insert(key.clone()) {
            continue;
        }
        models.push(HarnessModelOption {
            key,
            provider_id: provider_id.clone(),
            provider_name: "Codex".to_string(),
            model_id,
            reasoning_levels: read_levels(model.get("supported_reasoning_levels")),
            default_reasoning_level: model
                .get("default_reasoning_level")
                .and_then(Value::as_str)
                .and_then(non_empty),
        });
    }

    if models.is_empty() {
        return Err(HarnessConfigError("Codex 配置中没有可用模型".to_string()));
    }
    Ok(models)
}

fn locate_catalog(home_dir: &Path, configured: Option<&str>) -> HarnessConfigResult<PathBuf> {
    if let Some(configured) = configured {
        let candidate = PathBuf::from(configured);
        let candidate = if candidate.is_absolute() {
            candidate
        } else {
            home_dir.join(candidate)
        };
        if candidate.is_file() {
            return Ok(candidate);
        }
        return Err(HarnessConfigError("未找到 Codex 模型目录".to_string()));
    }

    let preferred = home_dir.join("ai-toolbox-codex-model-catalog.json");
    if preferred.is_file() {
        return Ok(preferred);
    }

    let entries = std::fs::read_dir(home_dir)
        .map_err(|_| HarnessConfigError("无法读取 Codex 配置目录".to_string()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(document) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        if document.get("models").is_some_and(Value::is_array) {
            return Ok(path);
        }
    }
    Err(HarnessConfigError("未找到 Codex 模型目录".to_string()))
}

/// 读取 TOML 根级字符串；遇到第一个表头后不再向下扫描。
fn read_root_string(text: &str, key: &str) -> Option<String> {
    for raw_line in text.lines() {
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            return None;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if name.trim() != key {
            continue;
        }
        if let Some(decoded) = parse_toml_string(value.trim()) {
            return Some(decoded);
        }
    }
    None
}

fn read_levels(value: Option<&Value>) -> Vec<String> {
    let Some(items) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut levels = Vec::new();
    for item in items {
        let level = match item {
            Value::String(text) => non_empty(text),
            Value::Object(map) => map.get("effort").and_then(Value::as_str).and_then(non_empty),
            _ => None,
        };
        if let Some(level) = level {
            if !levels.contains(&level) {
                levels.push(level);
            }
        }
    }
    levels
}

fn read_text(path: &Path, missing: &str, other: &str) -> HarnessConfigResult<String> {
    std::fs::read_to_string(path).map_err(|error| {
        HarnessConfigError(if error.kind() == std::io::ErrorKind::NotFound {
            missing.to_string()
        } else {
            other.to_string()
        })
    })
}

fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    let mut previous = '\0';
    for (index, character) in line.char_indices() {
        if character == '"' && previous != '\\' {
            in_string = !in_string;
        } else if character == '#' && !in_string {
            return &line[..index];
        }
        previous = character;
    }
    line
}

fn parse_toml_string(value: &str) -> Option<String> {
    if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
        let inner = &value[1..value.len() - 1];
        return Some(
            serde_json::from_str::<String>(&format!("\"{inner}\""))
                .unwrap_or_else(|_| inner.to_string()),
        );
    }
    if value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2 {
        return Some(value[1..value.len() - 1].to_string());
    }
    None
}

pub(crate) fn model_key(provider_id: &str, model_id: &str) -> String {
    serde_json::to_string(&[provider_id, model_id]).unwrap_or_else(|_| {
        format!("[\"{provider_id}\",\"{model_id}\"]")
    })
}

pub(crate) fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

pub(crate) fn home_dir() -> PathBuf {
    std::env::var("USERPROFILE")
        .ok()
        .and_then(|value| non_empty(&value))
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .and_then(|value| non_empty(&value))
                .map(PathBuf::from)
        })
        .unwrap_or_else(|| PathBuf::from("."))
}
