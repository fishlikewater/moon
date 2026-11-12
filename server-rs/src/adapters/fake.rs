//! 测试用适配器：按注入的脚本逐条产出事件。

use std::sync::Arc;

use crate::domain::agent::HarnessKind;
use crate::domain::run_event::{RunEvent, RunInput};

use super::types::{EventStream, HarnessAdapter};

/// 固定事件数组，或每次运行生成的工厂（便于每次运行产出独立结果）。
pub type FakeScript = Arc<dyn Fn() -> Vec<RunEvent> + Send + Sync>;

pub struct FakeHarnessAdapter {
    script: FakeScript,
}

impl FakeHarnessAdapter {
    pub fn new(events: Vec<RunEvent>) -> Self {
        Self {
            script: Arc::new(move || events.clone()),
        }
    }

    pub fn from_factory(script: FakeScript) -> Self {
        Self { script }
    }

    pub fn empty() -> Self {
        Self::new(Vec::new())
    }
}

impl HarnessAdapter for FakeHarnessAdapter {
    fn kind(&self) -> HarnessKind {
        HarnessKind::Fake
    }

    fn run(&self, _input: RunInput) -> EventStream {
        let events = (self.script)();
        Box::pin(tokio_stream::iter(events.into_iter().map(Ok::<_, crate::error::AppError>)))
    }
}

/// 供测试快速构造脚本事件。
pub fn text(text: impl Into<String>) -> RunEvent {
    RunEvent::TextDelta(text.into())
}

pub fn final_text(text: impl Into<String>) -> RunEvent {
    RunEvent::Final(text.into())
}
