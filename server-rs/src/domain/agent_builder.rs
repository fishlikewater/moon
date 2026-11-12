//! 智能体创建助手契约：对话、草稿与响应。

use serde::Serialize;

use crate::domain::agent::{HarnessKind, ModelConfig};

/// 创建助手对话消息；assistant 内容不包含结构化草稿块。
#[derive(Debug, Clone, PartialEq)]
pub struct AgentBuilderMessage {
    pub role: String,
    pub content: String,
}

/// 创建助手整理出的智能体草稿；空字符串表示该字段尚未确定。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDraft {
    pub name: String,
    pub persona: String,
    pub harness: HarnessKind,
    pub model_config: Option<ModelConfig>,
}

/// 单轮创建助手运行的元数据；不落库，只随响应返回给当前对话展示。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBuilderRunInfo {
    /// `completed`；失败路径不返回响应，因此当前只有完成态。
    pub status: String,
    /// 实际请求的模型；使用 harness 默认模型时为 `None`。
    pub model: Option<String>,
    pub duration_ms: Option<i64>,
    pub token_count: Option<i64>,
    pub tool_call_count: Option<i64>,
    pub context_size: Option<i64>,
}

/// `POST /api/agent-builder/chat` 响应。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBuilderChatResponse {
    /// 已移除 `moon-agent-draft` 代码块的展示文本。
    pub reply: String,
    /// 本轮没有可解析草稿时为 `None`；前端应保留上一版草稿。
    pub draft: Option<AgentDraft>,
    /// 需要用户注意的校验提示，例如重名或模型不在目录中。
    pub notes: Vec<String>,
    /// 本轮运行的模型、耗时与用量。
    pub run: AgentBuilderRunInfo,
}
