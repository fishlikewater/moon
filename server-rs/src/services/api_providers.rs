//! 模型供应商注册表业务规则。
//!
//! 入参形状校验、协议枚举、模型高级属性约束与「被引用删除 → 409」都在此层
//! 成立；路由只做参数转发与状态码编排，store 只负责持久化。

use serde_json::{Map, Value};

use crate::domain::provider::{ApiProvider, ProviderModel, ProviderProtocol};
use crate::error::{AppError, AppResult};
use crate::services::require_non_empty_string;
use crate::store::ApiProviderStore;

pub struct ApiProviderService {
    store: ApiProviderStore,
}

impl ApiProviderService {
    pub fn new(store: ApiProviderStore) -> Self {
        Self { store }
    }

    pub async fn list(&self) -> AppResult<Vec<ApiProvider>> {
        self.store.list().await
    }

    pub async fn get(&self, id: i64) -> AppResult<ApiProvider> {
        self.store
            .get_by_id(id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("模型供应商不存在：{id}")))
    }

    pub async fn create(&self, body: Map<String, Value>) -> AppResult<ApiProvider> {
        let name = require_non_empty_string(body.get("name"), "name")?;
        validate_provider_name(&name)?;
        let protocol = parse_protocol(body.get("protocol"))?;
        let base_url = require_non_empty_string(body.get("baseUrl"), "baseUrl")?;
        validate_base_url(&base_url)?;
        let api_key = optional_string(body.get("apiKey"))?;
        let enabled = parse_enabled(body.get("enabled"))?;
        let models = parse_models(body.get("models"))?;

        self.store
            .create(&name, protocol, &base_url, &api_key, enabled, &models)
            .await
    }

    pub async fn update(&self, id: i64, body: Map<String, Value>) -> AppResult<ApiProvider> {
        let Some(existing) = self.store.get_by_id(id).await? else {
            return Err(AppError::not_found(format!("模型供应商不存在：{id}")));
        };

        let name = match body.get("name") {
            Some(value) => {
                let name = require_non_empty_string(Some(value), "name")?;
                validate_provider_name(&name)?;
                name
            }
            None => existing.name.clone(),
        };
        let protocol = match body.get("protocol") {
            Some(value) => parse_protocol(Some(value))?,
            None => existing.protocol,
        };
        let base_url = match body.get("baseUrl") {
            Some(value) => {
                let base_url = require_non_empty_string(Some(value), "baseUrl")?;
                validate_base_url(&base_url)?;
                base_url
            }
            None => existing.base_url.clone(),
        };
        let api_key = match body.get("apiKey") {
            Some(_) => optional_string(body.get("apiKey"))?,
            None => existing.api_key.clone(),
        };
        let enabled = match body.get("enabled") {
            Some(_) => parse_enabled(body.get("enabled"))?,
            None => existing.enabled,
        };
        let models = match body.get("models") {
            Some(_) => parse_models(body.get("models"))?,
            None => existing.models.clone(),
        };

        let mut next = existing;
        next.name = name;
        next.protocol = protocol;
        next.base_url = base_url;
        next.api_key = api_key;
        next.enabled = enabled;
        next.models = models;
        if self.store.update(&mut next).await? {
            Ok(next)
        } else {
            Err(AppError::not_found(format!("模型供应商不存在：{id}")))
        }
    }

    /// 删除被智能体绑定的供应商时返回 409。
    pub async fn remove(&self, id: i64) -> AppResult<()> {
        let Some(existing) = self.store.get_by_id(id).await? else {
            return Err(AppError::not_found(format!("模型供应商不存在：{id}")));
        };
        if existing.agent_count > 0 {
            return Err(AppError::constraint(format!(
                "模型供应商正被 {} 个智能体绑定，请先修改对应智能体的模型供应商",
                existing.agent_count
            )));
        }
        self.store.delete(id).await?;
        Ok(())
    }
}

fn validate_provider_name(name: &str) -> AppResult<()> {
    const MAX_NAME_CHARS: usize = 100;
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(AppError::validation(format!(
            "name 不能超过 {MAX_NAME_CHARS} 个字符"
        )));
    }
    Ok(())
}

fn validate_base_url(base_url: &str) -> AppResult<()> {
    if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
        return Err(AppError::validation(
            "baseUrl 必须以 http:// 或 https:// 开头",
        ));
    }
    Ok(())
}

fn parse_protocol(value: Option<&Value>) -> AppResult<ProviderProtocol> {
    match value.and_then(Value::as_str) {
        Some(raw) => ProviderProtocol::parse(raw),
        None => Err(AppError::validation(format!(
            "protocol 必须是 {} 之一",
            ProviderProtocol::ALL
                .map(ProviderProtocol::as_str)
                .join(" | ")
        ))),
    }
}

