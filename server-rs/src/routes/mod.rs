//! HTTP 路由：薄层，只做参数转发与状态码编排。

pub mod acp_registry;
pub mod agent_builder;
pub mod agents;
pub mod attachments;
pub mod collaborations;
pub mod custom_harnesses;
pub mod discussions;
pub mod files;
pub mod harnesses;
pub mod messages;
pub mod orchestration;
pub mod rooms;
pub mod runs;
pub mod sessions;
pub mod settings;
pub mod tasks;
pub mod timeline;

use axum::body::Bytes;
use serde_json::{Map, Value};

use crate::error::{AppError, AppResult};

/// 解析 JSON 请求体；空 body 按历史契约落到空对象。
pub fn parse_json_body(bytes: &Bytes) -> AppResult<Value> {
    if bytes.is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    serde_json::from_slice(bytes)
        .map_err(|_| AppError::validation("请求体不是合法的 JSON"))
}
