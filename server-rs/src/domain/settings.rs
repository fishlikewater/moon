//! 应用级设置：服务端持久化的默认值与可调上限。
//!
//! 只保存跨页面共享的默认值；主题、侧栏折叠等纯浏览器偏好仍由前端
//! localStorage 管理，不进入这里。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::room::RoomPermissionMode;
use crate::error::{AppError, AppResult};

/// 新建房间默认权限模式。
pub const DEFAULT_ROOM_PERMISSION_MODE: RoomPermissionMode = RoomPermissionMode::Auto;
/// 新建房间默认讨论轮数。
pub const DEFAULT_ROOM_DISCUSSION_ROUNDS: i64 = 5;
/// 上下文压缩默认开启。
pub const DEFAULT_CONTEXT_COMPRESSION_ENABLED: bool = true;
/// 上下文提示词默认字符预算。
pub const DEFAULT_CONTEXT_BUDGET_CHARS: i64 = 24_000;
/// 单附件默认上限：25 MiB。
pub const DEFAULT_ATTACHMENT_MAX_FILE_BYTES: i64 = 26_214_400;
/// 单条消息默认附件数量上限。
pub const DEFAULT_ATTACHMENT_MAX_FILES_PER_MESSAGE: i64 = 10;

pub const MIN_ROOM_DISCUSSION_ROUNDS: i64 = 1;
pub const MAX_ROOM_DISCUSSION_ROUNDS: i64 = 50;
pub const MIN_CONTEXT_BUDGET_CHARS: i64 = 2_000;
pub const MAX_CONTEXT_BUDGET_CHARS: i64 = 1_000_000;
pub const MIN_ATTACHMENT_MAX_FILE_BYTES: i64 = 1_048_576;
pub const MAX_ATTACHMENT_MAX_FILE_BYTES: i64 = 104_857_600;
pub const MIN_ATTACHMENT_MAX_FILES_PER_MESSAGE: i64 = 1;
pub const MAX_ATTACHMENT_MAX_FILES_PER_MESSAGE: i64 = 20;
/// 运行总时长上限：`0` 表示不限制，其余取值范围为秒。
pub const MIN_RUN_TIMEOUT_SECS: i64 = 60;
pub const MAX_RUN_TIMEOUT_SECS: i64 = 86_400;

pub const SETTINGS_KEYS: [&str; 9] = [
    "defaultRoomPermissionMode",
    "defaultRoomDiscussionRounds",
    "contextCompressionEnabled",
    "contextBudgetChars",
    "attachmentMaxFileBytes",
    "attachmentMaxFilesPerMessage",
    "runTimeoutSecs",
    "zcodeBinPath",
    "codexBinPath",
];

/// harness 可执行文件覆盖路径的最大长度。
pub const MAX_BIN_PATH_CHARS: usize = 1024;

/// 应用设置全集；JSON 与前端契约均为 camelCase。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub default_room_permission_mode: RoomPermissionMode,
    pub default_room_discussion_rounds: i64,
    pub context_compression_enabled: bool,
    pub context_budget_chars: i64,
    pub attachment_max_file_bytes: i64,
    pub attachment_max_files_per_message: i64,
    /// 运行总时长上限（秒）；`0` 表示不限制。
    pub run_timeout_secs: i64,
    /// zcode 可执行文件覆盖；空字符串表示自动检测。
    pub zcode_bin_path: String,
    /// codex 可执行文件覆盖；空字符串表示自动检测。
    pub codex_bin_path: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            default_room_permission_mode: DEFAULT_ROOM_PERMISSION_MODE,
            default_room_discussion_rounds: DEFAULT_ROOM_DISCUSSION_ROUNDS,
            context_compression_enabled: DEFAULT_CONTEXT_COMPRESSION_ENABLED,
            context_budget_chars: DEFAULT_CONTEXT_BUDGET_CHARS,
            attachment_max_file_bytes: DEFAULT_ATTACHMENT_MAX_FILE_BYTES,
            attachment_max_files_per_message: DEFAULT_ATTACHMENT_MAX_FILES_PER_MESSAGE,
            run_timeout_secs: crate::adapters::DEFAULT_RUN_TIMEOUT_SECS as i64,
            zcode_bin_path: String::new(),
            codex_bin_path: String::new(),
        }
    }
}

