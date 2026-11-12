//! 领域对象与共享契约的 Rust 映射。
//!
//! 字段命名统一使用 camelCase 序列化，与 `shared/src/index.ts` 和前端
//! `web/src/api/client.ts` 的现有契约保持一致；时间统一为 epoch 毫秒。

pub mod agent;
pub mod agent_builder;
pub mod attachment;
pub mod collaboration;
pub mod discussion;
pub mod event;
pub mod harness;
pub mod message;
pub mod room;
pub mod run;
pub mod run_event;
pub mod session;
pub mod settings;
pub mod task;
pub mod timeline;

use std::time::{SystemTime, UNIX_EPOCH};

/// 当前 epoch 毫秒。
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}
