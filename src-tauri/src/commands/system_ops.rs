use crate::app_db;
use crate::cli::{self, CliKind};
use crate::context_menu;
use crate::error::{AppError, AppResult};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::Emitter;

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchDeleteOutcome {
    pub failed: Vec<(String, String)>, // (path, reason)
    pub succeeded: usize,
}

/// 文件已不存在 → 视为跳过（返回 Ok，计入成功）；存在则走系统回收站。
fn trash_or_skip(path: &str) -> Result<(), String> {
    if std::path::Path::new(path).exists() {
        trash::delete(path).map_err(|e| e.to_string())
    } else {
        Ok(())
    }
}

/// 逐路径删除，单个失败不中止；on_progress 每个路径处理完回调一次（done/total/current）。
/// 纯计数：`trash_one` 返回 Ok 计成功、Err 记失败；"跳过已不存在"由 `trash_or_skip` 决定。
pub fn run_batch_delete<F, G>(paths: &[String], mut trash_one: F, mut on_progress: G) -> BatchDeleteOutcome
where
    F: FnMut(&str) -> Result<(), String>,
    G: FnMut(usize, usize, &str),
{
    let total = paths.len();
    let mut out = BatchDeleteOutcome { failed: Vec::new(), succeeded: 0 };
    for (i, path) in paths.iter().enumerate() {
        match trash_one(path) {
            Ok(()) => out.succeeded += 1,
            Err(reason) => out.failed.push((path.clone(), reason)),
        }
        on_progress(i + 1, total, path);
    }
    out
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemDiagnostics {
    pub app_name: String,
    pub app_version: String,
    pub tauri_version: String,
    pub os: String,
    pub os_name: String,
    pub os_version: String,
    pub kernel_version: String,
    pub arch: String,
    pub cpu_model: String,
    pub cpu_cores: usize,
    pub memory_total: String,
    pub clis: Vec<CliDiagnosticInfo>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliDiagnosticInfo {
    pub id: String,
    pub name: String,
    pub command: String,
    pub installed: bool,
    pub has_sessions: bool,
}

#[tauri::command]
pub fn get_system_diagnostics(app: tauri::AppHandle) -> AppResult<SystemDiagnostics> {
    let app_version = app.package_info().version.to_string();
    let tauri_version = tauri::VERSION.to_string();
    let os = std::env::consts::OS.to_string();
    let arch = std::env::consts::ARCH.to_string();
    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);

    #[cfg(target_os = "macos")]
    let (os_name, os_version, kernel_version, cpu_model, memory_total) = {
        let p_name = std::process::Command::new("sw_vers")
            .arg("-productName")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "macOS".to_string());
        let p_ver = std::process::Command::new("sw_vers")
            .arg("-productVersion")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "".to_string());
        let b_ver = std::process::Command::new("sw_vers")
            .arg("-buildVersion")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "".to_string());
        let os_name = if p_ver.is_empty() { p_name } else { format!("{} {}", p_name, p_ver) };
        let os_version = if b_ver.is_empty() { p_ver } else { format!("Build {}", b_ver) };

        let kernel_version = std::process::Command::new("uname")
            .args(["-s", "-r"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "Darwin".to_string());

        let cpu_model = std::process::Command::new("sysctl")
            .args(["-n", "machdep.cpu.brand_string"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("Apple Silicon ({})", arch));

        let memory_total = std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse::<u64>().ok())
            .map(|bytes| format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0)))
            .unwrap_or_else(|| "Unknown".to_string());

        (os_name, os_version, kernel_version, cpu_model, memory_total)
    };

    #[cfg(target_os = "windows")]
    let (os_name, os_version, kernel_version, cpu_model, memory_total) = {
        let ver = std::process::Command::new("cmd")
            .args(["/c", "ver"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "Microsoft Windows".to_string());
        let cpu = std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_else(|_| "Unknown".to_string());
        ("Windows".to_string(), ver, "Windows NT".to_string(), cpu, "Unknown".to_string())
    };

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let (os_name, os_version, kernel_version, cpu_model, memory_total) = {
        let release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
        let pretty = release
            .lines()
            .find(|line| line.starts_with("PRETTY_NAME="))
            .map(|line| line.trim_start_matches("PRETTY_NAME=").trim_matches('"').to_string())
            .unwrap_or_else(|| "Linux".to_string());
        let kernel = std::process::Command::new("uname")
            .arg("-r")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "Linux".to_string());
        (pretty, "".to_string(), kernel, "Unknown".to_string(), "Unknown".to_string())
    };

    let clis = cli::CliKind::all()
        .into_iter()
        .map(|kind| CliDiagnosticInfo {
            id: kind.id().to_string(),
            name: kind.name().to_string(),
            command: kind.command().to_string(),
            installed: cli::detect_cli(kind),
            has_sessions: cli::has_sessions(kind),
        })
        .collect();

    Ok(SystemDiagnostics {
        app_name: "SessionDock".to_string(),
        app_version,
        tauri_version,
        os,
        os_name,
        os_version,
        kernel_version,
        arch,
        cpu_model,
        cpu_cores,
        memory_total,
        clis,
    })
}

