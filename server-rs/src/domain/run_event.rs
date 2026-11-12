//! harness 运行事件与输入。

use std::time::Duration;

use serde_json::Value;

use crate::domain::agent::ModelConfig;
use crate::domain::room::RoomPermissionMode;
use crate::domain::session::RunSessionContext;

/// harness 运行事件流。
///
/// - `TextDelta`：增量文本，调用方按到达顺序追加；
/// - `Reasoning`：模型思考片段，只能进入运行活动流；
/// - `ToolStart` / `ToolEnd`：工具调用开始与结束；
/// - `Usage`：本轮 token、工具次数与耗时；
/// - `Status`：阶段提示；
/// - `Session`：适配器解析到实际 harness 会话 ID，每个运行至多一次；
/// - `Final`：最终完整文本；
/// - `Error`：失败摘要。
#[derive(Debug, Clone)]
pub enum RunEvent {
    TextDelta(String),
    Reasoning(String),
    ToolStart {
        tool_call_id: String,
        name: String,
        input: Value,
    },
    ToolEnd {
        tool_call_id: Option<String>,
        name: Option<String>,
        ok: bool,
        detail: String,
    },
    Usage {
        token_count: Option<i64>,
        context_size: Option<i64>,
        tool_call_count: Option<i64>,
        duration_ms: Option<i64>,
    },
    Status(String),
    Session(String),
    Final(String),
    Error(String),
}

/// 一次 harness 运行的最小输入；适配器只做 I/O 转换。
#[derive(Debug, Clone)]
pub struct RunInput {
    pub prompt: String,
    pub workspace_dir: String,
    /// 主目录之外的附加工作目录；不支持多目录的适配器可以忽略。
    pub additional_directories: Vec<String>,
    pub config: Option<ModelConfig>,
    /// 会话上下文；None 表示本次运行独立，不保留会话。
    pub session: Option<RunSessionContext>,
    /// 房间级权限模式；None 时沿用适配器原有默认。
    pub permission_mode: Option<RoomPermissionMode>,
    /// 本次运行的总时长上限；None 表示不限制。
    pub timeout: Option<Duration>,
}
