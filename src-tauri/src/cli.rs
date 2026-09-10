use crate::app_db;
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
#[cfg(target_os = "macos")]
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(target_os = "macos")]
static ANSI_REGEX: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"[\u001b\u009b][\[()#;?]*(?:[0-9]{1,4}(?:;[0-9]{0,4})*)?[0-9A-ORZcf-nqry=><]")
        .expect("Failed to compile ANSI regex")
});

#[cfg(target_os = "macos")]
fn strip_ansi_codes(input: &str) -> String {
    ANSI_REGEX.replace_all(input, "").to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CliKind {
    Claude,
    Codex,
    Gemini,
    WorkBuddy,
    Dsh,
    Antigravity,
}

#[derive(Debug, Clone, Serialize)]
pub struct CliStatus {
    pub id: String,
    pub has_sessions: bool,
    pub has_binary: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliPathConfig {
    pub id: String,
    pub default_data_dir: String,
    pub effective_data_dir: String,
    pub sessions_dir: String,
    pub has_override: bool,
}

impl CliKind {
    pub fn id(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::WorkBuddy => "workbuddy",
            Self::Dsh => "dsh",
            Self::Antigravity => "antigravity",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
            Self::Gemini => "Gemini",
            Self::WorkBuddy => "WorkBuddy",
            Self::Dsh => "DSH",
            Self::Antigravity => "Antigravity",
        }
    }

    pub fn command(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            // 仅作展示/回退字符串:WorkBuddy 是桌面应用,恢复走深链,从不 exec 该命令
            Self::WorkBuddy => "workbuddy",
            Self::Dsh => "dsh",
            Self::Antigravity => "agy",
        }
    }

    pub fn launch_arguments(self, session_id: Option<&str>, skip_permissions: bool, settings_file: Option<&str>) -> Vec<String> {
        let mut parts = Vec::new();
        match self {
            Self::Claude => {
                if let Some(id) = session_id {
                    parts.push("--resume".to_string());
                    parts.push(id.to_string());
                }
                if skip_permissions {
                    parts.push("--dangerously-skip-permissions".to_string());
                }
                if let Some(path) = settings_file {
                    parts.push("--settings".to_string());
                    parts.push(path.to_string());
                }
            }
            Self::Codex => {
                if skip_permissions {
                    parts.push("--dangerously-bypass-approvals-and-sandbox".to_string());
                }
                if let Some(id) = session_id {
                    parts.push("resume".to_string());
                    parts.push(id.to_string());
                }
            }
            Self::Gemini => {
                if skip_permissions {
                    parts.push("-y".to_string());
                }
                if let Some(id) = session_id {
                    parts.push("--resume".to_string());
                    parts.push(id.to_string());
                }
            }
            // WorkBuddy 恢复走 workbuddy:// 深链,不经过终端参数
            Self::WorkBuddy => {}
            Self::Dsh => {
                if let Some(id) = session_id {
                    parts.push("--profile".to_string());
                    parts.push("tui".to_string());
                    parts.push("--resume".to_string());
                    parts.push(id.to_string());
                }
            }
            Self::Antigravity => {
                if let Some(id) = session_id {
                    parts.push("--conversation".to_string());
                    parts.push(id.to_string());
                }
            }
        }
        parts
    }

    pub fn data_dir_name(self) -> &'static str {
        match self {
            Self::Claude => ".claude",
            Self::Codex => ".codex",
            Self::Gemini => ".gemini",
            Self::WorkBuddy => ".workbuddy",
            Self::Dsh => ".dsh",
            Self::Antigravity => ".gemini/antigravity-cli",
        }
    }

    pub fn all() -> [Self; 6] {
        [Self::Claude, Self::Codex, Self::Gemini, Self::WorkBuddy, Self::Dsh, Self::Antigravity]
    }

    pub fn from_id(value: Option<&str>) -> Result<Self, String> {
        match value.map(str::trim).filter(|value| !value.is_empty()) {
            Some("claude") => Ok(Self::Claude),
            Some("codex") => Ok(Self::Codex),
            Some("gemini") => Ok(Self::Gemini),
            Some("workbuddy") => Ok(Self::WorkBuddy),
            Some("dsh") => Ok(Self::Dsh),
            Some("antigravity") => Ok(Self::Antigravity),
            Some(other) => Err(format!("不支持的 CLI: {}", other)),
            None => Err("缺少 cliId".to_string()),
        }
    }
}