impl AppSettings {
    /// 存储用的键值对；布尔用 `1`/`0`，数字用十进制字符串。
    pub fn to_pairs(&self) -> Vec<(&'static str, String)> {
        vec![
            (
                "defaultRoomPermissionMode",
                self.default_room_permission_mode.as_str().to_string(),
            ),
            (
                "defaultRoomDiscussionRounds",
                self.default_room_discussion_rounds.to_string(),
            ),
            (
                "contextCompressionEnabled",
                if self.context_compression_enabled {
                    "1".to_string()
                } else {
                    "0".to_string()
                },
            ),
            ("contextBudgetChars", self.context_budget_chars.to_string()),
            (
                "attachmentMaxFileBytes",
                self.attachment_max_file_bytes.to_string(),
            ),
            (
                "attachmentMaxFilesPerMessage",
                self.attachment_max_files_per_message.to_string(),
            ),
            ("runTimeoutSecs", self.run_timeout_secs.to_string()),
            ("zcodeBinPath", self.zcode_bin_path.clone()),
            ("codexBinPath", self.codex_bin_path.clone()),
        ]
    }

    /// 从数据库键值读取；缺失或损坏的单项回退默认值，避免坏数据阻塞启动。
    pub fn from_pairs(pairs: &std::collections::HashMap<String, String>) -> Self {
        let default = Self::default();
        let int_in_range = |key: &str, min: i64, max: i64, fallback: i64| {
            pairs
                .get(key)
                .and_then(|value| value.parse::<i64>().ok())
                .filter(|value| (*value >= min) && (*value <= max))
                .unwrap_or(fallback)
        };
        Self {
            default_room_permission_mode: pairs
                .get("defaultRoomPermissionMode")
                .and_then(|value| RoomPermissionMode::parse(value).ok())
                .unwrap_or(default.default_room_permission_mode),
            default_room_discussion_rounds: int_in_range(
                "defaultRoomDiscussionRounds",
                MIN_ROOM_DISCUSSION_ROUNDS,
                MAX_ROOM_DISCUSSION_ROUNDS,
                default.default_room_discussion_rounds,
            ),
            context_compression_enabled: pairs
                .get("contextCompressionEnabled")
                .map(|value| value == "1")
                .unwrap_or(default.context_compression_enabled),
            context_budget_chars: int_in_range(
                "contextBudgetChars",
                MIN_CONTEXT_BUDGET_CHARS,
                MAX_CONTEXT_BUDGET_CHARS,
                default.context_budget_chars,
            ),
            attachment_max_file_bytes: int_in_range(
                "attachmentMaxFileBytes",
                MIN_ATTACHMENT_MAX_FILE_BYTES,
                MAX_ATTACHMENT_MAX_FILE_BYTES,
                default.attachment_max_file_bytes,
            ),
            attachment_max_files_per_message: int_in_range(
                "attachmentMaxFilesPerMessage",
                MIN_ATTACHMENT_MAX_FILES_PER_MESSAGE,
                MAX_ATTACHMENT_MAX_FILES_PER_MESSAGE,
                default.attachment_max_files_per_message,
            ),
            run_timeout_secs: pairs
                .get("runTimeoutSecs")
                .and_then(|value| value.parse::<i64>().ok())
                .filter(|value| {
                    *value == 0
                        || (*value >= MIN_RUN_TIMEOUT_SECS && *value <= MAX_RUN_TIMEOUT_SECS)
                })
                // 环境变量只在设置库尚无该键时作为初始默认，之后以设置为准。
                .unwrap_or_else(initial_run_timeout_secs),
            zcode_bin_path: bin_path(pairs, "zcodeBinPath"),
            codex_bin_path: bin_path(pairs, "codexBinPath"),
        }
    }
}

/// 运行时长初始默认：`MOON_RUN_TIMEOUT_SECS` 派生值（未设置时 1800，`0` 映射为不限制）。
pub fn initial_run_timeout_secs() -> i64 {
    match crate::adapters::run_timeout_from_env() {
        Some(timeout) => timeout.as_secs().min(i64::MAX as u64) as i64,
        None => 0,
    }
}

/// `GET /api/settings` 响应；设置本身包含可编辑的运行时长。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettingsResponse {
    pub settings: AppSettings,
}

