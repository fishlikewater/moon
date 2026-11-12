//! harness 模型目录查询。
//!
//! 路由只做转发；此处负责 harness 枚举校验、不可用原因和本地配置失败降级。
//! Kimi Code 只读取模型配置，运行适配器仍未实现，message 中明确说明。

use std::path::Path;

use crate::adapters::codex_config::{read_codex_model_catalog, ReadCodexModelCatalogOptions};
use crate::adapters::kimi_config::{read_kimi_model_catalog, ReadKimiModelCatalogOptions};
use crate::adapters::zcode_config::{
    read_zcode_model_catalog, resolve_zcode_builtin_config_path,
};
use crate::domain::agent::HarnessKind;
use crate::domain::harness::{HarnessModelCatalog, HarnessModelOption};

pub fn get_harness_model_catalog(harness: HarnessKind) -> HarnessModelCatalog {
    match harness {
        HarnessKind::Zcode => {
            let builtin = resolve_zcode_builtin_config_path();
            read_catalog(
                harness,
                || {
                    read_zcode_model_catalog(builtin.as_deref()).map_err(|error| error.0)
                },
                None,
                "ZCode 配置中没有可用模型",
            )
        }
        HarnessKind::Codex => read_catalog(
            harness,
            || {
                read_codex_model_catalog(ReadCodexModelCatalogOptions { home_dir: None })
                    .map_err(|error| error.0)
            },
            Some("模型列表来自 Codex 本地配置。"),
            "Codex 配置中没有可用模型",
        ),
        HarnessKind::KimiCode => read_catalog(
            harness,
            || {
                read_kimi_model_catalog(ReadKimiModelCatalogOptions { home_dir: None })
                    .map_err(|error| error.0)
            },
            Some("模型列表来自 Kimi Code 本地配置；Kimi Code 运行适配器尚未实现。"),
            "Kimi 配置中没有可用模型",
        ),
        HarnessKind::Fake => HarnessModelCatalog {
            harness,
            available: false,
            models: Vec::new(),
            message: Some("fake harness 仅用于测试，不使用模型".to_string()),
        },
        HarnessKind::Acp => HarnessModelCatalog {
            harness,
            available: false,
            models: Vec::new(),
            message: Some(
                "自定义 ACP agent 的模型由 agent 侧决定；可在模型配置中手填 model id 留空使用 agent 默认模型。"
                    .to_string(),
            ),
        },
    }
}

/// 测试辅助：允许注入内置配置路径，避免读取真实安装目录。
pub fn get_harness_model_catalog_with_builtin(
    harness: HarnessKind,
    builtin_config_path: Option<&Path>,
) -> HarnessModelCatalog {
    if harness == HarnessKind::Zcode {
        read_catalog(
            harness,
            || read_zcode_model_catalog(builtin_config_path).map_err(|error| error.0),
            None,
            "ZCode 配置中没有可用模型",
        )
    } else {
        get_harness_model_catalog(harness)
    }
}

fn read_catalog(
    harness: HarnessKind,
    reader: impl FnOnce() -> Result<Vec<HarnessModelOption>, String>,
    success_message: Option<&str>,
    empty_message: &str,
) -> HarnessModelCatalog {
    match reader() {
        Ok(models) if models.is_empty() => HarnessModelCatalog {
            harness,
            available: false,
            models,
            message: Some(empty_message.to_string()),
        },
        Ok(models) => HarnessModelCatalog {
            harness,
            available: true,
            models,
            message: success_message.map(str::to_string),
        },
        Err(message) => HarnessModelCatalog {
            harness,
            available: false,
            models: Vec::new(),
            message: Some(message),
        },
    }
}