pub fn list_cli_statuses() -> Vec<CliStatus> {
    CliKind::all()
        .into_iter()
        .map(|kind| {
            let has_binary = detect_cli(kind);
            CliStatus {
                id: kind.id().to_string(),
                has_sessions: has_sessions(kind),
                has_binary,
            }
        })
        .collect()
}

/// 该 CLI 是否存在可恢复的会话。
/// 优先读会话索引计数;若索引暂不可用(如新接入的 Dsh 尚未建索引),
/// 回退到目录级探测:sessions_dir 存在且非空。
pub fn has_sessions(kind: CliKind) -> bool {
    if let Ok(count) = crate::app_db::count_session_list_index(kind) {
        if count > 0 {
            return true;
        }
    }
    let dir = match sessions_dir(kind) {
        Ok(d) => d,
        Err(_) => return false,
    };
    std::fs::read_dir(&dir)
        .map(|mut it| it.next().is_some())
        .unwrap_or(false)
}

pub fn default_data_dir(kind: CliKind) -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("无法获取用户目录")?;
    match kind {
        CliKind::Antigravity => Ok(home.join(".gemini").join("antigravity-cli")),
        _ => Ok(home.join(kind.data_dir_name())),
    }
}

pub fn data_dir(kind: CliKind) -> Result<PathBuf, String> {
    let overrides = read_cli_path_overrides();
    if let Some(path) = overrides.get(kind.id()) {
        if let Ok(normalized) = normalize_cli_data_dir(path) {
            return Ok(PathBuf::from(normalized));
        }
    }
    default_data_dir(kind)
}

pub fn history_path(kind: CliKind) -> Result<PathBuf, String> {
    Ok(data_dir(kind)?.join("history.jsonl"))
}

pub fn sessions_dir(kind: CliKind) -> Result<PathBuf, String> {
    match kind {
        CliKind::Claude => Ok(data_dir(kind)?.join("projects")),
        CliKind::Codex => Ok(data_dir(kind)?.join("sessions")),
        CliKind::Gemini => Ok(data_dir(kind)?.join("tmp")),
        CliKind::WorkBuddy => Ok(data_dir(kind)?.join("projects")),
        CliKind::Dsh => Ok(data_dir(kind)?.join("sessions")),
        CliKind::Antigravity => Ok(data_dir(kind)?.join("brain")),
    }
}

pub fn list_cli_path_configs() -> Result<Vec<CliPathConfig>, String> {
    let overrides = read_cli_path_overrides();

    CliKind::all()
        .into_iter()
        .map(|kind| {
            let default_data_dir = default_data_dir(kind)?;
            let effective_data_dir = data_dir(kind)?;
            let sessions_dir = sessions_dir(kind)?;

            Ok(CliPathConfig {
                id: kind.id().to_string(),
                default_data_dir: default_data_dir.to_string_lossy().to_string(),
                effective_data_dir: effective_data_dir.to_string_lossy().to_string(),
                sessions_dir: sessions_dir.to_string_lossy().to_string(),
                has_override: overrides.contains_key(kind.id()),
            })
        })
        .collect()
}

pub fn set_cli_data_dir_override(kind: CliKind, data_dir: Option<String>) -> Result<(), String> {
    let mut overrides = read_cli_path_overrides();

    match data_dir
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        Some(value) => {
            let normalized = normalize_cli_data_dir(&value)?;
            overrides.insert(kind.id().to_string(), normalized);
        }
        None => {
            overrides.remove(kind.id());
        }
    }

    write_cli_path_overrides(&overrides)
}

pub fn detect_cli(kind: CliKind) -> bool {
    find_cli_path(kind).is_some()
}

fn read_cli_path_overrides() -> HashMap<String, String> {
    app_db::read_cli_path_overrides().unwrap_or_default()
}

