//! 启动配置：全部来自环境变量，未设置时落到开发默认值。

use std::env;
use std::path::PathBuf;
use std::time::Duration;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub db_path: PathBuf,
    pub storage_root: PathBuf,
    /// harness 运行总时长上限；None 表示不限制。
    pub run_timeout: Option<Duration>,
}

impl Config {
    pub fn from_env() -> AppResult<Self> {
        let host = env::var("MOON_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());

        let port = match env::var("MOON_PORT") {
            Ok(raw) => raw
                .parse::<u16>()
                .ok()
                .filter(|value| *value != 0)
                .ok_or_else(|| AppError::validation(format!("MOON_PORT 不是合法端口：{raw}")))?,
            Err(_) => 3000,
        };

        let data_dir = env::var_os("MOON_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(default_data_dir);
        let db_path = env::var_os("MOON_DB_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| data_dir.join("moon.db"));
        let storage_root = env::var_os("MOON_STORAGE_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| data_dir.join("harness-sessions"));
        let run_timeout =
            crate::adapters::parse_run_timeout(env::var("MOON_RUN_TIMEOUT_SECS").ok().as_deref())?;

        Ok(Self {
            host,
            port,
            db_path,
            storage_root,
            run_timeout,
        })
    }
}

/// 开发默认目录：当前工作目录下的 `data/`。
fn default_data_dir() -> PathBuf {
    env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("data")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// 环境变量是进程级共享状态，串行化本模块的用例避免互相覆盖。
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn defaults_are_stable() {
        let _guard = ENV_LOCK.lock().unwrap();
        let saved: Vec<_> = ["MOON_HOST", "MOON_PORT", "MOON_DATA_DIR", "MOON_DB_PATH", "MOON_STORAGE_ROOT", "MOON_RUN_TIMEOUT_SECS"]
            .iter()
            .map(|key| (*key, env::var_os(key)))
            .collect();
        for (key, _) in &saved {
            env::remove_var(key);
        }

        let config = Config::from_env().expect("default config");
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 3000);
        assert!(config.db_path.ends_with("data/moon.db"));
        assert!(config.storage_root.ends_with("data/harness-sessions"));
        assert_eq!(
            config.run_timeout,
            Some(Duration::from_secs(crate::adapters::DEFAULT_RUN_TIMEOUT_SECS))
        );

        for (key, value) in saved {
            match value {
                Some(value) => env::set_var(key, value),
                None => env::remove_var(key),
            }
        }
    }

    #[test]
    fn invalid_port_is_rejected() {
        let _guard = ENV_LOCK.lock().unwrap();
        let saved = env::var_os("MOON_PORT");
        env::set_var("MOON_PORT", "not-a-port");
        let error = Config::from_env().expect_err("invalid port must fail");
        assert_eq!(error.code, crate::error::ErrorCode::ValidationFailed);
        match saved {
            Some(value) => env::set_var("MOON_PORT", value),
            None => env::remove_var("MOON_PORT"),
        }
    }

    #[test]
    fn invalid_run_timeout_is_rejected() {
        let _guard = ENV_LOCK.lock().unwrap();
        let saved = env::var_os("MOON_RUN_TIMEOUT_SECS");
        env::set_var("MOON_RUN_TIMEOUT_SECS", "not-seconds");
        let error = Config::from_env().expect_err("invalid run timeout must fail");
        assert_eq!(error.code, crate::error::ErrorCode::ValidationFailed);
        match saved {
            Some(value) => env::set_var("MOON_RUN_TIMEOUT_SECS", value),
            None => env::remove_var("MOON_RUN_TIMEOUT_SECS"),
        }
    }

    #[test]
    fn zero_run_timeout_disables_limit() {
        let _guard = ENV_LOCK.lock().unwrap();
        let saved = env::var_os("MOON_RUN_TIMEOUT_SECS");
        env::set_var("MOON_RUN_TIMEOUT_SECS", "0");
        let config = Config::from_env().expect("zero disables limit");
        assert_eq!(config.run_timeout, None);
        match saved {
            Some(value) => env::set_var("MOON_RUN_TIMEOUT_SECS", value),
            None => env::remove_var("MOON_RUN_TIMEOUT_SECS"),
        }
    }
}
