//! 智能体领域类型。

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::{AppError, AppResult};

/// harness 类型枚举；`zcode` / `codex` / `fake` 已实现，`kimi-code` 为占位，
/// `acp` 走自定义 ACP agent 注册表。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HarnessKind {
    Codex,
    Zcode,
    KimiCode,
    Fake,
    Acp,
}

impl HarnessKind {
    pub const ALL: [Self; 5] = [Self::Codex, Self::Zcode, Self::KimiCode, Self::Fake, Self::Acp];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Zcode => "zcode",
            Self::KimiCode => "kimi-code",
            Self::Fake => "fake",
            Self::Acp => "acp",
        }
    }

    pub fn parse(value: &str) -> AppResult<Self> {
        match value {
            "codex" => Ok(Self::Codex),
            "zcode" => Ok(Self::Zcode),
            "kimi-code" => Ok(Self::KimiCode),
            "fake" => Ok(Self::Fake),
            "acp" => Ok(Self::Acp),
            _ => Err(AppError::validation(format!(
                "harness 必须是 {} 之一",
                Self::ALL.map(Self::as_str).join(" | ")
            ))),
        }
    }
}

/// 智能体模型配置：任意 JSON 对象，字段语义由各 harness 适配器解释。
pub type ModelConfig = Map<String, Value>;

/// 内置智能体角色；当前只有房间主持人。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BuiltinAgentRole {
    Moderator,
}

impl BuiltinAgentRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Moderator => "moderator",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "moderator" => Some(Self::Moderator),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Agent {
    pub id: i64,
    pub name: String,
    pub persona: String,
    pub harness: HarnessKind,
    pub model_config: Option<ModelConfig>,
    /// 非 `None` 表示系统内置智能体；内置主持人不可删除或改身份字段。
    pub builtin_role: Option<BuiltinAgentRole>,
    /// harness 为 `acp` 时指向 `custom_harnesses` 的定义；其他 harness 为 `None`。
    pub custom_harness_id: Option<i64>,
    /// 智能体独立工作区；`None` 表示使用房间主工作区。
    pub workspace_dir: Option<String>,
    pub created_at: i64,
}

/// store 层创建参数。
#[derive(Debug, Clone)]
pub struct CreateAgentInput {
    pub name: String,
    pub persona: String,
    pub harness: HarnessKind,
    pub model_config: Option<ModelConfig>,
    pub builtin_role: Option<BuiltinAgentRole>,
    pub custom_harness_id: Option<i64>,
    pub workspace_dir: Option<String>,
}

/// 局部更新：未传入的字段保持原值，`model_config` / `custom_harness_id` /
/// `workspace_dir` 的 `Some(None)` 表示清空配置。
#[derive(Debug, Clone, Default)]
pub struct UpdateAgentInput {
    pub name: Option<String>,
    pub persona: Option<String>,
    pub harness: Option<HarnessKind>,
    pub model_config: Option<Option<ModelConfig>>,
    pub custom_harness_id: Option<Option<i64>>,
    pub workspace_dir: Option<Option<String>>,
}
