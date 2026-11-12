//! 房间业务规则。
//!
//! 关键不变量：工作空间目录必须真实存在且是目录；每个房间自动包含内置主持人，
//! 且主持人固定为内置主持人；客户端不能手动指定或更换主持人。
//! 三条都在服务层强制，路由层不做业务判断。

use std::sync::Arc;

use serde_json::Value;

use crate::domain::room::{
    CreateRoomInput, Room, RoomPermissionMode, RoomReplyMode, UpdateRoomInput,
    DEFAULT_ROOM_DISCUSSION_ROUNDS, DEFAULT_ROOM_PERMISSION_MODE,
    MAX_ROOM_DISCUSSION_ROUNDS, MIN_ROOM_DISCUSSION_ROUNDS,
};
use crate::error::{AppError, AppResult};
use crate::services::builtin_moderator::BuiltinModeratorService;
use crate::services::sessions::SessionService;
use crate::services::{
    as_positive_id, parse_id, require_body, require_non_empty_string,
};
use crate::store::{AgentStore, RoomStore};

pub struct RoomService {
    rooms: RoomStore,
    agents: AgentStore,
    builtin_moderator: BuiltinModeratorService,
    sessions: Arc<SessionService>,
}

impl RoomService {
    pub fn new(
        rooms: RoomStore,
        agents: AgentStore,
        builtin_moderator: BuiltinModeratorService,
        sessions: Arc<SessionService>,
    ) -> Self {
        Self {
            rooms,
            agents,
            builtin_moderator,
            sessions,
        }
    }

    pub async fn list(&self) -> AppResult<Vec<Room>> {
        self.rooms.list().await
    }

    pub async fn create(&self, input: Value) -> AppResult<Room> {
        let body = require_body(input)?;
        let name = require_non_empty_string(body.get("name"), "name")?;
        let (workspace_dir, workspace_dirs) =
            require_workspace_dirs(body.get("workspaceDirs"), body.get("workspaceDir")).await?;
        let member_ids = parse_member_ids(body.get("memberIds"))?;
        let reply_mode = match body.get("replyMode") {
            None => RoomReplyMode::Mentions,
            Some(value) => parse_reply_mode(value)?,
        };
        let discussion_max_rounds = match body.get("discussionMaxRounds") {
            None => DEFAULT_ROOM_DISCUSSION_ROUNDS,
            Some(value) => parse_discussion_max_rounds(value)?,
        };
        let permission_mode = match body.get("permissionMode") {
            None => DEFAULT_ROOM_PERMISSION_MODE,
            Some(value) => parse_permission_mode(value)?,
        };

        // 主持人由系统固定，不接受客户端手动指定。
        if body.contains_key("moderatorAgentId")
            && parse_moderator_id(body.get("moderatorAgentId"))?.is_some()
        {
            return Err(AppError::validation("主持人由系统内置，不支持手动指定"));
        }

        let moderator = self.builtin_moderator.ensure().await?;
        let mut members = member_ids;
        members.push(moderator.id);
        members.sort_unstable();
        members.dedup();

        for agent_id in &members {
            if *agent_id != moderator.id {
                self.require_agent(*agent_id).await?;
            }
        }

        self.rooms
            .create(CreateRoomInput {
                name,
                workspace_dir,
                workspace_dirs,
                moderator_agent_id: Some(moderator.id),
                member_ids: members,
                reply_mode,
                discussion_max_rounds,
                permission_mode,
            })
            .await
    }