/// 对部分更新做全量校验；未知键或非法值都拒绝，返回基于当前值的新设置。
pub fn apply_patch(current: &AppSettings, patch: &Value) -> AppResult<AppSettings> {
    let Some(map) = patch.as_object() else {
        return Err(AppError::validation("设置请求体必须是 JSON 对象"));
    };

    let mut next = current.clone();
    for (key, value) in map {
        match key.as_str() {
            "defaultRoomPermissionMode" => {
                let text = value
                    .as_str()
                    .ok_or_else(|| AppError::validation("defaultRoomPermissionMode 必须是字符串"))?;
                next.default_room_permission_mode = RoomPermissionMode::parse(text)?;
            }
            "defaultRoomDiscussionRounds" => {
                next.default_room_discussion_rounds = parse_int(
                    value,
                    key,
                    MIN_ROOM_DISCUSSION_ROUNDS,
                    MAX_ROOM_DISCUSSION_ROUNDS,
                )?;
            }
            "contextCompressionEnabled" => {
                next.context_compression_enabled = value
                    .as_bool()
                    .ok_or_else(|| AppError::validation("contextCompressionEnabled 必须是布尔值"))?;
            }
            "contextBudgetChars" => {
                next.context_budget_chars = parse_int(
                    value,
                    key,
                    MIN_CONTEXT_BUDGET_CHARS,
                    MAX_CONTEXT_BUDGET_CHARS,
                )?;
            }
            "attachmentMaxFileBytes" => {
                next.attachment_max_file_bytes = parse_int(
                    value,
                    key,
                    MIN_ATTACHMENT_MAX_FILE_BYTES,
                    MAX_ATTACHMENT_MAX_FILE_BYTES,
                )?;
            }
            "attachmentMaxFilesPerMessage" => {
                next.attachment_max_files_per_message = parse_int(
                    value,
                    key,
                    MIN_ATTACHMENT_MAX_FILES_PER_MESSAGE,
                    MAX_ATTACHMENT_MAX_FILES_PER_MESSAGE,
                )?;
            }
            "runTimeoutSecs" => {
                next.run_timeout_secs = parse_run_timeout_setting(value)?;
            }
            "zcodeBinPath" => {
                next.zcode_bin_path = parse_bin_path(value, key)?;
            }
            "codexBinPath" => {
                next.codex_bin_path = parse_bin_path(value, key)?;
            }
            other => {
                return Err(AppError::validation(format!("未知设置项：{other}")));
            }
        }
    }
    Ok(next)
}

fn parse_bin_path(value: &Value, key: &str) -> AppResult<String> {
    let text = value
        .as_str()
        .ok_or_else(|| AppError::validation(format!("{key} 必须是字符串")))?;
    let trimmed = text.trim();
    if trimmed.chars().count() > MAX_BIN_PATH_CHARS {
        return Err(AppError::validation(format!(
            "{key} 不能超过 {MAX_BIN_PATH_CHARS} 个字符"
        )));
    }
    Ok(trimmed.to_string())
}

fn bin_path(pairs: &std::collections::HashMap<String, String>, key: &str) -> String {
    pairs
        .get(key)
        .map(|value| value.trim().to_string())
        .filter(|value| value.chars().count() <= MAX_BIN_PATH_CHARS)
        .unwrap_or_default()
}

fn parse_int(value: &Value, key: &str, min: i64, max: i64) -> AppResult<i64> {
    let number = value
        .as_i64()
        .or_else(|| {
            value
                .as_f64()
                .filter(|number| number.fract() == 0.0)
                .map(|number| number as i64)
        })
        .ok_or_else(|| AppError::validation(format!("{key} 必须是整数")))?;
    if number < min || number > max {
        return Err(AppError::validation(format!(
            "{key} 必须在 {min} 到 {max} 之间"
        )));
    }
    Ok(number)
}

