use base64::{engine::general_purpose::STANDARD, Engine as _};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{LazyLock, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(unix)]
use std::sync::OnceLock;
use std::time::Duration;
use std::time::Instant;
use tauri::{Emitter, Manager, UserAttentionType};

// ---------------------------------------------------------------------------
// Shell environment loading (inspired by AionUi)
// ---------------------------------------------------------------------------

/// Cached shell environment (loaded once per process).
#[cfg(unix)]
static SHELL_ENV_CACHE: OnceLock<HashMap<String, String>> = OnceLock::new();
static STOP_GUARDIAN: AtomicBool = AtomicBool::new(false);

/// Resolve the user's login shell path.
/// On macOS: uses `dscl` to query Directory Service (reliable even from Finder).
/// On Linux: reads `/etc/passwd` via `getent`.
/// Falls back to $SHELL or /bin/zsh (macOS) / /bin/bash (Linux).
#[cfg(unix)]
fn resolve_login_shell() -> String {
    #[cfg(target_os = "macos")]
    {
        let username = std::env::var("USER").unwrap_or_default();
        if !username.is_empty() {
            if let Ok(output) = std::process::Command::new("dscl")
                .args([".", "-read", &format!("/Users/{}", username), "UserShell"])
                .output()
            {
                let text = String::from_utf8_lossy(&output.stdout);
                if let Some(shell) = text.trim().split_whitespace().last() {
                    if shell.starts_with('/') {
                        return shell.to_string();
                    }
                }
            }
        }
        return "/bin/zsh".to_string();
    }
    #[cfg(not(target_os = "macos"))]
    {
        let username = std::env::var("USER").unwrap_or_default();
        if !username.is_empty() {
            if let Ok(output) = std::process::Command::new("getent")
                .args(["passwd", &username])
                .output()
            {
                let text = String::from_utf8_lossy(&output.stdout);
                if let Some(shell) = text.trim().rsplit(':').next() {
                    if shell.starts_with('/') {
                        return shell.to_string();
                    }
                }
            }
        }
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
    }
}

/// Load environment variables from the user's login shell.
///
/// Strategy:
/// - zsh: use `-i -l` so both .zprofile and .zshrc are sourced. This is
///   necessary because macOS users commonly put PATH exports in .zshrc.
///   Safe here because we use Command::output() with no inherited tty/pty,
///   so the shell cannot call tcsetpgrp() on the app's terminal.
/// - bash/other: use `-l` only (.bashrc is for non-login shells and has
///   different semantics; sourcing it here could cause unexpected side effects).
///
/// stdout is guarded by begin/end markers so that any output from startup
/// files (banners, plugin messages, etc.) does not corrupt the env map.
/// A 5-second timeout prevents a hanging .zshrc from blocking app startup.
#[cfg(unix)]
fn load_shell_environment() -> HashMap<String, String> {
    const TIMEOUT: Duration = Duration::from_secs(5);
    const MARKER_BEGIN: &str = "__CLAUDIA_ENV_BEGIN__";
    const MARKER_END: &str = "__CLAUDIA_ENV_END__";

    let mut result = HashMap::new();
    let shell = resolve_login_shell();
    let shell_name = std::path::Path::new(&shell)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    tracing::info!("[shell_env] using shell: {}", shell);
    tracing::info!("[shell_env] process PATH before load: {}", std::env::var("PATH").unwrap_or_default());

    let args: Vec<&str> = match shell_name {
        // zsh: -i loads .zshrc (where PATH often lives on macOS).
        // Safe with no tty — zsh will not steal the foreground process group.
        "zsh" | "fish" => vec!["-i", "-l", "-c"],
        // bash/other: -l is enough; .bashrc is for non-login interactive shells.
        _ => vec!["-l", "-c"],
    };

    let env_cmd = format!(
        "printf '{}\\n'; /usr/bin/env; printf '{}\\n'",
        MARKER_BEGIN, MARKER_END
    );

    let mut child = match std::process::Command::new(&shell)
        .args(&args)
        .arg(&env_cmd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .envs(std::env::vars())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("[shell_env] failed to spawn shell: {}", e);
            return result;
        }
    };

    // Wait with timeout — kill if shell hangs.
    let deadline = std::time::Instant::now() + TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    tracing::warn!("[shell_env] shell timed out after {}s, killing", TIMEOUT.as_secs());
                    let _ = child.kill();
                    let _ = child.wait();
                    return result;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                tracing::error!("[shell_env] wait error: {}", e);
                return result;
            }
        }
    }

    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => {
            tracing::error!("[shell_env] failed to read output: {}", e);
            return result;
        }
    };

    if !output.stderr.is_empty() {
        tracing::debug!("[shell_env] shell stderr: {}", String::from_utf8_lossy(&output.stderr).trim());
    }

    // Only parse lines between markers to ignore startup file output.
    let text = String::from_utf8_lossy(&output.stdout);
    let mut in_block = false;
    for line in text.lines() {
        if line == MARKER_BEGIN {
            in_block = true;
            continue;
        }
        if line == MARKER_END {
            break;
        }
        if !in_block {
            continue;
        }
        if let Some(eq_pos) = line.find('=') {
            let key = &line[..eq_pos];
            let value = &line[eq_pos + 1..];
            result.insert(key.to_string(), value.to_string());
        }
    }

    tracing::info!(
        "[shell_env] loaded {} vars, PATH from shell: {}",
        result.len(),
        result.get("PATH").map(|s| s.as_str()).unwrap_or("(none)")
    );

    result
}

