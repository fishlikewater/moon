//! 自定义 ACP agent 注册表业务规则。
//!
//! 入参形状校验、name/command 非空约束与「被引用删除 → 409」都在此层成立；
//! 路由只做参数转发与状态码编排，store 只负责持久化。

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{Map, Value};

use crate::adapters::acp::probe_capabilities;
use crate::domain::harness::{
    AcpProbeResponse, CreateCustomHarnessInput, CustomHarness, UpdateCustomHarnessInput,
};
use crate::domain::now_ms;
use crate::error::{AppError, AppResult};
use crate::services::require_non_empty_string;
use crate::store::CustomHarnessStore;

/// 能力探测默认上限：initialize + session/new，必须远小于普通运行上限。
pub const PROBE_TIMEOUT_SECS: u64 = 15;
/// 首次探测放宽上限：npx 首次运行需要下载依赖，可能远超 15 秒。
pub const MAX_PROBE_TIMEOUT_SECS: u64 = 120;

pub struct CustomHarnessService {
    store: CustomHarnessStore,
}

impl CustomHarnessService {
    pub fn new(store: CustomHarnessStore) -> Self {
        Self { store }
    }

    pub async fn list(&self) -> AppResult<Vec<CustomHarness>> {
        self.store.list().await
    }

    pub async fn get(&self, id: i64) -> AppResult<CustomHarness> {
        self.store
            .get_by_id(id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("自定义 ACP agent 不存在：{id}")))
    }

    pub async fn create(&self, body: Map<String, Value>) -> AppResult<CustomHarness> {
        let name = require_non_empty_string(body.get("name"), "name")?;
        let command = require_non_empty_string(body.get("command"), "command")?;
        CustomHarness::validate_name(&name)?;
        CustomHarness::validate_command(&command)?;
        let args = parse_args(body.get("args"))?;
        let env = parse_env(body.get("env"))?;

        self.store
            .create(CreateCustomHarnessInput {
                name,
                command,
                args,
                env,
            })
            .await
    }

    pub async fn update(&self, id: i64, body: Map<String, Value>) -> AppResult<CustomHarness> {
        let mut patch = UpdateCustomHarnessInput::default();
        if let Some(value) = body.get("name") {
            let name = require_non_empty_string(Some(value), "name")?;
            CustomHarness::validate_name(&name)?;
            patch.name = Some(name);
        }
        if let Some(value) = body.get("command") {
            let command = require_non_empty_string(Some(value), "command")?;
            CustomHarness::validate_command(&command)?;
            patch.command = Some(command);
        }
        if let Some(value) = body.get("args") {
            patch.args = Some(parse_args(Some(value))?);
        }
        if let Some(value) = body.get("env") {
            patch.env = Some(parse_env(Some(value))?);
        }

        self.store
            .update(id, patch)
            .await?
            .ok_or_else(|| AppError::not_found(format!("自定义 ACP agent 不存在：{id}")))
    }

    /// 删除被智能体引用的定义时返回 409。
    pub async fn remove(&self, id: i64) -> AppResult<()> {
        let referenced = self.store.count_agents_referencing(id).await?;
        if referenced > 0 {
            return Err(AppError::constraint(format!(
                "自定义 ACP agent 正被 {referenced} 个智能体引用，请先修改对应智能体的 harness"
            )));
        }
        let deleted = self.store.delete(id).await?;
        if !deleted {
            return Err(AppError::not_found(format!(
                "自定义 ACP agent 不存在：{id}"
            )));
        }
        Ok(())
    }

    /// 探测 agent 自报的模型与模式；预检失败不 spawn，失败原因（含分类）落库。
    ///
    /// `timeout_secs` 缺省 15 秒，可放宽到 120 秒（首次 npx 下载场景）。
    /// 成功清空失败标记；失败统一在本层落库，保留旧能力快照。
    pub async fn probe(
        &self,
        id: i64,
        timeout_secs: Option<u64>,
    ) -> AppResult<AcpProbeResponse> {
        let harness = self.get(id).await?;
        match self.probe_inner(&harness, timeout_secs).await {
            Ok(response) => Ok(response),
            Err(error) => {
                let message = truncate_probe_error(&error.message);
                self.store
                    .save_probe_failure(id, &message, now_ms())
                    .await?;
                Err(error)
            }
        }
    }

    async fn probe_inner(
        &self,
        harness: &CustomHarness,
        timeout_secs: Option<u64>,
    ) -> AppResult<AcpProbeResponse> {
        // 预检失败不需要启动 agent：秒级返回结构化原因（issues 数组）。
        let preflight = crate::adapters::acp_preflight::check_command(&harness.command);
        if !preflight.is_ok() {
            let issues = serde_json::to_value(&preflight.issues)?;
            let message = truncate_probe_error(&preflight.message());
            return Err(AppError::validation(message).with_details(serde_json::json!({
                "issues": issues,
            })));
        }
        let timeout = Duration::from_secs(probe_timeout_secs(timeout_secs));
        let workspace_dir = std::env::current_dir()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|_| ".".to_string());
        let capabilities = probe_capabilities(harness, &workspace_dir, timeout).await?;
        let probed_at = now_ms();
        self.store
            .save_capabilities(harness.id, &capabilities, probed_at)
            .await?;
        Ok(AcpProbeResponse::new(capabilities, probed_at))
    }
}

