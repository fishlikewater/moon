//! Kimi Code 本地模型目录读取。
//!
//! `~/.kimi-code/config.toml` 的 `[models."..."]` 表包含 `provider`、`model`、
//! `support_efforts` 与 `default_effort`。这里只做定向只读解析，不处理其他配置表，
//! 也不输出任何凭据字段。

use std::collections::HashSet;
use std::path::PathBuf;

use crate::adapters::codex_config::{model_key, non_empty};
use crate::adapters::harness_config_error::{HarnessConfigError, HarnessConfigResult};
use crate::domain::harness::HarnessModelOption;

pub struct ReadKimiModelCatalogOptions {
    pub home_dir: Option<PathBuf>,
}

struct KimiModelSection {
    section_key: String,
    provider: Option<String>,
    model: Option<String>,
    levels: Vec<String>,
    default_level: Option<String>,
}

pub fn read_kimi_model_catalog(
    options: ReadKimiModelCatalogOptions,
) -> HarnessConfigResult<Vec<HarnessModelOption>> {
    let home_dir = std::env::var("KIMI_CODE_HOME")
        .ok()
        .and_then(|value| non_empty(&value))
        .or_else(|| {
            std::env::var("KIMI_HOME")
                .ok()
                .and_then(|value| non_empty(&value))
        })
        .map(PathBuf::from)
        .or(options.home_dir)
        .unwrap_or_else(|| crate::adapters::codex_config::home_dir().join(".kimi-code"));

    let config_path = home_dir.join("config.toml");
    let text = std::fs::read_to_string(&config_path).map_err(|error| {
        HarnessConfigError(if error.kind() == std::io::ErrorKind::NotFound {
            "未找到 Kimi Code 配置".to_string()
        } else {
            "无法读取 Kimi Code 配置".to_string()
        })
    })?;

    let models = parse_model_sections(&text);
    if models.is_empty() {
        return Err(HarnessConfigError("Kimi 配置中没有可用模型".to_string()));
    }
    Ok(models)
}

fn parse_model_sections(text: &str) -> Vec<HarnessModelOption> {
    let lines: Vec<&str> = text.lines().collect();
    let mut models = Vec::new();
    let mut seen = HashSet::new();
    let mut current: Option<KimiModelSection> = None;

    let mut commit = |section: Option<KimiModelSection>| {
        let Some(section) = section else {
            return;
        };
        let model_id = section
            .model
            .clone()
            .or_else(|| model_from_section_key(&section.section_key));
        let Some(model_id) = model_id else {
            return;
        };
        let provider_id = section
            .provider
            .clone()
            .unwrap_or_else(|| provider_from_section_key(&section.section_key));
        let key = model_key(&provider_id, &model_id);
        if seen.insert(key.clone()) {
            models.push(HarnessModelOption {
                key,
                provider_name: provider_id.clone(),
                provider_id,
                model_id,
                reasoning_levels: section.levels,
                default_reasoning_level: section.default_level,
            });
        }
    };

    let mut index = 0;
    while index < lines.len() {
        let line = strip_comment(lines[index]).trim().to_string();
        index += 1;
        if line.is_empty() {
            continue;
        }

        if line.starts_with('[') {
            commit(current.take());
            if let Some(section_key) = parse_model_section_header(&line) {
                current = Some(KimiModelSection {
                    section_key,
                    provider: None,
                    model: None,
                    levels: Vec::new(),
                    default_level: None,
                });
            }
            continue;
        }

        let Some(section) = current.as_mut() else {
            continue;
        };
        let Some((key, mut value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        value = value.trim();

        if key == "support_efforts" {
            let mut combined = value.to_string();
            while !combined.contains(']') && index < lines.len() {
                combined.push(' ');
                combined.push_str(strip_comment(lines[index]).trim());
                index += 1;
            }
            section.levels = parse_quoted_array(&combined);
            continue;
        }

        match key {
            "provider" => section.provider = parse_toml_string(value),
            "model" => section.model = parse_toml_string(value),
            "default_effort" => section.default_level = parse_toml_string(value),
            _ => {}
        }
    }
    commit(current.take());

    models
}

fn parse_model_section_header(line: &str) -> Option<String> {
    let inner = line.strip_prefix("[models.")?.strip_suffix(']')?;
    let raw = inner.trim();
    if raw.len() >= 2 && raw.starts_with('"') && raw.ends_with('"') {
        return Some(decode_quoted(&raw[1..raw.len() - 1]));
    }
    if raw.len() >= 2 && raw.starts_with('\'') && raw.ends_with('\'') {
        return Some(raw[1..raw.len() - 1].to_string());
    }
    Some(raw.to_string())
}

fn parse_quoted_array(value: &str) -> Vec<String> {
    let mut levels = Vec::new();
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'"' {
            index += 1;
            continue;
        }
        let start = index + 1;
        index = start;
        while index < bytes.len() {
            if bytes[index] == b'\\' {
                index += 2;
                continue;
            }
            if bytes[index] == b'"' {
                break;
            }
            index += 1;
        }
        if index >= bytes.len() {
            break;
        }
        if let Ok(raw) = std::str::from_utf8(&bytes[start..index]) {
            let level = decode_quoted(raw);
            if !level.is_empty() && !levels.contains(&level) {
                levels.push(level);
            }
        }
        index += 1;
    }
    levels
}

fn parse_toml_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        return Some(decode_quoted(&trimmed[1..trimmed.len() - 1]));
    }
    if trimmed.len() >= 2 && trimmed.starts_with('\'') && trimmed.ends_with('\'') {
        return Some(trimmed[1..trimmed.len() - 1].to_string());
    }
    non_empty(trimmed)
}

fn decode_quoted(value: &str) -> String {
    serde_json::from_str::<String>(&format!("\"{value}\""))
        .unwrap_or_else(|_| value.to_string())
}

fn model_from_section_key(section_key: &str) -> Option<String> {
    let separator = section_key.find('/')?;
    if separator == section_key.len() - 1 {
        return None;
    }
    Some(section_key[separator + 1..].to_string())
}

fn provider_from_section_key(section_key: &str) -> String {
    match section_key.find('/') {
        Some(0) | None => section_key.to_string(),
        Some(separator) => section_key[..separator].to_string(),
    }
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
