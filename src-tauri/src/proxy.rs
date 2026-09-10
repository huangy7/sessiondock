use crate::cli::{self, CliKind};
use crate::{history, parser, session};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{LazyLock, Mutex};

// Track subprocess PIDs per CLI — used to kill proxy on disable/app exit
static SUBPROCESS_PIDS: LazyLock<Mutex<HashMap<String, u32>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

// ─── Types ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyState {
    pub enabled: bool,
    pub port: u16,
    pub ws_port: u16,
    pub original_base_url: String,
    pub target_url: String,
    #[serde(default = "default_cli_id")]
    pub cli_id: String,
}

fn default_cli_id() -> String {
    "claude".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyStatus {
    pub enabled: bool,
    pub running: bool,
    pub port: u16,
    pub ws_port: u16,
    pub target_url: String,
    pub cli_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficSummary {
    pub id: String,
    pub timestamp: String,
    pub method: String,
    pub path: String,
    pub req_size: i64,
    pub status: Option<i32>,
    pub res_size: i64,
    pub duration_ms: i64,
    /// 从 SSE 响应体解析的 token usage（非 SSE/解析失败为 None）
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_tokens: Option<i64>,
    pub cache_creation_tokens: Option<i64>,
    /// 请求体携带的工具名列表（$.tools，Anthropic/OpenAI 双格式）
    pub tool_names: Vec<String>,
    /// 请求上下文中是否包含 Skill 工具的调用记录
    pub has_skill_call: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficDetail {
    pub id: String,
    pub timestamp: String,
    pub method: String,
    pub path: String,
    pub req_headers: Option<String>,
    pub req_body: Option<String>,
    pub req_size: i64,
    pub status: Option<i32>,
    pub res_headers: Option<String>,
    pub res_body: Option<String>,
    pub res_size: i64,
    pub duration_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficList {
    pub items: Vec<TrafficSummary>,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClearResult {
    pub deleted: u64,
    pub db_size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionTrafficSummary {
    pub session_id: Option<String>,
    pub display_name: String,
    pub project_path: String,
    pub request_count: u64,
    pub total_req_size: i64,
    pub total_res_size: i64,
    pub total_duration_ms: i64,
    pub first_timestamp: String,
    pub last_timestamp: String,
    pub ok_count: u64,
    pub error_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionTrafficList {
    pub items: Vec<SessionTrafficSummary>,
    pub total: u64,
}

// ─── Path helpers ────────────────────────────────────────────────────

/// DSH 代理本期及整个实施计划均不支持:所有 Dsh 代理分支均返回 Err。
/// 此端口仅是满足 match 穷尽性的占位值,实际不会被读取(永不生效)。
const DSH_PROXY_UNUSED_PORT: u16 = 18088;

pub fn default_port_for(kind: CliKind) -> u16 {
    match kind {
        CliKind::Claude => 18080,
        CliKind::Codex => 18082,
        CliKind::Gemini => 18084,
        CliKind::WorkBuddy => 18086,
        // DSH 代理暂不支持,占位端口,实际永不生效
        CliKind::Dsh => DSH_PROXY_UNUSED_PORT,
        CliKind::Antigravity => DSH_PROXY_UNUSED_PORT,
    }
}

fn default_ws_port_for(kind: CliKind) -> u16 {
    default_port_for(kind) + 1
}

fn app_data_dir() -> Result<PathBuf, String> {
    let data = dirs::data_dir().ok_or("无法获取应用数据目录")?;
    Ok(data.join("com.sessiondock.app"))
}

fn proxy_state_path(kind: CliKind) -> Result<PathBuf, String> {
    Ok(app_data_dir()?
        .join("config")
        .join(format!("proxy_state_{}.json", kind.id())))
}

fn traffic_db_path() -> Result<PathBuf, String> {
    let dir = app_data_dir()?.join("traffic");
    fs::create_dir_all(&dir).map_err(|e| format!("创建 traffic 目录失败: {}", e))?;
    Ok(dir.join("traffic.db"))
}

fn proxy_log_dir() -> Result<PathBuf, String> {
    let dir = app_data_dir()?.join("logs");
    fs::create_dir_all(&dir).map_err(|e| format!("创建 logs 目录失败: {}", e))?;
    Ok(dir)
}

fn proxy_pid_path(kind: CliKind) -> Result<PathBuf, String> {
    let dir = app_data_dir()?.join("data");
    fs::create_dir_all(&dir).map_err(|e| format!("创建 data 目录失败: {}", e))?;
    Ok(dir.join(format!("proxy_{}.pid", kind.id())))
}

fn parse_pid_file_content(content: &str) -> Option<u32> {
    content.trim().parse::<u32>().ok().filter(|pid| *pid > 0)
}

fn read_proxy_pid_file(kind: CliKind) -> Option<u32> {
    let path = proxy_pid_path(kind).ok()?;
    let content = fs::read_to_string(path).ok()?;
    parse_pid_file_content(&content)
}

fn write_proxy_pid_file(kind: CliKind, pid: u32) {
    match proxy_pid_path(kind) {
        Ok(path) => {
            if let Err(err) = fs::write(&path, format!("{}\n", pid)) {
                tracing::warn!("Failed to write proxy pid file {}: {}", path.display(), err);
            }
        }
        Err(err) => tracing::warn!("Failed to resolve proxy pid file: {}", err),
    }
}

fn remove_proxy_pid_file(kind: CliKind) {
    if let Ok(path) = proxy_pid_path(kind) {
        if let Err(err) = fs::remove_file(&path) {
            if err.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!("Failed to remove proxy pid file {}: {}", path.display(), err);
            }
        }
    }
}

fn command_name_looks_like_proxy(command: &str) -> bool {
    Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name == "sessiondock-proxy" || name == "sessiondock-proxy.exe")
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn process_command_name(pid: u32) -> Option<String> {
    fs::read_to_string(format!("/proc/{}/comm", pid))
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(all(unix, not(target_os = "linux")))]
fn process_command_name(pid: u32) -> Option<String> {
    Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(windows)]
fn process_command_name(pid: u32) -> Option<String> {
    win_command("tasklist")
        .args(["/FI", &format!("PID eq {}", pid), "/FO", "CSV", "/NH"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(unix)]
fn is_process_alive(pid: u32) -> bool {
    pid > 0 && unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[cfg(windows)]
fn is_process_alive(pid: u32) -> bool {
    pid > 0
        && win_command("tasklist")
            .args(["/FI", &format!("PID eq {}", pid)])
            .output()
            .map(|output| {
                output.status.success()
                    && String::from_utf8_lossy(&output.stdout).contains(&pid.to_string())
            })
            .unwrap_or(false)
}

fn is_proxy_process_alive(pid: u32) -> bool {
    if !is_process_alive(pid) {
        return false;
    }
    process_command_name(pid)
        .as_deref()
        .map(command_name_looks_like_proxy)
        .unwrap_or(false)
}

fn terminate_process_best_effort(pid: u32) {
    if !is_proxy_process_alive(pid) {
        return;
    }
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
    #[cfg(windows)]
    {
        let _ = win_command("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output();
    }
}

#[cfg(test)]
mod proxy_pid_tests {
    use super::*;

    #[test]
    fn parse_pid_file_content_accepts_positive_pid() {
        assert_eq!(parse_pid_file_content("12345\n"), Some(12345));
    }

    #[test]
    fn parse_pid_file_content_rejects_invalid_pid() {
        assert_eq!(parse_pid_file_content("not-a-pid"), None);
        assert_eq!(parse_pid_file_content("0"), None);
        assert_eq!(parse_pid_file_content("-1"), None);
    }

    #[test]
    fn command_name_match_only_accepts_sessiondock_proxy_processes() {
        assert!(command_name_looks_like_proxy("/Applications/SessionDock.app/Contents/MacOS/sessiondock-proxy"));
        assert!(command_name_looks_like_proxy("sessiondock-proxy.exe"));
        assert!(!command_name_looks_like_proxy("/bin/sleep"));
        assert!(!command_name_looks_like_proxy("sessiondock"));
    }

    #[test]
    fn extract_sse_usage_reads_anthropic_stream() {
        let body = "event: message_start\n\
                    data: {\"message\":{\"usage\":{\"input_tokens\":87,\"cache_read_input_tokens\":381568,\"cache_creation_input_tokens\":0,\"output_tokens\":0}}}\n\
                    event: content_block_delta\n\
                    data: {\"delta\":{\"text\":\"你\"}}\n\
                    event: message_delta\n\
                    data: {\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":79}}\n";
        let (i, o, cr, cc) = extract_sse_usage(body);
        assert_eq!(i, Some(87));
        assert_eq!(o, Some(79));
        assert_eq!(cr, Some(381568));
        assert_eq!(cc, Some(0));
    }

    #[test]
    fn extract_sse_usage_reads_openai_stream() {
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\
                    data: {\"choices\":[],\"usage\":{\"prompt_tokens\":120,\"completion_tokens\":30}}\n";
        let (i, o, cr, cc) = extract_sse_usage(body);
        assert_eq!(i, Some(120));
        assert_eq!(o, Some(30));
        assert_eq!(cr, None);
        assert_eq!(cc, None);
    }

    #[test]
    fn extract_sse_usage_reads_codex_responses_stream() {
        let body = "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":500,\"output_tokens\":64,\"input_tokens_details\":{\"cached_tokens\":200}}}}\n";
        let (i, o, cr, _cc) = extract_sse_usage(body);
        assert_eq!(i, Some(500));
        assert_eq!(o, Some(64));
        assert_eq!(cr, Some(200));
    }

    #[test]
    fn extract_sse_usage_tolerates_non_sse_body() {
        assert_eq!(
            extract_sse_usage("{\"usage\": \"not-sse-json-per-line\"}"),
            (None, None, None, None)
        );
    }
}

fn proxy_binary_path() -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    let binary_name = "sessiondock-proxy.exe";
    #[cfg(not(target_os = "windows"))]
    let binary_name = "sessiondock-proxy";

    // Development: use cargo target directory
    let dev_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(binary_name)));

    if let Some(ref path) = dev_path {
        if path.exists() {
            return Ok(path.clone());
        }
    }

    // Try workspace target/debug
    let workspace_debug = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.join("target").join("debug").join(binary_name));

    if let Some(ref path) = workspace_debug {
        if path.exists() {
            return Ok(path.clone());
        }
    }

    Err("找不到 sessiondock-proxy 二进制文件".to_string())
}

/// 供 assistant 模块复用的 proxy 二进制路径解析（proxy_binary_path 的 pub(crate) 包装）
pub(crate) fn proxy_binary_path_pub() -> Result<PathBuf, String> {
    proxy_binary_path()
}

// ─── State management ────────────────────────────────────────────────

fn read_proxy_state(kind: CliKind) -> Result<Option<ProxyState>, String> {
    read_proxy_state_file(&proxy_state_path(kind)?)
}

fn read_proxy_state_file(path: &Path) -> Result<Option<ProxyState>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(path).map_err(|e| format!("读取代理状态失败: {}", e))?;
    let state: ProxyState =
        serde_json::from_str(&content).map_err(|e| format!("解析代理状态失败: {}", e))?;
    Ok(Some(state))
}

fn write_proxy_state(kind: CliKind, state: &ProxyState) -> Result<(), String> {
    let path = proxy_state_path(kind)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {}", e))?;
    }
    let json = serde_json::to_string_pretty(state).map_err(|e| format!("序列化失败: {}", e))?;
    let mut file = fs::File::create(&path).map_err(|e| format!("写入代理状态失败: {}", e))?;
    file.write_all(json.as_bytes())
        .map_err(|e| format!("写入代理状态失败: {}", e))?;
    Ok(())
}

fn activate_proxy(kind: CliKind, state: &ProxyState, proxy_url: &str) -> Result<(), String> {
    write_proxy_state(kind, state)?;
    if let Err(err) = set_base_url_for_cli(kind, proxy_url) {
        let _ = remove_proxy_state(kind);
        return Err(err);
    }
    Ok(())
}

fn remove_proxy_state(kind: CliKind) -> Result<(), String> {
    let path = proxy_state_path(kind)?;
    if path.exists() {
        fs::remove_file(&path).map_err(|e| format!("删除代理状态失败: {}", e))?;
    }
    Ok(())
}

fn is_port_listening(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &format!("127.0.0.1:{}", port).parse().unwrap(),
        std::time::Duration::from_millis(100),
    )
    .is_ok()
}

#[cfg(target_os = "windows")]
fn win_command(program: &str) -> Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let mut cmd = Command::new(program);
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

// ─── Settings manipulation ───────────────────────────────────────────

/// Validate that a URL is safe to use in shell commands (no injection characters)
fn validate_url(url: &str) -> Result<(), String> {
    // Must start with http:// or https://
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(format!("无效的 URL 协议: {}", url));
    }
    // Reject shell metacharacters that could be used for command injection
    const FORBIDDEN: &[char] = &[
        '&', '|', ';', '`', '$', '(', ')', '{', '}', '<', '>', '!', '\n', '\r', '"', '\'', '\\',
    ];
    if let Some(c) = url.chars().find(|c| FORBIDDEN.contains(c)) {
        return Err(format!("URL 含非法字符 '{}': {}", c, url));
    }
    // Reject localhost URLs to prevent proxying internal services
    if let Ok(parsed) = url::Url::parse(url) {
        if let Some(host) = parsed.host() {
            let is_localhost = match host {
                url::Host::Domain("localhost") => true,
                url::Host::Ipv4(ip) => ip.is_loopback(),
                url::Host::Ipv6(ip) => ip.is_loopback(),
                _ => false,
            };
            if is_localhost {
                return Err(format!("不允许代理本地地址: {}", url));
            }
        }
    }
    Ok(())
}

fn settings_path_for(kind: CliKind) -> Result<PathBuf, String> {
    match kind {
        CliKind::Claude => Ok(cli::data_dir(kind)?.join("settings.json")),
        CliKind::Codex => Ok(cli::data_dir(kind)?.join("config.toml")),
        CliKind::Gemini => Err("Gemini 不支持 API 代理".to_string()),
        CliKind::WorkBuddy => Err("WorkBuddy 不支持 API 代理".to_string()),
        CliKind::Dsh => Err("DSH 不支持 API 代理".to_string()),
        CliKind::Antigravity => Err("Antigravity 不支持 API 代理".to_string()),
    }
}

fn codex_default_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}

