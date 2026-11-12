//! 可执行文件定位：优先读用户配置与安装器写入的系统元数据。
//!
//! 不做目录猜测（不枚举 Program Files 或各盘符），来源按可信度排序：
//! 显式配置 → URL 协议处理器 → 卸载注册表 → App Paths → 注册表 PATH → 厂商默认。
//! 每次结果都带来源与「已尝试来源」，便于把 `program not found` 变成可诊断信息。

use std::path::{Path, PathBuf};

/// 定位来源；写入日志与 `/api/harnesses/binaries` 响应。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocateSource {
    Configured,
    Protocol,
    Uninstall,
    AppPaths,
    Path,
    VendorDefault,
    Fallback,
}

impl LocateSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Configured => "configured",
            Self::Protocol => "protocol",
            Self::Uninstall => "uninstall",
            Self::AppPaths => "app_paths",
            Self::Path => "path",
            Self::VendorDefault => "vendor_default",
            Self::Fallback => "fallback",
        }
    }
}

/// 一次定位的结论；`program` 为可直接启动的命令或脚本路径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocatedProgram {
    pub program: String,
    pub source: LocateSource,
    /// 失败或降级时记录尝试过的来源，供错误信息与界面展示。
    pub searched: Vec<String>,
}

/// 定位请求；`directory_candidates` 是「在某来源给出的安装目录下按序尝试的相对路径」。
#[derive(Debug, Clone, Default)]
pub struct LocateRequest<'a> {
    pub explicit: Option<&'a str>,
    /// URL 协议名，读取 `HKCU\Software\Classes\<name>\shell\open\command`。
    pub protocol: Option<&'a str>,
    /// 卸载项 DisplayName 前缀（如 `ZCode`）。
    pub uninstall_prefix: Option<&'a str>,
    /// App Paths 注册的可执行文件名（如 `ZCode.exe`）。
    pub app_paths_exe: Option<&'a str>,
    /// 相对安装目录的候选路径；为空时直接使用来源给出的可执行文件。
    pub directory_candidates: &'a [&'a str],
    /// PATH 中查找的命令名（不含扩展名）。
    pub path_names: &'a [&'a str],
    /// 厂商文档固定的兜底文件（完整路径）。
    pub vendor_defaults: &'a [PathBuf],
}