/// Merge two PATH strings, removing duplicates while preserving order.
#[cfg(unix)]
fn merge_paths(path1: &str, path2: &str) -> String {
    let sep = ':';
    let mut seen = std::collections::HashSet::new();
    let mut merged = Vec::new();

    for p in path1.split(sep).chain(path2.split(sep)) {
        if !p.is_empty() && seen.insert(p.to_string()) {
            merged.push(p.to_string());
        }
    }

    merged.join(":")
}

/// Scan well-known POSIX tool directories and return any that exist
/// but are not already in the given PATH.
#[cfg(unix)]
fn get_posix_extra_tool_paths(current_path: &str) -> Vec<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let candidates = [
        format!("{}/.cargo/bin", home),
        format!("{}/.bun/bin", home),
        format!("{}/go/bin", home),
        format!("{}/.deno/bin", home),
        format!("{}/.local/bin", home),
    ];

    candidates
        .into_iter()
        .filter(|p| {
            std::path::Path::new(p).exists() && !current_path.contains(p.as_str())
        })
        .collect()
}

/// Get enhanced environment variables by merging shell env with process env.
/// For PATH: merges both sources + appends well-known tool directories.
fn get_enhanced_env() -> HashMap<String, String> {
    #[cfg(unix)]
    {
        let shell_env = SHELL_ENV_CACHE.get_or_init(load_shell_environment);

        let mut env: HashMap<String, String> = std::env::vars().collect();

        // Overlay shell env vars (these may be missing from Finder-launched apps)
        for (key, value) in shell_env {
            if key == "PATH" {
                continue; // PATH is merged separately below
            }
            env.entry(key.clone()).or_insert_with(|| value.clone());
        }

        // Merge PATH: process.env.PATH + shell PATH + extra tool paths
        let process_path = std::env::var("PATH").unwrap_or_default();
        let shell_path = shell_env.get("PATH").cloned().unwrap_or_default();
        let mut merged_path = merge_paths(&process_path, &shell_path);

        let extra_paths = get_posix_extra_tool_paths(&merged_path);
        if !extra_paths.is_empty() {
            merged_path = merge_paths(&merged_path, &extra_paths.join(":"));
        }

        env.insert("PATH".to_string(), merged_path);
        env
    }
    #[cfg(windows)]
    {
        std::env::vars().collect()
    }
}

