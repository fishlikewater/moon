//! harness 可执行文件解析状态：用户覆盖 + 安装器元数据定位。
//!
//! 只读快照，供 `/api/harnesses/binaries` 与设置页展示；不做缓存，
//! 注册表读取成本很低，每次请求重新解析可以立即反映用户刚保存的路径。

use serde::Serialize;

use crate::adapters::{codex, locate, zcode};

/// 单个 harness 的解析结论；`path` 为将要启动的程序或脚本。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HarnessBinaryStatus {
    pub path: String,
    /// `configured` / `protocol` / `uninstall` / `app_paths` / `path` / `vendor_default` / `fallback`。
    pub source: String,
    pub exists: bool,
    /// 尝试过的来源；失败时用于诊断。
    pub searched: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HarnessBinariesResponse {
    pub zcode: HarnessBinaryStatus,
    pub codex: HarnessBinaryStatus,
}

/// 生成本次快照；覆盖路径为空表示自动检测。
pub fn snapshot(
    zcode_override: Option<&str>,
    codex_override: Option<&str>,
) -> HarnessBinariesResponse {
    HarnessBinariesResponse {
        zcode: status(zcode::locate_zcode_program(zcode_override)),
        codex: status(codex::locate_codex_program(codex_override)),
    }
}

fn status(located: locate::LocatedProgram) -> HarnessBinaryStatus {
    let exists = std::path::Path::new(&located.program).is_file();
    HarnessBinaryStatus {
        path: located.program,
        source: located.source.as_str().to_string(),
        exists,
        searched: located.searched,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_override_is_reported_as_configured() {
        let response = snapshot(Some(r"D:\custom\zcode.cjs"), None);
        assert_eq!(response.zcode.source, "configured");
        assert_eq!(response.zcode.path, r"D:\custom\zcode.cjs");
        assert!(!response.zcode.exists);
    }

    #[test]
    fn snapshot_always_reports_both_harnesses() {
        let response = snapshot(None, None);
        assert!(!response.zcode.path.is_empty());
        assert!(!response.codex.path.is_empty());
        assert!(!response.zcode.source.is_empty());
        assert!(!response.codex.source.is_empty());
    }
}
