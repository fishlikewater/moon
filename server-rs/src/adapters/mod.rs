//! harness 适配器统一接口与工厂。

pub mod acp;
pub mod codex;
pub mod codex_config;
pub mod fake;
pub mod harness_config_error;
pub mod kimi_config;
pub mod locate;
pub mod types;
pub mod zcode;
pub mod zcode_app_server;
pub mod zcode_config;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::domain::agent::{Agent, HarnessKind};
use crate::error::{AppError, AppResult};
use crate::store::CustomHarnessStore;

pub use acp::AcpAdapter;
pub use codex::CodexAdapter;
pub use fake::{FakeHarnessAdapter, FakeScript};
pub use types::{EventStream, HarnessAdapter};
pub use zcode::ZcodeAdapter;
pub use crate::domain::run_event::RunInput;

/// 适配器工厂；测试通过它注入脚本化 fake 适配器。
pub type AdapterFactory = Arc<dyn Fn(&Agent) -> AppResult<Arc<dyn HarnessAdapter>> + Send + Sync>;

/// 默认工厂：`acp` 按注册表定义构造 `AcpAdapter`，其余按内置枚举创建。
pub fn default_adapter_factory(custom_harnesses: CustomHarnessStore) -> AdapterFactory {
    Arc::new(move |agent: &Agent| match agent.harness {
        HarnessKind::Acp => Ok(Arc::new(AcpAdapter::new(
            custom_harnesses.clone(),
            agent.custom_harness_id,
        )) as Arc<dyn HarnessAdapter>),
        _ => create_adapter(agent.harness),
    })
}

/// 按 harness 枚举创建适配器；`kimi-code` / `acp` 明确失败，不返回静默空实现。
///
/// `acp` 的实际创建由 `lib.rs` 的默认工厂承担：它按智能体的
/// `custom_harness_id` 从注册表取定义并构造 `AcpAdapter`；
/// 这里只保留枚举穷举分支，直接调用时给出可操作的错误指引。
pub fn create_adapter(harness: HarnessKind) -> AppResult<Arc<dyn HarnessAdapter>> {
    match harness {
        HarnessKind::Zcode => Ok(Arc::new(ZcodeAdapter)),
        HarnessKind::Codex => Ok(Arc::new(CodexAdapter)),
        HarnessKind::Fake => Ok(Arc::new(FakeHarnessAdapter::empty())),
        HarnessKind::Acp => Err(AppError::harness_unsupported(
            "acp harness 需要绑定自定义 ACP agent，请重新保存智能体或检查注册表",
        )),
        HarnessKind::KimiCode => Err(AppError::harness_unsupported(
            "暂不支持的 harness：kimi-code",
        )),
    }
}

/// harness 专用的可执行文件覆盖键（写入 modelConfig）；其他 harness 返回 None。
pub fn bin_config_key(harness: HarnessKind) -> Option<&'static str> {
    match harness {
        HarnessKind::Zcode => Some("zcodeBin"),
        HarnessKind::Codex => Some("codexBin"),
        _ => None,
    }
}

pub(crate) const STDERR_TAIL_LIMIT: usize = 1000;

/// 运行总时长默认上限：30 分钟；`MOON_RUN_TIMEOUT_SECS=0` 可关闭。
pub const DEFAULT_RUN_TIMEOUT_SECS: u64 = 1800;

/// 解析 `MOON_RUN_TIMEOUT_SECS`：未设置/空白用默认值，0 表示不限制。
pub fn parse_run_timeout(raw: Option<&str>) -> AppResult<Option<Duration>> {
    let Some(value) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(Some(Duration::from_secs(DEFAULT_RUN_TIMEOUT_SECS)));
    };
    let seconds: u64 = value.parse().map_err(|_| {
        AppError::validation(format!("MOON_RUN_TIMEOUT_SECS 不是合法秒数：{value}"))
    })?;
    Ok((seconds > 0).then(|| Duration::from_secs(seconds)))
}

/// 运行期读取超时配置；解析失败回退默认（启动期由 `Config` 校验）。
///
/// 已不再作为运行时来源：`runTimeoutSecs` 进入设置库后，这里只在设置键
/// 缺失时派生初始默认值（见 `domain::settings::initial_run_timeout_secs`）。
pub fn run_timeout_from_env() -> Option<Duration> {
    let raw = std::env::var("MOON_RUN_TIMEOUT_SECS").ok();
    parse_run_timeout(raw.as_deref())
        .unwrap_or(Some(Duration::from_secs(DEFAULT_RUN_TIMEOUT_SECS)))
}