fn get_agent_status_mode(agent_status_mode: Option<String>) -> crate::claude_hooks::HookMode {
    agent_status_mode
        .as_deref()
        .and_then(|s| match s {
            "osc" => Some(crate::claude_hooks::HookMode::Osc),
            "hook-relay" => Some(crate::claude_hooks::HookMode::HookRelay),
            _ => None,
        })
        .unwrap_or(crate::claude_hooks::HookMode::Osc)
}

static PTY_SESSIONS: LazyLock<Mutex<HashMap<String, PtySessionHandle>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn lock_sessions() -> std::sync::MutexGuard<'static, HashMap<String, PtySessionHandle>> {
    PTY_SESSIONS.lock().unwrap_or_else(|e| e.into_inner())
}

struct PtySessionHandle {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Option<Box<dyn portable_pty::Child + Send>>,
    info: PtySessionInfo,
    #[allow(dead_code)]
    last_output_at: Instant,
    settings_file_path: Option<String>,
    #[allow(dead_code)]
    claude_hook_runtime: Option<crate::claude_hooks::ClaudeHookRuntime>,
}

impl Drop for PtySessionHandle {
    fn drop(&mut self) {
        if let Some(ref path) = self.settings_file_path {
            let _ = std::fs::remove_file(path);
        }
        if let Some(mut child) = self.child.take() {
            let mut killer = child.clone_killer();
            let _ = child.kill();
            let (done_tx, done_rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = child.wait();
                let _ = done_tx.send(());
            });
            std::thread::spawn(move || {
                let wait_started = Instant::now();
                if done_rx.recv_timeout(Duration::from_secs(2)).is_err()
                    && should_force_kill_after(wait_started, Instant::now(), Duration::from_secs(2))
                {
                    let _ = killer.kill();
                }
            });
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PtySessionInfo {
    pub session_id: String,
    pub project_path: String,
    pub cli_kind: String,
    pub created_at: String,
    pub status: PtyStatus,
    pub status_source: crate::agent_status::PtyStatusSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PtyStatus {
    Active,
    Idle,
    WaitingInput,
    Exited,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PtyStatusChangedPayload {
    pub session_id: String,
    pub status: PtyStatus,
}

pub fn create_session(
    app: tauri::AppHandle,
    project_path: String,
    cli_kind: String,
    session_id: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
    resume_session_id: Option<String>,
    skip_permissions: bool,
    settings_file: Option<String>,
    agent_status_mode: Option<String>,
) -> Result<PtySessionInfo, String> {
    let id = session_id.unwrap_or_else(|| nanoid::nanoid!(10));
    #[cfg_attr(not(any(unix, windows)), allow(unused_mut))]
    let mut status_source = crate::agent_status::PtyStatusSource::FrontendFallback;
    #[cfg(any(unix, windows))]
    let mut claude_hook_runtime = None;

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: rows.unwrap_or(24),
            cols: cols.unwrap_or(220),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("无法创建 PTY: {}", e))?;

    let enhanced_env = get_enhanced_env();

    let cmd = if cli_kind == "shell" || cli_kind == "terminal" || cli_kind.is_empty() {
        #[cfg(unix)]
        {
            let shell = resolve_login_shell();
            let mut c = CommandBuilder::new(&shell);
            c.arg("-l");
            for (key, value) in &enhanced_env {
                c.env(key, value);
            }
            c
        }
        #[cfg(windows)]
        {
            let mut c = CommandBuilder::new("powershell.exe");
            c.arg("-NoLogo");
            for (key, value) in &enhanced_env {
                c.env(key, value);
            }
            c
        }
    } else {
        let cli = crate::cli::CliKind::from_id(Some(&cli_kind))?;
        let cli_path = crate::cli::find_cli_path(cli)
            .ok_or_else(|| format!("未检测到 {} CLI", cli.name()))?;

        let mut mode = get_agent_status_mode(agent_status_mode);
        let supports_osc =
            crate::claude_hooks::supports_terminal_sequence(std::path::Path::new(&cli_path));

        if mode == crate::claude_hooks::HookMode::Osc && !supports_osc {
            tracing::warn!(
                "[pty_manager] Claude Code version does not support terminalSequence; falling back"
            );
            let (major, minor, patch) = crate::claude_hooks::MIN_TERMINAL_SEQUENCE_VERSION;
            let required = format!("{}.{}.{}", major, minor, patch);
            let _ = app.emit(
                "claude-version-unsupported",
                serde_json::json!({
                    "required": required,
                    "fallback": "hook-relay"
                }),
            );
            mode = crate::claude_hooks::HookMode::HookRelay;
        }

        #[cfg(any(unix, windows))]
        if cli == crate::cli::CliKind::Claude {
            if let Some(settings_path) = settings_file.as_deref() {
                let relay_command = match crate::claude_hooks::resolve_hook_relay_command() {
                    Ok(cmd) => Some(cmd),
                    Err(e) => {
                        tracing::warn!(
                            "[pty_manager] failed to resolve hook relay command: {}",
                            e
                        );
                        None
                    }
                };

                if mode == crate::claude_hooks::HookMode::HookRelay {
                    if let Some(relay_command) = relay_command {
                        match crate::claude_hooks::start_runtime(
                            app.clone(),
                            id.clone(),
                            project_path.clone(),
                        ) {
                            Ok((runtime, socket_path)) => {
                                match merge_claude_hooks_into_settings_file_with_relay_command(
                                    settings_path,
                                    &id,
                                    &socket_path,
                                    &relay_command.to_string_lossy(),
                                    mode,
                                ) {
                                    Ok(true) => {
                                        status_source = crate::agent_status::PtyStatusSource::Hook;
                                        claude_hook_runtime = Some(runtime);
                                    }
                                    Ok(false) => {
                                        tracing::warn!(
                                            "[pty_manager] Claude hooks are disabled in settings; using frontend fallback status detection"
                                        );
                                        drop(runtime);
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            "[pty_manager] failed to merge Claude hooks into settings file: {}",
                                            e
                                        );
                                        drop(runtime);
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "[pty_manager] failed to start Claude hook runtime: {}",
                                    e
                                );
                            }
                        }
                    }
                } else {
                    let relay_command_str = relay_command
                        .as_ref()
                        .map(|p| p.to_string_lossy().into_owned());
                    match merge_osc_hooks_into_settings_file(
                        settings_path,
                        relay_command_str.as_deref(),
                    ) {
                        Ok(true) => {
                            status_source = crate::agent_status::PtyStatusSource::Hook;
                        }
                        Ok(false) => {
                            tracing::warn!(
                                "[pty_manager] Claude hooks are disabled in settings; using frontend fallback status detection"
                            );
                        }
                        Err(e) => {
                            tracing::warn!("[pty_manager] failed to merge OSC hooks: {}", e);
                        }
                    }
                }
            } else {
                tracing::warn!(
                    "[pty_manager] Claude hook runtime skipped because no temporary settings file was provided"
                );
            }
        }

        let launch_args = cli.launch_arguments(resume_session_id.as_deref(), skip_permissions, settings_file.as_deref());

        #[cfg(windows)]
        let mut builder = if cli_path.to_lowercase().ends_with(".cmd") {
            let mut c = CommandBuilder::new("cmd.exe");
            c.arg("/c");
            c.arg(&cli_path);
            c
        } else {
            CommandBuilder::new(&cli_path)
        };
        #[cfg(not(windows))]
        let builder = {
            let startup_command = crate::shell_launch::build_startup_command(&cli_path, &launch_args);
            let shell = resolve_login_shell();
            crate::shell_launch::build_posix_shell_command(&shell, &startup_command, &enhanced_env)
        };
        #[cfg(windows)]
        {
            for arg in &launch_args {
                builder.arg(arg);
            }
            for (key, value) in &enhanced_env {
                builder.env(key, value);
            }
        }
        builder
    };

    let mut cmd = cmd;
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    let safe_project_path = if project_path.trim().is_empty() || !std::path::Path::new(&project_path).exists() {
        std::env::var("HOME").unwrap_or_else(|_| ".".to_string())
    } else {
        project_path.clone()
    };
    cmd.cwd(&safe_project_path);

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("无法启动终端进程: {}", e))?;

    let writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("无法获取 PTY writer: {}", e))?;

    let now = chrono::Local::now().to_rfc3339();
    let info = PtySessionInfo {
        session_id: id.clone(),
        project_path: safe_project_path,
        cli_kind: if cli_kind.is_empty() { "shell".to_string() } else { cli_kind },
        created_at: now,
        status: PtyStatus::Active,
        status_source,
    };

    // Spawn reader thread to forward PTY output to frontend
    let reader_id = id.clone();
    let app_clone = app.clone();
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("无法克隆 PTY reader: {}", e))?;

    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut agent_detect = crate::agent_detect::AgentDetector::new();
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let data = &buf[..n];
                    agent_detect.process(data, |transition| {
                        let status = match transition {
                            crate::agent_detect::Transition::Started
                            | crate::agent_detect::Transition::Working => PtyStatus::Active,
                            crate::agent_detect::Transition::Attention => PtyStatus::WaitingInput,
                            crate::agent_detect::Transition::Finished => PtyStatus::Idle,
                            crate::agent_detect::Transition::Exited => PtyStatus::Exited,
                        };
                        update_session_status(&app_clone, &reader_id, status);
                    });

