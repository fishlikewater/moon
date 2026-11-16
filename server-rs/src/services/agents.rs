//! 智能体业务规则。
//!
//! harness 枚举、字段完整性与删除前置约束都在此层成立，保证即使调用方
//! 绕过前端校验也无法写入非法数据。

use std::sync::Arc;

use serde_json::Value;

use crate::domain::agent::{Agent, CreateAgentInput, HarnessKind, ModelConfig, UpdateAgentInput};
use crate::error::{AppError, AppResult};
use crate::services::rooms::require_workspace_dir_value;
use crate::services::sessions::SessionService;
use crate::services::{parse_id, require_body, require_non_empty_string};
use crate::store::{AgentStore, ApiProviderStore, CustomHarnessStore, RoomStore};

pub struct AgentService {
    agents: AgentStore,
    sessions: Arc<SessionService>,
    custom_harnesses: CustomHarnessStore,
    api_providers: ApiProviderStore,
    rooms: RoomStore,
}

impl AgentService {
    pub fn new(
        agents: AgentStore,
        sessions: Arc<SessionService>,
        custom_harnesses: CustomHarnessStore,
        api_providers: ApiProviderStore,
        rooms: RoomStore,
    ) -> Self {
        Self {
            agents,
            sessions,
            custom_harnesses,
            api_providers,
            rooms,
        }
    }

    pub async fn list(&self) -> AppResult<Vec<Agent>> {
        self.agents.list().await
    }

    pub async fn create(&self, input: Value) -> AppResult<Agent> {
        let body = require_body(input)?;
        let name = require_non_empty_string(body.get("name"), "name")?;
        let persona = require_non_empty_string(body.get("persona"), "persona")?;
        let harness = parse_harness(body.get("harness"))?;
        let model_config = parse_model_config(body.get("modelConfig"))?;
        let custom_harness_id = parse_custom_harness_ref(body.get("customHarnessId"))?;
        let workspace_dir = parse_agent_workspace(body.get("workspaceDir")).await?;
        self.validate_harness_binding(harness, custom_harness_id)
            .await?;
        self.validate_provider_ref(harness, model_config.as_ref())
            .await?;

        if self.agents.get_by_name(&name).await?.is_some() {
            return Err(AppError::validation(format!("智能体名称已存在：{name}")));
        }

        self.agents
            .create(CreateAgentInput {
                name,
                persona,
                harness,
                model_config,
                builtin_role: None,
                custom_harness_id,
                workspace_dir,
            })
            .await
    }

    pub async fn update(&self, raw_id: &str, input: Value) -> AppResult<Agent> {
        let id = parse_id(raw_id)?;
        let Some(existing) = self.agents.get_by_id(id).await? else {
            return Err(AppError::not_found(format!("智能体不存在：{id}")));
        };

        let body = require_body(input)?;

        // 内置主持人的名称、人设与工作区是系统不变量；harness 与模型配置可调，
        // 后续走与普通智能体相同的 patch 合并与绑定校验。
        if existing.builtin_role.is_some() {
            let allowed = ["harness", "modelConfig", "customHarnessId"];
            let illegal_keys: Vec<&String> = body
                .keys()
                .filter(|key| !allowed.contains(&key.as_str()))
                .collect();
            if !illegal_keys.is_empty() {
                return Err(AppError::constraint(
                    "内置主持人的名称、人设与工作区不可修改，只能调整 harness 与模型配置",
                ));
            }
            if body.is_empty() {
                return Err(AppError::validation(
                    "内置主持人需要提供 harness 或模型配置",
                ));
            }
        }

        let mut patch = UpdateAgentInput::default();
        if let Some(value) = body.get("name") {
            let name = require_non_empty_string(Some(value), "name")?;
            if let Some(owner) = self.agents.get_by_name(&name).await? {
                if owner.id != id {
                    return Err(AppError::validation(format!("智能体名称已存在：{name}")));
                }
            }
            patch.name = Some(name);
        }
        if let Some(value) = body.get("persona") {
            patch.persona = Some(require_non_empty_string(Some(value), "persona")?);
        }
        if let Some(value) = body.get("harness") {
            patch.harness = Some(parse_harness(Some(value))?);
        }
        if let Some(value) = body.get("modelConfig") {
            patch.model_config = Some(parse_model_config(Some(value))?);
        }
        if body.contains_key("customHarnessId") {
            patch.custom_harness_id = Some(parse_custom_harness_ref(body.get("customHarnessId"))?);
        }
        if body.contains_key("workspaceDir") {
            patch.workspace_dir = Some(parse_agent_workspace(body.get("workspaceDir")).await?);
        }

        // 以合并后的最终组合做校验：harness 离开 acp 且未显式指定绑定时
        // 自动清空残留绑定，不视为校验失败。
        let effective_harness = patch.harness.unwrap_or(existing.harness);
        if effective_harness != HarnessKind::Acp && patch.custom_harness_id.is_none() {
            if existing.custom_harness_id.is_some() {
                patch.custom_harness_id = Some(None);
            }
        } else {
            let effective_custom = match patch.custom_harness_id {
                Some(value) => value,
                None => existing.custom_harness_id,
            };
            self.validate_harness_binding(effective_harness, effective_custom)
                .await?;
        }

        // 供应商绑定在 modelConfig JSON 里：harness 离开 acp 时自动清空
        // 残留的 apiProviderId（镜像 customHarnessId 清空语义），
        // 最终组合做存在性与 harness 一致性校验。
        // patch 显式携带 modelConfig: null（Some(None)）表示清空，
        // effective_model_config 返回 None，不会进入本剥离分支。
        if effective_harness != HarnessKind::Acp {
            let carries_provider = effective_model_config(&patch, &existing)
                .is_some_and(|config| config.contains_key("apiProviderId"));
            if carries_provider {
                let mut config = effective_model_config(&patch, &existing)
                    .cloned()
                    .unwrap_or_default();
                config.remove("apiProviderId");
                patch.model_config = Some(Some(config));
            }
        }
        let final_config = effective_model_config(&patch, &existing);
        self.validate_provider_ref(effective_harness, final_config)
            .await?;

        let updated = self.agents.update(id, patch).await?;
        updated.ok_or_else(|| AppError::not_found(format!("智能体不存在：{id}")))
    }