fn parse_args(value: Option<&Value>) -> AppResult<Vec<String>> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => {
            let mut args = Vec::with_capacity(items.len());
            for item in items {
                let Some(raw) = item.as_str() else {
                    return Err(AppError::validation("args 必须是字符串数组"));
                };
                args.push(raw.to_string());
            }
            Ok(args)
        }
        Some(_) => Err(AppError::validation("args 必须是字符串数组")),
    }
}

fn parse_env(value: Option<&Value>) -> AppResult<BTreeMap<String, String>> {
    match value {
        None | Some(Value::Null) => Ok(BTreeMap::new()),
        Some(Value::Object(map)) => {
            let mut env = BTreeMap::new();
            for (key, item) in map {
                let Some(raw) = item.as_str() else {
                    return Err(AppError::validation(format!(
                        "env 的值必须是字符串：{key}"
                    )));
                };
                env.insert(key.clone(), raw.to_string());
            }
            Ok(env)
        }
        Some(_) => Err(AppError::validation("env 必须是字符串到字符串的对象")),
    }
}

/// 探测失败文案截断：按字符截断到 200，避免 stderr 摘要撑爆徽标与列表。
fn truncate_probe_error(message: &str) -> String {
    const LIMIT: usize = 200;
    let trimmed = message.trim();
    if trimmed.chars().count() <= LIMIT {
        return trimmed.to_string();
    }
    let truncated: String = trimmed.chars().take(LIMIT).collect();
    format!("{truncated}…")
}

/// 探测超时钳制：缺省 15 秒，下界 1 秒、上界 120 秒。
fn probe_timeout_secs(raw: Option<u64>) -> u64 {
    raw.unwrap_or(PROBE_TIMEOUT_SECS)
        .clamp(1, MAX_PROBE_TIMEOUT_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_timeout_secs_clamps_to_bounds() {
        assert_eq!(probe_timeout_secs(None), 15);
        assert_eq!(probe_timeout_secs(Some(0)), 1);
        assert_eq!(probe_timeout_secs(Some(90)), 90);
        assert_eq!(probe_timeout_secs(Some(999)), 120);
    }

    #[test]
    fn truncate_probe_error_keeps_first_200_chars() {
        let long = "长".repeat(260);
        let truncated = truncate_probe_error(&long);
        assert!(truncated.chars().count() == 201, "200 字符 + 省略号");
        assert!(truncated.ends_with('…'));
    }
}