/// 注册表只读访问；真实实现读 Windows 注册表，测试用夹具注入。
pub trait RegistryReader {
    fn read_string(&self, hive: Hive, path: &str, name: &str) -> Option<String>;
    fn subkeys(&self, hive: Hive, path: &str) -> Vec<String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Hive {
    CurrentUser,
    LocalMachine,
}

/// 入口：用当前进程所在平台的注册表读取实现。
pub fn locate_program(request: &LocateRequest<'_>) -> LocatedProgram {
    locate_with(&PlatformRegistry, request)
}

/// 把一个裸命令名解析为绝对路径（用于 ACP 自定义 agent 的 command）。
///
/// 命令本身带路径分隔符时原样返回（None），由调用方直接启动。
pub fn resolve_bare_command(command: &str) -> Option<String> {
    let trimmed = command.trim();
    if trimmed.is_empty() || trimmed.contains('/') || trimmed.contains('\\') {
        return None;
    }
    resolve_in_path(&PlatformRegistry, trimmed)
}

fn locate_with(registry: &dyn RegistryReader, request: &LocateRequest<'_>) -> LocatedProgram {
    let mut searched = Vec::new();

    if let Some(program) = normalize(request.explicit) {
        return LocatedProgram {
            program,
            source: LocateSource::Configured,
            searched,
        };
    }

    if let Some(name) = request.protocol {
        searched.push(format!("protocol:{name}"));
        let key = format!("Software\\Classes\\{name}\\shell\\open\\command");
        if let Some(command) = registry.read_string(Hive::CurrentUser, &key, "") {
            if let Some(found) = locate_from_command(registry, &command, request) {
                return LocatedProgram {
                    program: found,
                    source: LocateSource::Protocol,
                    searched,
                };
            }
        }
    }

    if let Some(prefix) = request.uninstall_prefix {
        searched.push(format!("uninstall:{prefix}"));
        for hive in [Hive::CurrentUser, Hive::LocalMachine] {
            for root in [
                "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
                "Software\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
            ] {
                for subkey in registry.subkeys(hive, root) {
                    let path = format!("{root}\\{subkey}");
                    let Some(display_name) = registry.read_string(hive, &path, "DisplayName") else {
                        continue;
                    };
                    if !display_name
                        .trim()
                        .to_lowercase()
                        .starts_with(&prefix.to_lowercase())
                    {
                        continue;
                    }
                    for dir in uninstall_directories(registry, hive, &path) {
                        if let Some(found) = first_candidate(&dir, request.directory_candidates, None)
                        {
                            return LocatedProgram {
                                program: found,
                                source: LocateSource::Uninstall,
                                searched,
                            };
                        }
                    }
                }
            }
        }
    }

    if let Some(exe) = request.app_paths_exe {
        searched.push(format!("app_paths:{exe}"));
        let key = format!("Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\{exe}");
        for hive in [Hive::CurrentUser, Hive::LocalMachine] {
            if let Some(command) = registry.read_string(hive, &key, "") {
                if let Some(found) = locate_from_command(registry, &command, request) {
                    return LocatedProgram {
                        program: found,
                        source: LocateSource::AppPaths,
                        searched,
                    };
                }
            }
        }
    }

    for name in request.path_names {
        searched.push(format!("path:{name}"));
        if let Some(found) = resolve_in_path(registry, name) {
            return LocatedProgram {
                program: found,
                source: LocateSource::Path,
                searched,
            };
        }
    }

    for path in request.vendor_defaults {
        searched.push(format!("vendor_default:{}", path.display()));
        if path.is_file() {
            return LocatedProgram {
                program: path.to_string_lossy().into_owned(),
                source: LocateSource::VendorDefault,
                searched,
            };
        }
    }

    LocatedProgram {
        program: request.path_names.first().copied().unwrap_or("").to_string(),
        source: LocateSource::Fallback,
        searched,
    }
}

/// 协议/App Paths 的命令行 → 安装目录 → 候选文件。
fn locate_from_command(
    _registry: &dyn RegistryReader,
    command: &str,
    request: &LocateRequest<'_>,
) -> Option<String> {
    let program = parse_command_program(command)?;
    let direct = Some(program.as_str());
    let dir = Path::new(&program).parent()?;
    first_candidate(dir, request.directory_candidates, direct)
}

/// 卸载项的三个可推导目录来源：InstallLocation、DisplayIcon、UninstallString。
fn uninstall_directories(
    registry: &dyn RegistryReader,
    hive: Hive,
    path: &str,
) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(location) = registry
        .read_string(hive, path, "InstallLocation")
        .and_then(|value| non_empty(&value))
    {
        dirs.push(PathBuf::from(location));
    }
    if let Some(icon) = registry
        .read_string(hive, path, "DisplayIcon")
        .and_then(|value| parse_display_icon(&value))
    {
        if let Some(parent) = icon.parent() {
            dirs.push(parent.to_path_buf());
        }
    }
    if let Some(uninstall) = registry
        .read_string(hive, path, "UninstallString")
        .and_then(|value| parse_uninstall_dir(&value))
    {
        dirs.push(uninstall);
    }
    dirs
}

fn first_candidate(dir: &Path, candidates: &[&str], direct: Option<&str>) -> Option<String> {
    if candidates.is_empty() {
        return direct
            .filter(|path| Path::new(path).is_file())
            .map(str::to_string);
    }
    candidates.iter().find_map(|relative| {
        let candidate = join_relative(dir, relative);
        candidate.is_file().then(|| candidate.to_string_lossy().into_owned())
    })
}

/// 按原生分隔符拼接相对路径；避免 `/` 与 `\` 混用出现在界面与错误信息里。
fn join_relative(dir: &Path, relative: &str) -> PathBuf {
    relative
        .split(['/', '\\'])
        .filter(|segment| !segment.is_empty())
        .fold(dir.to_path_buf(), |path, segment| path.join(segment))
}

/// 在注册表 PATH（回退进程 PATH）中查找命令，按 PATHEXT 补全扩展名。
fn resolve_in_path(registry: &dyn RegistryReader, name: &str) -> Option<String> {
    let extensions = path_extensions(registry);
    for dir in path_directories(registry) {
        if let Some(found) = match_path_entry(&dir, name, &extensions) {
            return Some(found.to_string_lossy().into_owned());
        }
    }
    None
}

