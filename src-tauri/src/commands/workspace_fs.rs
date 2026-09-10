use crate::error::{AppError, AppResult};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

// ─── helpers ─────────────────────────────────────────────────────────────────

/// 确保 path 在 project_root 内，防止路径穿越
fn validate_within(path: &Path, project_root: &Path) -> AppResult<()> {
    let canonical_path = path
        .canonicalize()
        .map_err(|e| AppError::business(e.to_string()))?;
    let canonical_root = project_root
        .canonicalize()
        .map_err(|e| AppError::business(e.to_string()))?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err(AppError::business(format!(
            "路径 {} 不在项目目录 {} 内",
            canonical_path.display(),
            canonical_root.display()
        )));
    }
    Ok(())
}

fn is_ignored(name: &str) -> bool {
    matches!(
        name.to_lowercase().as_str(),
        "node_modules" | ".git" | "target" | ".ds_store" | "dist" | ".next" | "__pycache__"
    )
}

// ─── types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub children: Option<Vec<FileEntry>>,
}

// ─── commands ─────────────────────────────────────────────────────────────────

/// 列出项目目录文件树（递归，忽略常见噪音目录）
#[tauri::command]
pub async fn list_project_files(project_path: String, project_root: String, filter: Option<String>) -> AppResult<Vec<FileEntry>> {
    let root = PathBuf::from(&project_path);
    let root_boundary = PathBuf::from(&project_root);
    validate_within(&root, &root_boundary)?;
    if !root.is_dir() {
        return Err(AppError::business(format!("目录不存在: {}", project_path)));
    }
    let filter_lower = filter.as_deref().map(|s| s.to_lowercase());
    Ok(scan_dir(&root, filter_lower.as_deref())?)
}

#[tauri::command]
pub async fn read_directory(path: String, project_root: String) -> AppResult<Vec<FileEntry>> {
    let p = PathBuf::from(&path);
    let root = PathBuf::from(&project_root);
    validate_within(&p, &root)?;
    if !p.is_dir() {
        return Err(AppError::business(format!("不是目录: {}", path)));
    }
    let mut entries: Vec<FileEntry> = Vec::new();
    let read = fs::read_dir(&p).map_err(|e| AppError::business(e.to_string()))?;
    for entry in read.filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || is_ignored(&name) {
            continue;
        }
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        entries.push(FileEntry {
            name,
            path: entry.path().to_string_lossy().to_string(),
            is_dir,
            children: None,
        });
    }
    entries.sort_by_key(|e| (!e.is_dir, e.name.clone()));
    Ok(entries)
}

const MAX_SCAN_DEPTH: usize = 32;

fn scan_dir(dir: &Path, filter: Option<&str>) -> AppResult<Vec<FileEntry>> {
    scan_dir_inner(dir, filter, 0)
}

fn scan_dir_inner(dir: &Path, filter: Option<&str>, depth: usize) -> AppResult<Vec<FileEntry>> {
    if depth > MAX_SCAN_DEPTH {
        return Err(AppError::business("目录嵌套层级过深"));
    }
    let mut entries: Vec<FileEntry> = Vec::new();
    let read = fs::read_dir(dir).map_err(|e| AppError::business(e.to_string()))?;

    let mut items: Vec<_> = read
        .filter_map(|e| e.ok())
        .collect();
    items.sort_by_key(|e| {
        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
        (!is_dir, e.file_name())
    });

    for entry in items {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let ft = entry.file_type()?;
        let path_str = entry.path().to_string_lossy().to_string();

        if ft.is_dir() {
            if is_ignored(&name) {
                continue;
            }
            let children = scan_dir_inner(&entry.path(), filter, depth + 1)?;
            entries.push(FileEntry {
                name,
                path: path_str,
                is_dir: true,
                children: Some(children),
            });
        } else {
            if let Some(f) = filter {
                if !name.to_lowercase().contains(f) {
                    continue;
                }
            }
            entries.push(FileEntry {
                name,
                path: path_str,
                is_dir: false,
                children: None,
            });
        }
    }
    Ok(entries)
}

#[tauri::command]
pub async fn read_file_content(path: String, project_root: String) -> AppResult<String> {
    let p = PathBuf::from(&path);
    let root = PathBuf::from(&project_root);
    validate_within(&p, &root)?;
    fs::read_to_string(&p).map_err(|e| AppError::business(e.to_string()))
}

#[tauri::command]
pub async fn validate_workspace_file(path: String, project_root: String) -> AppResult<String> {
    let p = PathBuf::from(&path);
    let root = PathBuf::from(&project_root);
    validate_within(&p, &root)?;
    if !p.is_file() {
        return Err(AppError::business(format!("文件不存在或不是文件: {}", path)));
    }
    p.canonicalize()
        .map(|path| path.to_string_lossy().to_string())
        .map_err(|e| AppError::business(e.to_string()))
}

#[tauri::command]
pub async fn save_file(path: String, content: String, project_root: String) -> AppResult<()> {
    let p = PathBuf::from(&path);
    let root = PathBuf::from(&project_root);
    // Validate parent (file may not exist yet for new files)
    let parent = p.parent().ok_or("路径无父目录")?;
    validate_within(parent, &root)?;
    if let Some(par) = p.parent() {
        fs::create_dir_all(par).map_err(|e| AppError::business(e.to_string()))?;
    }
    fs::write(&p, content).map_err(|e| AppError::business(e.to_string()))
}