/// 用总时长上限包裹一次协议运行；超时返回可读错误，由调用方清理进程。
pub(crate) async fn with_run_timeout<F, T>(timeout: Option<Duration>, future: F) -> AppResult<T>
where
    F: std::future::Future<Output = AppResult<T>>,
{
    let Some(limit) = timeout else {
        return future.await;
    };
    match tokio::time::timeout(limit, future).await {
        Ok(result) => result,
        Err(_) => Err(AppError::harness_run_failed(format!(
            "运行超过 {} 秒未结束，已终止；可在设置页调整「运行总时长上限」（0 表示不限制）",
            limit.as_secs()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_run_timeout_defaults_and_bounds() {
        assert_eq!(
            parse_run_timeout(None).unwrap(),
            Some(Duration::from_secs(DEFAULT_RUN_TIMEOUT_SECS))
        );
        assert_eq!(
            parse_run_timeout(Some("  ")).unwrap(),
            Some(Duration::from_secs(DEFAULT_RUN_TIMEOUT_SECS))
        );
        assert_eq!(parse_run_timeout(Some("0")).unwrap(), None);
        assert_eq!(
            parse_run_timeout(Some("90")).unwrap(),
            Some(Duration::from_secs(90))
        );
        assert!(parse_run_timeout(Some("-1")).is_err());
        assert!(parse_run_timeout(Some("abc")).is_err());
    }

    #[tokio::test]
    async fn with_run_timeout_reports_limit_and_termination() {
        let error = with_run_timeout(Some(Duration::from_millis(20)), async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Ok(())
        })
        .await
        .expect_err("超时应返回错误");

        assert!(
            error.message.contains("运行超过 0 秒未结束"),
            "文案应包含配置的上限：{}",
            error.message
        );
        assert!(
            error.message.contains("已终止"),
            "文案应说明已终止：{}",
            error.message
        );
    }
}

/// 用完即删的临时目录/文件载体；`Drop` 时尽力清理，失败静默。
pub(crate) struct Cleanup {
    path: Option<PathBuf>,
}

impl Cleanup {
    pub(crate) fn none() -> Self {
        Self { path: None }
    }

    pub(crate) fn directory(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    pub(crate) fn path(&self) -> Option<&PathBuf> {
        self.path.as_ref()
    }

    pub(crate) fn run(&mut self) {
        let Some(path) = self.path.take() else {
            return;
        };
        for attempt in 0..5 {
            match std::fs::remove_dir_all(&path) {
                Ok(()) => return,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
                Err(_) if attempt < 4 => std::thread::sleep(std::time::Duration::from_millis(50)),
                Err(_) => return,
            }
        }
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        self.run();
    }
}

/// 创建唯一临时目录；不做密码学随机，只需进程内不冲突。
pub(crate) fn unique_temp_dir(prefix: &str) -> std::io::Result<PathBuf> {
    unique_temp_dir_in(&std::env::temp_dir(), prefix)
}

pub(crate) fn unique_temp_dir_in(base: &std::path::Path, prefix: &str) -> std::io::Result<PathBuf> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    for _ in 0..100 {
        let suffix = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = base.join(format!("{prefix}{}-{suffix}", std::process::id()));
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "无法创建唯一临时目录",
    ))
}

pub(crate) fn tail(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= STDERR_TAIL_LIMIT {
        return text.to_string();
    }
    chars[chars.len() - STDERR_TAIL_LIMIT..].iter().collect()
}

/// 把常见 spawn 失败翻译成可读中文；未知错误返回 None。
pub(crate) fn describe_spawn_failure(error: &std::io::Error, harness: &str) -> Option<String> {
    match error.kind() {
        std::io::ErrorKind::NotFound => Some(format!(
            "未找到可执行的 {harness}：请检查智能体配置中的 {harness}Bin，或确认已安装"
        )),
        std::io::ErrorKind::PermissionDenied => Some(format!(
            "没有权限执行 {harness}：请检查可执行文件路径与文件权限"
        )),
        _ => None,
    }
}
