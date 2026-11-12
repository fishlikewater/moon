//! 应用设置访问层。

use std::collections::HashMap;

use sqlx::SqlitePool;

use crate::db::Db;
use crate::domain::now_ms;
use crate::domain::settings::AppSettings;
use crate::error::AppResult;

#[derive(Debug, Clone)]
pub struct SettingsStore {
    db: Db,
}

impl SettingsStore {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    fn pool(&self) -> &SqlitePool {
        self.db.pool()
    }

    /// 读取全部设置；缺失键用默认值补全，损坏值单键回退。
    pub async fn load(&self) -> AppResult<AppSettings> {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT key, value FROM app_settings")
                .fetch_all(self.pool())
                .await?;
        let pairs: HashMap<String, String> = rows.into_iter().collect();
        Ok(AppSettings::from_pairs(&pairs))
    }

    /// 在单事务内写入全部键，保证不会出现半更新。
    pub async fn save(&self, settings: &AppSettings) -> AppResult<()> {
        let mut tx = self.db.pool().begin().await?;
        let updated_at = now_ms();
        for (key, value) in settings.to_pairs() {
            sqlx::query(
                "INSERT INTO app_settings (key, value, updated_at)
                 VALUES (?, ?, ?)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            )
            .bind(key)
            .bind(value)
            .bind(updated_at)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
}
