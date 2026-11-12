//! 设置服务：默认值合并、部分更新校验与运行时长读取。

use std::time::Duration;

use serde_json::Value;

use crate::domain::settings::{apply_patch, AppSettings, AppSettingsResponse};
use crate::error::AppResult;
use crate::store::SettingsStore;

pub struct SettingsService {
    store: SettingsStore,
}

impl SettingsService {
    pub fn new(store: SettingsStore) -> Self {
        Self { store }
    }

    /// 设置全集；运行时长已并入设置，可直接编辑。
    pub async fn get(&self) -> AppResult<AppSettingsResponse> {
        Ok(AppSettingsResponse {
            settings: self.store.load().await?,
        })
    }

    /// 部分更新：先校验完整结果，再在单事务内写入。
    pub async fn update(&self, patch: &Value) -> AppResult<AppSettings> {
        let current = self.store.load().await?;
        let next = apply_patch(&current, patch)?;
        self.store.save(&next).await?;
        Ok(next)
    }

    /// 运行总时长上限；`0`（不限制）返回 `None`。
    pub async fn run_timeout(&self) -> AppResult<Option<Duration>> {
        let settings = self.store.load().await?;
        Ok((settings.run_timeout_secs > 0)
            .then(|| Duration::from_secs(settings.run_timeout_secs as u64)))
    }
}