fn default_base_url_for(kind: CliKind) -> String {
    match kind {
        CliKind::Claude => "https://api.anthropic.com".to_string(),
        CliKind::Codex => codex_default_base_url(),
        CliKind::Gemini => String::new(),
        CliKind::WorkBuddy => String::new(),
        CliKind::Dsh => String::new(),
        CliKind::Antigravity => String::new(),
    }
}

fn ensure_parent_dir(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {}", e))?;
    }
    Ok(())
}

fn read_json_file(path: &Path, default_content: &str) -> Result<serde_json::Value, String> {
    if !path.exists() {
        return serde_json::from_str(default_content).map_err(|e| format!("默认 JSON 无效: {}", e));
    }
    let raw = fs::read_to_string(path).map_err(|e| format!("读取配置失败: {}", e))?;
    if raw.trim().is_empty() {
        return serde_json::from_str(default_content).map_err(|e| format!("默认 JSON 无效: {}", e));
    }
    serde_json::from_str(&raw).map_err(|e| format!("解析 JSON 失败: {}", e))
}

fn write_json_file(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    ensure_parent_dir(path)?;
    let json =
        serde_json::to_string_pretty(value).map_err(|e| format!("序列化 JSON 失败: {}", e))?;
    let mut file = fs::File::create(path).map_err(|e| format!("写入配置失败: {}", e))?;
    file.write_all(json.as_bytes())
        .map_err(|e| format!("写入配置失败: {}", e))
}

