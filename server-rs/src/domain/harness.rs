//! harness 模型目录契约与自定义 ACP agent 注册表模型。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::domain::agent::HarnessKind;
use crate::error::{AppError, AppResult};

/// 注册表中的一个自定义 ACP agent：可执行的本地 ACP 程序。
///
/// `command` 是可执行文件路径或 PATH 可解析名称；`args` 原样传给进程，
/// Moon 不做占位符替换（ACP 的 prompt 经 stdin 传递，无需命令行参数）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomHarness {
    pub id: i64,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    /// 环境变量覆盖；与父进程环境合并后传给子进程。
    pub env: BTreeMap<String, String>,
    pub created_at: i64,
    /// 引用该定义的智能体数量；查询时实时统计，不落库。
    pub agent_count: i64,
    /// 最近一次探测到的 ACP 能力快照；未探测为 `None`。
    pub capabilities: Option<AcpCapabilities>,
    /// 最近一次探测时间（epoch 毫秒）；未探测为 `None`。
    pub probed_at: Option<i64>,
}

/// ACP agent 自报的模式与模型能力快照。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AcpCapabilities {
    /// 可用模型；来自 `category="model"` 的配置项或旧式 `models`。
    pub models: Vec<AcpModelOption>,
    pub current_model_id: Option<String>,
    pub modes: Vec<AcpMode>,
    pub current_mode_id: Option<String>,
    /// agent 宣告的全部会话配置项；前端展示与模型映射共同使用。
    pub config_options: Vec<AcpConfigOption>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpMode {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpModelOption {
    pub value: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpConfigOption {
    pub id: String,
    pub name: String,
    pub category: Option<String>,
    /// ACP 的 `type` 字段；`select` 才带 options。
    pub option_type: Option<String>,
    pub current_value: Option<String>,
    pub options: Vec<AcpConfigChoice>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpConfigChoice {
    pub value: String,
    pub name: String,
}

/// `POST /api/custom-harnesses/:id/probe` 响应。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpProbeResponse {
    pub models: Vec<AcpModelOption>,
    pub modes: Vec<AcpMode>,
    pub config_options: Vec<AcpConfigOption>,
    pub current_model_id: Option<String>,
    pub current_mode_id: Option<String>,
    pub probed_at: i64,
}

impl AcpProbeResponse {
    pub fn new(capabilities: AcpCapabilities, probed_at: i64) -> Self {
        Self {
            models: capabilities.models,
            modes: capabilities.modes,
            config_options: capabilities.config_options,
            current_model_id: capabilities.current_model_id,
            current_mode_id: capabilities.current_mode_id,
            probed_at,
        }
    }
}

/// 注册表创建入参。
#[derive(Debug, Clone)]
pub struct CreateCustomHarnessInput {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

/// 注册表局部更新：未传入的字段保持原值。
#[derive(Debug, Clone, Default)]
pub struct UpdateCustomHarnessInput {
    pub name: Option<String>,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub env: Option<BTreeMap<String, String>>,
}

impl CustomHarness {
    /// name 的非空约束；store 与路由共用同一份校验。
    pub fn validate_name(name: &str) -> AppResult<()> {
        if name.trim().is_empty() {
            return Err(AppError::validation("自定义 ACP agent 名称不能为空"));
        }
        Ok(())
    }

    /// command 的非空约束。
    pub fn validate_command(command: &str) -> AppResult<()> {
        if command.trim().is_empty() {
            return Err(AppError::validation("自定义 ACP agent 的启动命令不能为空"));
        }
        Ok(())
    }
}

/// 模型下拉中的一个可选项。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessModelOption {
    /// 前端 option 的稳定值；按 providerId + modelId 生成。
    pub key: String,
    pub provider_id: String,
    pub provider_name: String,
    pub model_id: String,
    /// 该模型可选的思考等级；空数组表示未声明。
    pub reasoning_levels: Vec<String>,
    /// 模型默认思考等级；`None` 表示未声明。
    pub default_reasoning_level: Option<String>,
}

/// 模型目录响应。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessModelCatalog {
    pub harness: HarnessKind,
    /// 是否成功读取到可选项；false 时仍可留空使用 harness 默认模型。
    pub available: bool,
    pub models: Vec<HarnessModelOption>,
    /// 不可用原因；正常为 `None`。
    pub message: Option<String>,
}

/// ACP registry 目录中的一个 agent（已按平台修剪）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpRegistryAgent {
    pub id: String,
    pub name: String,
    pub version: String,
    /// 一句话说明；CDN 缺失时为空串。
    pub summary: String,
    /// npx 分发：包名（含锁定版本）与附加参数。
    pub npx: Option<AcpRegistryNpx>,
    /// windows-x86_64 二进制分发（供「需本机安装」类预设展示）。
    pub windows_binary: Option<AcpRegistryBinary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpRegistryNpx {
    /// npm 包名，可含 `@version` 锁定。
    pub package: String,
    /// 启动 ACP 模式所需的附加参数。
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpRegistryBinary {
    pub cmd: String,
    pub args: Vec<String>,
}

/// registry 目录数据来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RegistrySource {
    /// 本次调用真实拉取 CDN 成功。
    Network,
    /// 命中内存缓存或拉取失败后回退的最近一次成功结果。
    Cache,
    /// 无任何成功结果时回退的内置快照。
    Snapshot,
}

/// `GET /api/acp-registry` 响应。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpRegistryPayload {
    pub agents: Vec<AcpRegistryAgent>,
    pub source: RegistrySource,
    /// 目录数据取得时间（epoch 毫秒）；快照回退为 0。
    pub fetched_at: i64,
}
