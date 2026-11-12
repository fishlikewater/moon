//! 业务规则层：路由只做参数转发，校验与不变量在 service 内强制。

pub mod acp_registry;
pub mod agent_builder;
pub mod agents;
pub mod attachments;
pub mod builtin_moderator;
pub mod chat;
pub mod collaborations;
pub mod context_compaction;
pub mod custom_harnesses;
pub mod discussion;
pub mod files;
pub mod harness_models;
pub mod message_artifacts;
pub mod moderator;
pub mod orchestrator;
pub mod reactions;
pub mod recovery;
pub mod room_history;
pub mod rooms;
pub mod run_activity;
pub mod sessions;
pub mod settings;
pub mod task_plan;
pub mod task_scheduler;
pub mod tasks;
pub mod timeline;

use serde_json::{Map, Value};

use crate::error::{AppError, AppResult};

/// JS `Number.MAX_SAFE_INTEGER`；沿用历史整数边界契约。
pub(crate) const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

pub fn require_body(value: Value) -> AppResult<Map<String, Value>> {
    match value {
        Value::Object(map) => Ok(map),
        _ => Err(AppError::validation("请求体必须是 JSON 对象")),
    }
}

/// 非空字符串（trim 后判空，返回值保留原样）。
pub fn require_non_empty_string(value: Option<&Value>, field: &str) -> AppResult<String> {
    match value.and_then(Value::as_str) {
        Some(text) if !text.trim().is_empty() => Ok(text.to_string()),
        _ => Err(AppError::validation(format!("{field} 必须是非空字符串"))),
    }
}

pub fn parse_id(raw: &str) -> AppResult<i64> {
    parse_id_label(raw, "")
}

/// 路径/请求体里的 id：仅接受十进制正整数，且不超过 JS 安全整数范围。
pub fn parse_id_label(raw: &str, label: &str) -> AppResult<i64> {
    let digits_only = !raw.is_empty() && raw.bytes().all(|byte| byte.is_ascii_digit());
    let parsed = raw
        .parse::<i64>()
        .ok()
        .filter(|id| *id > 0 && *id <= MAX_SAFE_INTEGER);
    match (digits_only, parsed) {
        (true, Some(id)) => Ok(id),
        _ => Err(AppError::validation(format!("非法的{label} id：{raw}"))),
    }
}

/// 模拟 `total` 中的 `Number.isSafeInteger(value) && value > 0`。
pub fn as_positive_id(value: &Value) -> Option<i64> {
    if let Some(id) = value.as_i64() {
        return (id > 0 && id <= MAX_SAFE_INTEGER).then_some(id);
    }
    value
        .as_f64()
        .filter(|number| number.fract() == 0.0 && *number > 0.0)
        .filter(|number| *number <= MAX_SAFE_INTEGER as f64)
        .map(|number| number as i64)
}

/// 模拟 `String(value ?? "")`：缺失与 null 都变成空串，交给 id 解析拒绝。
pub fn value_to_id_string(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::Bool(flag)) => flag.to_string(),
        Some(Value::Array(_)) | Some(Value::Object(_)) => "「无法解析」".to_string(),
    }
}