    pub async fn update(&self, raw_id: &str, input: Value) -> AppResult<Room> {
        let id = parse_id(raw_id)?;
        self.require_room(id).await?;
        let body = require_body(input)?;
        let mut patch = UpdateRoomInput::default();

        if body.contains_key("moderatorAgentId") {
            return Err(AppError::validation("主持人由系统内置，不支持修改"));
        }
        if let Some(value) = body.get("name") {
            patch.name = Some(require_non_empty_string(Some(value), "name")?);
        }
        if body.contains_key("workspaceDirs") {
            let (_, dirs) =
                require_workspace_dirs(body.get("workspaceDirs"), None).await?;
            patch.workspace_dirs = Some(dirs);
        } else if let Some(value) = body.get("workspaceDir") {
            patch.workspace_dir = Some(require_workspace_dir(Some(value)).await?);
        }
        if let Some(value) = body.get("replyMode") {
            patch.reply_mode = Some(parse_reply_mode(value)?);
        }
        if let Some(value) = body.get("discussionMaxRounds") {
            patch.discussion_max_rounds = Some(parse_discussion_max_rounds(value)?);
        }
        if let Some(value) = body.get("permissionMode") {
            patch.permission_mode = Some(parse_permission_mode(value)?);
        }

        // 修复历史房间：确保内置主持人仍是成员并担任主持人。
        let moderator = self.builtin_moderator.ensure().await?;
        self.rooms.add_member(id, moderator.id).await?;
        patch.moderator_agent_id = Some(Some(moderator.id));

        self.rooms
            .update(id, patch)
            .await?
            .ok_or_else(|| AppError::not_found(format!("房间不存在：{id}")))
    }

    /// 删除房间及其全部房间级数据。
    ///
    /// 运行中的讨论或委派批次仍在后台循环，直接删除会与消息写入竞争，
    /// 因此先拒绝并要求停止。
    pub async fn remove(&self, raw_id: &str) -> AppResult<()> {
        let id = parse_id(raw_id)?;
        self.require_room(id).await?;

        if self.rooms.has_running_discussion(id).await? {
            return Err(AppError::constraint("房间有进行中的讨论，请先停止讨论"));
        }
        if self.rooms.has_running_collaboration(id).await? {
            return Err(AppError::constraint("房间有进行中的委派，请先停止委派"));
        }

        if self.rooms.delete(id).await? {
            // 会话记录已随事务删除；存储目录按房间整体清理，失败只留下临时目录。
            self.sessions.delete_room_storage(id);
        }
        Ok(())
    }

    pub async fn add_member(&self, raw_room_id: &str, raw_agent_id: &str) -> AppResult<Room> {
        let room = self.require_room(parse_id(raw_room_id)?).await?;
        let agent_id = parse_id(raw_agent_id)?;
        self.require_agent(agent_id).await?;

        self.rooms.add_member(room.id, agent_id).await?;
        self.require_room(room.id).await
    }

    pub async fn remove_member(
        &self,
        raw_room_id: &str,
        raw_agent_id: &str,
    ) -> AppResult<Room> {
        let room = self.require_room(parse_id(raw_room_id)?).await?;
        let agent_id = parse_id(raw_agent_id)?;
        let agent = self.agents.get_by_id(agent_id).await?;

        if agent
            .as_ref()
            .is_some_and(|item| item.builtin_role.is_some())
        {
            return Err(AppError::constraint("内置主持人不能移出房间"));
        }
        if room.moderator_agent_id == Some(agent_id) {
            return Err(AppError::constraint(
                "该成员是房间主持人，请先更换主持人",
            ));
        }

        if !self.rooms.remove_member(room.id, agent_id).await? {
            return Err(AppError::not_found(format!(
                "房间成员不存在：{agent_id}"
            )));
        }

        // 成员关系已删除；归档其活动会话，避免重新加入后继续续接旧上下文。
        // 归档失败不阻塞移除，结构化日志便于排查。
        if let Err(error) = self.sessions.archive_by_agent(room.id, agent_id).await {
            tracing::error!(
                room_id = room.id,
                agent_id,
                error = %error,
                "agent_session_archive_failed"
            );
        }

        self.require_room(room.id).await
    }

    async fn require_room(&self, id: i64) -> AppResult<Room> {
        self.rooms
            .get_by_id(id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("房间不存在：{id}")))
    }

    async fn require_agent(&self, agent_id: i64) -> AppResult<()> {
        if self.agents.get_by_id(agent_id).await?.is_none() {
            return Err(AppError::not_found(format!(
                "智能体不存在：{agent_id}"
            )));
        }
        Ok(())
    }
}

fn parse_member_ids(value: Option<&Value>) -> AppResult<Vec<i64>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let Value::Array(items) = value else {
        return Err(AppError::validation("memberIds 必须是智能体 id 数组"));
    };
    let mut ids = Vec::with_capacity(items.len());
    for item in items {
        let Some(id) = as_positive_id(item) else {
            return Err(AppError::validation("memberIds 必须是智能体 id 数组"));
        };
        ids.push(id);
    }
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