fn read_toml_file(path: &Path) -> Result<toml::Value, String> {
    if !path.exists() {
        return Ok(toml::Value::Table(toml::map::Map::new()));
    }
    let raw = fs::read_to_string(path).map_err(|e| format!("读取配置失败: {}", e))?;
    if raw.trim().is_empty() {
        return Ok(toml::Value::Table(toml::map::Map::new()));
    }
    raw.parse::<toml::Value>()
        .map_err(|e| format!("解析 TOML 失败: {}", e))
}

fn write_toml_file(path: &Path, value: &toml::Value) -> Result<(), String> {
    ensure_parent_dir(path)?;
    let content = toml::to_string_pretty(value).map_err(|e| format!("序列化 TOML 失败: {}", e))?;
    let mut file = fs::File::create(path).map_err(|e| format!("写入配置失败: {}", e))?;
    file.write_all(content.as_bytes())
        .map_err(|e| format!("写入配置失败: {}", e))
}

fn get_base_url_for_cli(kind: CliKind) -> Result<String, String> {
    match kind {
        CliKind::Claude => {
            let settings = read_json_file(&settings_path_for(kind)?, "{}")?;
            Ok(settings
                .get("env")
                .and_then(|env| env.get("ANTHROPIC_BASE_URL"))
                .and_then(|v| v.as_str())
                .unwrap_or("https://api.anthropic.com")
                .to_string())
        }
        CliKind::Codex => {
            let config = read_toml_file(&settings_path_for(kind)?)?;
            let provider_id = config
                .get("model_provider")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .unwrap_or("openai-chat-completions");
            Ok(config
                .get("model_providers")
                .and_then(|v| v.as_table())
                .and_then(|providers| providers.get(provider_id))
                .and_then(|v| v.as_table())
                .and_then(|provider| provider.get("base_url"))
                .and_then(|v| v.as_str())
                .unwrap_or("https://api.openai.com/v1")
                .to_string())
        }
        CliKind::Gemini => Err("Gemini 不支持 API 代理".to_string()),
        CliKind::WorkBuddy => Err("WorkBuddy 不支持 API 代理".to_string()),
        CliKind::Dsh => Err("DSH 不支持 API 代理".to_string()),
        CliKind::Antigravity => Err("Antigravity 不支持 API 代理".to_string()),
    }
}

fn set_base_url_for_cli(kind: CliKind, url: &str) -> Result<(), String> {
    match kind {
        CliKind::Claude => {
            let path = settings_path_for(kind)?;
            let mut settings = read_json_file(&path, "{}")?;
            let env = settings
                .as_object_mut()
                .ok_or("设置不是 JSON 对象")?
                .entry("env")
                .or_insert_with(|| serde_json::json!({}));
            env.as_object_mut().ok_or("env 不是 JSON 对象")?.insert(
                "ANTHROPIC_BASE_URL".to_string(),
                serde_json::Value::String(url.to_string()),
            );
            write_json_file(&path, &settings)
        }
        CliKind::Codex => {
            let path = settings_path_for(kind)?;
            let mut config = read_toml_file(&path)?;
            if !config.is_table() {
                config = toml::Value::Table(toml::map::Map::new());
            }
            let root = config.as_table_mut().ok_or("Codex 配置不是 TOML 表")?;
            let provider_id = root
                .get("model_provider")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .unwrap_or("openai-chat-completions")
                .to_string();
            root.insert(
                "model_provider".to_string(),
                toml::Value::String(provider_id.clone()),
            );

            let providers = root
                .entry("model_providers".to_string())
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            if !providers.is_table() {
                *providers = toml::Value::Table(toml::map::Map::new());
            }
            let providers_table = providers
                .as_table_mut()
                .ok_or("model_providers 不是 TOML 表")?;
            let provider_entry = providers_table
                .entry(provider_id)
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            if !provider_entry.is_table() {
                *provider_entry = toml::Value::Table(toml::map::Map::new());
            }
            let provider_table = provider_entry
                .as_table_mut()
                .ok_or("provider 配置不是 TOML 表")?;
            provider_table.insert("base_url".to_string(), toml::Value::String(url.to_string()));
            provider_table
                .entry("wire_api".to_string())
                .or_insert_with(|| toml::Value::String("responses".to_string()));
            write_toml_file(&path, &config)
        }
        CliKind::Gemini => Err("Gemini 不支持 API 代理".to_string()),
        CliKind::WorkBuddy => Err("WorkBuddy 不支持 API 代理".to_string()),
        CliKind::Dsh => Err("DSH 不支持 API 代理".to_string()),
        CliKind::Antigravity => Err("Antigravity 不支持 API 代理".to_string()),
    }
}

fn proxy_status_from_state(state: ProxyState, running: bool) -> ProxyStatus {
    ProxyStatus {
        enabled: state.enabled,
        running,
        port: state.port,
        ws_port: state.ws_port,
        target_url: state.target_url,
        cli_id: state.cli_id,
    }
}

fn default_proxy_status(kind: CliKind) -> ProxyStatus {
    ProxyStatus {
        enabled: false,
        running: false,
        port: default_port_for(kind),
        ws_port: default_ws_port_for(kind),
        target_url: String::new(),
        cli_id: kind.id().to_string(),
    }
}

fn is_proxy_running(state: &ProxyState) -> bool {
    is_port_listening(state.port)
}

fn ensure_port_available(port: u16, usage: &str) -> Result<(), String> {
    if is_port_listening(port) {
        return Err(format!("端口 {} 已被占用，无法启动{}", port, usage));
    }
    Ok(())
}

fn ensure_proxy_ports_available(kind: CliKind, port: u16, ws_port: u16) -> Result<(), String> {
    ensure_port_available(port, &format!("{} API 代理", kind.name()))?;
    ensure_port_available(ws_port, &format!("{} API 调试 WebSocket", kind.name()))?;
    Ok(())
}

fn require_query_kind(kind: Option<CliKind>, context: &str) -> Result<CliKind, String> {
    kind.ok_or_else(|| format!("{} 缺少 cliId", context))
}