fn write_cli_path_overrides(overrides: &HashMap<String, String>) -> Result<(), String> {
    app_db::write_cli_path_overrides(overrides).map_err(|e| e.to_string())
}

fn normalize_cli_data_dir(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("路径不能为空。".to_string());
    }

    let expanded = expand_home_path(trimmed)?;
    if !expanded.is_absolute() {
        return Err("请输入绝对路径，或使用以 ~ 开头的用户目录路径。".to_string());
    }

    Ok(expanded.to_string_lossy().to_string())
}

fn expand_home_path(value: &str) -> Result<PathBuf, String> {
    if value == "~" {
        return dirs::home_dir().ok_or("无法获取用户目录".to_string());
    }

    if let Some(stripped) = value.strip_prefix("~/") {
        let home = dirs::home_dir().ok_or("无法获取用户目录".to_string())?;
        return Ok(home.join(stripped));
    }

    Ok(PathBuf::from(value))
}

pub fn cli_version(kind: CliKind) -> Result<String, String> {
    if kind == CliKind::WorkBuddy {
        // WorkBuddy 是桌面应用,从 bundle Info.plist 读版本
        let app_path = find_workbuddy_app().ok_or("未检测到 WorkBuddy 应用")?;
        let output = Command::new("defaults")
            .args(["read", &format!("{}/Contents/Info", app_path), "CFBundleShortVersionString"])
            .output()
            .map_err(|e| format!("读取 WorkBuddy 版本失败: {}", e))?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
        }
        return Err("获取 WorkBuddy 版本失败".to_string());
    }
    let cli_path = find_cli_path(kind).ok_or_else(|| format!("未检测到 {} CLI", kind.name()))?;
    #[allow(unused_mut)]
    let mut cmd = Command::new(cli_path);
    cmd.arg("--version");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let output = cmd.output().map_err(|_| format!("未检测到 {} CLI", kind.name()))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
} else {
        Err("获取 CLI 版本失败".to_string())
    }
}

pub fn build_launch_command_string(
    kind: CliKind,
    project_path: &str,
    session_id: Option<&str>,
    skip_permissions: bool,
    settings_file: Option<&str>,
    terminal_app: Option<&str>,
) -> String {
    if kind == CliKind::WorkBuddy {
        // 复制命令等场景给出真实可执行的深链命令
        return match workbuddy_deeplink(session_id) {
            Some(url) => format!("open \"{}\"", url),
            None => "open -a WorkBuddy".to_string(),
        };
    }
    let cli_path = find_cli_path(kind).unwrap_or_else(|| kind.command().to_string());
    let launch_args = kind.launch_arguments(session_id, skip_permissions, settings_file);

    #[cfg(target_os = "windows")]
    {
        // 跟随终端设置生成粘贴可执行的语法：
        // 只有显式选 CMD 才用 cmd 语法（cd /d + &&）；Windows Terminal 默认 profile
        // 通常是 PowerShell，与 PowerShell 选项一样生成 PS 语法（PS 5.1 不支持 &&）
        if crate::terminal::resolve_windows_choice(terminal_app)
            != crate::terminal::WINDOWS_CMD
        {
            return build_powershell_command_line(&cli_path, &launch_args, project_path, settings_file);
        }
        let cli_command = build_windows_command_line(&cli_path, &launch_args);
        let mut cmd = format!("cd /d \"{}\" && {}", project_path, cli_command);
        if let Some(file) = settings_file {
            if file.contains("sessiondock-settings-") {
                cmd = format!("{} & del /f /q \"{}\"", cmd, file);
            }
        }
        cmd
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = terminal_app;
        let cli_command = launch_args
            .iter()
            .fold(posix_shell_escape(&cli_path), |acc, arg| {
                format!("{} {}", acc, posix_shell_escape(arg))
            });
        let mut cmd = format!("cd {} && {}", posix_shell_escape(project_path), cli_command);
        if let Some(file) = settings_file {
            if file.contains("sessiondock-settings-") {
                cmd = format!("{}; rm -f {}", cmd, posix_shell_escape(file));
            }
        }
        cmd
    }
}

