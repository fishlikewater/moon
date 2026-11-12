//! 智能体访问层。

use sqlx::SqlitePool;

use crate::db::Db;
use crate::domain::agent::{
    Agent, BuiltinAgentRole, CreateAgentInput, HarnessKind, ModelConfig, UpdateAgentInput,
};
use crate::domain::now_ms;
use crate::error::{AppError, AppResult};

pub const AGENT_COLUMNS: &str =
    "id, name, persona, harness, model_config_json, builtin_role, custom_harness_id, workspace_dir, created_at";

#[derive(Debug, Clone)]
pub struct AgentStore {
    db: Db,
}

impl AgentStore {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    fn pool(&self) -> &SqlitePool {
        self.db.pool()
    }

    pub async fn create(&self, input: CreateAgentInput) -> AppResult<Agent> {
        let created_at = now_ms();
        let model_config_json = encode_model_config(input.model_config.as_ref())?;
        let result = sqlx::query(
            "INSERT INTO agents
               (name, persona, harness, model_config_json, builtin_role, custom_harness_id, workspace_dir, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&input.name)
        .bind(&input.persona)
        .bind(input.harness.as_str())
        .bind(model_config_json)
        .bind(input.builtin_role.map(BuiltinAgentRole::as_str))
        .bind(input.custom_harness_id)
        .bind(&input.workspace_dir)
        .bind(created_at)
        .execute(self.pool())
        .await?;

        Ok(Agent {
            id: result.last_insert_rowid(),
            name: input.name,
            persona: input.persona,
            harness: input.harness,
            model_config: input.model_config,
            builtin_role: input.builtin_role,
            custom_harness_id: input.custom_harness_id,
            workspace_dir: input.workspace_dir,
            created_at,
        })
    }

    pub async fn get_by_id(&self, id: i64) -> AppResult<Option<Agent>> {
        let row = sqlx::query_as::<_, AgentRow>(&format!(
            "SELECT {AGENT_COLUMNS} FROM agents WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(self.pool())
        .await?;

        row.map(map_agent_row).transpose()
    }

    /// 按名称查询；`name` 有唯一约束，用于服务层重名校验。
    pub async fn get_by_name(&self, name: &str) -> AppResult<Option<Agent>> {
        let row = sqlx::query_as::<_, AgentRow>(&format!(
            "SELECT {AGENT_COLUMNS} FROM agents WHERE name = ?"
        ))
        .bind(name)
        .fetch_optional(self.pool())
        .await?;

        row.map(map_agent_row).transpose()
    }

    /// 按内置角色查询；唯一索引保证最多一条。
    pub async fn get_by_builtin_role(
        &self,
        role: BuiltinAgentRole,
    ) -> AppResult<Option<Agent>> {
        let row = sqlx::query_as::<_, AgentRow>(&format!(
            "SELECT {AGENT_COLUMNS} FROM agents WHERE builtin_role = ?"
        ))
        .bind(role.as_str())
        .fetch_optional(self.pool())
        .await?;

        row.map(map_agent_row).transpose()
    }

    pub async fn list(&self) -> AppResult<Vec<Agent>> {
        // 内置智能体置顶，普通智能体按创建顺序；不改变普通记录的相对顺序。
        let rows = sqlx::query_as::<_, AgentRow>(&format!(
            "SELECT {AGENT_COLUMNS} FROM agents
              ORDER BY CASE WHEN builtin_role IS NULL THEN 1 ELSE 0 END, id"
        ))
        .fetch_all(self.pool())
        .await?;

        rows.into_iter().map(map_agent_row).collect()
    }

    pub async fn update(&self, id: i64, patch: UpdateAgentInput) -> AppResult<Option<Agent>> {
        let Some(existing) = self.get_by_id(id).await? else {
            return Ok(None);
        };

        let name = patch.name.unwrap_or_else(|| existing.name.clone());
        let persona = patch.persona.unwrap_or_else(|| existing.persona.clone());
        let harness = patch.harness.unwrap_or(existing.harness);
        let model_config = match patch.model_config {
            Some(value) => value,
            None => existing.model_config.clone(),
        };
        let model_config_json = encode_model_config(model_config.as_ref())?;
        // 三态：未传保持原值；显式 Some(None) 清空；Some(Some(id)) 覆盖。
        let custom_harness_id = patch.custom_harness_id.unwrap_or(existing.custom_harness_id);
        let workspace_dir = patch
            .workspace_dir
            .unwrap_or_else(|| existing.workspace_dir.clone());

        sqlx::query(
            "UPDATE agents
                SET name = ?, persona = ?, harness = ?, model_config_json = ?, custom_harness_id = ?,
                    workspace_dir = ?
              WHERE id = ?",
        )
        .bind(&name)
        .bind(&persona)
        .bind(harness.as_str())
        .bind(model_config_json)
        .bind(custom_harness_id)
        .bind(&workspace_dir)
        .bind(id)
        .execute(self.pool())
        .await?;

        Ok(Some(Agent {
            id,
            name,
            persona,
            harness,
            model_config,
            builtin_role: existing.builtin_role,
            custom_harness_id,
            workspace_dir,
            created_at: existing.created_at,
        }))
    }

    /// 删除智能体并清理所有引用。
    ///
    /// 历史消息与任务/讨论保留内容，只把发送者或创建者置空；
    /// 运行记录没有独立展示价值，直接删除；讨论参与者随智能体一起清理。
    /// 全部语句在同一事务内，避免出现只清理一半的中间状态。
    pub async fn delete(&self, id: i64) -> AppResult<bool> {
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;

        sqlx::query("UPDATE messages SET sender_agent_id = NULL WHERE sender_agent_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE tasks SET created_by_agent_id = NULL WHERE created_by_agent_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE discussions SET created_by_agent_id = NULL WHERE created_by_agent_id = ?",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE rooms SET moderator_agent_id = NULL WHERE moderator_agent_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM room_members WHERE agent_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM collaboration_assignments WHERE agent_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        // messages.run_id 引用 agent_runs：先解除引用，再删运行记录，
        // 否则删除有过消息的智能体会撞外键约束。
        sqlx::query(
            "UPDATE messages SET run_id = NULL
              WHERE run_id IN (SELECT id FROM agent_runs WHERE agent_id = ?)",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM agent_runs WHERE agent_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        // agent_sessions 通过外键引用 agents，必须先删记录再删主表；
        // 存储目录由服务层在删除成功后清理。
        sqlx::query("DELETE FROM agent_sessions WHERE agent_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        // 与 discussion_participants.agent_id 的 ON DELETE CASCADE 冗余；
        // 显式清理不依赖连接是否开启外键开关。
        sqlx::query("DELETE FROM discussion_participants WHERE agent_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;

        let result = sqlx::query("DELETE FROM agents WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn has_running_discussion_for_agent(&self, agent_id: i64) -> AppResult<bool> {
        let exists: i64 = sqlx::query_scalar(
            "SELECT EXISTS(
               SELECT 1 FROM discussion_participants p
                 JOIN discussions d ON d.id = p.discussion_id
                WHERE p.agent_id = ? AND d.status = 'running'
             )",
        )
        .bind(agent_id)
        .fetch_one(self.pool())
        .await?;
        Ok(exists != 0)
    }

    pub async fn has_running_collaboration_for_agent(&self, agent_id: i64) -> AppResult<bool> {
        let exists: i64 = sqlx::query_scalar(
            "SELECT EXISTS(
               SELECT 1 FROM collaboration_assignments a
                 JOIN collaboration_batches b ON b.id = a.batch_id
                WHERE a.agent_id = ? AND b.status = 'running'
             )",
        )
        .bind(agent_id)
        .fetch_one(self.pool())
        .await?;
        Ok(exists != 0)
    }
}

#[derive(sqlx::FromRow)]
pub(crate) struct AgentRow {
    id: i64,
    name: String,
    persona: String,
    harness: String,
    model_config_json: Option<String>,
    builtin_role: Option<String>,
    custom_harness_id: Option<i64>,
    workspace_dir: Option<String>,
    created_at: i64,
}

pub(crate) fn map_agent_row(row: AgentRow) -> AppResult<Agent> {
    let harness = HarnessKind::parse(&row.harness).map_err(|_| {
        AppError::internal(format!("agents.harness 非法：{}", row.harness))
    })?;
    let model_config = decode_model_config(row.model_config_json.as_deref())?;
    Ok(Agent {
        id: row.id,
        name: row.name,
        persona: row.persona,
        harness,
        model_config,
        builtin_role: row
            .builtin_role
            .as_deref()
            .and_then(BuiltinAgentRole::parse),
        custom_harness_id: row.custom_harness_id,
        workspace_dir: row.workspace_dir,
        created_at: row.created_at,
    })
}

pub fn encode_model_config(config: Option<&ModelConfig>) -> AppResult<Option<String>> {
    config
        .map(|value| serde_json::to_string(value).map_err(AppError::from))
        .transpose()
}

pub fn decode_model_config(raw: Option<&str>) -> AppResult<Option<ModelConfig>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    serde_json::from_str::<ModelConfig>(raw)
        .map(Some)
        .map_err(|error| {
            AppError::with_source(
                crate::error::ErrorCode::InternalError,
                "agents.model_config_json 不是合法 JSON 对象",
                error,
            )
        })
}
