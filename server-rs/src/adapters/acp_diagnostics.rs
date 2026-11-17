//! ACP 上游错误诊断。
//!
//! Kimi Code 在 provider 请求失败时把错误写进自己的会话 wire 日志，但对 ACP
//! 只返回 `end_turn` 且不输出文本，Moon 只能看到空回复。这里在空回复时读取
//! 该会话最后一条失败 turn 的错误字段，供适配器透传给用户。
//!
//! 只读取 `error.message` / `statusCode` 与会话日志的 `llm request failed`
//! 行；prompt、system prompt、请求体与密钥都不进入诊断文本。

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::tail;

const WIRE_TAIL_BYTES: u64 = 256 * 1024;
const LOG_TAIL_BYTES: u64 = 64 * 1024;

/// 读取 Kimi 会话最后一次失败 turn 的上游 provider 错误。
///
/// 会话目录从 `KIMI_CODE_HOME/session_index.jsonl` 解析；优先读 wire 中最后
/// 一条 `turn.ended`（只有失败才返回），缺失时回退到会话日志的
/// `llm request failed` 行。
pub fn kimi_last_upstream_error(home: &Path, session_id: &str) -> Option<String> {
    let session_dir = session_dir_from_index(home, session_id)?;
    let wire = read_tail(
        &session_dir.join("agents").join("main").join("wire.jsonl"),
        WIRE_TAIL_BYTES,
    );
    match parse_wire_error(&wire) {
        WireDiagnosis::Failed(Some(detail)) => Some(detail),
        // wire 失败但没带 message，或尾部没有 turn 记录时，日志仍是可靠来源。
        WireDiagnosis::Failed(None) | WireDiagnosis::Missing => parse_log_error(&read_tail(
            &session_dir.join("logs").join("kimi-code.log"),
            LOG_TAIL_BYTES,
        )),
        // 最后一条 turn 正常完成：即便日志里有更早的失败，也不能当成本轮错误。
        WireDiagnosis::NotFailed => None,
    }
}

/// wire 尾部最后一条 turn 的诊断状态。
enum WireDiagnosis {
    Failed(Option<String>),
    NotFailed,
    Missing,
}


/// 在会话索引里找 `sessionId` 对应的目录；重复条目取最后一条。
fn session_dir_from_index(home: &Path, session_id: &str) -> Option<PathBuf> {
    let text = fs::read_to_string(home.join("session_index.jsonl")).ok()?;
    let mut found = None;
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if value.get("sessionId").and_then(Value::as_str) != Some(session_id) {
            continue;
        }
        if let Some(dir) = value.get("sessionDir").and_then(Value::as_str) {
            found = Some(PathBuf::from(dir));
        }
    }
    found
}

/// 从后向前找最后一条 `turn.ended`，返回其失败诊断或“非失败”状态。
///
/// 如果最后一条 turn 正常完成，不再回退到更早的失败记录或日志，避免复用
/// 会话时把历史错误当成本轮错误。
fn parse_wire_error(text: &str) -> WireDiagnosis {
    for line in text.lines().rev() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) != Some("turn.ended") {
            continue;
        }
        if value.get("reason").and_then(Value::as_str) != Some("failed") {
            return WireDiagnosis::NotFailed;
        }
        let Some(error) = value.get("error") else {
            return WireDiagnosis::Failed(None);
        };
        let Some(message) = error.get("message").and_then(Value::as_str) else {
            return WireDiagnosis::Failed(None);
        };
        let status = error
            .pointer("/details/statusCode")
            .and_then(Value::as_u64)
            .and_then(|code| u16::try_from(code).ok());
        return WireDiagnosis::Failed(Some(format_upstream(message, status)));
    }
    WireDiagnosis::Missing
}

/// 解析会话日志最后一条 `llm request failed` 的 `errorMessage` 与 `statusCode`。
fn parse_log_error(text: &str) -> Option<String> {
    for line in text.lines().rev() {
        if !line.contains("llm request failed") {
            continue;
        }
        let Some(message) = quoted_value(line, "errorMessage=") else {
            continue;
        };
        let status = field_value(line, "statusCode=").and_then(|value| value.parse::<u16>().ok());
        return Some(format_upstream(&message, status));
    }
    None
}