/// 构造 WorkBuddy 深链。session_id 含非法字符时返回 None(防御性校验)。
fn workbuddy_deeplink(session_id: Option<&str>) -> Option<String> {
    match session_id {
        Some(id)
            if !id.is_empty()
                && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') =>
        {
            Some(format!("workbuddy://chat/{}", id))
        }
        Some(_) => None,
        None => Some("workbuddy://chat".to_string()),
    }
}

/// 定位 WorkBuddy 桌面应用 bundle(macOS)。
fn find_workbuddy_app() -> Option<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    [
        "/Applications/WorkBuddy.app".to_string(),
        format!("{}/Applications/WorkBuddy.app", home),
    ]
    .into_iter()
    .find(|p| std::path::Path::new(p).exists())
}

pub fn open_in_terminal(
    kind: CliKind,
    project_path: &str,
    session_id: Option<&str>,
    skip_permissions: bool,
    settings_file: Option<&str>,
    terminal_app: Option<&str>,
) -> Result<(), String> {
    if kind == CliKind::WorkBuddy {
        // 恢复会话 = 打开 workbuddy:// 深链跳转到 WorkBuddy 应用,不走终端
        #[cfg(target_os = "macos")]
        {
            let url = workbuddy_deeplink(session_id)
                .ok_or_else(|| format!("非法的 WorkBuddy 会话 ID: {:?}", session_id))?;
            Command::new("open")
                .arg(&url)
                .spawn()
                .map_err(|e| format!("无法打开 WorkBuddy: {}", e))?;
            return Ok(());
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = session_id;
            return Err("WorkBuddy 目前仅支持 macOS".to_string());
        }
    }
    #[cfg(target_os = "macos")]
    {
        let tmp_script = create_temp_launch_script(
            "sh",
            &{
                let mut script = format!(
                    "#!/bin/bash -l\n{}\n",
                    build_launch_command_string(kind, project_path, session_id, skip_permissions, settings_file, terminal_app)
                );
                if let Some(sf) = settings_file {
                    script.push_str(&format!("rm -f {}\n", posix_shell_escape(sf)));
                }
                script.push_str("rm -f \"$0\"\n");
                script
            },
        )?;

        Command::new("chmod")
            .args(["+x", tmp_script.to_str().unwrap_or("")])
            .output()
            .map_err(|e| format!("chmod 失败: {}", e))?;

        let script_path = tmp_script.to_str().unwrap_or("").to_string();

        let terminal_app = crate::terminal::macos_app_name(terminal_app);

        Command::new("open")
            .args(["-a", terminal_app, &script_path])
            .spawn()
            .map_err(|e| format!("未检测到终端应用。{}", e))?;

        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        let cli_path = find_cli_path(kind).unwrap_or_else(|| kind.command().to_string());
        let launch_args = kind.launch_arguments(session_id, skip_permissions, settings_file);

        if crate::terminal::resolve_windows_choice(terminal_app)
            == crate::terminal::WINDOWS_POWERSHELL
        {
            let script_content = build_powershell_launch_script(
                &cli_path,
                &launch_args,
                project_path,
                settings_file,
            );
            let tmp_script = create_temp_launch_script("ps1", &script_content)?;
            let script_path = tmp_script
                .to_str()
                .ok_or("临时脚本路径含非 UTF-8 字符")?
                .to_string();
            Command::new("cmd")
                .args([
                    "/d", "/c", "start", "", "powershell", "-NoExit", "-ExecutionPolicy",
                    "Bypass", "-File", &script_path,
                ])
                .spawn()
                .map_err(|e| format!("未检测到终端应用。{}", e))?;
            return Ok(());
        }

        let script_content =
            build_windows_launch_script(&cli_path, &launch_args, project_path, settings_file);
        let tmp_script = create_temp_launch_script("cmd", &script_content)?;
        let script_path = tmp_script
            .to_str()
            .ok_or("临时脚本路径含非 UTF-8 字符")?
            .to_string();

        let use_wt = crate::terminal::resolve_windows_choice(terminal_app)
            == crate::terminal::WINDOWS_TERMINAL;
        if use_wt {
            let wt_result = Command::new("wt.exe")
                .args(["-d", project_path, "cmd", "/k", &script_path])
                .spawn();
            if wt_result.is_ok() {
                return Ok(());
            }
        }

        Command::new("cmd")
            .args(["/d", "/c", "start", "", "cmd", "/k", &script_path])
            .spawn()
            .map_err(|e| format!("未检测到终端应用。{}", e))?;
        return Ok(());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (
            kind,
            project_path,
            session_id,
            skip_permissions,
            settings_file,
        );
        Err("不支持的平台".to_string())
    }
}