fn parse_enabled(value: Option<&Value>) -> AppResult<bool> {
    match value {
        None | Some(Value::Null) => Ok(true),
        Some(flag @ Value::Bool(_)) => Ok(flag.as_bool().unwrap_or(true)),
        Some(_) => Err(AppError::validation("enabled 必须是布尔值")),
    }
}

fn optional_string(value: Option<&Value>) -> AppResult<String> {
    match value {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(raw)) => Ok(raw.clone()),
        Some(_) => Err(AppError::validation("必须是字符串或 null")),
    }
}

/// 解析并校验模型列表：数组、每项形状、高级属性约束、同供应商内 id 唯一。
fn parse_models(value: Option<&Value>) -> AppResult<Vec<ProviderModel>> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => {
            let mut models = Vec::with_capacity(items.len());
            for item in items {
                let Some(map) = item.as_object() else {
                    return Err(AppError::validation("models 每项必须是对象"));
                };
                let Some(id) = map.get("id").and_then(Value::as_str) else {
                    return Err(AppError::validation("models 每项必须包含字符串 id"));
                };
                let model = ProviderModel {
                    id: id.to_string(),
                    name: map
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or(id)
                        .to_string(),
                    context_window: match map.get("contextWindow") {
                        None | Some(Value::Null) => None,
                        Some(Value::Number(number)) => Some(number.as_i64().ok_or_else(|| {
                            AppError::validation(format!("模型「{id}」的 contextWindow 必须是整数"))
                        })?),
                        Some(_) => {
                            return Err(AppError::validation(format!(
                                "模型「{id}」的 contextWindow 必须是整数或 null"
                            )));
                        }
                    },
                    reasoning_levels: parse_string_list(
                        map.get("reasoningLevels"),
                        "reasoningLevels",
                        id,
                    )?,
                    input_types: parse_string_list(map.get("inputTypes"), "inputTypes", id)?,
                };
                model.validate()?;
                models.push(model);
            }
            let mut seen = std::collections::BTreeSet::new();
            for model in &models {
                if !seen.insert(model.id.as_str()) {
                    return Err(AppError::validation(format!("模型 id 重复：{}", model.id)));
                }
            }
            Ok(models)
        }
        Some(_) => Err(AppError::validation("models 必须是数组")),
    }
}

fn parse_string_list(value: Option<&Value>, field: &str, model_id: &str) -> AppResult<Vec<String>> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let Some(raw) = item.as_str() else {
                    return Err(AppError::validation(format!(
                        "模型「{model_id}」的 {field} 必须是字符串数组"
                    )));
                };
                out.push(raw.to_string());
            }
            Ok(out)
        }
        Some(_) => Err(AppError::validation(format!(
            "模型「{model_id}」的 {field} 必须是字符串数组"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_models_validates_advanced_attributes() {
        let models = json!([
            {
                "id": "deepseek-chat",
                "name": "DeepSeek Chat",
                "contextWindow": 65536,
                "reasoningLevels": ["standard", "deep"],
                "inputTypes": ["text"]
            }
        ]);
        let parsed = parse_models(Some(&models)).expect("合法模型列表");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].context_window, Some(65536));
        assert_eq!(parsed[0].reasoning_levels, vec!["standard", "deep"]);
    }

    #[test]
    fn parse_models_rejects_bad_context_window_and_input_types() {
        let bad_window = json!([{ "id": "m", "name": "M", "contextWindow": 0 }]);
        assert!(parse_models(Some(&bad_window)).is_err());

        let bad_type = json!([{ "id": "m", "name": "M", "inputTypes": ["audio"] }]);
        assert!(parse_models(Some(&bad_type)).is_err());

        let missing_id = json!([{ "name": "M" }]);
        assert!(parse_models(Some(&missing_id)).is_err());

        let not_array = json!({ "id": "m" });
        assert!(parse_models(Some(&not_array)).is_err());
    }

    #[test]
    fn parse_models_rejects_duplicate_ids() {
        let dup = json!([
            { "id": "m", "name": "A" },
            { "id": "m", "name": "B" }
        ]);
        let error = parse_models(Some(&dup)).expect_err("重复 id 必须拒绝");
        assert!(error.message.contains("模型 id 重复"));
    }

    #[test]
    fn parse_enabled_defaults_true_and_requires_bool() {
        assert!(parse_enabled(None).unwrap());
        assert!(parse_enabled(Some(&Value::Null)).unwrap());
        assert!(!parse_enabled(Some(&json!(false))).unwrap());
        assert!(parse_enabled(Some(&json!("yes"))).is_err());
    }

    #[test]
    fn validate_base_url_requires_http_scheme() {
        assert!(validate_base_url("https://api.deepseek.com").is_ok());
        assert!(validate_base_url("http://localhost:11434").is_ok());
        assert!(validate_base_url("api.deepseek.com").is_err());
        assert!(validate_base_url("").is_err());
    }
}
