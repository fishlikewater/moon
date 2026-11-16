//! 桌面端旧数据目录迁移：`AppData\com.moon.workbench` → `~/.moon`。
//!
//! 触发条件：旧目录存在 `moon.db` 且目标目录无 `moon.db`；条件天然幂等
//! （搬移后旧位置不再有主库文件，二次启动即为 no-op）。
//!
//! 搬移范围只有 Moon 自建数据：`moon.db` / `moon.db-wal` / `moon.db-shm`
//! 与 `sessions/`（改名 `harness-sessions`，与独立运行态命名统一）。
//! 旧目录中的 Tauri/webview 运行时文件一律不碰；数据库内容零改写
//! （`agent_sessions.storage_dir` 为相对路径，附件在房间工作区）。
//!
//! 失败语义：rename 失败重试后回退「复制到 .partial → 校验 → 原子改名 →
//! 删源」；任何一步失败都清理残留并返回错误，源保留，下次启动重试自愈。
//! 搬移顺序刻意把主库放在最后——「源主库存在」始终表示迁移未完成，
//! 任何中断后重启都会重新走完整个迁移，不会跳过剩余项。任何失败都由
//! 调用方阻断启动，禁止用空库静默降级。

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::{AppError, AppResult};

/// SQLite 主库与伴生文件；在会话目录之后搬移，主库最后搬（完成标志）。
const DB_FILES: [&str; 3] = ["moon.db-wal", "moon.db-shm", "moon.db"];
/// 旧会话存储目录名 → 新家目录名。
const SESSIONS_DIR: (&str, &str) = ("sessions", "harness-sessions");
/// rename 失败重试参数；与 `adapters::Cleanup` 的清理重试节奏一致。
const RENAME_ATTEMPTS: usize = 5;
const RENAME_RETRY: Duration = Duration::from_millis(50);

/// 迁移结果：是否发生搬移，以及逐项「源 → 目标」明细。
#[derive(Debug, Clone, Default)]
pub struct MigrationReport {
    pub migrated: bool,
    pub items: Vec<(PathBuf, PathBuf)>,
}

impl MigrationReport {
    fn skipped() -> Self {
        Self::default()
    }
}

/// 把旧桌面数据目录整体搬移到新家；不满足触发条件时返回 skipped。
pub fn migrate_legacy_desktop_data(
    legacy_dir: &Path,
    target_dir: &Path,
) -> AppResult<MigrationReport> {
    let legacy_db = legacy_dir.join("moon.db");
    let target_db = target_dir.join("moon.db");
    if !legacy_db.is_file() || target_db.exists() {
        tracing::info!(
            legacy = %legacy_dir.display(),
            target = %target_dir.display(),
            "data dir migration skipped"
        );
        return Ok(MigrationReport::skipped());
    }
    if let Err(error) = std::fs::create_dir_all(target_dir) {
        return Err(AppError::with_source(
            crate::error::ErrorCode::InternalError,
            format!(
                "无法创建新数据目录 {}，迁移中止；请手动把旧目录 {} 中的 moon.db 与 sessions 移动过去",
                target_dir.display(),
                legacy_dir.display()
            ),
            error,
        ));
    }

    let mut report = MigrationReport {
        migrated: true,
        items: Vec::new(),
    };
    // 会话目录先搬、主库最后搬：中断后「源主库存在」恒等于「迁移未完成」，
    // 重启会重走迁移，已搬走的项由源消失容错自然跳过。
    let legacy_sessions = legacy_dir.join(SESSIONS_DIR.0);
    if legacy_sessions.is_dir() {
        let target_sessions = target_dir.join(SESSIONS_DIR.1);
        move_item(&legacy_sessions, &target_sessions)?;
        report.items.push((legacy_sessions, target_sessions));
    }
    for name in DB_FILES {
        let source = legacy_dir.join(name);
        if !source.exists() {
            continue;
        }
        let target = target_dir.join(name);
        move_item(&source, &target)?;
        report.items.push((source, target));
    }

    tracing::info!(
        legacy = %legacy_dir.display(),
        target = %target_dir.display(),
        items = report.items.len(),
        "legacy data dir migrated"
    );
    Ok(report)
}