fn create_temp_launch_script(extension: &str, content: &str) -> Result<PathBuf, String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let pid = std::process::id();
    let temp_dir = if cfg!(target_os = "macos") {
        PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    };
    let path = temp_dir.join(format!(
        "sessiondock-launch-{}-{}.{}",
        pid, timestamp, extension
    ));
    fs::write(&path, content).map_err(|e| format!("无法创建临时脚本: {}", e))?;
    Ok(path)
}

#[allow(dead_code)]
fn posix_shell_escape(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(target_os = "windows")]
fn build_windows_launch_script(
    cli_path: &str,
    launch_args: &[String],
    project_path: &str,
    settings_file: Option<&str>,
) -> String {
    let cli_command = build_windows_command_line(cli_path, launch_args);
    let mut script = format!(
        "@echo off\r\nsetlocal\r\ncd /d \"{}\"\r\ncall {}\r\n",
        escape_batch_value(project_path),
        cli_command,
    );
    if let Some(sf) = settings_file {
        script.push_str(&format!("del /f /q \"{}\" >nul 2>&1\r\n", escape_batch_value(sf)));
    }
    script
}

// cfg(any(windows, test))：纯字符串构造，不含 Windows API，测试在 macOS 上也能跑
#[cfg(any(target_os = "windows", test))]
fn ps_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// 单行 PowerShell 命令：用于「复制命令」等直接粘贴执行的场景
#[cfg(any(target_os = "windows", test))]
fn build_powershell_command_line(
    cli_path: &str,
    launch_args: &[String],
    project_path: &str,
    settings_file: Option<&str>,
) -> String {
    let mut parts = vec![ps_single_quote(cli_path)];
    parts.extend(launch_args.iter().map(|arg| ps_single_quote(arg)));
    let mut cmd = format!(
        "Set-Location -LiteralPath {}; & {}",
        ps_single_quote(project_path),
        parts.join(" ")
    );
    if let Some(sf) = settings_file {
        if sf.contains("sessiondock-settings-") {
            cmd = format!(
                "{}; Remove-Item -LiteralPath {} -Force -ErrorAction SilentlyContinue",
                cmd,
                ps_single_quote(sf)
            );
        }
    }
    cmd
}

#[cfg(any(target_os = "windows", test))]
fn build_powershell_launch_script(
    cli_path: &str,
    launch_args: &[String],
    project_path: &str,
    settings_file: Option<&str>,
) -> String {
    let mut parts = vec![ps_single_quote(cli_path)];
    parts.extend(launch_args.iter().map(|arg| ps_single_quote(arg)));
    let mut script = format!(
        "Set-Location -LiteralPath {}\r\n& {}\r\n",
        ps_single_quote(project_path),
        parts.join(" ")
    );
    if let Some(sf) = settings_file {
        script.push_str(&format!(
            "Remove-Item -LiteralPath {} -Force -ErrorAction SilentlyContinue\r\n",
            ps_single_quote(sf)
        ));
    }
    script
}

#[cfg(target_os = "windows")]
pub fn build_windows_context_menu_script(
    kind: CliKind,
    cli_path: &str,
    skip_permissions: bool,
) -> String {
    let cli_command =
        build_windows_command_line(cli_path, &kind.launch_arguments(None, skip_permissions, None));
    format!(
        "@echo off\r\nsetlocal\r\nif \"%~1\"==\"\" (\r\n  echo 未提供目录路径。\r\n  exit /b 1\r\n)\r\ncd /d \"%~1\"\r\ncall {}\r\n",
        cli_command,
    )
}