/// `0` 表示不限制，其余必须在 60–86400 秒之间。
fn parse_run_timeout_setting(value: &Value) -> AppResult<i64> {
    let number = parse_int(value, "runTimeoutSecs", i64::MIN, i64::MAX)?;
    if number != 0 && (number < MIN_RUN_TIMEOUT_SECS || number > MAX_RUN_TIMEOUT_SECS) {
        return Err(AppError::validation(format!(
            "runTimeoutSecs 必须为 0（不限制）或在 {MIN_RUN_TIMEOUT_SECS} 到 {MAX_RUN_TIMEOUT_SECS} 之间"
        )));
    }
    Ok(number)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_are_stable() {
        let settings = AppSettings::default();
        assert_eq!(
            settings.default_room_permission_mode,
            RoomPermissionMode::Auto
        );
        assert_eq!(settings.default_room_discussion_rounds, 5);
        assert!(settings.context_compression_enabled);
        assert_eq!(settings.context_budget_chars, 24_000);
        assert_eq!(settings.attachment_max_file_bytes, 26_214_400);
        assert_eq!(settings.attachment_max_files_per_message, 10);
    }

    #[test]
    fn patch_accepts_all_known_keys() {
        let next = apply_patch(
            &AppSettings::default(),
            &json!({
                "defaultRoomPermissionMode": "plan",
                "defaultRoomDiscussionRounds": 8,
                "contextCompressionEnabled": false,
                "contextBudgetChars": 8000,
                "attachmentMaxFileBytes": 1048576,
                "attachmentMaxFilesPerMessage": 3,
            }),
        )
        .unwrap();
        assert_eq!(next.default_room_permission_mode, RoomPermissionMode::Plan);
        assert_eq!(next.default_room_discussion_rounds, 8);
        assert!(!next.context_compression_enabled);
        assert_eq!(next.context_budget_chars, 8_000);
        assert_eq!(next.attachment_max_file_bytes, 1_048_576);
        assert_eq!(next.attachment_max_files_per_message, 3);
    }

    #[test]
    fn patch_rejects_unknown_and_out_of_range_values() {
        let current = AppSettings::default();
        assert!(apply_patch(&current, &json!({"unknownKey": 1})).is_err());
        assert!(apply_patch(&current, &json!({"defaultRoomPermissionMode": "nope"})).is_err());
        assert!(apply_patch(&current, &json!({"defaultRoomDiscussionRounds": 0})).is_err());
        assert!(apply_patch(&current, &json!({"defaultRoomDiscussionRounds": 51})).is_err());
        assert!(apply_patch(&current, &json!({"contextBudgetChars": 1999})).is_err());
        assert!(apply_patch(&current, &json!({"attachmentMaxFileBytes": 1024})).is_err());
        assert!(apply_patch(&current, &json!({"attachmentMaxFilesPerMessage": 21})).is_err());
        assert!(apply_patch(&current, &json!({"contextCompressionEnabled": "yes"})).is_err());
    }

    #[test]
    fn pair_round_trip_preserves_values() {
        let settings = AppSettings {
            default_room_permission_mode: RoomPermissionMode::Full,
            default_room_discussion_rounds: 9,
            context_compression_enabled: false,
            context_budget_chars: 12_345,
            attachment_max_file_bytes: 2_000_000,
            attachment_max_files_per_message: 4,
            run_timeout_secs: 1_800,
            zcode_bin_path: r"D:\ZCode\ZCode.exe".to_string(),
            codex_bin_path: r"D:\tools\codex.exe".to_string(),
        };
        let mut pairs = std::collections::HashMap::new();
        for (key, value) in settings.to_pairs() {
            pairs.insert(key.to_string(), value);
        }
        assert_eq!(AppSettings::from_pairs(&pairs), settings);
    }

    #[test]
    fn bin_path_patch_trims_and_rejects_overlong_values() {
        let current = AppSettings::default();
        let trimmed = apply_patch(&current, &json!({ "zcodeBinPath": "  D:\\ZCode\\ZCode.exe  " }))
            .unwrap();
        assert_eq!(trimmed.zcode_bin_path, r"D:\ZCode\ZCode.exe");
        assert_eq!(trimmed.codex_bin_path, "");

        let cleared = apply_patch(&trimmed, &json!({ "zcodeBinPath": "  " })).unwrap();
        assert_eq!(cleared.zcode_bin_path, "");

        let overlong = "x".repeat(MAX_BIN_PATH_CHARS + 1);
        assert!(apply_patch(&current, &json!({ "codexBinPath": overlong })).is_err());
        assert!(apply_patch(&current, &json!({ "codexBinPath": 42 })).is_err());
    }
}
