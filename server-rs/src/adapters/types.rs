//! harness 适配器统一接口。
//!
//! 适配器只做 I/O 转换：不写库、不发 WS，调用方负责持久化与广播。

use std::pin::Pin;

use futures_core::Stream;

use crate::domain::agent::HarnessKind;
use crate::domain::run_event::{RunEvent, RunInput};
use crate::error::AppResult;

pub type EventStream = Pin<Box<dyn Stream<Item = AppResult<RunEvent>> + Send>>;

pub trait HarnessAdapter: Send + Sync {
    fn kind(&self) -> HarnessKind;
    fn run(&self, input: RunInput) -> EventStream;
}