    /// 删除智能体并清理引用。
    ///
    /// 内置主持人是系统不变量；正在参与运行中讨论或委派的智能体先拒绝，
    /// 避免循环在删除后继续引用已不存在的成员。
    pub async fn delete(&self, raw_id: &str) -> AppResult<()> {
        let id = parse_id(raw_id)?;
        let Some(agent) = self.agents.get_by_id(id).await? else {
            return Err(AppError::not_found(format!("智能体不存在：{id}")));
        };

        if agent.builtin_role.is_some() {
            return Err(AppError::constraint("内置主持人是系统智能体，不能删除"));
        }
        if self.agents.has_running_discussion_for_agent(id).await? {
            return Err(AppError::constraint(
                "智能体正在参与进行中的讨论，请先停止讨论",
            ));
        }
        if self.agents.has_running_collaboration_for_agent(id).await? {
            return Err(AppError::constraint(
                "智能体正在参与进行中的委派，请先停止委派",
            ));
        }

        // 直聊与智能体一一对应：先整删直聊（房间行/消息/会话存储/沙盒目录），
        // 避免留下成员被清空后不可见的孤儿 direct 房间。历史并发残留可能有多条，
        // 全部清理；相同沙盒路径只删一次。
        let mut cleaned_workspaces = std::collections::HashSet::new();
        for direct in self.rooms.list_direct_for_agent(id).await? {
            self.rooms.delete(direct.id).await?;
            self.sessions.delete_room_storage(direct.id);
            if cleaned_workspaces.insert(direct.workspace_dir.clone()) {
                remove_direct_workspace(&direct.workspace_dir);
            }
        }

        if self.agents.delete(id).await? {
            // 会话记录已随事务删除；存储目录按智能体整体清理，失败只留下临时目录。
            self.sessions.delete_agent_storage(id);
        }
        Ok(())
    }
}

/// 清理直聊沙盒目录；只接受 `.../direct-workspaces/direct-*` 形态的路径，
/// 其它目录一律不动（防止工作区被改成用户真实目录时误删）。
fn remove_direct_workspace(workspace_dir: &str) {
    let path = std::path::Path::new(workspace_dir);
    let expected_parent = path
        .parent()
        .and_then(|parent| parent.file_name())
        .is_some_and(|name| name == "direct-workspaces");
    let expected_leaf = path
        .file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with("direct-"));
    if !expected_parent || !expected_leaf {
        tracing::warn!(
            workspace = %path.display(),
            "direct workspace path unexpected; skipping directory cleanup"
        );
        return;
    }
    if let Err(error) = std::fs::remove_dir_all(path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(
                workspace = %path.display(),
                error = %error,
                "direct workspace cleanup failed"
            );
        }
    }
}

/// 局部更新的最终 modelConfig：patch 携带时用携带值（`Some(None)` 即显式
/// 清空），否则沿用现值。`UpdateAgentInput.model_config` 是双层 Option。
fn effective_model_config<'a>(
    patch: &'a UpdateAgentInput,
    existing: &'a Agent,
) -> Option<&'a ModelConfig> {
    match &patch.model_config {
        Some(value) => value.as_ref(),
        None => existing.model_config.as_ref(),
    }
}

