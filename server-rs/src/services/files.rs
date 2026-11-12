//! 房间工作区与成员智能体工作区的只读文件访问。
//!
//! 清单与下载都以「当前房间可访问的根」为白名单：房间工作区在前，
//! 成员智能体工作区在后；相同路径只保留第一次出现，避免重复展示。

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use serde::Serialize;

use crate::error::{AppError, AppResult};
use crate::store::RoomStore;

/// 每个根最多下探的目录层数（根目录本身为第 0 层）。
pub const MAX_LIST_DEPTH: usize = 4;
/// 每个根最多返回的文件数；超出后标记 `truncated`。
pub const MAX_FILES_PER_ROOT: usize = 200;
/// 单文件下载上限。
pub const MAX_DOWNLOAD_BYTES: u64 = 25 * 1024 * 1024;
/// 体积通常很大且与业务无关的目录，不进入清单。
const SKIPPED_DIRS: [&str; 2] = ["node_modules", "target"];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomFileEntry {
    /// 相对根目录的路径，统一使用 `/` 分隔。
    pub path: String,
    pub size: u64,
    /// epoch 毫秒；无法读取时回落为 0。
    pub modified_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomFileRoot {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub agent_id: Option<i64>,
    pub path: String,
    pub files: Vec<RoomFileEntry>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomFiles {
    pub room_id: i64,
    pub roots: Vec<RoomFileRoot>,
}

pub struct DownloadedFile {
    pub file_name: String,
    pub bytes: Vec<u8>,
}

struct RootSpec {
    id: String,
    label: String,
    kind: &'static str,
    agent_id: Option<i64>,
    path: String,
}

pub async fn list_room_files(rooms: &RoomStore, room_id: i64) -> AppResult<RoomFiles> {
    let roots = accessible_roots(rooms, room_id).await?;
    let mut result = Vec::with_capacity(roots.len());
    for root in roots {
        let (files, truncated) = list_root_files(&root.path).await?;
        result.push(RoomFileRoot {
            id: root.id,
            label: root.label,
            kind: root.kind.to_string(),
            agent_id: root.agent_id,
            path: root.path,
            files,
            truncated,
        });
    }
    Ok(RoomFiles {
        room_id,
        roots: result,
    })
}

pub async fn read_room_file(
    rooms: &RoomStore,
    room_id: i64,
    root_id: &str,
    relative_path: &str,
) -> AppResult<DownloadedFile> {
    let roots = accessible_roots(rooms, room_id).await?;
    let Some(root) = roots.iter().find(|root| root.id == root_id) else {
        return Err(AppError::not_found(format!("文件根不存在：{root_id}")));
    };
    let relative = PathBuf::from(relative_path);
    if relative_path.trim().is_empty() || relative.is_absolute() {
        return Err(AppError::validation("文件路径非法"));
    }
    // 只接受普通路径段；`..`、`.`、Windows 盘符前缀与根前缀都在这里被拒绝。
    if relative
        .components()
        .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(AppError::validation("文件路径非法"));
    }

    let root_dir = PathBuf::from(&root.path);
    let canonical_root = tokio::fs::canonicalize(&root_dir)
        .await
        .map_err(|_| AppError::validation("工作区目录不可用"))?;
    let candidate = root_dir.join(&relative);
    let canonical_file = tokio::fs::canonicalize(&candidate)
        .await
        .map_err(|_| AppError::not_found(format!("文件不存在：{relative_path}")))?;
    // 解析符号链接后再比较前缀，阻止通过链接逃逸到根之外。
    if !canonical_file.starts_with(&canonical_root) {
        return Err(AppError::validation("文件路径超出工作区"));
    }
    let metadata = tokio::fs::metadata(&canonical_file)
        .await
        .map_err(|_| AppError::not_found(format!("文件不存在：{relative_path}")))?;
    if !metadata.is_file() {
        return Err(AppError::not_found(format!("文件不存在：{relative_path}")));
    }
    if metadata.len() > MAX_DOWNLOAD_BYTES {
        return Err(AppError::validation("文件超过 25 MiB，暂不支持下载"));
    }
    let bytes = tokio::fs::read(&canonical_file)
        .await
        .map_err(|error| AppError::internal(format!("读取文件失败：{error}")))?;
    let file_name = canonical_file
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "download".to_string());
    Ok(DownloadedFile {
        file_name: sanitize_download_name(&file_name),
        bytes,
    })
}