pub fn enable(kind: CliKind, port: u16) -> Result<ProxyStatus, String> {
    tracing::info!("Enabling proxy for {} on port {}", kind.id(), port);

    if let Some(state) = read_proxy_state(kind)? {
        if state.enabled && is_proxy_running(&state) {
            return Ok(proxy_status_from_state(state, true));
        }
        cleanup_dead_proxy(kind, &state);
    }

    let ws_port = port + 1;
    ensure_proxy_ports_available(kind, port, ws_port)?;

    let original_base_url = get_base_url_for_cli(kind)?;
    let original_base_url = if original_base_url.starts_with("http://127.0.0.1:") {
        default_base_url_for(kind)
    } else {
        original_base_url
    };
    validate_url(&original_base_url)?;

    let binary = proxy_binary_path()?;
    let db_path = traffic_db_path()?;
    let db_path_arg = db_path
        .to_str()
        .ok_or("数据库路径含非 UTF-8 字符")?
        .to_string();
    let listen_addr = format!("127.0.0.1:{}", port);
    let log_dir = proxy_log_dir()?;
    let log_dir_arg = log_dir
        .to_str()
        .ok_or("日志目录路径含非 UTF-8 字符")?
        .to_string();

    let state = ProxyState {
        enabled: true,
        port,
        ws_port,
        original_base_url: original_base_url.clone(),
        target_url: original_base_url.clone(),
        cli_id: kind.id().to_string(),
    };

    let proxy_url = format!("http://127.0.0.1:{}", port);
    activate_proxy(kind, &state, &proxy_url)?;

    #[allow(unused_mut)]
    let mut cmd = Command::new(&binary);
    cmd.arg("--listen")
        .arg(&listen_addr)
        .arg("--target")
        .arg(&original_base_url)
        .arg("--ws-port")
        .arg(ws_port.to_string())
        .arg("--db")
        .arg(&db_path_arg)
        .arg("--log-dir")
        .arg(&log_dir_arg)
        .arg("--cli-id")
        .arg(kind.id())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(|e| {
        rollback_proxy(kind, &original_base_url);
        format!("启动代理子进程失败: {}", e)
    })?;
    let child_pid = child.id();

    if let Ok(mut pids) = SUBPROCESS_PIDS.lock() {
        pids.insert(kind.id().to_string(), child_pid);
    }
    write_proxy_pid_file(kind, child_pid);

    let mut ready = false;
    for _ in 0..100 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if is_port_listening(port) {
            ready = true;
            break;
        }
        if let Ok(Some(status)) = child.try_wait() {
            rollback_proxy(kind, &original_base_url);
            return Err(format!("代理子进程提前退出，状态: {}", status));
        }
    }

    if !ready {
        rollback_proxy(kind, &original_base_url);

        let log_dir = proxy_log_dir().unwrap_or_default();
        let stderr_log = log_dir.join("proxy.log");
        let log_msg = fs::read_to_string(&stderr_log)
            .ok()
            .map(|s| {
                let trimmed = if s.len() > 500 {
                    let start = s.len().saturating_sub(500);
                    // Find safe UTF-8 char boundary
                    let safe_start = s.floor_char_boundary(start);
                    &s[safe_start..]
                } else {
                    &s
                };
                trimmed.trim().to_string()
            })
            .unwrap_or_default();

        return Err(format!(
            "代理启动超时，端口未就绪。{}",
            if log_msg.is_empty() {
                String::new()
            } else {
                format!(" 日志: {}", log_msg)
            }
        ));
    }

    tracing::info!(
        "Proxy started successfully on port {} for {}",
        port,
        kind.id()
    );
    Ok(proxy_status_from_state(state, true))
}

/// Stop proxy and rollback settings on failure
fn rollback_proxy(kind: CliKind, original_base_url: &str) {
    kill_subprocess(kind);
    if let Err(e) = set_base_url_for_cli(kind, original_base_url) {
        tracing::error!("Failed to rollback {} base_url: {}", kind.id(), e);
    }
    if let Err(e) = remove_proxy_state(kind) {
        tracing::error!("Failed to remove {} proxy state: {}", kind.id(), e);
    }
}

/// Kill the subprocess by CLI
fn kill_subprocess(kind: CliKind) {
    let mut pid_to_kill = None;
    if let Ok(mut pid_guard) = SUBPROCESS_PIDS.lock() {
        pid_to_kill = pid_guard.remove(kind.id());
    }

    if pid_to_kill.is_none() {
        pid_to_kill = read_proxy_pid_file(kind);
    }

    if let Some(pid) = pid_to_kill {
        if is_proxy_process_alive(pid) {
            terminate_process_best_effort(pid);
        } else {
            tracing::warn!(
                "Skip terminating pid {} for {} because it is not a sessiondock-proxy process",
                pid,
                kind.id()
            );
        }
    }
    remove_proxy_pid_file(kind);
}

fn disable_kind(kind: CliKind) -> Result<bool, String> {
    let Some(state) = read_proxy_state(kind)? else {
        return Ok(false);
    };

    kill_subprocess(kind);
    set_base_url_for_cli(kind, &state.original_base_url)?;
    remove_proxy_state(kind)?;
    tracing::info!("Proxy disabled successfully for {}", kind.id());
    Ok(true)
}

pub fn disable(kind: Option<CliKind>) -> Result<(), String> {
    tracing::info!("Disabling proxy");

    match kind {
        Some(kind) => {
            if disable_kind(kind)? {
                Ok(())
            } else {
                Err("代理未启用".to_string())
            }
        }
        None => {
            let mut disabled_any = false;
            for cli_kind in CliKind::all() {
                disabled_any |= disable_kind(cli_kind)?;
            }
            if disabled_any {
                Ok(())
            } else {
                Err("代理未启用".to_string())
            }
        }
    }
}

fn status_for_kind(kind: CliKind) -> Result<ProxyStatus, String> {
    match read_proxy_state(kind)? {
        Some(state) => Ok(proxy_status_from_state(
            state.clone(),
            is_proxy_running(&state),
        )),
        None => Ok(default_proxy_status(kind)),
    }
}

pub fn status(kind: Option<CliKind>) -> Result<ProxyStatus, String> {
    if let Some(kind) = kind {
        return status_for_kind(kind);
    }

    for cli_kind in CliKind::all() {
        let status = status_for_kind(cli_kind)?;
        if status.enabled || status.running {
            return Ok(status);
        }
    }

    Ok(default_proxy_status(CliKind::Claude))
}

// ─── State consistency check ─────────────────────────────────────────

fn check_consistency_for_kind(kind: CliKind) {
    let state = match read_proxy_state(kind) {
        Ok(Some(s)) => s,
        Ok(None) => return,
        Err(e) => {
            tracing::error!(
                "Proxy consistency: failed to read state for {}: {}",
                kind.id(),
                e
            );
            return;
        }
    };

    if !state.enabled {
        tracing::warn!(
            "Proxy consistency: disabled state file found for {}, cleaning up",
            kind.id()
        );
        cleanup_dead_proxy(kind, &state);
        return;
    }

    if !is_proxy_running(&state) {
        tracing::warn!(
            "Proxy consistency: proxy port not listening for {}, cleaning up",
            kind.id()
        );
        cleanup_dead_proxy(kind, &state);
    }
}

pub fn check_consistency() {
    tracing::info!("Checking proxy state consistency");
    for kind in CliKind::all() {
        check_consistency_for_kind(kind);
    }
}

pub fn cleanup_stale_proxy_processes() {
    for kind in CliKind::all() {
        let Some(pid) = read_proxy_pid_file(kind) else {
            continue;
        };

        if is_proxy_process_alive(pid) {
            tracing::warn!(
                "Found stale proxy process {} for {}, terminating before startup",
                pid,
                kind.id()
            );
            terminate_process_best_effort(pid);
        } else if is_process_alive(pid) {
            tracing::warn!(
                "Stale proxy pid file for {} points to non-proxy process {}, removing pid file only",
                kind.id(),
                pid
            );
        }
        remove_proxy_pid_file(kind);
    }
}

fn cleanup_dead_proxy(kind: CliKind, state: &ProxyState) {
    kill_subprocess(kind);
    if let Err(e) = set_base_url_for_cli(kind, &state.original_base_url) {
        tracing::error!("Failed to restore {} base_url in cleanup: {}", kind.id(), e);
    }
    if let Err(e) = remove_proxy_state(kind) {
        tracing::error!("Failed to remove {} proxy state in cleanup: {}", kind.id(), e);
    }
}

/// Called from lib.rs on app exit to clean up proxy subprocesses
pub fn cleanup_on_exit() {
    for kind in CliKind::all() {
        if let Ok(Some(state)) = read_proxy_state(kind) {
            if state.enabled {
                tracing::info!("App exiting, cleaning up proxy for {}", kind.id());
                let _ = disable(Some(kind));
            }
        }
    }
}

// ─── Traffic queries (read from SQLite) ──────────────────────────────

