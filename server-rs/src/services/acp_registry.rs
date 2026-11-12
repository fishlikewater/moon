//! ACP registry 目录服务：CDN 拉取 → 内存 TTL 缓存 → 内置快照回退。
//!
//! registry 数据只读不落库；拉取失败映射为回退（cache/snapshot）而非错误，
//! 保证该端点不把网络故障扩散给调用方。fetch 依赖可注入，测试不发真实请求。

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;

use crate::domain::harness::{
    AcpRegistryAgent, AcpRegistryBinary, AcpRegistryNpx, AcpRegistryPayload, RegistrySource,
};
use crate::domain::now_ms;

/// 官方 ACP registry 的 CDN 地址；`MOON_ACP_REGISTRY_URL` 可覆盖。
pub const DEFAULT_REGISTRY_URL: &str =
    "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";

/// 内存缓存 TTL；到期后下一次读取才重新拉取。
const REGISTRY_TTL: Duration = Duration::from_secs(6 * 3600);

const FETCH_TIMEOUT_SECS: u64 = 15;

/// registry 文本拉取函数；真实实现走 reqwest，测试注入假实现。
pub type RegistryFetcher = Arc<
    dyn Fn(String) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>> + Send + Sync,
>;

struct CachedPayload {
    fetched_at: i64,
    payload: AcpRegistryPayload,
}

pub struct AcpRegistryService {
    url: String,
    ttl: Duration,
    snapshot: &'static str,
    fetch: RegistryFetcher,
    cache: Mutex<Option<CachedPayload>>,
}

impl AcpRegistryService {
    pub fn new(snapshot: &'static str) -> Self {
        Self::with_fetcher(snapshot, default_fetcher())
    }

    /// 测试入口：注入假 fetcher 与自定义 TTL。
    pub fn with_fetcher(snapshot: &'static str, fetch: RegistryFetcher) -> Self {
        Self {
            url: std::env::var("MOON_ACP_REGISTRY_URL")
                .ok()
                .filter(|url| !url.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_REGISTRY_URL.to_string()),
            ttl: REGISTRY_TTL,
            snapshot,
            fetch,
            cache: Mutex::new(None),
        }
    }

    /// 读取目录：TTL 内直接命中缓存；过期或失败时按 network → cache → snapshot 降级。
    pub async fn list(&self) -> AcpRegistryPayload {
        let now = now_ms();
        if let Some(payload) = self.fresh_cache(now) {
            return payload;
        }

        let text = (self.fetch)(self.url.clone()).await;
        match text.and_then(|text| parse_registry_text(&text, now)) {
            Ok(mut payload) => {
                payload.source = RegistrySource::Network;
                payload.fetched_at = now;
                if let Ok(mut guard) = self.cache.lock() {
                    *guard = Some(CachedPayload {
                        fetched_at: now,
                        payload: payload.clone(),
                    });
                }
                payload
            }
            Err(cause) => {
                tracing::warn!(error = %cause, url = %self.url, "ACP registry 拉取失败，使用回退数据");
                self.fallback()
            }
        }
    }

    /// TTL 内的缓存命中；命中后把来源改写为 cache。
    fn fresh_cache(&self, now: i64) -> Option<AcpRegistryPayload> {
        let mut guard = self.cache.lock().ok()?;
        let cached = guard.as_mut()?;
        if now.saturating_sub(cached.fetched_at) >= self.ttl.as_millis() as i64 {
            return None;
        }
        let mut payload = cached.payload.clone();
        payload.source = RegistrySource::Cache;
        Some(payload)
    }

    /// 拉取失败后的降级：优先最近一次成功结果，否则内置快照。
    fn fallback(&self) -> AcpRegistryPayload {
        if let Ok(guard) = self.cache.lock() {
            if let Some(cached) = guard.as_ref() {
                let mut payload = cached.payload.clone();
                payload.source = RegistrySource::Cache;
                return payload;
            }
        }
        let mut payload = parse_registry_text(self.snapshot, 0).unwrap_or_else(|_| {
            AcpRegistryPayload {
                agents: Vec::new(),
                source: RegistrySource::Snapshot,
                fetched_at: 0,
            }
        });
        payload.source = RegistrySource::Snapshot;
        payload
    }
}