#[cfg(target_os = "windows")]
fn build_windows_command_line(cli_path: &str, launch_args: &[String]) -> String {
    let mut parts = vec![quote_cmd_argument(cli_path)];
    parts.extend(launch_args.iter().map(|arg| quote_cmd_argument(arg)));
    parts.join(" ")
}

#[cfg(target_os = "windows")]
fn quote_cmd_argument(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".to_string();
    }

    let needs_quotes = value.chars().any(|ch| {
        ch.is_whitespace() || matches!(ch, '"' | '&' | '|' | '<' | '>' | '^' | '(' | ')' | '%')
    });

    if !needs_quotes {
        return value.to_string();
    }

    let mut escaped = String::new();
    let mut backslashes = 0usize;

    for ch in value.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                escaped.push_str(&"\\".repeat(backslashes * 2 + 1));
                escaped.push('"');
                backslashes = 0;
            }
            '%' => {
                if backslashes > 0 {
                    escaped.push_str(&"\\".repeat(backslashes));
                    backslashes = 0;
                }
                escaped.push_str("%%");
            }
            _ => {
                if backslashes > 0 {
                    escaped.push_str(&"\\".repeat(backslashes));
                    backslashes = 0;
                }
                escaped.push(ch);
            }
        }
    }

    if backslashes > 0 {
        escaped.push_str(&"\\".repeat(backslashes * 2));
    }

    format!("\"{}\"", escaped)
}

#[cfg(target_os = "windows")]
fn escape_batch_value(value: &str) -> String {
    value.replace('%', "%%")
}

pub fn find_cli_path(kind: CliKind) -> Option<String> {
    // WorkBuddy 是桌面应用而非 PATH 上的二进制,按 bundle 存在性检测
    if kind == CliKind::WorkBuddy {
        return find_workbuddy_app();
    }
    #[cfg(target_os = "macos")]
    {
        return find_cli_path_macos(kind.command());
    }

    #[cfg(target_os = "windows")]
    {
        return find_cli_path_windows(kind.command());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = kind;
        None
    }
}