fn ensure_traffic_columns(conn: &Connection) -> Result<(), String> {
    let column_names = conn
        .prepare("PRAGMA table_info(traffic)")
        .map_err(|e| format!("读取流量表结构失败: {}", e))?
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| format!("读取流量表结构失败: {}", e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("读取流量表结构失败: {}", e))?;

    if !column_names.iter().any(|name| name == "session_id") {
        conn.execute_batch("ALTER TABLE traffic ADD COLUMN session_id TEXT;")
            .map_err(|e| format!("迁移 session_id 字段失败: {}", e))?;
    }
    if !column_names.iter().any(|name| name == "cli_id") {
        conn.execute_batch("ALTER TABLE traffic ADD COLUMN cli_id TEXT DEFAULT 'claude';")
            .map_err(|e| format!("迁移 cli_id 字段失败: {}", e))?;
        conn.execute(
            "UPDATE traffic SET cli_id = 'claude' WHERE cli_id IS NULL OR TRIM(cli_id) = ''",
            [],
        )
        .map_err(|e| format!("回填 cli_id 失败: {}", e))?;
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_traffic_session_timestamp ON traffic(session_id, timestamp);
         CREATE INDEX IF NOT EXISTS idx_traffic_cli_timestamp ON traffic(cli_id, timestamp);",
    )
    .map_err(|e| format!("创建流量索引失败: {}", e))?;
    Ok(())
}

fn open_traffic_db() -> Result<Connection, String> {
    let path = traffic_db_path()?;
    if !path.exists() {
        return Err("流量数据库不存在".to_string());
    }
    let conn = Connection::open(&path).map_err(|e| format!("打开流量数据库失败: {}", e))?;
    // Allow up to 5s for locks held by the proxy subprocess (important on Windows)
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| format!("设置 busy_timeout 失败: {}", e))?;
    // WAL mode allows concurrent reads while proxy is writing
    conn.execute_batch("PRAGMA journal_mode=WAL;")
        .map_err(|e| format!("设置 WAL 模式失败: {}", e))?;
    ensure_traffic_columns(&conn)?;
    Ok(conn)
}

pub fn get_traffic(limit: u32, offset: u32, kind: Option<CliKind>) -> Result<TrafficList, String> {
    let conn = open_traffic_db()?;

    let total: u64 = match kind {
        Some(kind) => conn
            .query_row(
                "SELECT COUNT(*) FROM traffic WHERE cli_id = ?1",
                params![kind.id()],
                |row| row.get(0),
            )
            .map_err(|e| format!("查询总数失败: {}", e))?,
        None => conn
            .query_row("SELECT COUNT(*) FROM traffic", [], |row| row.get(0))
            .map_err(|e| format!("查询总数失败: {}", e))?,
    };

    let (sql, filtered) = match kind {
        Some(_) => (
            "SELECT id, timestamp, method, path, req_size, status, res_size, duration_ms
             FROM traffic
             WHERE cli_id = ?1
             ORDER BY timestamp DESC LIMIT ?2 OFFSET ?3",
            true,
        ),
        None => (
            "SELECT id, timestamp, method, path, req_size, status, res_size, duration_ms
             FROM traffic ORDER BY timestamp DESC LIMIT ?1 OFFSET ?2",
            false,
        ),
    };

    let mut stmt = conn.prepare(sql).map_err(|e| format!("查询失败: {}", e))?;
    let items = if filtered {
        let cli_id = kind.unwrap().id().to_string();
        let rows = stmt
            .query_map(params![cli_id, limit, offset], |row| {
                Ok(TrafficSummary {
                    id: row.get(0)?,
                    timestamp: row.get(1)?,
                    method: row.get(2)?,
                    path: row.get(3)?,
                    req_size: row.get(4)?,
                    status: row.get(5)?,
                    res_size: row.get(6)?,
                    duration_ms: row.get(7)?,
                    input_tokens: None,
                    output_tokens: None,
                    cache_read_tokens: None,
                    cache_creation_tokens: None,
                    tool_names: vec![],
                    has_skill_call: false,
                })
            })
            .map_err(|e| format!("查询失败: {}", e))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("读取行失败: {}", e))?
    } else {
        let rows = stmt
            .query_map(params![limit, offset], |row| {
                Ok(TrafficSummary {
                    id: row.get(0)?,
                    timestamp: row.get(1)?,
                    method: row.get(2)?,
                    path: row.get(3)?,
                    req_size: row.get(4)?,
                    status: row.get(5)?,
                    res_size: row.get(6)?,
                    duration_ms: row.get(7)?,
                    input_tokens: None,
                    output_tokens: None,
                    cache_read_tokens: None,
                    cache_creation_tokens: None,
                    tool_names: vec![],
                    has_skill_call: false,
                })
            })
            .map_err(|e| format!("查询失败: {}", e))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("读取行失败: {}", e))?
    };

    Ok(TrafficList { items, total })
}

pub fn get_detail(id: String) -> Result<TrafficDetail, String> {
    let conn = open_traffic_db()?;
    conn.query_row(
        "SELECT id, timestamp, method, path, req_headers, req_body, req_size, status, res_headers, res_body, res_size, duration_ms
         FROM traffic WHERE id = ?1",
        params![id],
        |row| {
            Ok(TrafficDetail {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                method: row.get(2)?,
                path: row.get(3)?,
                req_headers: row.get(4)?,
                req_body: row.get(5)?,
                req_size: row.get(6)?,
                status: row.get(7)?,
                res_headers: row.get(8)?,
                res_body: row.get(9)?,
                res_size: row.get(10)?,
                duration_ms: row.get(11)?,
            })
        },
    )
    .map_err(|e| format!("查询详情失败: {}", e))
}

pub fn clear_traffic(
    before_days: Option<u32>,
    kind: Option<CliKind>,
) -> Result<ClearResult, String> {
    let conn = open_traffic_db()?;

    let deleted = match (before_days, kind) {
        (Some(days), Some(kind)) => {
            let cutoff = chrono::Utc::now() - chrono::Duration::days(days as i64);
            let cutoff_str = cutoff.to_rfc3339();
            conn.execute(
                "DELETE FROM traffic WHERE timestamp < ?1 AND cli_id = ?2",
                params![cutoff_str, kind.id()],
            )
            .map_err(|e| format!("清理失败: {}", e))?
        }
        (Some(days), None) => {
            let cutoff = chrono::Utc::now() - chrono::Duration::days(days as i64);
            let cutoff_str = cutoff.to_rfc3339();
            conn.execute(
                "DELETE FROM traffic WHERE timestamp < ?1",
                params![cutoff_str],
            )
            .map_err(|e| format!("清理失败: {}", e))?
        }
        (None, Some(kind)) => conn
            .execute("DELETE FROM traffic WHERE cli_id = ?1", params![kind.id()])
            .map_err(|e| format!("清理失败: {}", e))?,
        (None, None) => conn
            .execute("DELETE FROM traffic", [])
            .map_err(|e| format!("清理失败: {}", e))?,
    };

    conn.execute_batch("VACUUM")
        .map_err(|e| format!("VACUUM 失败: {}", e))?;

    let db_size = db_size_inner()?;

    Ok(ClearResult {
        deleted: deleted as u64,
        db_size,
    })
}

