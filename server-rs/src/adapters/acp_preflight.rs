//! ACP agent 预检：spawn 之前先确认命令与 Node.js 可解析。
//!
//! 预检与 spawn 同源：裸命令解析复用 `locate::resolve_bare_command`
//! （读注册表 PATH、展开 %VAR%），避免「预检误报、spawn 却能跑」的回归。
//! 测试通过注入解析器覆盖各类环境。

use std::path::PathBuf;

use serde::Serialize;

/// 单条预检结论；文案固定、面向用户，结构化随错误响应下发。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightIssue {
    /// 稳定分类键：commandMissing / nodeMissing。
    pub kind: PreflightIssueKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PreflightIssueKind {
    /// 命令本身在 PATH 或给定路径中找不到。
    CommandMissing,
    /// 命令依赖 Node.js 运行时（npx/npm 系包装器），但 node 不可解析。
    NodeMissing,
}

impl PreflightIssueKind {
    /// 与 serde 序列化一致的分类键，供测试与 shared 类型对齐。
    pub fn as_str(&self) -> &'static str {
        match self {
            PreflightIssueKind::CommandMissing => "commandMissing",
            PreflightIssueKind::NodeMissing => "nodeMissing",
        }
    }
}

/// 预检报告：命令与 Node 的可解析性 + 按优先级排列的问题列表。
#[derive(Debug, Clone, PartialEq)]
pub struct PreflightReport {
    pub command_found: bool,
    pub node_found: bool,
    pub issues: Vec<PreflightIssue>,
}

impl PreflightReport {
    pub fn is_ok(&self) -> bool {
        self.issues.is_empty()
    }