#[cfg(target_os = "macos")]
fn find_cli_path_macos(command_name: &str) -> Option<String> {
    for shell in ["/bin/zsh", "/bin/bash"] {
        if let Ok(output) = Command::new(shell)
            .args(["-l", "-c", &format!("which {}", command_name)])
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    let cleaned = strip_ansi_codes(line).trim().to_string();
                    if !cleaned.is_empty()
                        && cleaned.starts_with('/')
                        && std::path::Path::new(&cleaned).exists()
                    {
                        return Some(cleaned);
                    }
                }
            }
        }
    }

    let home = std::env::var("HOME").unwrap_or_default();
    let candidates = [
        format!("{}/.local/bin/{}", home, command_name),
        format!("{}/.npm-global/bin/{}", home, command_name),
        format!("/opt/homebrew/bin/{}", command_name),
        format!("/usr/local/bin/{}", command_name),
        format!("/usr/bin/{}", command_name),
    ];

    candidates
        .into_iter()
        .find(|path| std::path::Path::new(path).exists())
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_hidden_command(program: &str) -> Command {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let mut cmd = Command::new(program);
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

#[cfg(target_os = "windows")]
fn find_cli_path_windows(command_name: &str) -> Option<String> {
    let mut candidates = Vec::new();
    if let Ok(appdata) = std::env::var("APPDATA") {
        candidates.push(
            PathBuf::from(&appdata)
                .join("npm")
                .join(format!("{}.cmd", command_name)),
        );
        candidates.push(
            PathBuf::from(&appdata)
                .join("npm")
                .join(format!("{}.exe", command_name)),
        );
    }
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        candidates.push(
            PathBuf::from(&local_app_data)
                .join("Microsoft")
                .join("WindowsApps")
                .join(format!("{}.exe", command_name)),
        );
    }

    if let Some(path) = candidates
        .into_iter()
        .find(|path| path.exists())
        .map(|path| path.to_string_lossy().to_string())
    {
        return Some(path);
    }

    let path_var = std::env::var_os("PATH")?;
    let path_exts = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
        .split(';')
        .filter_map(|ext| {
            let trimmed = ext.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .collect::<Vec<_>>();

    for dir in std::env::split_paths(&path_var) {
        let direct = dir.join(command_name);
        if direct.exists() {
            return Some(direct.to_string_lossy().to_string());
        }

        for ext in &path_exts {
            let candidate = dir.join(format!("{}{}", command_name, ext));
            if candidate.exists() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
    }

    let output = windows_hidden_command("where")
        .arg(command_name)
        .output()
        .ok()?;
    if output.status.success() {
        let first = String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        if !first.is_empty() {
            return Some(first);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn powershell_command_line_uses_set_location_and_call_operator() {
        let args = vec!["--resume".to_string(), "abc-123".to_string()];
        let cmd = build_powershell_command_line(
            "C:\\Users\\y\\npm\\claude.cmd",
            &args,
            "D:\\codes\\proj",
            None,
        );
        assert_eq!(
            cmd,
            "Set-Location -LiteralPath 'D:\\codes\\proj'; & 'C:\\Users\\y\\npm\\claude.cmd' '--resume' 'abc-123'"
        );
    }

    #[test]
    fn powershell_command_line_escapes_single_quotes() {
        let cmd = build_powershell_command_line("claude", &[], "D:\\it's here", None);
        assert!(cmd.contains("'D:\\it''s here'"));
    }

    #[test]
    fn powershell_command_line_appends_settings_cleanup_only_for_temp_settings() {
        let args = vec!["--settings".to_string(), "C:\\tmp\\sessiondock-settings-1.json".to_string()];
        let cmd = build_powershell_command_line(
            "claude",
            &args,
            "D:\\proj",
            Some("C:\\tmp\\sessiondock-settings-1.json"),
        );
        assert!(cmd.contains(
            "; Remove-Item -LiteralPath 'C:\\tmp\\sessiondock-settings-1.json' -Force -ErrorAction SilentlyContinue"
        ));

        let keep = build_powershell_command_line("claude", &[], "D:\\proj", Some("C:\\tmp\\my-settings.json"));
        assert!(!keep.contains("Remove-Item"));
    }

    #[test]
    fn powershell_launch_script_contains_set_location_and_invocation() {
        let args = vec!["--resume".to_string(), "abc".to_string()];
        let script = build_powershell_launch_script("claude", &args, "D:\\proj", None);
        assert!(script.starts_with("Set-Location -LiteralPath 'D:\\proj'\r\n"));
        assert!(script.contains("& 'claude' '--resume' 'abc'\r\n"));
    }

    #[test]
    fn dsh_default_data_dir_is_home_dsh() {
        let dir = default_data_dir(CliKind::Dsh).unwrap();
        assert!(dir.ends_with(".dsh"));
    }
    #[test]
    fn dsh_sessions_dir_is_sessions_subdir() {
        let dir = sessions_dir(CliKind::Dsh).unwrap();
        assert_eq!(dir.file_name().unwrap().to_str(), Some("sessions"));
    }
    #[test]
    fn antigravity_default_data_dir_is_home_gemini_antigravity() {
        let dir = default_data_dir(CliKind::Antigravity).unwrap();
        assert!(dir.ends_with(".gemini/antigravity-cli") || dir.ends_with(".gemini\\antigravity-cli"));
    }
    #[test]
    fn antigravity_sessions_dir_is_brain_subdir() {
        let dir = sessions_dir(CliKind::Antigravity).unwrap();
        assert_eq!(dir.file_name().unwrap().to_str(), Some("brain"));
    }
    #[test]
    fn cli_status_carries_antigravity() {
        let statuses = list_cli_statuses();
        let agy = statuses.iter().find(|s| s.id == "antigravity").expect("antigravity 在列表中");
        assert_eq!(
            agy.has_sessions && agy.has_binary,
            agy.has_binary && agy.has_sessions
        );
    }
}
