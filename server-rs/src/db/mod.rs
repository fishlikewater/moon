//! SQLite 连接与迁移入口。
//!
//! 与历史 TypeScript 后端保持一致：`PRAGMA foreign_keys` 是连接级开关，
//! 必须在每个新连接上显式开启；打开数据库后立即把迁移推进到最新版本。

pub mod migrations;

use std::path::Path;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::SqlitePool;

use crate::error::{AppError, AppResult, ErrorCode};

#[derive(Debug, Clone)]
pub struct Db {
    pool: SqlitePool,
}

impl Db {
    /// 打开（或创建）数据库文件并确保迁移到位。
    pub async fn open(path: impl AsRef<Path>) -> AppResult<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|error| {
                    AppError::with_source(
                        ErrorCode::InternalError,
                        format!("无法创建数据库目录：{}", parent.display()),
                        error,
                    )
                })?;
            }
        }

        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal);
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await?;
        migrations::run_migrations(&pool).await?;
        Ok(Self { pool })
    }

    /// 单个内存连接：测试专用，避免 `:memory:` 在连接池里出现多个独立库。
    pub async fn open_in_memory() -> AppResult<Self> {
        let options = SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        migrations::run_migrations(&pool).await?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn user_version(&self) -> AppResult<i64> {
        Ok(sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&self.pool)
            .await?)
    }

    /// 关闭连接池；SQLite WAL 在最后一个连接关闭时完成落盘。
    pub async fn close(self) {
        self.pool.close().await;
    }
}