fn parse_moderator_id(value: Option<&Value>) -> AppResult<Option<i64>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => as_positive_id(value)
            .map(Some)
            .ok_or_else(|| {
                AppError::validation("moderatorAgentId 必须是智能体 id 或 null")
            }),
    }
}

fn parse_discussion_max_rounds(value: &Value) -> AppResult<i64> {
    match as_positive_id(value) {
        Some(rounds)
            if (MIN_ROOM_DISCUSSION_ROUNDS..=MAX_ROOM_DISCUSSION_ROUNDS)
                .contains(&rounds) =>
        {
            Ok(rounds)
        }
        _ => Err(AppError::validation(format!(
            "discussionMaxRounds 必须是 {MIN_ROOM_DISCUSSION_ROUNDS}..{MAX_ROOM_DISCUSSION_ROUNDS} 的整数"
        ))),
    }
}

fn parse_reply_mode(value: &Value) -> AppResult<RoomReplyMode> {
    value
        .as_str()
        .ok_or_else(|| {
            AppError::validation(format!(
                "replyMode 必须是 {} 之一",
                RoomReplyMode::ALL.map(RoomReplyMode::as_str).join(" | ")
            ))
        })
        .and_then(RoomReplyMode::parse)
}

fn parse_permission_mode(value: &Value) -> AppResult<RoomPermissionMode> {
    value
        .as_str()
        .ok_or_else(|| {
            AppError::validation(format!(
                "permissionMode 必须是 {} 之一",
                RoomPermissionMode::ALL
                    .map(RoomPermissionMode::as_str)
                    .join(" | ")
            ))
        })
        .and_then(RoomPermissionMode::parse)
}

/// 解析 1–32 个已存在目录；第一个作为运行 cwd。
async fn require_workspace_dirs(
    dirs_value: Option<&Value>,
    single_value: Option<&Value>,
) -> AppResult<(String, Vec<String>)> {
    let mut raw_dirs = Vec::new();
    if let Some(value) = dirs_value {
        let items = value
            .as_array()
            .ok_or_else(|| AppError::validation("workspaceDirs 必须是字符串数组"))?;
        if items.is_empty() {
            return Err(AppError::validation("workspaceDirs 至少需要一个目录"));
        }
        for item in items {
            let dir = item
                .as_str()
                .map(str::trim)
                .filter(|dir| !dir.is_empty())
                .ok_or_else(|| {
                    AppError::validation("workspaceDirs 每项必须是非空字符串")
                })?;
            raw_dirs.push(dir.to_string());
        }
    } else if let Some(value) = single_value {
        raw_dirs.push(require_non_empty_string(Some(value), "workspaceDir")?);
    } else {
        return Err(AppError::validation(
            "缺少 workspaceDirs 或 workspaceDir",
        ));
    }

    let mut dirs: Vec<String> = Vec::new();
    for dir in raw_dirs {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    if dirs.len() > 32 {
        return Err(AppError::validation("workspaceDirs 最多 32 个目录"));
    }

    for dir in &dirs {
        match tokio::fs::metadata(dir).await {
            Err(_) => {
                return Err(AppError::validation(format!(
                    "工作空间目录不存在：{dir}"
                )))
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(AppError::validation(format!(
                    "工作空间路径不是目录：{dir}"
                )))
            }
            Ok(_) => {}
        }
    }

    let primary = dirs[0].clone();
    Ok((primary, dirs))
}

/// 显式判定存在性与目录类型，错误统一转成 400。
pub(crate) async fn require_workspace_dir(value: Option<&Value>) -> AppResult<String> {
    let dir = require_non_empty_string(value, "workspaceDir")?;
    require_workspace_dir_value(&dir).await
}

/// 已去空的目录路径校验；供需要先 trim 的调用方复用。
pub(crate) async fn require_workspace_dir_value(dir: &str) -> AppResult<String> {
    match tokio::fs::metadata(dir).await {
        Err(_) => Err(AppError::validation(format!(
            "工作空间目录不存在：{dir}"
        ))),
        Ok(metadata) if !metadata.is_dir() => Err(AppError::validation(format!(
            "工作空间路径不是目录：{dir}"
        ))),
        Ok(_) => Ok(dir.to_string()),
    }
}