fn parse_registry_text(text: &str, fetched_at: i64) -> Result<AcpRegistryPayload, String> {
    let raw: RawRegistry =
        serde_json::from_str(text).map_err(|error| format!("registry JSON 解析失败：{error}"))?;
    let agents: Vec<AcpRegistryAgent> = raw
        .agents
        .into_iter()
        .filter(|agent| !agent.id.is_empty() && !agent.version.is_empty())
        .map(|agent| {
            let RawAgent {
                id,
                name,
                version,
                description,
                npx,
                windows_binary,
                distribution,
            } = agent;
            let npx = npx.or_else(|| distribution.as_ref().and_then(|d| d.npx.clone()));
            let windows_binary = windows_binary.or_else(|| {
                distribution
                    .as_ref()
                    .and_then(|d| d.binary.as_ref())
                    .and_then(|binary| binary.get("windows-x86_64").cloned())
            });
            AcpRegistryAgent {
                name: if name.is_empty() { id.clone() } else { name },
                id,
                version,
                summary: description.trim().to_string(),
                npx: npx.map(|npx| AcpRegistryNpx {
                    package: npx.package,
                    args: npx.args,
                }),
                windows_binary: windows_binary.map(|binary| AcpRegistryBinary {
                    cmd: binary.cmd,
                    args: binary.args,
                }),
            }
        })
        .collect();
    Ok(AcpRegistryPayload {
        agents,
        source: RegistrySource::Snapshot,
        fetched_at,
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRegistry {
    #[serde(default)]
    agents: Vec<RawAgent>,
}

/// 兼容两种输入形状：CDN 原始结构（`description` + `distribution`）与
/// 内置快照的修剪结构（`summary` + 顶层 `npx`/`windowsBinary`）。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawAgent {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default, alias = "summary")]
    description: String,
    #[serde(default)]
    npx: Option<RawNpx>,
    #[serde(default)]
    windows_binary: Option<RawBinary>,
    #[serde(default)]
    distribution: Option<RawDistribution>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawDistribution {
    #[serde(default)]
    npx: Option<RawNpx>,
    #[serde(default)]
    binary: Option<std::collections::BTreeMap<String, RawBinary>>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RawNpx {
    package: String,
    #[serde(default)]
    args: Vec<String>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RawBinary {
    cmd: String,
    #[serde(default)]
    args: Vec<String>,
}

/// 默认拉取：reqwest + rustls，带超时与 UA；错误统一映射为字符串。
fn default_fetcher() -> RegistryFetcher {
    Arc::new(|url: String| {
        Box::pin(async move {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(FETCH_TIMEOUT_SECS))
                .user_agent("Moon-Workbench/0.1")
                .build()
                .map_err(|error| format!("HTTP 客户端构建失败：{error}"))?;
            let response = client
                .get(&url)
                .send()
                .await
                .map_err(|error| format!("registry 请求失败：{error}"))?
                .error_for_status()
                .map_err(|error| format!("registry 响应异常：{error}"))?;
            response
                .text()
                .await
                .map_err(|error| format!("registry 响应读取失败：{error}"))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    const SNAPSHOT: &str = include_str!("../assets/acp_registry_snapshot.json");

    fn static_fetcher(
        text: &'static str,
        counter: Arc<StdMutex<usize>>,
    ) -> RegistryFetcher {
        Arc::new(move |_url| {
            let counter = counter.clone();
            Box::pin(async move {
                if let Ok(mut guard) = counter.lock() {
                    *guard += 1;
                }
                Ok(text.to_string())
            })
        })
    }

    fn failing_fetcher(counter: Arc<StdMutex<usize>>) -> RegistryFetcher {
        Arc::new(move |_url| {
            let counter = counter.clone();
            Box::pin(async move {
                if let Ok(mut guard) = counter.lock() {
                    *guard += 1;
                }
                Err("网络不可达".to_string())
            })
        })
    }

    fn fetch_count(counter: &Arc<StdMutex<usize>>) -> usize {
        counter.lock().map(|guard| *guard).unwrap_or(0)
    }

    #[tokio::test]
    async fn snapshot_parses_official_shape() {
        let payload = parse_registry_text(SNAPSHOT, 0).expect("快照必须可解析");
        assert!(payload.agents.len() >= 40, "快照应包含全部条目");
        let claude = payload
            .agents
            .iter()
            .find(|agent| agent.id == "claude-acp")
            .expect("claude-acp 必须在快照中");
        let npx = claude.npx.as_ref().expect("claude-acp 有 npx 分发");
        assert!(npx.package.starts_with("@agentclientprotocol/claude-agent-acp@"));
    }

    #[tokio::test]
    async fn cdn_raw_shape_maps_distribution() {
        let text = r#"{"agents":[{"id":"demo","name":"Demo","version":"1.0.0","description":"D","distribution":{"npx":{"package":"demo@1.0.0","args":["--acp"]},"binary":{"windows-x86_64":{"cmd":"./demo.exe","args":["serve"]}}}}]}"#;
        let payload = parse_registry_text(text, 0).expect("raw 形状必须可解析");
        let agent = &payload.agents[0];
        assert_eq!(agent.summary, "D");
        assert_eq!(agent.npx.as_ref().unwrap().package, "demo@1.0.0");
        assert_eq!(agent.windows_binary.as_ref().unwrap().cmd, "./demo.exe");
    }

    #[tokio::test]
    async fn network_then_cache_within_ttl() {
        let counter = Arc::new(StdMutex::new(0));
        let service = AcpRegistryService::with_fetcher(SNAPSHOT, static_fetcher("{}", counter.clone()));
        let first = service.list().await;
        assert_eq!(first.source, RegistrySource::Network);
        let second = service.list().await;
        assert_eq!(second.source, RegistrySource::Cache);
        assert_eq!(fetch_count(&counter), 1, "TTL 内不得重复拉取");
    }

    #[tokio::test]
    async fn expired_cache_refetches() {
        let counter = Arc::new(StdMutex::new(0));
        let mut service = AcpRegistryService::with_fetcher(SNAPSHOT, static_fetcher("{}", counter.clone()));
        service.ttl = Duration::ZERO;
        let _ = service.list().await;
        let _ = service.list().await;
        assert_eq!(fetch_count(&counter), 2, "TTL 过期后应重新拉取");
    }

    #[tokio::test]
    async fn fetch_failure_falls_back_to_snapshot() {
        let counter = Arc::new(StdMutex::new(0));
        let service = AcpRegistryService::with_fetcher(SNAPSHOT, failing_fetcher(counter.clone()));
        let payload = service.list().await;
        assert_eq!(payload.source, RegistrySource::Snapshot);
        assert!(payload.fetched_at == 0);
        assert!(!payload.agents.is_empty());
        assert_eq!(fetch_count(&counter), 1);
    }

    #[tokio::test]
    async fn fetch_failure_serves_stale_cache() {
        let counter = Arc::new(StdMutex::new(0));
        let good = static_fetcher("{\"agents\":[{\"id\":\"demo\",\"version\":\"1.0.0\"}]}", counter.clone());
        let bad = failing_fetcher(counter.clone());
        // 通过 Mutex 包一层切换 fetcher：第一次成功、第二次失败。
        let switch: Arc<StdMutex<RegistryFetcher>> = Arc::new(StdMutex::new(good));
        let switch_for_fetch = switch.clone();
        let fetcher: RegistryFetcher = Arc::new(move |url| {
            let current = switch_for_fetch.lock().unwrap().clone();
            current(url)
        });
        let service = AcpRegistryService::with_fetcher(SNAPSHOT, fetcher);
        let first = service.list().await;
        assert_eq!(first.source, RegistrySource::Network);
        *switch.lock().unwrap() = bad;
        let second = service.list().await;
        assert_eq!(second.source, RegistrySource::Cache);
        assert_eq!(second.agents.len(), 1);
    }

    #[tokio::test]
    async fn invalid_json_falls_back_to_snapshot() {
        let counter = Arc::new(StdMutex::new(0));
        let service = AcpRegistryService::with_fetcher(SNAPSHOT, static_fetcher("not-json", counter.clone()));
        let payload = service.list().await;
        assert_eq!(payload.source, RegistrySource::Snapshot);
        assert!(!payload.agents.is_empty());
    }
}
