//! 内置主持人种子与房间回填。
//!
//! 幂等：重复调用不会新增第二条内置记录，也不会重复插入成员关系。
//! 名称冲突时使用带后缀的名称，避免启动失败；身份由 `builtin_role` 决定。

use crate::domain::agent::{
    Agent, BuiltinAgentRole, CreateAgentInput, HarnessKind,
};
use crate::error::AppResult;
use crate::store::{AgentStore, RoomStore};

/// 当前唯一的内置角色；`builtin_role` 预留后续扩展。
pub const BUILTIN_MODERATOR_ROLE: BuiltinAgentRole = BuiltinAgentRole::Moderator;

/// 内置主持人默认名称；名称冲突时依次追加后缀。
pub const BUILTIN_MODERATOR_NAME: &str = "Moon 主持人";

/// 内置主持人的固定人设；实际编排规则由 orchestrator 提示词注入。
pub const BUILTIN_MODERATOR_PERSONA: &str = "你是房间的固定主持人。你的职责是理解目标、把工作拆成可验收的子任务、安排合适的成员执行、检查产出并给出验收证据。你不代替成员完成具体工作，也不自动把任务标记完成；最终由人验收。";

pub struct BuiltinModeratorService {
    agents: AgentStore,
    rooms: RoomStore,
    harness: HarnessKind,
}

impl BuiltinModeratorService {
    pub fn new(agents: AgentStore, rooms: RoomStore) -> Self {
        Self {
            agents,
            rooms,
            harness: HarnessKind::Zcode,
        }
    }

    /// 测试或自定义装配时指定默认 harness。
    pub fn with_harness(mut self, harness: HarnessKind) -> Self {
        self.harness = harness;
        self
    }

    /// 确保内置主持人存在并返回；已存在时直接返回现有记录。
    pub async fn ensure(&self) -> AppResult<Agent> {
        if let Some(existing) = self
            .agents
            .get_by_builtin_role(BUILTIN_MODERATOR_ROLE)
            .await?
        {
            return Ok(existing);
        }

        self.agents
            .create(CreateAgentInput {
                name: self.available_name().await?,
                persona: BUILTIN_MODERATOR_PERSONA.to_string(),
                harness: self.harness,
                model_config: None,
                builtin_role: Some(BUILTIN_MODERATOR_ROLE),
                custom_harness_id: None,
                workspace_dir: None,
            })
            .await
    }

    /// 确保内置主持人存在，并把它加入所有现有房间、设为唯一主持人。
    ///
    /// 已有自定义主持人保留为普通成员；房间的 moderator 统一指向内置主持人。
    pub async fn ensure_and_backfill(&self) -> AppResult<Agent> {
        let moderator = self.ensure().await?;

        for room in self.rooms.list().await? {
            self.rooms.add_member(room.id, moderator.id).await?;
            if room.moderator_agent_id != Some(moderator.id) {
                let mut patch = crate::domain::room::UpdateRoomInput::default();
                patch.moderator_agent_id = Some(Some(moderator.id));
                self.rooms.update(room.id, patch).await?;
            }
        }

        Ok(moderator)
    }

    async fn available_name(&self) -> AppResult<String> {
        let mut candidates = vec![
            BUILTIN_MODERATOR_NAME.to_string(),
            format!("{BUILTIN_MODERATOR_NAME}（内置）"),
        ];
        for index in 2..100 {
            candidates.push(format!("{BUILTIN_MODERATOR_NAME}（内置 {index}）"));
        }

        for name in candidates {
            if self.agents.get_by_name(&name).await?.is_none() {
                return Ok(name);
            }
        }

        // 名称池耗尽属于不可恢复的本地数据问题；使用带时间戳的兜底名称，
        // 保证启动仍可完成，身份仍由唯一 builtin_role 约束决定。
        Ok(format!(
            "{BUILTIN_MODERATOR_NAME}（内置 {}）",
            crate::domain::now_ms()
        ))
    }
}