async fn accessible_roots(rooms: &RoomStore, room_id: i64) -> AppResult<Vec<RootSpec>> {
    let Some(room) = rooms.get_by_id(room_id).await? else {
        return Err(AppError::not_found(format!("房间不存在：{room_id}")));
    };
    let mut seen: HashSet<String> = HashSet::new();
    let mut roots = Vec::new();
    for (index, dir) in room.workspace_dirs.iter().enumerate() {
        if !seen.insert(dir.clone()) || !is_existing_dir(dir).await {
            continue;
        }
        roots.push(RootSpec {
            id: format!("room-{index}"),
            label: "房间工作区".to_string(),
            kind: "room",
            agent_id: None,
            path: dir.clone(),
        });
    }
    for member in rooms.list_members(room_id).await? {
        let Some(dir) = member.workspace_dir else {
            continue;
        };
        if !seen.insert(dir.clone()) || !is_existing_dir(&dir).await {
            continue;
        }
        roots.push(RootSpec {
            id: format!("agent-{}", member.id),
            label: format!("智能体：{}", member.name),
            kind: "agent",
            agent_id: Some(member.id),
            path: dir,
        });
    }
    Ok(roots)
}

async fn is_existing_dir(path: &str) -> bool {
    tokio::fs::metadata(path)
        .await
        .map(|metadata| metadata.is_dir())
        .unwrap_or(false)
}

async fn list_root_files(root: &str) -> AppResult<(Vec<RoomFileEntry>, bool)> {
    let root = PathBuf::from(root);
    let mut files = Vec::new();
    let mut truncated = false;
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.clone(), 0)];
    'walk: while let Some((dir, depth)) = stack.pop() {
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(entries) => entries,
            // 根目录在扫描期间被删除时按空清单处理，不影响其他根。
            Err(_) => continue,
        };
        loop {
            let entry = match entries.next_entry().await {
                Ok(Some(entry)) => entry,
                _ => break,
            };
            if files.len() >= MAX_FILES_PER_ROOT {
                truncated = true;
                break 'walk;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || SKIPPED_DIRS.contains(&name.as_str()) {
                continue;
            }
            let file_type = match entry.file_type().await {
                Ok(file_type) => file_type,
                Err(_) => continue,
            };
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                if depth < MAX_LIST_DEPTH {
                    stack.push((entry.path(), depth + 1));
                }
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let metadata = match entry.metadata().await {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            let relative = entry
                .path()
                .strip_prefix(&root)
                .unwrap_or(&entry.path())
                .to_string_lossy()
                .replace('\\', "/");
            files.push(RoomFileEntry {
                path: relative,
                size: metadata.len(),
                modified_at: modified_ms(&metadata),
            });
        }
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok((files, truncated))
}

fn modified_ms(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

/// 响应头只接受 ASCII 安全字符；非 ASCII 文件名降级为下划线，
/// 前端展示仍使用原始相对路径。
pub fn sanitize_download_name(name: &str) -> String {
    let mut sanitized = String::with_capacity(name.len());
    for character in name.chars() {
        if character.is_ascii_alphanumeric()
            || matches!(character, '.' | '-' | '_' | ' ' | '(' | ')')
        {
            sanitized.push(character);
        } else {
            sanitized.push('_');
        }
    }
    if sanitized.is_empty() {
        "download".to_string()
    } else {
        sanitized
    }
}