/// 返回指定会话的所有流量记录时间戳（仅 API 请求，排除 count_tokens 等辅助路径）。
/// 前端用这些时间戳做 120s 近邻匹配，只在有对应流量的消息上显示 ⚡ 按钮。
pub fn session_traffic_timestamps(session_id: &str, kind: CliKind) -> Vec<String> {
    let conn = match open_traffic_db() {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    let path_filter = match kind {
        CliKind::Claude => "(path LIKE '/v1/messages%' AND path NOT LIKE '/v1/messages/count_tokens%')",
        CliKind::Codex => "(path LIKE '/v1/responses%' OR path LIKE '/v1/chat/completions%' OR path LIKE '/chat/completions%' OR path LIKE '/responses%')",
        CliKind::Gemini | CliKind::WorkBuddy | CliKind::Dsh | CliKind::Antigravity => return vec![],
    };
    let sql = format!(
        "SELECT timestamp FROM traffic WHERE session_id = ?1 AND cli_id = ?2 AND {path_filter} ORDER BY timestamp"
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    stmt.query_map(rusqlite::params![session_id, kind.id()], |row| row.get::<_, String>(0))
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
}

pub fn db_size() -> Result<u64, String> {
    db_size_inner()
}

fn db_size_inner() -> Result<u64, String> {
    let path = traffic_db_path()?;
    if !path.exists() {
        return Ok(0);
    }
    let metadata = fs::metadata(&path).map_err(|e| format!("获取文件大小失败: {}", e))?;
    Ok(metadata.len())
}

// ─── Session-grouped queries ─────────────────────────────────────────

pub fn get_traffic_sessions(
    limit: u32,
    offset: u32,
    kind: Option<CliKind>,
) -> Result<SessionTrafficList, String> {
    let conn = open_traffic_db()?;
    let query_kind = require_query_kind(kind, "按会话查看代理流量")?;

    let total: u64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT session_id)
             FROM traffic
             WHERE session_id IS NOT NULL
               AND cli_id = ?1",
            params![query_kind.id()],
            |row| row.get(0),
        )
        .map_err(|e| format!("查询总数失败: {}", e))?;

    let mut stmt = conn
        .prepare(
            "SELECT session_id,
                    COUNT(*) as request_count,
                    SUM(req_size) as total_req_size,
                    SUM(res_size) as total_res_size,
                    SUM(duration_ms) as total_duration_ms,
                    MIN(timestamp) as first_timestamp,
                    MAX(timestamp) as last_timestamp,
                    SUM(CASE WHEN status >= 200 AND status < 300 THEN 1 ELSE 0 END) as ok_count,
                    SUM(CASE WHEN status >= 400 THEN 1 ELSE 0 END) as error_count
             FROM traffic
             WHERE session_id IS NOT NULL
               AND cli_id = ?1
             GROUP BY session_id
             ORDER BY MAX(timestamp) DESC
             LIMIT ?2 OFFSET ?3",
        )
        .map_err(|e| format!("查询失败: {}", e))?;

    let rows = stmt
        .query_map(params![query_kind.id(), limit, offset], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, u64>(7)?,
                row.get::<_, u64>(8)?,
            ))
        })
        .map_err(|e| format!("查询失败: {}", e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("读取行失败: {}", e))?;

    let (display_map, session_project_map) = load_session_metadata(query_kind)?;

    let mut items: Vec<SessionTrafficSummary> = rows
        .into_iter()
        .map(
            |(
                session_id,
                request_count,
                total_req_size,
                total_res_size,
                total_duration_ms,
                first_timestamp,
                last_timestamp,
                ok_count,
                error_count,
            )| {
                let (display_name, project_path) = match &session_id {
                    Some(sid) => {
                        let display = display_map
                            .get(sid)
                            .cloned()
                            .unwrap_or_else(|| format!("会话 {}", &sid[..8.min(sid.len())]));
                        let project = session_project_map.get(sid).cloned().unwrap_or_default();
                        (display, project)
                    }
                    None => ("未分组请求".to_string(), String::new()),
                };

                SessionTrafficSummary {
                    session_id,
                    display_name,
                    project_path,
                    request_count,
                    total_req_size,
                    total_res_size,
                    total_duration_ms,
                    first_timestamp,
                    last_timestamp,
                    ok_count,
                    error_count,
                }
            },
        )
        .collect();

    if offset == 0 {
        let null_count: u64 = conn
            .query_row(
                "SELECT COUNT(*)
                 FROM traffic
                 WHERE session_id IS NULL
                   AND cli_id = ?1",
                params![query_kind.id()],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if null_count > 0 {
            let null_row: Result<(i64, i64, i64, i64, String, String, u64, u64), _> = conn
                .query_row(
                    "SELECT SUM(req_size), SUM(res_size), SUM(duration_ms), COUNT(*),
                        MIN(timestamp), MAX(timestamp),
                        SUM(CASE WHEN status >= 200 AND status < 300 THEN 1 ELSE 0 END),
                        SUM(CASE WHEN status >= 400 THEN 1 ELSE 0 END)
                 FROM traffic
                 WHERE session_id IS NULL
                   AND cli_id = ?1",
                    params![query_kind.id()],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                            row.get(6)?,
                            row.get(7)?,
                        ))
                    },
                );
            if let Ok((total_req, total_res, total_dur, cnt, first_ts, last_ts, ok_cnt, err_cnt)) =
                null_row
            {
                items.push(SessionTrafficSummary {
                    session_id: None,
                    display_name: "未关联会话".to_string(),
                    project_path: String::new(),
                    request_count: cnt as u64,
                    total_req_size: total_req,
                    total_res_size: total_res,
                    total_duration_ms: total_dur,
                    first_timestamp: first_ts,
                    last_timestamp: last_ts,
                    ok_count: ok_cnt,
                    error_count: err_cnt,
                });
            }
        }
    }

    Ok(SessionTrafficList { items, total })
}

/// 加载会话展示元数据：sid → 展示名（与会话列表同一套 resolve_display_name 解析链）、sid → 项目路径。
fn load_session_metadata(
    kind: CliKind,
) -> Result<(HashMap<String, String>, HashMap<String, String>), String> {
    match kind {
        CliKind::Claude | CliKind::Codex => {
            let history_path = cli::history_path(kind)?;
            // history.jsonl 只解析一次，同时拿到 display 兜底 map 与项目路径 map
            // （此前 Claude 分支解析了两遍，sessions 全量拉取时 CPU 翻倍）
            let (history_map, history_project_map) = match kind {
                CliKind::Claude => history::parse_history(history_path.to_str().unwrap_or("")),
                _ => (
                    history::parse_codex_history(history_path.to_str().unwrap_or("")),
                    HashMap::new(),
                ),
            };
            let mut project_map = match kind {
                CliKind::Claude => build_claude_session_project_map(
                    &cli::sessions_dir(kind)?,
                    &history_project_map,
                ),
                _ => build_codex_session_project_map(&cli::sessions_dir(kind)?),
            };

            // 会话列表索引库：自定义重命名 / 原生标题 / 清洗后首条用户消息 / 项目路径
            let records = crate::app_db::read_session_list_index(kind).unwrap_or_default();
            let custom_names = crate::app_db::load_session_names().unwrap_or_default();
            let mut display_map = HashMap::new();
            for record in records.values() {
                let sid = record.session_id.trim();
                if sid.is_empty() {
                    continue;
                }
                let display = crate::db::title_resolver::resolve_display_name(
                    crate::commands::session::custom_session_name(
                        &custom_names,
                        kind,
                        &record.session_path,
                    ),
                    record.title.as_deref(),
                    record.first_user_message.as_deref(),
                    history_map
                        .get(sid)
                        .map(|h| h.display.as_str())
                        .filter(|v| !v.is_empty()),
                    sid,
                );
                display_map.insert(sid.to_string(), display);
                if let Some(p) = record.project_path.as_ref().filter(|p| !p.trim().is_empty()) {
                    project_map.insert(sid.to_string(), p.clone());
                }
            }

            // 兜底：对仅在 history_map 中出现（索引库尚未收录）的 session，使用 history display 解析
            for (sid, hist) in &history_map {
                if !display_map.contains_key(sid) {
                    let display = crate::db::title_resolver::resolve_display_name(
                        None,
                        None,
                        None,
                        Some(hist.display.as_str()).filter(|v| !v.is_empty()),
                        sid,
                    );
                    display_map.insert(sid.clone(), display);
                }
            }

            Ok((display_map, project_map))
        }
        CliKind::Gemini => Ok((HashMap::new(), HashMap::new())),
        CliKind::WorkBuddy => Ok((HashMap::new(), HashMap::new())),
        CliKind::Dsh => Ok((HashMap::new(), HashMap::new())),
        CliKind::Antigravity => Ok((HashMap::new(), HashMap::new())),
    }
}