                    let event_name = format!("pty-output-{}", reader_id);
                    let encoded = STANDARD.encode(data);
                    let _ = app_clone.emit(&event_name, encoded);

                    {
                        let mut sessions = lock_sessions();
                        if let Some(handle) = sessions.get_mut(&reader_id) {
                            handle.last_output_at = Instant::now();
                        }
                    }
                }
                Err(_) => break,
            }
        }
        agent_detect.finish(|transition| {
            if let crate::agent_detect::Transition::Exited = transition {
                update_session_status(&app_clone, &reader_id, PtyStatus::Exited);
            }
        });
        update_session_status(&app_clone, &reader_id, PtyStatus::Exited);
    });

    let handle = PtySessionHandle {
        master: pair.master,
        writer,
        child: Some(child),
        info: info.clone(),
        last_output_at: Instant::now(),
        settings_file_path: settings_file,
        claude_hook_runtime,
    };

    lock_sessions().insert(id, handle);

    Ok(info)
}

#[cfg(any(unix, windows))]
fn merge_claude_hooks_into_settings_file_with_relay_command(
    path: &str,
    session_id: &str,
    socket_path: &std::path::Path,
    relay_command: &str,
    mode: crate::claude_hooks::HookMode,
) -> Result<bool, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut settings: serde_json::Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    if settings
        .get("disableAllHooks")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return Ok(false);
    }
    crate::claude_hooks::merge_sessiondock_hooks(
        &mut settings,
        session_id,
        &socket_path.to_string_lossy(),
        relay_command,
        mode,
    );
    let next = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    std::fs::write(path, next).map_err(|e| e.to_string())?;
    Ok(true)
}

