//! 自定义 ACP agent 注册表访问层。

use sqlx::SqlitePool;

use crate::db::Db;
use crate::domain::harness::{
    AcpCapabilities, CreateCustomHarnessInput, CustomHarness, UpdateCustomHarnessInput,
};
use crate::domain::now_ms;
use crate::error::AppResult;

const CUSTOM_HARNESS_COLUMNS: &str = "h.id, h.name, h.command, h.args_json, h.env_json, h.created_at,
    h.capabilities_json, h.probed_at,
    (SELECT COUNT(*) FROM agents a WHERE a.custom_harness_id = h.id) AS agent_count";

#[derive(Debug, Clone)]
pub struct CustomHarnessStore {
    db: Db,
}

impl CustomHarnessStore {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    fn pool(&self) -> &SqlitePool {
        self.db.pool()
    }

    pub async fn create(&self, input: CreateCustomHarnessInput) -> AppResult<CustomHarness> {
        let created_at = now_ms();
        let args_json = serde_json::to_string(&input.args)?;
        let env_json = serde_json::to_string(&input.env)?;
        let result = sqlx::query(
            "INSERT INTO custom_harnesses (name, command, args_json, env_json, created_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&input.name)
        .bind(&input.command)
        .bind(args_json)
        .bind(env_json)
        .bind(created_at)
        .execute(self.pool())
        .await?;

        Ok(CustomHarness {
            id: result.last_insert_rowid(),
            name: input.name,
            command: input.command,
            args: input.args,
            env: input.env,
            created_at,
            agent_count: 0,
            capabilities: None,
            probed_at: None,
        })
    }

    pub async fn get_by_id(&self, id: i64) -> AppResult<Option<CustomHarness>> {
        let row = sqlx::query_as::<_, CustomHarnessRow>(&format!(
            "SELECT {CUSTOM_HARNESS_COLUMNS} FROM custom_harnesses h WHERE h.id = ?"
        ))
        .bind(id)
        .fetch_optional(self.pool())
        .await?;

        row.map(map_row).transpose()
    }

    pub async fn list(&self) -> AppResult<Vec<CustomHarness>> {
        let rows = sqlx::query_as::<_, CustomHarnessRow>(&format!(
            "SELECT {CUSTOM_HARNESS_COLUMNS} FROM custom_harnesses h ORDER BY h.id"
        ))
        .fetch_all(self.pool())
        .await?;

        rows.into_iter().map(map_row).collect()
    }

    pub async fn update(
        &self,
        id: i64,
        patch: UpdateCustomHarnessInput,
    ) -> AppResult<Option<CustomHarness>> {
        let Some(existing) = self.get_by_id(id).await? else {
            return Ok(None);
        };

        let name = patch.name.unwrap_or_else(|| existing.name.clone());
        let command = patch.command.unwrap_or_else(|| existing.command.clone());
        let args = patch.args.unwrap_or_else(|| existing.args.clone());
        let env = patch.env.unwrap_or_else(|| existing.env.clone());
        let args_json = serde_json::to_string(&args)?;
        let env_json = serde_json::to_string(&env)?;

        sqlx::query(
            "UPDATE custom_harnesses
                SET name = ?, command = ?, args_json = ?, env_json = ?
              WHERE id = ?",
        )
        .bind(&name)
        .bind(&command)
        .bind(args_json)
        .bind(env_json)
        .bind(id)
        .execute(self.pool())
        .await?;

        let agent_count = self.count_agents_referencing(id).await?;

        Ok(Some(CustomHarness {
            id,
            name,
            command,
            args,
            env,
            created_at: existing.created_at,
            agent_count,
            capabilities: existing.capabilities,
            probed_at: existing.probed_at,
        }))
    }

    /// 写入最近一次探测结果；失败时调用方保留旧缓存（不调用本方法）。
    pub async fn save_capabilities(
        &self,
        id: i64,
        capabilities: &AcpCapabilities,
        probed_at: i64,
    ) -> AppResult<bool> {
        let capabilities_json = serde_json::to_string(capabilities)?;
        let result = sqlx::query(
            "UPDATE custom_harnesses
                SET capabilities_json = ?, probed_at = ?
              WHERE id = ?",
        )
        .bind(capabilities_json)
        .bind(probed_at)
        .bind(id)
        .execute(self.pool())
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn delete(&self, id: i64) -> AppResult<bool> {
        let result = sqlx::query("DELETE FROM custom_harnesses WHERE id = ?")
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// 引用该定义的智能体数量；删除前由服务层检查。
    pub async fn count_agents_referencing(&self, id: i64) -> AppResult<i64> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM agents WHERE custom_harness_id = ?",
        )
        .bind(id)
        .fetch_one(self.pool())
        .await?;
        Ok(count)
    }
}

#[derive(sqlx::FromRow)]
struct CustomHarnessRow {
    id: i64,
    name: String,
    command: String,
    args_json: String,
    env_json: String,
    created_at: i64,
    capabilities_json: Option<String>,
    probed_at: Option<i64>,
    agent_count: i64,
}

fn map_row(row: CustomHarnessRow) -> AppResult<CustomHarness> {
    let args: Vec<String> = serde_json::from_str(&row.args_json).map_err(|_| {
        crate::error::AppError::internal(format!(
            "custom_harnesses.args_json 不是合法 JSON 数组：id={}",
            row.id
        ))
    })?;
    let env: std::collections::BTreeMap<String, String> =
        serde_json::from_str(&row.env_json).map_err(|_| {
            crate::error::AppError::internal(format!(
                "custom_harnesses.env_json 不是合法 JSON 对象：id={}",
                row.id
            ))
        })?;

    let capabilities = row
        .capabilities_json
        .as_deref()
        .map(serde_json::from_str::<AcpCapabilities>)
        .transpose()
        .map_err(|_| {
            crate::error::AppError::internal(format!(
                "custom_harnesses.capabilities_json 不是合法能力快照：id={}",
                row.id
            ))
        })?;

    Ok(CustomHarness {
        id: row.id,
        name: row.name,
        command: row.command,
        args,
        env,
        created_at: row.created_at,
        agent_count: row.agent_count,
        capabilities,
        probed_at: row.probed_at,
    })
}