fn collect_jsonl_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl_files(&path, files)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            files.push(path);
        }
    }
    Ok(())
}

fn build_claude_session_project_map(
    projects_dir: &Path,
    project_map: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Ok(entries) = fs::read_dir(projects_dir) {
        for entry in entries.flatten() {
            let dir_name = entry.file_name().to_string_lossy().to_string();
            let dir_path = entry.path();
            if !dir_path.is_dir() {
                continue;
            }
            let fallback_project_path =
                session::resolve_project_path(&dir_name, None, Some(project_map));
            if let Ok(files) = fs::read_dir(&dir_path) {
                for file in files.flatten() {
                    let fname = file.file_name().to_string_lossy().to_string();
                    if let Some(session_id) = fname.strip_suffix(".jsonl") {
                        let file_path = file.path();
                        let project_path = file_path
                            .to_str()
                            .and_then(parser::read_project_path)
                            .map(|value| value.trim().to_string())
                            .filter(|value| !value.is_empty())
                            .or_else(|| {
                                session::find_project_path_in_map(&dir_name, project_map)
                                    .map(|value| value.to_string())
                            })
                            .unwrap_or_else(|| fallback_project_path.clone());
                        map.insert(session_id.to_string(), project_path);
                    }
                }
            }
        }
    }
    map
}

fn build_codex_session_project_map(sessions_dir: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut files = Vec::new();
    if collect_jsonl_files(sessions_dir, &mut files).is_err() {
        return map;
    }

    for session_path in files {
        let file_path = match session_path.to_str() {
            Some(value) => value,
            None => continue,
        };
        let Some(session_id) = parser::read_session_id(file_path) else {
            continue;
        };
        let project_path = parser::read_project_path(file_path).unwrap_or_default();
        map.insert(session_id, project_path);
    }

    map
}

/// 从 SSE 响应体提取 token usage。
/// 兼容 Anthropic（message_start / message_delta）、OpenAI chat/completions（usage）
/// 与 Codex responses API（response.completed → response.usage）。
fn extract_sse_usage(res_body: &str) -> (Option<i64>, Option<i64>, Option<i64>, Option<i64>) {
    let mut input = None;
    let mut output = None;
    let mut cache_read = None;
    let mut cache_creation = None;
    for line in res_body.lines() {
        let Some(json) = line.strip_prefix("data: ") else {
            continue;
        };
        if !json.contains("usage") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
            continue;
        };
        // Anthropic message_start: .message.usage（输入侧 + cache）
        if let Some(u) = v.get("message").and_then(|m| m.get("usage")) {
            input = u.get("input_tokens").and_then(|x| x.as_i64()).or(input);
            cache_read = u
                .get("cache_read_input_tokens")
                .and_then(|x| x.as_i64())
                .or(cache_read);
            cache_creation = u
                .get("cache_creation_input_tokens")
                .and_then(|x| x.as_i64())
                .or(cache_creation);
        }
        // Anthropic message_delta: 顶层 .usage（output_tokens 累计，取最后一条）
        // OpenAI chat/completions: .usage（prompt/completion_tokens）
        if let Some(u) = v.get("usage") {
            if let Some(o) = u.get("output_tokens").and_then(|x| x.as_i64()) {
                output = Some(o);
            }
            if let Some(i) = u.get("prompt_tokens").and_then(|x| x.as_i64()) {
                input = Some(i);
            }
            if let Some(o) = u.get("completion_tokens").and_then(|x| x.as_i64()) {
                output = Some(o);
            }
        }
        // Codex responses API: response.completed → .response.usage
        if let Some(u) = v.get("response").and_then(|r| r.get("usage")) {
            input = u.get("input_tokens").and_then(|x| x.as_i64()).or(input);
            if let Some(o) = u.get("output_tokens").and_then(|x| x.as_i64()) {
                output = Some(o);
            }
            cache_read = u
                .pointer("/input_tokens_details/cached_tokens")
                .and_then(|x| x.as_i64())
                .or(cache_read);
        }
    }
    (input, output, cache_read, cache_creation)
}

pub fn get_session_traffic(
    session_id: Option<String>,
    limit: u32,
    offset: u32,
    kind: Option<CliKind>,
    search: Option<String>,
    presets: Option<Vec<String>>,
    tool_names: Option<Vec<String>>,
) -> Result<TrafficList, String> {
    let conn = open_traffic_db()?;
    let query_kind = require_query_kind(kind, "查看单个会话代理流量")?;

    // LIKE 预转义（ESCAPE '\'），None 时绑定 NULL 使条件短路
    let search_pattern = search
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(|s| {
            let escaped = s
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            format!("%{}%", escaped)
        });
    let preset_list = presets.unwrap_or_default();
    let has_tools = preset_list.iter().any(|p| p == "has_tools") as i32;
    let has_system = preset_list.iter().any(|p| p == "has_system") as i32;
    let has_thinking = preset_list.iter().any(|p| p == "has_thinking") as i32;
    let has_tool_result = preset_list.iter().any(|p| p == "has_tool_result") as i32;
    let has_image = preset_list.iter().any(|p| p == "has_image") as i32;
    let has_subagent = preset_list.iter().any(|p| p == "has_subagent") as i32;
    let has_session_ref = preset_list.iter().any(|p| p == "has_session_ref") as i32;

    // 工具名过滤（值来自库内提取，仍做单引号转义防御）；未选择时子句恒真
    let tool_names: Vec<String> = tool_names
        .unwrap_or_default()
        .into_iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    let tool_clause = if tool_names.is_empty() {
        "1=1".to_string()
    } else {
        let quoted = tool_names
            .iter()
            .map(|t| format!("'{}'", t.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "(json_valid(req_body) AND EXISTS (SELECT 1 FROM json_each(req_body, '$.tools') t \
             WHERE COALESCE(json_extract(t.value, '$.name'), json_extract(t.value, '$.function.name')) IN ({quoted})))"
        )
    };

    // :sid 为 NULL 时匹配未分组记录（session_id IS NULL）
    // json_valid 前置保护：空 body / 非 JSON body（如错误页）会让 JSON 函数抛 malformed JSON
    let where_sql = format!("FROM traffic
         WHERE ((:sid IS NULL AND session_id IS NULL) OR session_id = :sid)
           AND cli_id = :cli
           AND (:search IS NULL OR req_body LIKE :search ESCAPE '\\' OR res_body LIKE :search ESCAPE '\\')
           AND (:tools = 0 OR (json_valid(req_body) AND json_array_length(req_body, '$.tools') > 0))
           AND (:system = 0 OR (json_valid(req_body) AND (
                json_type(req_body, '$.system') IS NOT NULL
                OR json_type(req_body, '$.instructions') IS NOT NULL
                OR EXISTS (SELECT 1 FROM json_each(req_body, '$.messages') m
                           WHERE json_extract(m.value, '$.role') = 'system'))))
           AND (:thinking = 0 OR (json_valid(req_body) AND json_type(req_body, '$.thinking') IS NOT NULL))
           AND (:tool_result = 0 OR req_body LIKE '%\"tool_result\"%')
           AND (:image = 0 OR req_body LIKE '%\"type\":\"image\"%' OR req_body LIKE '%\"type\": \"image\"%' OR req_body LIKE '%image_url%')
           AND (:subagent = 0 OR (json_valid(req_body) AND EXISTS (
                SELECT 1 FROM json_each(req_body, '$.tools') t
                WHERE COALESCE(json_extract(t.value, '$.name'), json_extract(t.value, '$.function.name')) = 'Task')))
           AND (:session_ref = 0 OR req_body LIKE '%session_id%')
           AND {tool_clause}"
    );

    let count_sql = format!("SELECT COUNT(*) {where_sql}");
    // tool_names: 单条 json_each 子查询一次解析完成提取（带 json_valid 保护）；
    // has_skill_call: 纯 instr 文本检测，无需 JSON 解析
    // res_body 不整列读入：usage 信息只分布在流的头（Anthropic message_start）与
    // 尾（message_delta / OpenAI 末块 / Codex response.completed），用头 8KB + 尾 256KB
    // 窗口拼接喂给 extract_sse_usage，避免长 SSE 响应（可达数百 MB）整行物化进内存。
    // 头尾之间补换行防止拼出伪行；body 短于窗口时两段重复，重复解析同值幂等无害。
    let query_sql = format!(
        "SELECT id, timestamp, method, path, req_size, status, res_size, duration_ms,
                (substr(res_body, 1, 8192) || char(10) || substr(res_body, -262144)),
                CASE WHEN req_size < 300000 AND json_valid(req_body) AND json_type(req_body, '$.tools') = 'array'
                     THEN (SELECT json_group_array(name) FROM (
                             SELECT DISTINCT COALESCE(json_extract(t.value, '$.name'), json_extract(t.value, '$.function.name')) AS name
                             FROM json_each(req_body, '$.tools') t WHERE name IS NOT NULL))
                     ELSE '[]' END,
                (instr(req_body, '\"name\":\"Skill\"') > 0 OR instr(req_body, '\"name\": \"Skill\"') > 0)
         {where_sql}
         ORDER BY timestamp DESC LIMIT :limit OFFSET :offset"
    );

    let total: u64 = conn
        .query_row(
            &count_sql,
            rusqlite::named_params! {
                ":sid": session_id,
                ":cli": query_kind.id(),
                ":search": search_pattern,
                ":tools": has_tools,
                ":system": has_system,
                ":thinking": has_thinking,
                ":tool_result": has_tool_result,
                ":image": has_image,
                ":subagent": has_subagent,
                ":session_ref": has_session_ref,
            },
            |row| row.get(0),
        )
        .map_err(|e| format!("查询总数失败: {}", e))?;

    let mut stmt = conn
        .prepare(&query_sql)
        .map_err(|e| format!("查询失败: {}", e))?;
    let rows = stmt
        .query_map(
            rusqlite::named_params! {
                ":sid": session_id,
                ":cli": query_kind.id(),
                ":search": search_pattern,
                ":tools": has_tools,
                ":system": has_system,
                ":thinking": has_thinking,
                ":tool_result": has_tool_result,
                ":image": has_image,
                ":subagent": has_subagent,
                ":session_ref": has_session_ref,
                ":limit": limit,
                ":offset": offset,
            },
            |row| {
                let res_body: Option<String> = row.get(8)?;
                let (input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens) =
                    res_body
                        .as_deref()
                        .map(extract_sse_usage)
                        .unwrap_or((None, None, None, None));
                let tool_names_json: String = row.get(9)?;
                let tool_names: Vec<String> =
                    serde_json::from_str(&tool_names_json).unwrap_or_default();
                let has_skill_call: bool = row.get(10)?;
                Ok(TrafficSummary {
                    id: row.get(0)?,
                    timestamp: row.get(1)?,
                    method: row.get(2)?,
                    path: row.get(3)?,
                    req_size: row.get(4)?,
                    status: row.get(5)?,
                    res_size: row.get(6)?,
                    duration_ms: row.get(7)?,
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                    cache_creation_tokens,
                    tool_names,
                    has_skill_call,
                })
            },
        )
        .map_err(|e| format!("查询失败: {}", e))?;
    let items = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("读取行失败: {}", e))?;

    Ok(TrafficList { items, total })
}