#[tauri::command]
pub fn detect_cli(cli_id: Option<String>) -> AppResult<bool> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    Ok(cli::detect_cli(kind))
}

#[tauri::command]
pub fn delete_to_trash(paths: Vec<String>) -> AppResult<()> {
    for path in &paths {
        trash::delete(path).map_err(|_| "删除失败，文件可能正在被其他程序使用。".to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn delete_sessions_to_trash(
    app: tauri::AppHandle,
    cli_id: Option<String>,
    paths: Vec<String>,
) -> AppResult<BatchDeleteOutcome> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    let mut session_paths: Vec<String> = paths
        .into_iter()
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty())
        .collect();
    session_paths.sort();
    session_paths.dedup();
    let paths_for_db = session_paths.clone();

    let outcome = tauri::async_runtime::spawn_blocking(move || {
        run_batch_delete(
            &session_paths,
            trash_or_skip,
            |done, total, current| {
                let _ = app.emit(
                    "batch-delete-progress",
                    serde_json::json!({
                        "done": done,
                        "total": total,
                        "currentPath": current,
                    }),
                );
            },
        )
    })
    .await
    .map_err(|e| AppError::business(format!("批量删除执行失败: {}", e)))?;

    app_db::delete_session_records(kind, &paths_for_db)?;
    Ok(outcome)
}

#[tauri::command]
pub fn get_launch_command(
    cli_id: Option<String>,
    project_path: String,
    session_id: Option<String>,
    skip_permissions: bool,
    profile_name: Option<String>,
    terminal_app: Option<String>,
) -> AppResult<String> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    let settings_file = match profile_name {
        Some(name) if !name.is_empty() => Some(crate::commands::profile::build_profile_settings_file(
            Some(kind.id().to_string()),
            name,
            None,
        )?),
        _ => None,
    };
    Ok(cli::build_launch_command_string(
        kind,
        &project_path,
        session_id.as_deref(),
        skip_permissions,
        settings_file.as_deref(),
        terminal_app.as_deref(),
    ))
}

#[tauri::command]
pub fn clean_temp_configs() -> AppResult<usize> {
    let temp_dir = if cfg!(target_os = "macos") {
        std::path::PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    };
    
    let mut deleted = 0;
    if let Ok(entries) = std::fs::read_dir(temp_dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.starts_with("sessiondock-settings-") && name.ends_with(".json") {
                    if std::fs::remove_file(entry.path()).is_ok() {
                        deleted += 1;
                    }
                }
            }
        }
    }
    Ok(deleted)
}

#[tauri::command]
pub fn detect_terminal_apps() -> Vec<String> {
    crate::terminal::detect_installed()
        .into_iter()
        .map(str::to_string)
        .collect()
}