fn merge_osc_hooks_into_settings_file(
    path: &str,
    relay_command: Option<&str>,
) -> Result<bool, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut settings: serde_json::Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    if settings
        .get("disableAllHooks")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return Ok(false);
    }
    let relay_command = relay_command.unwrap_or("sessiondock-proxy");
    crate::claude_hooks::merge_sessiondock_hooks(
        &mut settings,
        "",
        "",
        relay_command,
        crate::claude_hooks::HookMode::Osc,
    );
    let next = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    std::fs::write(path, next).map_err(|e| e.to_string())?;
    Ok(true)
}

pub fn write_session(session_id: &str, data: &[u8]) -> Result<(), String> {
    let mut sessions = lock_sessions();
    let handle = sessions
        .get_mut(session_id)
        .ok_or_else(|| format!("会话不存在: {}", session_id))?;
    handle
        .writer
        .write_all(data)
        .map_err(|e| format!("写入失败: {}", e))?;
    handle
        .writer
        .flush()
        .map_err(|e| format!("flush 失败: {}", e))?;
    Ok(())
}

pub fn resize_session(session_id: &str, cols: u16, rows: u16) -> Result<(), String> {
    let sessions = lock_sessions();
    let handle = sessions
        .get(session_id)
        .ok_or_else(|| format!("会话不存在: {}", session_id))?;
    handle
        .master
        .resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("resize 失败: {}", e))?;
    Ok(())
}

