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

/// 能力探测上限：initialize + session/new，必须远小于普通运行上限。
pub const PROBE_TIMEOUT_SECS: u64 = 15;

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

    /// 探测 agent 自报的模型与模式；失败时不覆盖旧缓存。
    pub async fn probe(&self, id: i64) -> AppResult<AcpProbeResponse> {
        let harness = self.get(id).await?;
        let workspace_dir = std::env::current_dir()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|_| ".".to_string());
        let capabilities = probe_capabilities(
            &harness,
            &workspace_dir,
            Duration::from_secs(PROBE_TIMEOUT_SECS),
        )
        .await?;
        let probed_at = now_ms();
        self.store
            .save_capabilities(id, &capabilities, probed_at)
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
