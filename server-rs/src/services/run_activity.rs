//! 运行活动注册表。
//!
//! 只保存在进程内：运行期间供实时广播、运行结束后供「最近运行」回看。
//! 不写数据库；进程重启后活动为空，运行本身仍可从 `agent_runs` 读取。

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use serde_json::Value;

use crate::domain::now_ms;
use crate::domain::run::{RunActivity, RunActivityKind};

/// 单个运行保留的活动条数上限；超出时丢弃最旧条目。
pub const RUN_ACTIVITY_LIMIT: usize = 300;
/// 单条活动明细的最大字符数。
pub const RUN_ACTIVITY_DETAIL_LIMIT: usize = 800;
/// 注册表最多保留的运行数。
pub const RUN_ACTIVITY_RUN_LIMIT: usize = 200;

const RUN_ACTIVITY_TITLE_LIMIT: usize = 120;
const TOOL_PAYLOAD_LIMIT: usize = 400;

const PREFERRED_INPUT_KEYS: [&str; 8] = [
    "command",
    "file_path",
    "path",
    "pattern",
    "query",
    "url",
    "description",
    "prompt",
];

#[derive(Default)]
struct RegistryState {
    activities: HashMap<i64, Vec<RunActivity>>,
    seq_by_run: HashMap<i64, i64>,
    order: VecDeque<i64>,
}

#[derive(Default)]
pub struct RunActivityRegistry {
    state: Mutex<RegistryState>,
}

impl RunActivityRegistry {
    /// 追加一条活动并返回落定快照；`seq` 在单个运行内严格递增。
    pub fn record(
        &self,
        run_id: i64,
        kind: RunActivityKind,
        title: &str,
        detail: Option<&str>,
    ) -> RunActivity {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let seq = state.seq_by_run.get(&run_id).copied().unwrap_or(0) + 1;
        state.seq_by_run.insert(run_id, seq);

        let trimmed = detail.unwrap_or("").trim();
        let activity = RunActivity {
            run_id,
            seq,
            kind,
            title: truncate(title, RUN_ACTIVITY_TITLE_LIMIT),
            detail: if trimmed.is_empty() {
                None
            } else {
                Some(truncate(trimmed, RUN_ACTIVITY_DETAIL_LIMIT))
            },
            at: now_ms(),
        };

        if !state.activities.contains_key(&run_id) {
            state.order.push_back(run_id);
        }
        let list = state.activities.entry(run_id).or_default();
        list.push(activity.clone());
        if list.len() > RUN_ACTIVITY_LIMIT {
            let excess = list.len() - RUN_ACTIVITY_LIMIT;
            list.drain(0..excess);
        }

        while state.activities.len() > RUN_ACTIVITY_RUN_LIMIT {
            let Some(evicted) = state.order.pop_front() else {
                break;
            };
            state.activities.remove(&evicted);
            state.seq_by_run.remove(&evicted);
        }
        activity
    }

    /// 按 `seq` 升序返回某次运行的当前活动快照（副本）。
    pub fn list(&self, run_id: i64) -> Vec<RunActivity> {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .activities
            .get(&run_id)
            .cloned()
            .unwrap_or_default()
    }

    /// 已缓存的运行数量（测试与诊断用）。
    pub fn run_count(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .activities
            .len()
    }
}

/// 把工具输入压缩成一行可读摘要；返回值已截断。
pub fn summarize_tool_payload(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(truncate(trimmed, TOOL_PAYLOAD_LIMIT))
            }
        }
        Value::Number(_) | Value::Bool(_) => Some(value.to_string()),
        Value::Object(map) => {
            for key in PREFERRED_INPUT_KEYS {
                if let Some(Value::String(candidate)) = map.get(key) {
                    let trimmed = candidate.trim();
                    if !trimmed.is_empty() {
                        return Some(truncate(trimmed, TOOL_PAYLOAD_LIMIT));
                    }
                }
            }
            let text = serde_json::to_string(value).ok()?;
            if text.is_empty() {
                None
            } else {
                Some(truncate(&text, TOOL_PAYLOAD_LIMIT))
            }
        }
        Value::Array(_) => {
            let text = serde_json::to_string(value).ok()?;
            if text.is_empty() {
                None
            } else {
                Some(truncate(&text, TOOL_PAYLOAD_LIMIT))
            }
        }
    }
}

/// 组织本轮用量文案；全部缺失时返回 None，避免产生空活动。
pub fn format_usage_summary(
    token_count: Option<i64>,
    tool_call_count: Option<i64>,
    duration_ms: Option<i64>,
) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(tokens) = token_count {
        parts.push(format!("tokens {tokens}"));
    }
    if let Some(tools) = tool_call_count {
        parts.push(format!("工具 {tools} 次"));
    }
    if let Some(duration) = duration_ms {
        parts.push(format!("{:.1}s", duration as f64 / 1000.0));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    }
}

pub fn truncate(text: &str, limit: usize) -> String {
    let count = text.chars().count();
    if count <= limit {
        return text.to_string();
    }
    let mut truncated: String = text.chars().take(limit).collect();
    truncated.push('…');
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn summarizes_preferred_fields() {
        assert_eq!(
            summarize_tool_payload(&json!({ "command": "  cargo test  ", "path": "/x" })),
            Some("cargo test".to_string())
        );
        assert_eq!(
            summarize_tool_payload(&json!({ "other": 1 })),
            Some("{\"other\":1}".to_string())
        );
        assert_eq!(summarize_tool_payload(&Value::Null), None);
    }

    #[test]
    fn formats_usage() {
        assert_eq!(
            format_usage_summary(Some(120), Some(2), Some(1500)),
            Some("tokens 120 · 工具 2 次 · 1.5s".to_string())
        );
        assert_eq!(format_usage_summary(None, None, None), None);
    }

    #[test]
    fn activity_seq_and_limits() {
        let registry = RunActivityRegistry::default();
        let first = registry.record(1, RunActivityKind::Tool, "调用 shell", Some(" cargo "));
        let second = registry.record(1, RunActivityKind::Reasoning, "思考", None);
        assert_eq!(first.seq, 1);
        assert_eq!(first.detail.as_deref(), Some("cargo"));
        assert_eq!(second.seq, 2);
        assert_eq!(registry.list(1).len(), 2);
        assert_eq!(registry.list(2).len(), 0);
    }
}