/// 搬移单个文件/目录：rename 优先（同卷原子），重试后回退复制。
///
/// 重试期间源消失（如旧版进程刚退出、SQLite 延迟清理掉 wal/shm）
/// 视为无事可做，不算失败。
fn move_item(source: &Path, target: &Path) -> AppResult<()> {
    let mut last_error: Option<std::io::Error> = None;
    for attempt in 0..RENAME_ATTEMPTS {
        match std::fs::rename(source, target) {
            Ok(()) => return Ok(()),
            Err(error) => {
                if !source.exists() {
                    tracing::info!(
                        source = %source.display(),
                        "migration item vanished during retry; nothing to move"
                    );
                    return Ok(());
                }
                last_error = Some(error);
                if attempt + 1 < RENAME_ATTEMPTS {
                    std::thread::sleep(RENAME_RETRY);
                }
            }
        }
    }
    if let Some(error) = last_error {
        tracing::warn!(
            source = %source.display(),
            target = %target.display(),
            error = %error,
            "rename failed, falling back to copy"
        );
    }
    copy_and_remove(source, target)
}

/// 复制回退：目标完整复制成功后尽力删源（删源失败只降级为冗余副本，
/// 幂等条件保证不会重复迁移）；复制失败则清理目标残留并报错，源保留
/// 供下次启动重试。
fn copy_and_remove(source: &Path, target: &Path) -> AppResult<()> {
    if !source.exists() {
        return Ok(());
    }
    // 复制先落 .partial、校验字节数一致后再原子改名：中途被杀只会留下
    // .partial 残留（下次启动清理后重试），最终名永远是完整副本。
    let partial = partial_path(target);
    clear_partial(&partial);
    let copied_bytes = if source.is_dir() {
        copy_dir_recursive(source, &partial)
    } else {
        std::fs::copy(source, &partial)
    };
    let copied_bytes = match copied_bytes {
        Ok(bytes) => bytes,
        Err(error) => {
            clear_partial(&partial);
            return Err(migration_failed(source, target, error));
        }
    };
    if let Some(expected) = source_bytes(source) {
        if copied_bytes != expected {
            clear_partial(&partial);
            return Err(migration_failed(
                source,
                target,
                format!(
                    "复制不完整：预期 {expected} 字节，实际 {copied_bytes} 字节"
                ),
            ));
        }
    }
    if let Err(error) = std::fs::rename(&partial, target) {
        clear_partial(&partial);
        return Err(migration_failed(source, target, error));
    }
    let removed = if source.is_dir() {
        std::fs::remove_dir_all(source)
    } else {
        std::fs::remove_file(source)
    };
    if let Err(error) = removed {
        tracing::warn!(
            source = %source.display(),
            error = %error,
            "legacy copy removed failed; a redundant copy remains at the old location"
        );
    }
    Ok(())
}

/// 目标路径对应的临时副本路径：`<父目录>/<文件名>.partial`。
fn partial_path(target: &Path) -> PathBuf {
    let mut name = target
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    name.push(".partial");
    target.with_file_name(name)
}

/// 清理上次中断遗留的临时副本；失败仅记录（下次启动会再试）。
fn clear_partial(partial: &Path) {
    let result = if partial.is_dir() {
        std::fs::remove_dir_all(partial)
    } else {
        std::fs::remove_file(partial)
    };
    if let Err(error) = result {
        if error.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(
                partial = %partial.display(),
                error = %error,
                "stale migration partial cleanup failed"
            );
        }
    }
}