/// 在单个目录内查找命令：先精确名，再按 PATHEXT 顺序补全。
pub fn match_path_entry(dir: &Path, name: &str, extensions: &[String]) -> Option<PathBuf> {
    let direct = dir.join(name);
    if direct.is_file() {
        return Some(direct);
    }
    for extension in extensions {
        let candidate = dir.join(format!("{name}{extension}"));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// 按平台切分 PATH：Windows 用 `;`，其他平台按系统路径分隔符（进程 PATH）。
#[cfg(windows)]
fn split_path_value(raw: &str) -> Vec<String> {
    raw.split(';').map(str::to_string).collect()
}

#[cfg(not(windows))]
fn split_path_value(raw: &str) -> Vec<String> {
    std::env::split_paths(raw)
        .map(|path| path.to_string_lossy().into_owned())
        .collect()
}

fn path_directories(registry: &dyn RegistryReader) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut push_raw = |raw: Option<String>| {
        let Some(raw) = raw else {
            return;
        };
        for part in split_path_value(&raw) {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let expanded = expand_env_with(part, |key| registry_env_var(registry, key));
            if expanded.is_empty() {
                continue;
            }
            dirs.push(PathBuf::from(expanded));
        }
    };
    push_raw(registry.read_string(Hive::CurrentUser, "Environment", "Path"));
    push_raw(registry.read_string(
        Hive::LocalMachine,
        "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
        "Path",
    ));
    push_raw(std::env::var("PATH").ok());

    let mut seen: Vec<String> = Vec::new();
    dirs.retain(|dir| {
        let key = dir.to_string_lossy().to_lowercase();
        if seen.contains(&key) {
            false
        } else {
            seen.push(key);
            true
        }
    });
    dirs
}

fn path_extensions(registry: &dyn RegistryReader) -> Vec<String> {
    let raw = registry
        .read_string(
            Hive::LocalMachine,
            "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
            "PATHEXT",
        )
        .or_else(|| std::env::var("PATHEXT").ok());
    let Some(raw) = raw else {
        return Vec::new();
    };
    raw.split(';')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

/// 读取环境变量：进程环境优先，其次用户与系统注册表环境。
fn registry_env_var(registry: &dyn RegistryReader, key: &str) -> Option<String> {
    if let Ok(value) = std::env::var(key) {
        if !value.trim().is_empty() {
            return Some(value);
        }
    }
    registry
        .read_string(Hive::CurrentUser, "Environment", key)
        .or_else(|| {
            registry.read_string(
                Hive::LocalMachine,
                "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
                key,
            )
        })
        .filter(|value| !value.trim().is_empty())
}

/// 解析命令行里的程序路径：支持引号包裹与 `%VAR%` 展开。
pub fn parse_command_program(command: &str) -> Option<String> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return None;
    }
    let program = if let Some(rest) = trimmed.strip_prefix('"') {
        rest.split('"').next()?
    } else {
        trimmed.split_whitespace().next()?
    };
    let expanded = expand_env_with(program, |key| std::env::var(key).ok());
    non_empty(&expanded)
}

/// `DisplayIcon` 可能是 `path.exe,0` 形式，去掉尾部图标索引。
///
/// 该值本身是路径而非命令行，未加引号时也可能含空格，因此不按空白切分。
pub fn parse_display_icon(value: &str) -> Option<PathBuf> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let unquoted = match trimmed.strip_prefix('"') {
        Some(rest) => rest.split('"').next()?.to_string(),
        None => trimmed.to_string(),
    };
    let cleaned = match unquoted.rfind(',') {
        Some(index)
            if !unquoted[index + 1..].trim().is_empty()
                && unquoted[index + 1..]
                    .trim()
                    .chars()
                    .all(|ch| ch.is_ascii_digit()) =>
        {
            unquoted[..index].to_string()
        }
        _ => unquoted,
    };
    non_empty(&cleaned).map(PathBuf::from)
}

/// 从卸载命令行推导安装目录（程序文件所在目录）。
pub fn parse_uninstall_dir(command: &str) -> Option<PathBuf> {
    let program = parse_command_program(command)?;
    Path::new(&program).parent().map(Path::to_path_buf)
}