/// Find the closest traffic record matching a session_id and timestamp.
/// Used to link a ChatView assistant message to its corresponding API request.
pub fn find_traffic_by_timestamp(
    session_id: &str,
    timestamp: &str,
    kind: Option<CliKind>,
) -> Result<Option<TrafficDetail>, String> {
    let conn = match open_traffic_db() {
        Ok(c) => c,
        Err(_) => return Ok(None),
    };

    let query_kind = require_query_kind(kind, "按时间戳查找代理请求")?;
    let path_filter = match query_kind {
        CliKind::Claude => "(path LIKE '/v1/messages%' AND path NOT LIKE '/v1/messages/count_tokens%')",
        CliKind::Codex => "(path LIKE '/v1/responses%' OR path LIKE '/v1/chat/completions%' OR path LIKE '/chat/completions%' OR path LIKE '/responses%')",
        CliKind::Gemini => "1=0",
        CliKind::WorkBuddy => "1=0",
        CliKind::Dsh => "1=0",
        CliKind::Antigravity => "1=0",
    };
    let sql = format!(
        "SELECT id, timestamp, method, path, req_headers, req_body, req_size, status, res_headers, res_body, res_size, duration_ms
         FROM traffic
         WHERE session_id = ?1
           AND cli_id = ?2
           AND {path_filter}
           AND ABS(julianday(timestamp) - julianday(?3)) < (120.0 / 86400.0)
         ORDER BY ABS(julianday(timestamp) - julianday(?3))
         LIMIT 1"
    );

    let result = conn.query_row(
        &sql,
        params![session_id, query_kind.id(), timestamp],
        |row| {
            Ok(TrafficDetail {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                method: row.get(2)?,
                path: row.get(3)?,
                req_headers: row.get(4)?,
                req_body: row.get(5)?,
                req_size: row.get(6)?,
                status: row.get(7)?,
                res_headers: row.get(8)?,
                res_body: row.get(9)?,
                res_size: row.get(10)?,
                duration_ms: row.get(11)?,
            })
        },
    );

    match result {
        Ok(detail) => Ok(Some(detail)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(format!("查询失败: {}", e)),
    }
}

/// Backfill session_id for existing records that have req_body but no session_id
#[allow(dead_code)]
pub fn backfill_session_ids() -> Result<u64, String> {
    let conn = open_traffic_db()?;

    let mut stmt = conn
        .prepare(
            "SELECT id, req_body FROM traffic WHERE session_id IS NULL AND req_body IS NOT NULL",
        )
        .map_err(|e| format!("查询失败: {}", e))?;

    let rows: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| format!("查询失败: {}", e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("读取行失败: {}", e))?;

    let mut updated = 0u64;
    for (id, req_body) in &rows {
        if let Some(session_id) = extract_session_id_from_body(req_body) {
            conn.execute(
                "UPDATE traffic SET session_id = ?1 WHERE id = ?2",
                params![session_id, id],
            )
            .map_err(|e| format!("更新失败: {}", e))?;
            updated += 1;
        }
    }

    Ok(updated)
}

/// Extract session UUID from request body (same logic as proxy binary)
#[allow(dead_code)]
fn extract_session_id_from_body(req_body: &str) -> Option<String> {
    let val: serde_json::Value = serde_json::from_str(req_body).ok()?;
    let user_id = val.get("metadata")?.get("user_id")?.as_str()?;

    // Windows format: user_id is a JSON string containing session_id key
    if user_id.trim_start().starts_with('{') {
        let inner: serde_json::Value = serde_json::from_str(user_id).ok()?;
        return inner.get("session_id")?.as_str().map(|s| s.to_string());
    }

    // macOS format: user_<hash>_account__session_<UUID>
    let marker = "_session_";
    let pos = user_id.find(marker)?;
    let uuid_part = &user_id[pos + marker.len()..];
    if uuid_part.len() >= 36 && uuid_part.as_bytes()[8] == b'-' {
        Some(uuid_part[..36].to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod load_session_metadata_smoke {
    use super::*;

    #[test]
    fn load_session_metadata_smoke() {
        // 冒烟：真实环境缺 GLOBAL_POOL 时应走容错路径而非 panic
        let r = load_session_metadata(CliKind::Claude);
        assert!(r.is_ok());
    }
}