#[tauri::command]
pub async fn create_file(path: String, project_root: String) -> AppResult<()> {
    let p = PathBuf::from(&path);
    let root = PathBuf::from(&project_root);
    let parent = p.parent().ok_or("路径无父目录")?;
    validate_within(parent, &root)?;
    if p.exists() {
        return Err(AppError::business(format!("文件已存在: {}", path)));
    }
    if let Some(par) = p.parent() {
        fs::create_dir_all(par).map_err(|e| AppError::business(e.to_string()))?;
    }
    fs::write(&p, "").map_err(|e| AppError::business(e.to_string()))
}

#[tauri::command]
pub async fn rename_file(old_path: String, new_path: String, project_root: String) -> AppResult<()> {
    let old = PathBuf::from(&old_path);
    let new = PathBuf::from(&new_path);
    let root = PathBuf::from(&project_root);
    validate_within(&old, &root)?;
    let new_parent = new.parent().ok_or("新路径无父目录")?;
    validate_within(new_parent, &root)?;
    fs::rename(&old, &new).map_err(|e| AppError::business(e.to_string()))
}

#[tauri::command]
pub async fn delete_file(path: String, project_root: String) -> AppResult<()> {
    let p = PathBuf::from(&path);
    let root = PathBuf::from(&project_root);
    validate_within(&p, &root)?;
    trash::delete(&p).map_err(|e| AppError::business(e.to_string()))
}

#[tauri::command]
pub async fn create_dir(path: String, project_root: String) -> AppResult<()> {
    let p = PathBuf::from(&path);
    let root = PathBuf::from(&project_root);
    let parent = p.parent().ok_or("路径无父目录")?;
    validate_within(parent, &root)?;
    if p.exists() {
        return Err(AppError::business(format!("目录已存在: {}", path)));
    }
    fs::create_dir_all(&p).map_err(|e| AppError::business(e.to_string()))
}

// ─── File watcher state ────────────────────────────────────────────────────

use notify::{recommended_watcher, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::sync::{mpsc, Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

struct WatcherEntry {
    _watcher: RecommendedWatcher,
    stop_flag: Arc<AtomicBool>,
}

pub struct FileWatcherState {
    watchers: HashMap<String, WatcherEntry>,
}

impl Default for FileWatcherState {
    fn default() -> Self {
        Self {
            watchers: HashMap::new(),
        }
    }
}

pub type FileWatcherStateType = Mutex<FileWatcherState>;

/// 开始监听指定文件，变更事件经 300ms 防抖后推送 `file-changed` 事件。
/// 重复调用同一路径会先关闭旧 watcher 再创建新的。
#[tauri::command]
pub async fn watch_file(
    path: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, FileWatcherStateType>,
) -> AppResult<()> {
    use tauri::Emitter;
    let mut guard = state.lock().map_err(|_| AppError::business("无法获取文件监听锁"))?;
    // Canonicalize path to prevent duplicate watchers for different representations
    let key = std::fs::canonicalize(&path)
        .unwrap_or_else(|_| PathBuf::from(&path))
        .to_string_lossy()
        .to_string();
    // Remove old watcher for same path (sets stop_flag)
    if let Some(old) = guard.watchers.remove(&key) {
        old.stop_flag.store(true, Ordering::SeqCst);
    }

    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();
    let mut watcher =
        recommended_watcher(tx).map_err(|e| AppError::business(e.to_string()))?;
    watcher
        .watch(Path::new(&path), RecursiveMode::NonRecursive)
        .map_err(|e| AppError::business(e.to_string()))?;

    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_flag_clone = stop_flag.clone();
    let path_clone = path.clone();
    let app_clone = app.clone();

    std::thread::spawn(move || {
        let debounce = Duration::from_millis(300);
        let mut last_event: Option<Instant> = None;
        loop {
            if stop_flag_clone.load(Ordering::SeqCst) {
                break;
            }
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(Ok(event)) => {
                    if matches!(
                        event.kind,
                        EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
                    ) {
                        last_event = Some(Instant::now());
                    }
                }
                Ok(Err(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if let Some(t) = last_event {
                if t.elapsed() >= debounce {
                    let _ = app_clone.emit("file-changed", &path_clone);
                    last_event = None;
                }
            }
        }
    });

    guard.watchers.insert(key, WatcherEntry { _watcher: watcher, stop_flag });
    Ok(())
}

/// 停止监听指定文件（关闭对应 watcher，停止 debounce 线程）
#[tauri::command]
pub async fn unwatch_file(
    path: String,
    state: tauri::State<'_, FileWatcherStateType>,
) -> AppResult<()> {
    let mut guard = state.lock().map_err(|_| AppError::business("无法获取文件监听锁"))?;
    let key = std::fs::canonicalize(&path)
        .unwrap_or_else(|_| PathBuf::from(&path))
        .to_string_lossy()
        .to_string();
    if let Some(entry) = guard.watchers.remove(&key) {
        entry.stop_flag.store(true, Ordering::SeqCst);
    }
    Ok(())
}