/// 展开 `%VAR%`；未知变量保持原样，避免吞掉合法路径片段。
pub fn expand_env_with(input: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find('%') {
        output.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('%') else {
            output.push_str(&rest[start..]);
            return output;
        };
        let name = &after[..end];
        match lookup(name) {
            Some(value) if !name.is_empty() => output.push_str(&value),
            _ => {
                output.push('%');
                output.push_str(name);
                output.push('%');
            }
        }
        rest = &after[end + 1..];
    }
    output.push_str(rest);
    output
}

fn normalize(value: Option<&str>) -> Option<String> {
    value.and_then(non_empty)
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_matches('"').trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[cfg(windows)]
struct PlatformRegistry;

#[cfg(windows)]
impl RegistryReader for PlatformRegistry {
    fn read_string(&self, hive: Hive, path: &str, name: &str) -> Option<String> {
        use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
        use winreg::RegKey;

        let root = match hive {
            Hive::CurrentUser => RegKey::predef(HKEY_CURRENT_USER),
            Hive::LocalMachine => RegKey::predef(HKEY_LOCAL_MACHINE),
        };
        let key = root.open_subkey(path).ok()?;
        key.get_value::<String, _>(name).ok()
    }

    fn subkeys(&self, hive: Hive, path: &str) -> Vec<String> {
        use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
        use winreg::RegKey;

        let root = match hive {
            Hive::CurrentUser => RegKey::predef(HKEY_CURRENT_USER),
            Hive::LocalMachine => RegKey::predef(HKEY_LOCAL_MACHINE),
        };
        let Ok(key) = root.open_subkey(path) else {
            return Vec::new();
        };
        key.enum_keys().filter_map(Result::ok).collect()
    }
}

#[cfg(not(windows))]
struct PlatformRegistry;

#[cfg(not(windows))]
impl RegistryReader for PlatformRegistry {
    fn read_string(&self, _hive: Hive, _path: &str, _name: &str) -> Option<String> {
        None
    }

    fn subkeys(&self, _hive: Hive, _path: &str) -> Vec<String> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct FakeRegistry {
        values: HashMap<(Hive, String, String), String>,
        keys: HashMap<(Hive, String), Vec<String>>,
    }

    impl FakeRegistry {
        fn set(&mut self, hive: Hive, path: &str, name: &str, value: &str) {
            self.values
                .insert((hive, path.to_string(), name.to_string()), value.to_string());
        }

        fn keys(&mut self, hive: Hive, path: &str, names: &[&str]) {
            self.keys.insert(
                (hive, path.to_string()),
                names.iter().map(|name| name.to_string()).collect(),
            );
        }
    }

    impl RegistryReader for FakeRegistry {
        fn read_string(&self, hive: Hive, path: &str, name: &str) -> Option<String> {
            self.values
                .get(&(hive, path.to_string(), name.to_string()))
                .cloned()
        }

        fn subkeys(&self, hive: Hive, path: &str) -> Vec<String> {
            self.keys
                .get(&(hive, path.to_string()))
                .cloned()
                .unwrap_or_default()
        }
    }

    fn temp_dir_with(files: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for file in files {
            let path = dir.path().join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"stub").unwrap();
        }
        dir
    }

    #[test]
    fn parse_command_program_handles_quotes_and_env() {
        std::env::set_var("MOON_LOCATE_TEST_ROOT", r"D:\Apps");
        assert_eq!(
            parse_command_program(r#""%MOON_LOCATE_TEST_ROOT%\ZCode\ZCode.exe" -- "%1""#),
            Some(r"D:\Apps\ZCode\ZCode.exe".to_string())
        );
        assert_eq!(
            parse_command_program(r"C:\Program Files\App\app.exe --open"),
            Some("C:\\Program".to_string())
        );
        assert_eq!(parse_command_program("   "), None);
        std::env::remove_var("MOON_LOCATE_TEST_ROOT");
    }

    #[test]
    fn parse_uninstall_and_icon_extract_directories() {
        assert_eq!(
            parse_uninstall_dir(r#""D:\Program Files\ZCode\Uninstall ZCode.exe" /currentuser"#),
            Some(PathBuf::from(r"D:\Program Files\ZCode"))
        );
        assert_eq!(
            parse_display_icon(r"D:\Program Files\ZCode\uninstallerIcon.ico,0"),
            Some(PathBuf::from(r"D:\Program Files\ZCode\uninstallerIcon.ico"))
        );
        assert_eq!(
            parse_display_icon(r#""D:\Apps\icon.ico", 3"#),
            Some(PathBuf::from(r"D:\Apps\icon.ico"))
        );
    }

    #[test]
    fn expand_env_keeps_unknown_placeholders() {
        let expanded = expand_env_with("%ROOT%\\bin;%MISSING%", |name| {
            (name == "ROOT").then(|| "D:\\Apps".to_string())
        });
        assert_eq!(expanded, "D:\\Apps\\bin;%MISSING%");
    }

    #[test]
    fn match_path_entry_prefers_exact_then_pathext_order() {
        let dir = temp_dir_with(&["tool", "tool.EXE", "tool.cmd"]);
        let extensions = vec![".EXE".to_string(), ".CMD".to_string()];
        let found = match_path_entry(dir.path(), "tool", &extensions).unwrap();
        assert_eq!(found.file_name().unwrap().to_string_lossy(), "tool");

        let only_ext = temp_dir_with(&["tool.cmd"]);
        let found = match_path_entry(only_ext.path(), "tool", &extensions).unwrap();
        assert_eq!(
            found.file_name().unwrap().to_string_lossy().to_lowercase(),
            "tool.cmd"
        );
    }

    #[test]
    fn configured_wins_over_protocol() {
        let dir = temp_dir_with(&["ZCode/resources/glm/zcode.cjs"]);
        let exe = dir.path().join("ZCode/ZCode.exe");
        std::fs::write(&exe, b"stub").unwrap();

        let mut registry = FakeRegistry::default();
        registry.set(
            Hive::CurrentUser,
            "Software\\Classes\\zcode\\shell\\open\\command",
            "",
            &format!("\"{}\" -- \"%1\"", exe.display()),
        );

        let explicit_path = dir.path().join("custom/zcode.cjs").to_string_lossy().into_owned();
        let request = LocateRequest {
            explicit: Some(&explicit_path),
            protocol: Some("zcode"),
            directory_candidates: &["resources/glm/zcode.cjs"],
            path_names: &["zcode"],
            ..Default::default()
        };
        let located = locate_with(&registry, &request);
        assert_eq!(located.source, LocateSource::Configured);
    }

    #[test]
    fn protocol_command_resolves_relative_candidate() {
        let dir = temp_dir_with(&["ZCode/resources/glm/zcode.cjs"]);
        let exe = dir.path().join("ZCode/ZCode.exe");
        std::fs::write(&exe, b"stub").unwrap();

        let mut registry = FakeRegistry::default();
        registry.set(
            Hive::CurrentUser,
            "Software\\Classes\\zcode\\shell\\open\\command",
            "",
            &format!("\"{}\" -- \"%1\"", exe.display()),
        );

        let request = LocateRequest {
            protocol: Some("zcode"),
            directory_candidates: &["resources/glm/zcode.cjs"],
            path_names: &["zcode"],
            ..Default::default()
        };
        let located = locate_with(&registry, &request);
        assert_eq!(located.source, LocateSource::Protocol);
        assert!(located.program.ends_with("zcode.cjs"));
    }

    #[test]
    fn app_paths_hit_reports_source_and_searched() {
        let dir = temp_dir_with(&["ZCode/resources/glm/zcode.cjs"]);
        let exe = dir.path().join("ZCode/ZCode.exe");
        std::fs::write(&exe, b"stub").unwrap();

        let mut registry = FakeRegistry::default();
        registry.set(
            Hive::LocalMachine,
            "Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\ZCode.exe",
            "",
            &format!("\"{}\"", exe.display()),
        );

        let request = LocateRequest {
            app_paths_exe: Some("ZCode.exe"),
            directory_candidates: &["resources/glm/zcode.cjs"],
            path_names: &["zcode"],
            ..Default::default()
        };
        let located = locate_with(&registry, &request);
        assert_eq!(located.source, LocateSource::AppPaths);
        assert!(located.program.ends_with("zcode.cjs"));
        assert!(located.searched.iter().any(|item| item == "app_paths:ZCode.exe"));
    }

    #[test]
    fn empty_candidates_use_source_program_directly() {
        let dir = temp_dir_with(&["ZCode/ZCode.exe"]);
        let exe = dir.path().join("ZCode/ZCode.exe");

        let mut registry = FakeRegistry::default();
        registry.set(
            Hive::CurrentUser,
            "Software\\Classes\\zcode\\shell\\open\\command",
            "",
            &format!("\"{}\" -- \"%1\"", exe.display()),
        );

        let request = LocateRequest {
            protocol: Some("zcode"),
            ..Default::default()
        };
        let located = locate_with(&registry, &request);
        assert_eq!(located.source, LocateSource::Protocol);
        assert_eq!(located.program, exe.to_string_lossy());
    }

    #[test]
    fn resolve_bare_command_ignores_empty_and_path_like_commands() {
        assert_eq!(resolve_bare_command("   "), None);
        assert_eq!(resolve_bare_command(r"C:\Tools\npx.cmd"), None);
        assert_eq!(resolve_bare_command("./scripts/agent"), None);
    }

    #[test]
    fn uninstall_install_location_is_used_when_protocol_missing() {
        let dir = temp_dir_with(&["ZCode/resources/glm/zcode.cjs"]);
        let root = dir.path().join("ZCode");
        let mut registry = FakeRegistry::default();
        registry.keys(
            Hive::CurrentUser,
            "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
            &["{guid}"],
        );
        registry.set(
            Hive::CurrentUser,
            "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{guid}",
            "DisplayName",
            "ZCode 3.18.2",
        );
        registry.set(
            Hive::CurrentUser,
            "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{guid}",
            "InstallLocation",
            &root.to_string_lossy(),
        );

        let request = LocateRequest {
            uninstall_prefix: Some("ZCode"),
            directory_candidates: &["resources/glm/zcode.cjs"],
            path_names: &["zcode"],
            ..Default::default()
        };
        let located = locate_with(&registry, &request);
        assert_eq!(located.source, LocateSource::Uninstall);
        assert!(located.program.ends_with("zcode.cjs"));
    }

    #[test]
    fn missing_sources_reports_searched_and_fallback_name() {
        let registry = FakeRegistry::default();
        let request = LocateRequest {
            protocol: Some("zcode"),
            app_paths_exe: Some("ZCode.exe"),
            path_names: &["zcode"],
            ..Default::default()
        };
        let located = locate_with(&registry, &request);
        assert_eq!(located.source, LocateSource::Fallback);
        assert_eq!(located.program, "zcode");
        assert!(located.searched.iter().any(|item| item == "protocol:zcode"));
        assert!(located.searched.iter().any(|item| item == "app_paths:ZCode.exe"));
        assert!(located.searched.iter().any(|item| item == "path:zcode"));
    }

    #[test]
    fn path_lookup_expands_registry_path_and_pathext() {
        let dir = temp_dir_with(&["bin/tool.EXE"]);
        let mut registry = FakeRegistry::default();
        registry.set(
            Hive::CurrentUser,
            "Environment",
            "MOON_TEST_BIN",
            &dir.path().join("bin").to_string_lossy(),
        );
        registry.set(
            Hive::CurrentUser,
            "Environment",
            "Path",
            "%MOON_TEST_BIN%",
        );
        registry.set(
            Hive::LocalMachine,
            "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
            "PATHEXT",
            ".EXE;.CMD",
        );

        let found = resolve_in_path(&registry, "tool").expect("应在注册表 PATH 中找到命令");
        let found = Path::new(&found);
        assert_eq!(found.file_name().unwrap(), "tool.EXE");
        assert_eq!(found.parent().unwrap().file_name().unwrap(), "bin");
    }

    #[test]
    fn join_relative_uses_native_separators() {
        let joined = join_relative(Path::new("D:\\ZCode"), "resources/glm/zcode.cjs");
        assert_eq!(
            joined,
            Path::new("D:\\ZCode")
                .join("resources")
                .join("glm")
                .join("zcode.cjs")
        );
        #[cfg(windows)]
        assert!(
            !joined.to_string_lossy().contains('/'),
            "展示路径不应混用分隔符：{joined:?}"
        );
    }
}
