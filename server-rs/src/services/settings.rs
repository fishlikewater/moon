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

    /// 按 harness 返回用户覆盖的可执行文件路径；空字符串表示自动检测。
    pub async fn harness_bin_override(
        &self,
        harness: crate::domain::agent::HarnessKind,
    ) -> AppResult<Option<String>> {
        use crate::domain::agent::HarnessKind;

        let settings = self.store.load().await?;
        let value = match harness {
            HarnessKind::Zcode => settings.zcode_bin_path,
            HarnessKind::Codex => settings.codex_bin_path,
            _ => return Ok(None),
        };
        let trimmed = value.trim();
        Ok((!trimmed.is_empty()).then(|| trimmed.to_string()))
    }

    /// 把用户覆盖路径合并进模型配置；无覆盖时原样返回，调用方无需分支。
    pub async fn with_harness_bin_override(
        &self,
        harness: crate::domain::agent::HarnessKind,
        config: Option<crate::domain::agent::ModelConfig>,
    ) -> AppResult<Option<crate::domain::agent::ModelConfig>> {
        let Some(key) = crate::adapters::bin_config_key(harness) else {
            return Ok(config);
        };
        let Some(path) = self.harness_bin_override(harness).await? else {
            return Ok(config);
        };
        let mut map = config.unwrap_or_default();
        map.insert(key.to_string(), Value::String(path));
        Ok(Some(map))
    }

    /// 运行总时长上限；`0`（不限制）返回 `None`。
    pub async fn run_timeout(&self) -> AppResult<Option<Duration>> {
        let settings = self.store.load().await?;
        Ok((settings.run_timeout_secs > 0)
            .then(|| Duration::from_secs(settings.run_timeout_secs as u64)))
    }
}
