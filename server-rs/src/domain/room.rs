//! 房间领域类型与权限/回复模式枚举。

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// 房间默认回复模式：仅响应 @ 提及，或由内置主持人接管无 @ 消息。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoomReplyMode {
    Mentions,
    Moderator,
}

impl RoomReplyMode {
    pub const ALL: [Self; 2] = [Self::Mentions, Self::Moderator];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mentions => "mentions",
            Self::Moderator => "moderator",
        }
    }

    pub fn parse(value: &str) -> AppResult<Self> {
        match value {
            "mentions" => Ok(Self::Mentions),
            "moderator" => Ok(Self::Moderator),
            _ => Err(AppError::validation(format!(
                "replyMode 必须是 {} 之一",
                Self::ALL.map(Self::as_str).join(" | ")
            ))),
        }
    }
}

/// 房间级 harness 权限模式。
///
/// - `plan`：计划模式，只读规划；
/// - `auto`：自主模式，可在工作区读写与执行，危险操作由 harness 拦截；
/// - `full`：完全访问，无审批、无沙箱。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoomPermissionMode {
    Plan,
    Auto,
    Full,
}

impl RoomPermissionMode {
    pub const ALL: [Self; 3] = [Self::Plan, Self::Auto, Self::Full];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Auto => "auto",
            Self::Full => "full",
        }
    }

    pub fn parse(value: &str) -> AppResult<Self> {
        match value {
            "plan" => Ok(Self::Plan),
            "auto" => Ok(Self::Auto),
            "full" => Ok(Self::Full),
            _ => Err(AppError::validation(format!(
                "permissionMode 必须是 {} 之一",
                Self::ALL.map(Self::as_str).join(" | ")
            ))),
        }
    }
}

/// 新房间默认自主模式；已有房间由 v10 迁移保持为 `full`。
pub const DEFAULT_ROOM_PERMISSION_MODE: RoomPermissionMode = RoomPermissionMode::Auto;

/// 房间讨论轮数上限的合法范围与默认值。
pub const MIN_ROOM_DISCUSSION_ROUNDS: i64 = 1;
pub const MAX_ROOM_DISCUSSION_ROUNDS: i64 = 50;
pub const DEFAULT_ROOM_DISCUSSION_ROUNDS: i64 = 5;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Room {
    pub id: i64,
    pub name: String,
    /// 主工作目录，也是 harness 的运行 cwd。
    pub workspace_dir: String,
    /// 房间绑定的全部目录；第一个与 `workspace_dir` 相同。
    pub workspace_dirs: Vec<String>,
    /// 主持人必须是房间成员；内置主持人方案下始终指向内置主持人。
    pub moderator_agent_id: Option<i64>,
    /// 房间成员 id，按 id 升序。
    pub member_ids: Vec<i64>,
    pub reply_mode: RoomReplyMode,
    pub discussion_max_rounds: i64,
    pub permission_mode: RoomPermissionMode,
    pub created_at: i64,
    /// 最近一条消息时间（epoch 毫秒）；无消息为 `None`，仅列表查询注入。
    pub last_activity_at: Option<i64>,
}

/// store 层创建参数。
#[derive(Debug, Clone)]
pub struct CreateRoomInput {
    pub name: String,
    pub workspace_dir: String,
    pub workspace_dirs: Vec<String>,
    pub moderator_agent_id: Option<i64>,
    pub member_ids: Vec<i64>,
    pub reply_mode: RoomReplyMode,
    pub discussion_max_rounds: i64,
    pub permission_mode: RoomPermissionMode,
}

/// 局部更新：未传入的字段保持原值，`moderator_agent_id: Some(None)` 表示清空。
#[derive(Debug, Clone, Default)]
pub struct UpdateRoomInput {
    pub name: Option<String>,
    pub workspace_dir: Option<String>,
    pub workspace_dirs: Option<Vec<String>>,
    pub moderator_agent_id: Option<Option<i64>>,
    pub reply_mode: Option<RoomReplyMode>,
    pub discussion_max_rounds: Option<i64>,
    pub permission_mode: Option<RoomPermissionMode>,
}