fn parse_harness(value: Option<&Value>) -> AppResult<HarnessKind> {
    match value.and_then(Value::as_str) {
        Some(raw) => HarnessKind::parse(raw),
        None => Err(AppError::validation(format!(
            "harness 必须是 {} 之一",
            HarnessKind::ALL.map(HarnessKind::as_str).join(" | ")
        ))),
    }
}

fn parse_custom_harness_ref(value: Option<&Value>) -> AppResult<Option<i64>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => {
            Ok(Some(number.as_i64().ok_or_else(|| {
                AppError::validation("customHarnessId 必须是整数")
            })?))
        }
        Some(_) => Err(AppError::validation("customHarnessId 必须是整数或 null")),
    }
}

/// 智能体工作区：缺失、null 或空白字符串表示使用房间主工作区。
async fn parse_agent_workspace(value: Option<&Value>) -> AppResult<Option<String>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => {
            let dir = raw.trim();
            if dir.is_empty() {
                Ok(None)
            } else {
                require_workspace_dir_value(dir).await.map(Some)
            }
        }
        Some(_) => Err(AppError::validation("workspaceDir 必须是字符串或 null")),
    }
}

impl AgentService {
    /// harness 与自定义 ACP 绑定的一致性：acp 必须绑定存在的定义，
    /// 其他 harness 显式携带绑定时直接拒绝，避免静默忽略。
    async fn validate_harness_binding(
        &self,
        harness: HarnessKind,
        custom_harness_id: Option<i64>,
    ) -> AppResult<()> {
        match harness {
            HarnessKind::Acp => {
                let Some(id) = custom_harness_id else {
                    return Err(AppError::validation(
                        "harness 为 acp 时必须选择自定义 ACP agent",
                    ));
                };
                if self.custom_harnesses.get_by_id(id).await?.is_none() {
                    return Err(AppError::validation(format!(
                        "自定义 ACP agent 不存在：{id}"
                    )));
                }
                Ok(())
            }
            _ => {
                if custom_harness_id.is_some() {
                    return Err(AppError::validation(
                        "只有 harness 为 acp 的智能体可以绑定自定义 ACP agent",
                    ));
                }
                Ok(())
            }
        }
    }

    /// modelConfig 里的供应商绑定：数字 id 必须指向存在的供应商，
    /// 且只有 acp harness 允许携带；null/缺失视为未绑定。
    async fn validate_provider_ref(
        &self,
        harness: HarnessKind,
        model_config: Option<&ModelConfig>,
    ) -> AppResult<()> {
        let Some(value) = model_config.and_then(|config| config.get("apiProviderId")) else {
            return Ok(());
        };
        match value {
            Value::Null => Ok(()),
            Value::Number(number) => {
                let id = number
                    .as_i64()
                    .ok_or_else(|| AppError::validation("modelConfig.apiProviderId 必须是整数"))?;
                if harness != HarnessKind::Acp {
                    return Err(AppError::validation(
                        "只有 harness 为 acp 的智能体可以绑定模型供应商",
                    ));
                }
                if self.api_providers.get_by_id(id).await?.is_none() {
                    return Err(AppError::validation(format!("模型供应商不存在：{id}")));
                }
                Ok(())
            }
            _ => Err(AppError::validation(
                "modelConfig.apiProviderId 必须是整数或 null",
            )),
        }
    }
}

fn parse_model_config(value: Option<&Value>) -> AppResult<Option<ModelConfig>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(map)) => Ok(Some(map.clone())),
        Some(_) => Err(AppError::validation("modelConfig 必须是对象或 null")),
    }
}

#[cfg(test)]
mod tests {
    use super::remove_direct_workspace;

    #[test]
    fn direct_workspace_cleanup_only_touches_sandbox_leaf() {
        let root = tempfile::tempdir().unwrap();
        let sandbox_parent = root.path().join("direct-workspaces");
        let safe = sandbox_parent.join("direct-7");
        let unsafe_leaf = sandbox_parent.join("user-project");
        let outside = root.path().join("real-project");
        for dir in [&safe, &unsafe_leaf, &outside] {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join("keep.txt"), b"x").unwrap();
        }

        remove_direct_workspace(safe.to_str().unwrap());
        remove_direct_workspace(unsafe_leaf.to_str().unwrap());
        remove_direct_workspace(outside.to_str().unwrap());

        assert!(!safe.exists(), "direct-* 沙盒目录应在删除智能体时清理");
        assert!(
            unsafe_leaf.join("keep.txt").exists(),
            "direct-workspaces 下的非 direct-* 目录不得删除"
        );
        assert!(outside.exists(), "direct-workspaces 之外的目录不得删除");
    }
}
