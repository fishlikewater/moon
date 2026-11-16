//! 模型供应商注册表访问层。

use sqlx::SqlitePool;

use crate::db::Db;
use crate::domain::now_ms;
use crate::domain::provider::{ApiProvider, ProviderModel, ProviderProtocol};
use crate::error::{AppError, AppResult};

/// agent 绑定计数：`apiProviderId` 存于 agents.model_config_json，
/// 用 JSON1 实时统计，与 custom_harnesses 的 agent_count 同语义。
const API_PROVIDER_COLUMNS: &str = "p.id, p.name, p.protocol, p.base_url, p.api_key, p.enabled,
    p.models_json, p.created_at, p.updated_at,
    (SELECT COUNT(*) FROM agents
      WHERE model_config_json IS NOT NULL
        AND json_extract(model_config_json, '$.apiProviderId') IS NOT NULL
        AND CAST(json_extract(model_config_json, '$.apiProviderId') AS INTEGER) = p.id
    ) AS agent_count";

#[derive(Debug, Clone)]
pub struct ApiProviderStore {
    db: Db,
}

impl ApiProviderStore {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    fn pool(&self) -> &SqlitePool {
        self.db.pool()
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        &self,
        name: &str,
        protocol: ProviderProtocol,
        base_url: &str,
        api_key: &str,
        enabled: bool,
        models: &[ProviderModel],
    ) -> AppResult<ApiProvider> {
        let now = now_ms();
        let models_json = serde_json::to_string(models)?;
        let result = sqlx::query(
            "INSERT INTO api_providers (name, protocol, base_url, api_key, enabled, models_json, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(name)
        .bind(protocol.as_str())
        .bind(base_url)
        .bind(api_key)
        .bind(enabled)
        .bind(models_json)
        .bind(now)
        .bind(now)
        .execute(self.pool())
        .await?;

        Ok(ApiProvider {
            id: result.last_insert_rowid(),
            name: name.to_string(),
            protocol,
            base_url: base_url.to_string(),
            api_key: api_key.to_string(),
            enabled,
            models: models.to_vec(),
            agent_count: 0,
            created_at: now,
            updated_at: now,
        })
    }

    pub async fn get_by_id(&self, id: i64) -> AppResult<Option<ApiProvider>> {
        let row = sqlx::query_as::<_, ApiProviderRow>(&format!(
            "SELECT {API_PROVIDER_COLUMNS} FROM api_providers p WHERE p.id = ?"
        ))
        .bind(id)
        .fetch_optional(self.pool())
        .await?;

        row.map(map_row).transpose()
    }

    pub async fn list(&self) -> AppResult<Vec<ApiProvider>> {
        let rows = sqlx::query_as::<_, ApiProviderRow>(&format!(
            "SELECT {API_PROVIDER_COLUMNS} FROM api_providers p ORDER BY p.id"
        ))
        .fetch_all(self.pool())
        .await?;

        rows.into_iter().map(map_row).collect()
    }

    /// 全量字段更新（业务层已把缺失字段回填为现值）；
    /// 更新时间写回 provider，保证响应与回读一致。
    pub async fn update(&self, provider: &mut ApiProvider) -> AppResult<bool> {
        let models_json = serde_json::to_string(&provider.models)?;
        let updated_at = now_ms();
        let result = sqlx::query(
            "UPDATE api_providers
                SET name = ?, protocol = ?, base_url = ?, api_key = ?, enabled = ?, models_json = ?, updated_at = ?
              WHERE id = ?",
        )
        .bind(&provider.name)
        .bind(provider.protocol.as_str())
        .bind(&provider.base_url)
        .bind(&provider.api_key)
        .bind(provider.enabled)
        .bind(models_json)
        .bind(updated_at)
        .bind(provider.id)
        .execute(self.pool())
        .await?;
        if result.rows_affected() > 0 {
            provider.updated_at = updated_at;
        }
        Ok(result.rows_affected() > 0)
    }

    pub async fn delete(&self, id: i64) -> AppResult<bool> {
        let result = sqlx::query("DELETE FROM api_providers WHERE id = ?")
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected() > 0)
    }
}

#[derive(sqlx::FromRow)]
struct ApiProviderRow {
    id: i64,
    name: String,
    protocol: String,
    base_url: String,
    api_key: String,
    enabled: i64,
    models_json: String,
    created_at: i64,
    updated_at: i64,
    agent_count: i64,
}

fn map_row(row: ApiProviderRow) -> AppResult<ApiProvider> {
    let protocol = ProviderProtocol::parse(&row.protocol).map_err(|_| {
        AppError::internal(format!(
            "api_providers.protocol 不是合法协议：id={}",
            row.id
        ))
    })?;
    let models: Vec<ProviderModel> = serde_json::from_str(&row.models_json).map_err(|_| {
        AppError::internal(format!(
            "api_providers.models_json 不是合法模型数组：id={}",
            row.id
        ))
    })?;

    Ok(ApiProvider {
        id: row.id,
        name: row.name,
        protocol,
        base_url: row.base_url,
        api_key: row.api_key,
        enabled: row.enabled != 0,
        models,
        agent_count: row.agent_count,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}