#[tauri::command]
pub fn open_in_terminal(
    cli_id: Option<String>,
    project_path: String,
    session_id: Option<String>,
    skip_permissions: bool,
    profile_name: Option<String>,
    terminal_app: Option<String>,
) -> AppResult<()> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    let settings_file = match profile_name {
        Some(name) if !name.is_empty() => Some(crate::commands::profile::build_profile_settings_file(
            Some(kind.id().to_string()),
            name,
            None,
        )?),
        _ => None,
    };
    cli::open_in_terminal(
        kind,
        &project_path,
        session_id.as_deref(),
        skip_permissions,
        settings_file.as_deref(),
        terminal_app.as_deref(),
    )
    .map_err(AppError::from)
}

#[tauri::command]
pub fn register_context_menu(
    skip_permissions: bool,
    terminal_app: Option<String>,
) -> AppResult<String> {
    context_menu::register(skip_permissions, terminal_app.as_deref()).map_err(AppError::from)
}

#[tauri::command]
pub fn unregister_context_menu() -> AppResult<String> {
    context_menu::unregister().map_err(AppError::from)
}

#[tauri::command]
pub fn is_context_menu_registered() -> AppResult<bool> {
    context_menu::is_registered().map_err(AppError::from)
}

#[tauri::command]
pub fn open_data_dir(cli_id: Option<String>) -> AppResult<()> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    let dir = cli::data_dir(kind)?;
    super::open_path(&dir)
}

#[tauri::command]
pub fn check_paths_exist(paths: Vec<String>) -> std::collections::HashMap<String, bool> {
    let mut map = std::collections::HashMap::new();
    for p in paths {
        let exists = Path::new(&p).exists();
        map.insert(p, exists);
    }
    map
}

#[tauri::command]
pub fn open_path_in_file_manager(path: String) -> AppResult<()> {
    let target = PathBuf::from(&path);
    if !target.exists() {
        return Err(AppError::business(format!("路径不存在：{}", path)));
    }

    if target.is_file() {
        reveal_file_in_manager(&target)
    } else {
        super::open_path(&target)
    }
}

fn reveal_file_in_manager(path: &Path) -> AppResult<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn()
            .map_err(|e| AppError::business(e.to_string()))?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg("/select,")
            .arg(path)
            .spawn()
            .map_err(|e| AppError::business(e.to_string()))?;
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(parent) = path.parent() {
            std::process::Command::new("xdg-open")
                .arg(parent)
                .spawn()
                .map_err(|e| AppError::business(e.to_string()))?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn get_cli_version(cli_id: Option<String>) -> AppResult<String> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    cli::cli_version(kind).map_err(AppError::from)
}

#[tauri::command]
pub fn get_claude_cli_version() -> AppResult<String> {
    cli::cli_version(CliKind::Claude).map_err(AppError::from)
}

fn next_available_path(path: &Path) -> AppResult<PathBuf> {
    if !path.exists() {
        return Ok(path.to_path_buf());
    }

    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("migrated");
    let ext = path.extension().and_then(|value| value.to_str());

    for index in 1..1000usize {
        let candidate_name = match ext {
            Some(ext) if !ext.is_empty() => format!("{}-migrated-{}.{}", stem, index, ext),
            _ => format!("{}-migrated-{}", stem, index),
        };
        let candidate = parent.join(candidate_name);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(AppError::business(format!(
        "无法为 {} 生成唯一文件名：候选名称已用完（1000个）",
        path.display()
    )))
}

fn move_legacy_file_with_fallback(src: &Path, dst: &Path) -> AppResult<()> {
    if !src.exists() {
        return Ok(());
    }

    let target = if dst.exists() {
        next_available_path(dst)?
    } else {
        dst.to_path_buf()
    };

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| AppError::business(e.to_string()))?;
    }

    match fs::rename(src, &target) {
        Ok(_) => Ok(()),
        Err(_) => {
            fs::copy(src, &target).map_err(|e| AppError::business(e.to_string()))?;
            fs::remove_file(src).map_err(|e| AppError::business(e.to_string()))
        }
    }
}