/// 读取文件尾部；从中间开始时丢弃可能被截断的首行。
fn read_tail(path: &Path, limit: u64) -> String {
    let Ok(mut file) = File::open(path) else {
        return String::new();
    };
    let len = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    let start = len.saturating_sub(limit);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return String::new();
    }
    let mut bytes = Vec::new();
    if file.read_to_end(&mut bytes).is_err() {
        return String::new();
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    if start == 0 {
        return text;
    }
    match text.find('\n') {
        Some(index) => text[index + 1..].to_string(),
        None => String::new(),
    }
}

/// 读取 `key="..."` 的值，按 tracing 的转义规则还原 `\"`、`\\`、`\n`、`\t`。
fn quoted_value(line: &str, key: &str) -> Option<String> {
    let start = line.find(key)? + key.len();
    let rest = line.get(start..)?.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => break,
            },
            '"' => return Some(out),
            other => out.push(other),
        }
    }
    None
}

/// 读取 `key=value` 的 value（到空白为止）。
fn field_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let start = line.find(key)? + key.len();
    line.get(start..)?.split_whitespace().next()
}

fn format_upstream(message: &str, status: Option<u16>) -> String {
    let message = tail(message.trim());
    match status {
        Some(code) => format!("{message}（HTTP {code}）"),
        None => message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_session(home: &Path, session_id: &str, wire: Option<&str>, log: Option<&str>) {
        let session_dir = home.join("sessions").join("wd_test").join("session_test");
        fs::create_dir_all(session_dir.join("agents").join("main")).unwrap();
        fs::create_dir_all(session_dir.join("logs")).unwrap();
        let index = serde_json::json!({
            "sessionId": session_id,
            "sessionDir": session_dir.to_string_lossy(),
            "workDir": home.to_string_lossy(),
        });
        fs::write(
            home.join("session_index.jsonl"),
            format!("{}\n", serde_json::to_string(&index).unwrap()),
        )
        .unwrap();
        if let Some(wire) = wire {
            fs::write(session_dir.join("agents").join("main").join("wire.jsonl"), wire).unwrap();
        }
        if let Some(log) = log {
            fs::write(session_dir.join("logs").join("kimi-code.log"), log).unwrap();
        }
    }

    #[test]
    fn wire_failure_is_reported_with_status() {
        let dir = tempfile::tempdir().unwrap();
        let wire = format!(
            "{}\n",
            serde_json::json!({
                "type": "turn.ended",
                "agentId": "main",
                "turnId": 3,
                "reason": "failed",
                "error": {
                    "code": "provider.api_error",
                    "message": "Request is missing x-opencode-session",
                    "details": { "statusCode": 400 }
                }
            })
        );
        write_session(dir.path(), "session_1", Some(&wire), None);

        let error = kimi_last_upstream_error(dir.path(), "session_1").unwrap();
        assert_eq!(error, "Request is missing x-opencode-session（HTTP 400）");
    }

    #[test]
    fn completed_turn_does_not_reuse_older_error() {
        let dir = tempfile::tempdir().unwrap();
        let wire = format!(
            "{}\n{}\n",
            serde_json::json!({
                "type": "turn.ended",
                "reason": "failed",
                "error": { "message": "old error", "details": { "statusCode": 500 } }
            }),
            serde_json::json!({
                "type": "turn.ended",
                "reason": "completed"
            })
        );
        let log = "2026-11-17T04:41:50.234Z WARN  llm request failed  \
                   errorMessage=\"old log error\" statusCode=500\n";
        write_session(dir.path(), "session_1", Some(&wire), Some(log));

        assert!(kimi_last_upstream_error(dir.path(), "session_1").is_none());
    }

    #[test]
    fn log_fallback_unescapes_quotes() {
        let dir = tempfile::tempdir().unwrap();
        let log = concat!(
            "2026-11-17T04:41:48.860Z INFO  llm config  provider=openai\n",
            "2026-11-17T04:41:50.234Z WARN  llm request failed  ",
            "errorName=APIStatusError errorMessage=\"quote \\\"inside\\\" and path C:\\\\tmp\" ",
            "statusCode=400\n",
        );
        write_session(dir.path(), "session_1", None, Some(log));

        let error = kimi_last_upstream_error(dir.path(), "session_1").unwrap();
        assert_eq!(error, "quote \"inside\" and path C:\\tmp（HTTP 400）");
    }

    #[test]
    fn missing_index_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(kimi_last_upstream_error(dir.path(), "session_missing").is_none());
    }
}
