//! 模型供应商领域对象：协议枚举、模型高级属性与协议→env 映射。
//!
//! env 键名按协议由内置映射表决定：claude code 系消费 ANTHROPIC_BASE_URL/
//! ANTHROPIC_AUTH_TOKEN，OpenAI 兼容系 CLI 消费 OPENAI_BASE_URL/OPENAI_API_KEY。

use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// 供应商协议；决定 spawn 注入的 env 键名组。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderProtocol {
    Anthropic,
    Openai,
}

impl ProviderProtocol {
    pub const ALL: [Self; 2] = [Self::Anthropic, Self::Openai];

    pub const fn as_str(self) -> &'static str {
        match self {
            ProviderProtocol::Anthropic => "anthropic",
            ProviderProtocol::Openai => "openai",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, AppError> {
        match raw {
            "anthropic" => Ok(Self::Anthropic),
            "openai" => Ok(Self::Openai),
            _ => Err(AppError::validation(format!(
                "protocol 必须是 {} 之一",
                Self::ALL.map(Self::as_str).join(" | ")
            ))),
        }
    }
}

/// 供应商下的一个模型及其高级属性（对齐 ZCode 桌面端模型管理的关键设置）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModel {
    pub id: String,
    pub name: String,
    /// 上下文窗口（tokens）；缺省视为未设置。
    #[serde(default)]
    pub context_window: Option<i64>,
    /// 支持的思考等级（如 standard / deep）；空数组表示未配置。
    #[serde(default)]
    pub reasoning_levels: Vec<String>,
    /// 支持的输入类型（text/image/video 子集）；空数组视为仅文本。
    #[serde(default)]
    pub input_types: Vec<String>,
}

impl ProviderModel {
    /// 合法输入类型全集；供应商模型只能声明其子集。
    pub const INPUT_TYPES: [&str; 3] = ["text", "image", "video"];

    pub fn validate(&self) -> Result<(), AppError> {
        if self.id.trim().is_empty() {
            return Err(AppError::validation("模型 id 不能为空"));
        }
        if self.name.trim().is_empty() {
            return Err(AppError::validation(format!(
                "模型「{}」的 name 不能为空",
                self.id
            )));
        }
        if self.context_window.is_some_and(|window| window <= 0) {
            return Err(AppError::validation(format!(
                "模型「{}」的 contextWindow 必须是正整数",
                self.id
            )));
        }
        for level in &self.reasoning_levels {
            if level.trim().is_empty() {
                return Err(AppError::validation(format!(
                    "模型「{}」的思考等级不能为空字符串",
                    self.id
                )));
            }
        }
        for kind in &self.input_types {
            if !Self::INPUT_TYPES.contains(&kind.as_str()) {
                return Err(AppError::validation(format!(
                    "模型「{}」的输入类型「{kind}」无效，只支持 {}",
                    self.id,
                    Self::INPUT_TYPES.join(" / ")
                )));
            }
        }
        Ok(())
    }
}

/// 模型供应商注册表条目。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiProvider {
    pub id: i64,
    pub name: String,
    pub protocol: ProviderProtocol,
    pub base_url: String,
    /// 本地明文存储（与 custom_harnesses.env_json 同一先例）。
    pub api_key: String,
    pub enabled: bool,
    pub models: Vec<ProviderModel>,
    /// 绑定该供应商的智能体数量；查询时实时统计，不落库。
    pub agent_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 协议→spawn 注入 env 键值对；注入时覆盖手填同名词。
///
/// api_key 为空（本地/免鉴权服务）时跳过 key 注入，
/// 避免空值覆盖手填 token。
pub fn provider_env_pairs(
    protocol: ProviderProtocol,
    base_url: &str,
    api_key: &str,
) -> Vec<(&'static str, String)> {
    let base = match protocol {
        ProviderProtocol::Anthropic => ("ANTHROPIC_BASE_URL", base_url.to_string()),
        ProviderProtocol::Openai => ("OPENAI_BASE_URL", base_url.to_string()),
    };
    let key_name = match protocol {
        ProviderProtocol::Anthropic => "ANTHROPIC_AUTH_TOKEN",
        ProviderProtocol::Openai => "OPENAI_API_KEY",
    };
    let mut pairs = vec![base];
    if !api_key.trim().is_empty() {
        pairs.push((key_name, api_key.to_string()));
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_parse_roundtrip() {
        for protocol in ProviderProtocol::ALL {
            assert_eq!(
                ProviderProtocol::parse(protocol.as_str()).unwrap(),
                protocol
            );
        }
        assert!(ProviderProtocol::parse("gemini").is_err());
        assert!(ProviderProtocol::parse("").is_err());
    }

    #[test]
    fn protocol_serializes_kebab() {
        assert_eq!(
            serde_json::to_string(&ProviderProtocol::Anthropic).unwrap(),
            "\"anthropic\""
        );
        assert_eq!(
            serde_json::to_string(&ProviderProtocol::Openai).unwrap(),
            "\"openai\""
        );
    }

    #[test]
    fn env_pairs_map_by_protocol() {
        let anthropic = provider_env_pairs(ProviderProtocol::Anthropic, "https://a", "key-a");
        assert_eq!(
            anthropic,
            vec![
                ("ANTHROPIC_BASE_URL", "https://a".to_string()),
                ("ANTHROPIC_AUTH_TOKEN", "key-a".to_string()),
            ]
        );
        let openai = provider_env_pairs(ProviderProtocol::Openai, "https://o", "key-o");
        assert_eq!(
            openai,
            vec![
                ("OPENAI_BASE_URL", "https://o".to_string()),
                ("OPENAI_API_KEY", "key-o".to_string()),
            ]
        );
    }

    #[test]
    fn env_pairs_skip_empty_api_key() {
        let pairs = provider_env_pairs(ProviderProtocol::Anthropic, "https://local", "  ");
        assert_eq!(
            pairs,
            vec![("ANTHROPIC_BASE_URL", "https://local".to_string())],
            "空 api_key 只注入 BASE_URL，避免空值覆盖手填 token"
        );
    }

    #[test]
    fn model_validation_rejects_bad_shapes() {
        let mut model = ProviderModel {
            id: "m1".to_string(),
            name: "M1".to_string(),
            context_window: None,
            reasoning_levels: Vec::new(),
            input_types: Vec::new(),
        };
        assert!(model.validate().is_ok());

        model.id = "  ".to_string();
        assert!(model.validate().is_err());
        model.id = "m1".to_string();

        model.context_window = Some(0);
        assert!(model.validate().is_err());
        model.context_window = Some(-5);
        assert!(model.validate().is_err());
        model.context_window = Some(65536);

        model.input_types = vec!["audio".to_string()];
        assert!(model.validate().is_err());
        model.input_types = vec!["text".to_string(), "video".to_string()];
        assert!(model.validate().is_ok());
    }
}