fn move_legacy_dir_with_fallback(src: &Path, dst: &Path) -> AppResult<()> {
    if !src.exists() {
        return Ok(());
    }

    if !dst.exists() {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|e| AppError::business(e.to_string()))?;
        }
        if fs::rename(src, dst).is_ok() {
            return Ok(());
        }
    }

    fs::create_dir_all(dst).map_err(|e| AppError::business(e.to_string()))?;
    for entry in fs::read_dir(src).map_err(|e| AppError::business(e.to_string()))? {
        let entry = entry.map_err(|e| AppError::business(e.to_string()))?;
        let from = entry.path();
        let to = dst.join(entry.file_name());

        if from.is_dir() {
            move_legacy_dir_with_fallback(&from, &to)?;
        } else {
            move_legacy_file_with_fallback(&from, &to)?;
        }
    }

    if src.exists() {
        fs::remove_dir_all(src).map_err(|e| AppError::business(e.to_string()))?;
    }
    Ok(())
}


fn remove_dir_if_empty(path: &Path) -> AppResult<()> {
    if !path.exists() {
        return Ok(());
    }

    let is_empty = fs::read_dir(path)
        .map_err(|e| AppError::business(e.to_string()))?
        .next()
        .is_none();
    if is_empty {
        fs::remove_dir(path).map_err(|e| AppError::business(e.to_string()))?;
    }
    Ok(())
}

fn cleanup_empty_legacy_dirs() -> AppResult<()> {
    let root = super::app_data_dir()?;
    for dir in [root.join("config").join("cli"), root.join("config"), root.join("data")] {
        remove_dir_if_empty(&dir)?;
    }
    Ok(())
}

/// Migrate legacy metadata and reports layout into app.db + root reports/.
pub fn migrate_data_layout() -> AppResult<()> {
    app_db::migrate_legacy_metadata()?;
    cleanup_empty_legacy_dirs()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_paths(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("/tmp/ghost-{}.jsonl", i)).collect()
    }

    #[test]
    fn run_batch_delete_all_success_counts_and_progress() {
        let paths = make_paths(3);
        let mut progress = Vec::new();
        let out = run_batch_delete(&paths, |_| Ok(()), |done, total, cur| {
            progress.push((done, total, cur.to_string()));
        });
        assert_eq!(out.failed, vec![]);
        assert_eq!(out.succeeded, 3);
        assert_eq!(progress, vec![
            (1, 3, "/tmp/ghost-0.jsonl".to_string()),
            (2, 3, "/tmp/ghost-1.jsonl".to_string()),
            (3, 3, "/tmp/ghost-2.jsonl".to_string()),
        ]);
    }

    #[test]
    fn run_batch_delete_failure_does_not_abort() {
        let paths = make_paths(3);
        let out = run_batch_delete(
            &paths,
            |p| if p.ends_with("ghost-1.jsonl") { Err("被占用".to_string()) } else { Ok(()) },
            |_, _, _| {},
        );
        assert_eq!(out.succeeded, 2);
        assert_eq!(out.failed.len(), 1);
        assert_eq!(out.failed[0].0, "/tmp/ghost-1.jsonl");
        assert_eq!(out.failed[0].1, "被占用");
    }

    #[test]
    fn trash_or_skip_returns_ok_for_missing_file() {
        assert!(trash_or_skip("/definitely/not/here.jsonl").is_ok());
    }

    #[test]
    fn run_batch_delete_with_skip_counts_missing_as_success() {
        // 不依赖 /tmp/ghost-* 恰好不存在：用 temp_dir + 随机后缀生成必然不冲突的路径，
        // 验证 trash_or_skip 对缺失文件返回 Ok、缺失文件计入成功。
        let uniq = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let paths: Vec<String> = (0..2)
            .map(|i| {
                std::env::temp_dir()
                    .join(format!("sessiondock-ghost-{}-{}.jsonl", uniq, i))
                    .to_string_lossy()
                    .to_string()
            })
            .collect();
        let out = run_batch_delete(&paths, trash_or_skip, |_, _, _| {});
        assert_eq!(out.succeeded, 2);
        assert_eq!(out.failed, vec![]);
    }
}