pub fn close_session(session_id: &str) -> Result<(), String> {
    let handle = {
        let mut sessions = lock_sessions();
        sessions
            .remove(session_id)
            .ok_or_else(|| format!("会话不存在: {}", session_id))?
    };
    drop(handle);
    Ok(())
}

pub fn list_sessions() -> Vec<PtySessionInfo> {
    lock_sessions().values().map(|h| h.info.clone()).collect()
}

/// Update session status from frontend (xterm.js buffer detection).
/// Returns true if the status actually changed.
/// Triggers Dock bounce / taskbar flash when the window is not focused.
pub fn update_session_status(app: &tauri::AppHandle, session_id: &str, status: PtyStatus) -> bool {
    let mut sessions = lock_sessions();
    if let Some(handle) = sessions.get_mut(session_id) {
        let old_status = handle.info.status;
        let Some(next_status) = crate::agent_status::next_status(old_status, status) else {
            tracing::info!(
                target: "pty_status",
                session_id = %session_id,
                old = ?old_status,
                requested = ?status,
                "status transition rejected by gate"
            );
            return false;
        };
        handle.info.status = next_status;
        tracing::info!(
            target: "pty_status",
            session_id = %session_id,
            old = ?old_status,
            new = ?next_status,
            source = ?handle.info.status_source,
            "session status updated"
        );
        let _ = app.emit(
            "pty-status-changed",
            PtyStatusChangedPayload {
                session_id: session_id.to_string(),
                status: next_status,
            },
        );
        // Drop the lock before accessing the window
        drop(sessions);
        request_attention(app, old_status, next_status);
        return true;
    }
    false
}

fn request_attention(app: &tauri::AppHandle, old: PtyStatus, new: PtyStatus) {
    let window = match app.get_webview_window("main") {
        Some(w) => w,
        None => return,
    };
    if window.is_focused().unwrap_or(true) {
        return;
    }

    match (old, new) {
        (_, PtyStatus::WaitingInput) => {
            let _ = window.request_user_attention(Some(UserAttentionType::Critical));
        }
        (PtyStatus::Active, PtyStatus::Idle) => {
            let _ = window.request_user_attention(Some(UserAttentionType::Informational));
        }
        (PtyStatus::WaitingInput, _) => {
            let _ = window.request_user_attention(None);
        }
        _ => {}
    }
}

pub fn cleanup_all() {
    lock_sessions().clear();
}

fn should_force_kill_after(
    wait_started: Instant,
    now: Instant,
    timeout: Duration,
) -> bool {
    now.duration_since(wait_started) >= timeout
}

/// Preload shell environment in a background thread so the first
/// `create_session` call doesn't block the UI.
pub fn preload_shell_env() {
    std::thread::spawn(|| {
        let _ = get_enhanced_env();
    });
}