    /// 把问题拼成一段用户可读文案；多条时按换行分隔。
    pub fn message(&self) -> String {
        self.issues
            .iter()
            .map(|issue| issue.message.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// 预检命令：命令是否存在、npx 系命令是否具备 Node.js。
///
/// 裸命令解析与 spawn 同源（`locate::resolve_bare_command`）；
/// 命令带路径分隔符时按显式路径检查文件存在性。
pub fn check_command(command: &str) -> PreflightReport {
    check_command_with(command, &|name| {
        crate::adapters::locate::resolve_bare_command(name)
    })
}

/// 注入式预检：`resolve` 把裸命令名解析为绝对路径（None = 不可解析），
/// 与 spawn 的解析函数保持同一来源即可保证预检不误报。
pub fn check_command_with(
    command: &str,
    resolve: &dyn Fn(&str) -> Option<String>,
) -> PreflightReport {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return PreflightReport {
            command_found: false,
            node_found: false,
            issues: vec![PreflightIssue {
                kind: PreflightIssueKind::CommandMissing,
                message: "命令不能为空".to_string(),
            }],
        };
    }

    let explicit_path = trimmed.contains('\\') || trimmed.contains('/');
    let command_found = if explicit_path {
        PathBuf::from(trimmed).is_file()
    } else {
        resolve(trimmed).is_some()
    };
    let needs_node = needs_node_runtime(trimmed);
    let node_found = !needs_node || resolve("node").is_some();

    let mut issues = Vec::new();
    if !command_found {
        issues.push(PreflightIssue {
            kind: PreflightIssueKind::CommandMissing,
            message: format!(
                "找不到命令「{trimmed}」：请确认已安装并加入 PATH，或编辑该 agent 的命令路径"
            ),
        });
    }
    if !node_found {
        issues.push(PreflightIssue {
            kind: PreflightIssueKind::NodeMissing,
            message: "未检测到 Node.js：该命令需要 Node.js 运行时，请安装后重启 Moon（https://nodejs.org/）"
                .to_string(),
        });
    }

    PreflightReport {
        command_found,
        node_found,
        issues,
    }
}

/// 判断命令是否依赖 Node.js 运行时：npx/npm 系包装器或 .js 脚本。
///
/// bun/deno 是独立运行时，不算 Node 系。
fn needs_node_runtime(command: &str) -> bool {
    let name = std::path::Path::new(command);
    let stem = name
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let extension = name
        .extension()
        .map(|ext| ext.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    const NODE_RUNTIME_COMMANDS: [&str; 4] = ["npx", "npm", "pnpm", "yarn"];
    NODE_RUNTIME_COMMANDS.contains(&stem.as_str()) || extension == "js"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn unique_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "moon-preflight-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    /// 内存解析器：在给定目录中按 .cmd/.exe 扩展名匹配裸命令名。
    fn resolver_with(dir: &Path) -> impl Fn(&str) -> Option<String> {
        let dir = dir.to_path_buf();
        move |name: &str| {
            for ext in [".cmd", ".exe"] {
                let candidate = dir.join(format!("{name}{ext}"));
                if candidate.is_file() {
                    return Some(candidate.to_string_lossy().into_owned());
                }
        }
            None
        }
    }

    #[test]
    fn empty_command_reports_missing() {
        let report = check_command_with("  ", &|_| None);
        assert!(!report.is_ok());
        assert_eq!(report.issues[0].kind.as_str(), "commandMissing");
    }

    #[test]
    fn bare_name_resolves_via_injected_resolver() {
        let dir = unique_dir("resolver");
        fs::write(dir.join("fake-agent.cmd"), "@echo off\r\n").expect("write stub");
        let report =
            check_command_with("fake-agent", &resolver_with(&dir));
        assert!(report.is_ok(), "issues: {:?}", report.issues);
        assert!(report.command_found);
        // 非 Node 系命令不检查 node。
        assert!(!report
            .issues
            .iter()
            .any(|issue| issue.kind == PreflightIssueKind::NodeMissing));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_command_reports_clear_message() {
        let report = check_command_with("moon-not-installed-agent", &|_| None);
        assert!(!report.command_found);
        assert_eq!(report.issues[0].kind.as_str(), "commandMissing");
        assert!(report.issues[0].message.contains("找不到命令"));
    }

    #[test]
    fn npx_without_node_reports_node_missing() {
        let dir = unique_dir("npx-nonode");
        fs::write(dir.join("npx.cmd"), "@echo off\r\n").expect("write stub");
        let report = check_command_with("npx", &resolver_with(&dir));
        assert!(report.command_found);
        assert!(!report.is_ok());
        assert_eq!(report.issues[0].kind.as_str(), "nodeMissing");
        assert!(report.issues[0].message.contains("Node.js"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn npx_with_node_passes() {
        let dir = unique_dir("npx-node");
        fs::write(dir.join("npx.cmd"), "@echo off\r\n").expect("write npx stub");
        fs::write(dir.join("node.exe"), "").expect("write node stub");
        let report = check_command_with("npx", &resolver_with(&dir));
        assert!(report.is_ok(), "issues: {:?}", report.issues);
        assert!(report.node_found);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn js_script_requires_node() {
        let dir = unique_dir("js-script");
        fs::write(dir.join("agent.js"), "console.log(1)\n").expect("write script");
        fs::write(dir.join("node.exe"), "").expect("write node stub");
        let script = dir.join("agent.js");
        let report =
            check_command_with(&script.to_string_lossy(), &resolver_with(&dir));
        assert!(report.command_found);
        assert!(report.node_found);
        assert!(report.is_ok(), "issues: {:?}", report.issues);
        let _ = fs::remove_dir_all(&dir);
    }

    /// bun/deno 是独立运行时：只装 bun 的机器不应误报 NodeMissing。
    #[test]
    fn bun_and_deno_do_not_require_node() {
        let dir = unique_dir("bun-deno");
        fs::write(dir.join("bun.exe"), "").expect("write bun stub");
        fs::write(dir.join("deno.exe"), "").expect("write deno stub");
        let resolve = resolver_with(&dir);
        for name in ["bun", "deno"] {
            let report = check_command_with(name, &resolve);
            assert!(
                report.is_ok(),
                "{name} 不应要求 Node.js：{:?}",
                report.issues
            );
            assert!(report.command_found);
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