/// 统计源文件/目录树的总字节数；不可读时返回 None（跳过校验）。
fn source_bytes(source: &Path) -> Option<u64> {
    let metadata = std::fs::metadata(source).ok()?;
    if !metadata.is_dir() {
        return Some(metadata.len());
    }
    dir_bytes(source)
}

fn dir_bytes(dir: &Path) -> Option<u64> {
    let mut total = 0_u64;
    for entry in std::fs::read_dir(dir).ok()? {
        let entry = entry.ok()?;
        if entry.file_type().ok()?.is_dir() {
            total += dir_bytes(&entry.path())?;
        } else {
            total += entry.metadata().ok()?.len();
        }
    }
    Some(total)
}

fn migration_failed(
    source: &Path,
    target: &Path,
    cause: impl std::fmt::Display,
) -> AppError {
    AppError::with_source(
        crate::error::ErrorCode::InternalError,
        format!(
            "数据迁移失败（{} → {}）；旧数据完好保留，请手动搬移 moon.db 与 sessions 后重试",
            source.display(),
            target.display()
        ),
        std::io::Error::new(std::io::ErrorKind::Other, cause.to_string()),
    )
}

/// 递归复制目录树，返回复制总字节数。
fn copy_dir_recursive(source: &Path, target: &Path) -> std::io::Result<u64> {
    std::fs::create_dir_all(target)?;
    let mut total = 0_u64;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let entry_target = target.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            total += copy_dir_recursive(&entry.path(), &entry_target)?;
        } else {
            total += std::fs::copy(entry.path(), &entry_target)?;
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_file(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().expect("parent dir")).unwrap();
        std::fs::write(path, content).unwrap();
    }

    /// 布一个完整旧版布局：db 三件套 + sessions/room-1/db.sqlite。
    fn seed_legacy(dir: &Path) {
        write_file(&dir.join("moon.db"), "legacy-db");
        write_file(&dir.join("moon.db-wal"), "legacy-wal");
        write_file(&dir.join("moon.db-shm"), "legacy-shm");
        write_file(&dir.join("sessions").join("room-1").join("db.sqlite"), "session-db");
        write_file(&dir.join("webview-cache.bin"), "must-not-move");
    }

    #[test]
    fn migrates_when_legacy_exists_and_target_missing() {
        let legacy = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let legacy_dir = legacy.path().join("com.moon.workbench");
        let target_dir = target.path().join(".moon");
        seed_legacy(&legacy_dir);

        let report = migrate_legacy_desktop_data(&legacy_dir, &target_dir).unwrap();

        assert!(report.migrated);
        assert_eq!(report.items.len(), 4);
        let session_db = target_dir
            .join("harness-sessions")
            .join("room-1")
            .join("db.sqlite");
        assert_eq!(std::fs::read_to_string(&session_db).unwrap(), "session-db");
        assert_eq!(
            std::fs::read_to_string(target_dir.join("moon.db")).unwrap(),
            "legacy-db"
        );
        assert!(!legacy_dir.join("moon.db").exists());
        assert!(!legacy_dir.join("sessions").exists());
        // webview 运行时文件不属于搬移范围。
        assert!(legacy_dir.join("webview-cache.bin").exists());
    }

    #[test]
    fn skips_when_target_exists() {
        let legacy = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let legacy_dir = legacy.path().join("com.moon.workbench");
        let target_dir = target.path().join(".moon");
        seed_legacy(&legacy_dir);
        write_file(&target_dir.join("moon.db"), "already-new");

        let report = migrate_legacy_desktop_data(&legacy_dir, &target_dir).unwrap();

        assert!(!report.migrated);
        assert!(report.items.is_empty());
        // 目标内容不被覆盖。
        assert_eq!(
            std::fs::read_to_string(target_dir.join("moon.db")).unwrap(),
            "already-new"
        );
        assert!(legacy_dir.join("moon.db").exists());
    }

    #[test]
    fn skips_when_legacy_missing() {
        let legacy = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let report = migrate_legacy_desktop_data(
            &legacy.path().join("com.moon.workbench"),
            &target.path().join(".moon"),
        )
        .unwrap();

        assert!(!report.migrated);
        assert!(!target.path().join(".moon").join("moon.db").exists());
    }

    #[cfg(windows)]
    #[test]
    fn copy_fallback_completes_when_rename_blocked() {
        use std::os::windows::fs::OpenOptionsExt;

        let legacy = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let legacy_dir = legacy.path().join("com.moon.workbench");
        let target_dir = target.path().join(".moon");
        seed_legacy(&legacy_dir);

        // 以无 DELETE 共享位持有源主库句柄，逼 rename 失败、走复制回退。
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(legacy_dir.join("moon.db"))
            .unwrap();

        let report = migrate_legacy_desktop_data(&legacy_dir, &target_dir).unwrap();

        assert!(report.migrated);
        assert_eq!(
            std::fs::read_to_string(target_dir.join("moon.db")).unwrap(),
            "legacy-db"
        );
        assert!(target_dir.join("harness-sessions").join("room-1").join("db.sqlite").exists());
        // 源主库被句柄锁定无法删除：迁移仍成功（目标完整），旧副本冗余保留。
        assert!(legacy_dir.join("moon.db").exists());
        drop(lock);
    }

    /// 目标会话目录已存在且非空：rename（Windows 语义为 REPLACE_EXISTING，
    /// 不接受非空目录）与复制改名都失败，迁移必须报错且源数据原封不动
    /// （失败阻断语义，AC-003；也证明不会静默覆盖已有目标内容）。
    #[test]
    fn fails_and_keeps_source_when_target_blocked() {
        let legacy = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let legacy_dir = legacy.path().join("com.moon.workbench");
        let target_dir = target.path().join(".moon");
        seed_legacy(&legacy_dir);
        // 目标 harness-sessions 是非空目录：rename 与 partial 改名都进不去。
        write_file(&target_dir.join("harness-sessions").join("occupied"), "x");

        let error = migrate_legacy_desktop_data(&legacy_dir, &target_dir).unwrap_err();

        assert!(error.message.contains("数据迁移失败"));
        assert!(error.message.contains(&legacy_dir.display().to_string()));
        // 主库与会话目录都还在源处，等待用户处理后重试。
        assert!(legacy_dir.join("moon.db").exists());
        assert!(legacy_dir.join("sessions").join("room-1").join("db.sqlite").exists());
        assert!(!target_dir.join("moon.db").exists());
        // 被占用目标的内容不被覆盖。
        assert_eq!(
            std::fs::read_to_string(target_dir.join("harness-sessions").join("occupied"))
                .unwrap(),
            "x"
        );
    }

    /// 模拟「上次中断」：会话目录已到位、主库仍在源处。重启迁移必须
    /// 只补搬主库并保留已迁内容（中断自愈，AC-003）。
    #[test]
    fn resumes_when_sessions_already_migrated() {
        let legacy = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let legacy_dir = legacy.path().join("com.moon.workbench");
        let target_dir = target.path().join(".moon");
        write_file(&legacy_dir.join("moon.db"), "legacy-db");
        write_file(&legacy_dir.join("moon.db-wal"), "legacy-wal");
        write_file(
            &target_dir
                .join("harness-sessions")
                .join("room-1")
                .join("db.sqlite"),
            "already-here",
        );

        let report = migrate_legacy_desktop_data(&legacy_dir, &target_dir).unwrap();

        assert!(report.migrated);
        assert_eq!(
            std::fs::read_to_string(target_dir.join("moon.db")).unwrap(),
            "legacy-db"
        );
        // 已迁内容不被触碰。
        assert_eq!(
            std::fs::read_to_string(
                target_dir
                    .join("harness-sessions")
                    .join("room-1")
                    .join("db.sqlite")
            )
            .unwrap(),
            "already-here"
        );
        assert!(!legacy_dir.join("moon.db").exists());
    }
}