/// Inject the user's login shell environment into the current process.
/// Call this once at app startup so all child processes (Command::new, etc.)
/// automatically inherit the full PATH and other variables that are missing
/// when the app is launched from Finder/launchd instead of a terminal.
#[cfg(unix)]
pub fn inject_shell_env_into_process() {
    tracing::info!("[shell_env] inject_shell_env_into_process: start, process PATH: {}", std::env::var("PATH").unwrap_or_default());
    let shell_env = SHELL_ENV_CACHE.get_or_init(load_shell_environment);

    for (key, value) in shell_env {
        if key == "PATH" {
            let process_path = std::env::var("PATH").unwrap_or_default();
            let merged = merge_paths(&process_path, value);
            let extra = get_posix_extra_tool_paths(&merged);
            let final_path = if extra.is_empty() {
                merged
            } else {
                merge_paths(&merged, &extra.join(":"))
            };
            tracing::info!("[shell_env] injected PATH: {}", final_path);
            std::env::set_var("PATH", final_path);
        } else {
            if std::env::var(key).is_err() {
                std::env::set_var(key, value);
            }
        }
    }
    tracing::info!("[shell_env] inject done, process PATH: {}", std::env::var("PATH").unwrap_or_default());
}

#[cfg(not(unix))]
pub fn inject_shell_env_into_process() {
    // No-op on Windows — PATH inheritance works correctly there.
}

/// Guardian thread: only monitors process exit.
/// Status detection is handled by the frontend (xterm.js buffer).
pub fn start_guardian(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            if STOP_GUARDIAN.load(Ordering::Relaxed) {
                break;
            }

            let mut exited_ids = Vec::new();

            let mut removed_handles = Vec::new();

            {
                let mut sessions = lock_sessions();
                for (id, handle) in sessions.iter_mut() {
                    if handle.info.status == PtyStatus::Exited {
                        continue;
                    }
                    if let Some(child) = handle.child.as_mut() {
                        if let Ok(Some(_)) = child.try_wait() {
                            handle.child = None;
                            handle.info.status = PtyStatus::Exited;
                            exited_ids.push(id.clone());
                        }
                    }
                }
                for id in &exited_ids {
                    if let Some(handle) = sessions.remove(id) {
                        removed_handles.push(handle);
                    }
                }
            }
            drop(removed_handles);

            for id in exited_ids {
                let _ = app.emit(
                    "pty-status-changed",
                    PtyStatusChangedPayload {
                        session_id: id,
                        status: PtyStatus::Exited,
                    },
                );
            }
        }
    });
}

/// Signal the guardian thread to stop at its next 2-second wake-up.
pub fn shutdown_guardian() {
    STOP_GUARDIAN.store(true, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Read, Write};
    use std::sync::mpsc;

    #[derive(Debug)]
    struct DummyChild {
        waited_tx: mpsc::Sender<()>,
    }

    impl portable_pty::ChildKiller for DummyChild {
        fn kill(&mut self) -> io::Result<()> {
            Ok(())
        }

        fn clone_killer(&self) -> Box<dyn portable_pty::ChildKiller + Send + Sync> {
            Box::new(DummyKiller)
        }
    }

    impl portable_pty::Child for DummyChild {
        fn try_wait(&mut self) -> io::Result<Option<portable_pty::ExitStatus>> {
            Ok(None)
        }

        fn wait(&mut self) -> io::Result<portable_pty::ExitStatus> {
            let _ = self.waited_tx.send(());
            Ok(portable_pty::ExitStatus::with_exit_code(0))
        }

        fn process_id(&self) -> Option<u32> {
            None
        }
    }

    #[derive(Debug)]
    struct DummyKiller;

    impl portable_pty::ChildKiller for DummyKiller {
        fn kill(&mut self) -> io::Result<()> {
            Ok(())
        }

        fn clone_killer(&self) -> Box<dyn portable_pty::ChildKiller + Send + Sync> {
            Box::new(DummyKiller)
        }
    }

    struct DummyMaster;

    impl MasterPty for DummyMaster {
        fn resize(&self, _size: PtySize) -> anyhow::Result<()> {
            Ok(())
        }

        fn get_size(&self) -> anyhow::Result<PtySize> {
            Ok(PtySize::default())
        }

        fn try_clone_reader(&self) -> anyhow::Result<Box<dyn Read + Send>> {
            Ok(Box::new(io::empty()))
        }

        fn take_writer(&self) -> anyhow::Result<Box<dyn Write + Send>> {
            Ok(Box::new(io::sink()))
        }

        #[cfg(unix)]
        fn process_group_leader(&self) -> Option<libc::pid_t> {
            None
        }

        #[cfg(unix)]
        fn as_raw_fd(&self) -> Option<portable_pty::unix::RawFd> {
            None
        }
    }

    #[test]
    fn dropping_pty_handle_reaps_child_after_kill() {
        let (waited_tx, waited_rx) = mpsc::channel();
        let handle = PtySessionHandle {
            master: Box::new(DummyMaster),
            writer: Box::new(io::sink()),
            child: Some(Box::new(DummyChild { waited_tx })),
            info: PtySessionInfo {
                session_id: "test".to_string(),
                project_path: "/tmp".to_string(),
                cli_kind: "claude".to_string(),
                created_at: "2026-05-07T00:00:00+00:00".to_string(),
                status: PtyStatus::Active,
                status_source: crate::agent_status::PtyStatusSource::FrontendFallback,
            },
            last_output_at: Instant::now(),
            settings_file_path: None,
            claude_hook_runtime: None,
        };

        drop(handle);

        waited_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("dropped PTY handle should wait on the killed child");
    }

    #[test]
    fn pty_force_kill_policy_waits_until_timeout() {
        let started = Instant::now();

        assert!(!should_force_kill_after(
            started,
            started + Duration::from_millis(1999),
            Duration::from_secs(2),
        ));
        assert!(should_force_kill_after(
            started,
            started + Duration::from_secs(2),
            Duration::from_secs(2),
        ));
    }

    #[cfg(unix)]
    #[test]
    fn merge_claude_hooks_into_settings_file_preserves_existing_settings() {
        let path = std::env::temp_dir().join(format!(
            "sessiondock-hook-merge-test-{}-{}.json",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::write(&path, r#"{"env":{"FOO":"bar"}}"#).unwrap();

        merge_claude_hooks_into_settings_file_with_relay_command(
            path.to_str().unwrap(),
            "session-abc",
            std::path::Path::new("/tmp/sessiondock-agent-session-abc.sock"),
            "/Applications/SessionDock.app/Contents/MacOS/sessiondock-proxy",
            crate::claude_hooks::HookMode::HookRelay,
        )
        .unwrap();

        let settings: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(settings["env"]["FOO"], "bar");
        assert_eq!(
            settings["hooks"]["Notification"][0]["hooks"][0]["args"][4],
            "session-abc"
        );

        let _ = std::fs::remove_file(path);
    }

    #[cfg(unix)]
    #[test]
    fn merge_claude_hooks_into_settings_file_respects_disable_all_hooks() {
        let path = std::env::temp_dir().join(format!(
            "sessiondock-hook-disabled-test-{}-{}.json",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::write(&path, r#"{"disableAllHooks":true}"#).unwrap();

        let enabled = merge_claude_hooks_into_settings_file_with_relay_command(
            path.to_str().unwrap(),
            "session-disabled",
            std::path::Path::new("/tmp/sessiondock-agent-session-disabled.sock"),
            "/Applications/SessionDock.app/Contents/MacOS/sessiondock-proxy",
            crate::claude_hooks::HookMode::HookRelay,
        )
        .unwrap();

        let settings: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(!enabled);
        assert_eq!(settings["disableAllHooks"], true);
        assert!(settings.get("hooks").is_none());

        let _ = std::fs::remove_file(path);
    }
}
