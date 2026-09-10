use crate::cli::{self, CliKind};
use crate::commands;
use notify::{recommended_watcher, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{
    mpsc::{self, Receiver, Sender},
    LazyLock, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

const SESSION_INDEX_WATCH_DEBOUNCE_MS: u64 = 3000;

struct SessionIndexWatcherRuntime {
    _watchers: Vec<RecommendedWatcher>,
    _refresh_tx: Sender<CliKind>,
}

static SESSION_INDEX_WATCH_RUNTIME: LazyLock<Mutex<Option<SessionIndexWatcherRuntime>>> =
    LazyLock::new(|| Mutex::new(None));

pub(crate) fn restart_session_index_watchers(app: &AppHandle) -> Result<(), String> {
    let runtime = build_runtime(app.clone())?;
    let mut guard = SESSION_INDEX_WATCH_RUNTIME
        .lock()
        .map_err(|_| "无法获取会话索引监听状态锁".to_string())?;
    *guard = Some(runtime);
    Ok(())
}

/// Drop the file-watcher runtime, which disconnects the channel and causes the
/// background refresh worker thread to exit gracefully.
pub(crate) fn shutdown_session_index_watchers() {
    if let Ok(mut guard) = SESSION_INDEX_WATCH_RUNTIME.lock() {
        *guard = None;
    }
}

fn build_runtime(app: AppHandle) -> Result<SessionIndexWatcherRuntime, String> {
    let (refresh_tx, refresh_rx) = mpsc::channel::<CliKind>();
    spawn_refresh_worker(app, refresh_rx);

    let mut watchers = Vec::new();
    for kind in CliKind::all() {
        let data_dir = cli::data_dir(kind)?;
        if !data_dir.exists() {
            tracing::info!(
                "跳过会话索引监听，数据目录不存在: cli={}, path={}",
                kind.id(),
                data_dir.to_string_lossy()
            );
            continue;
        }

        let sessions_dir = cli::sessions_dir(kind)?;
        let refresh_tx_clone = refresh_tx.clone();
        let data_dir_for_log = data_dir.clone();
        let sessions_dir_for_filter = sessions_dir.clone();

        let mut watcher = recommended_watcher(move |result: notify::Result<Event>| match result {
            Ok(event)
                if is_relevant_session_index_event(
                    &event,
                    kind,
                    &sessions_dir_for_filter,
                ) =>
            {
                if let Err(err) = refresh_tx_clone.send(kind) {
                    tracing::warn!(
                        "发送会话索引刷新任务失败: cli={}, err={}",
                        kind.id(),
                        err
                    );
                }
            }
            Ok(_) => {}
            Err(err) => {
                tracing::warn!(
                    "监听会话目录变化失败: cli={}, path={}, err={}",
                    kind.id(),
                    data_dir_for_log.to_string_lossy(),
                    err
                );
            }
        })
        .map_err(|err| format!("创建会话目录监听失败: {}", err))?;

        watcher
            .watch(&data_dir, RecursiveMode::Recursive)
            .map_err(|err| {
                format!(
                    "监听会话数据目录失败: cli={}, path={}, err={}",
                    kind.id(),
                    data_dir.to_string_lossy(),
                    err
                )
            })?;

        watchers.push(watcher);
    }

    Ok(SessionIndexWatcherRuntime {
        _watchers: watchers,
        _refresh_tx: refresh_tx,
    })
}

fn is_relevant_session_index_event(event: &Event, kind: CliKind, sessions_dir: &Path) -> bool {
    // 仅当会话结构发生实质性变化（新建会话文件/目录、删除会话文件/目录）时才触发会话树重拉；
    // 正在进行的会话内部日志内容写入（Modify）、工具运行日志（tasks/*.log）、临时文件（scratch/*）
    // 以及 history.jsonl 增量追加均不触发全量重扫，确保聊天过程中左侧会话树保持静止稳定无闪烁。
    let is_struct_change = matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Remove(_)
    );

    if !is_struct_change {
        return false;
    }

    event.paths.iter().any(|path| is_relevant_session_path(path, kind, sessions_dir))
}

fn is_relevant_session_path(path: &Path, kind: CliKind, sessions_dir: &Path) -> bool {
    if !path.starts_with(sessions_dir) {
        return false;
    }

    let path_str = path.to_string_lossy();

    // 过滤各种内部临时文件/任务日志/缓存文件
    if path_str.contains("/tasks/")
        || path_str.contains("\\tasks\\")
        || path_str.contains("/scratch/")
        || path_str.contains("\\scratch\\")
        || path_str.contains("/artifacts/")
        || path_str.contains("\\artifacts\\")
        || path_str.contains("/.system_generated/tasks/")
        || path_str.contains("\\.system_generated\\tasks\\")
        || path_str.ends_with(".log")
        || path_str.ends_with(".timestamp")
        || path_str.ends_with(".tmp")
    {
        return false;
    }

    match kind {
        CliKind::Antigravity => {
            if path.file_name().and_then(|n| n.to_str()) == Some("transcript.jsonl") {
                return true;
            }
            if let Some(parent) = path.parent() {
                if parent == sessions_dir {
                    return true;
                }
            }
            false
        }
        CliKind::Claude => {
            if path_str.contains("/subagents/") || path_str.contains("\\subagents\\") {
                return false;
            }
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                return true;
            }
            if let Some(parent) = path.parent() {
                if parent.parent() == Some(sessions_dir) || parent == sessions_dir {
                    return true;
                }
            }
            false
        }
        CliKind::Codex => {
            // Codex 会话文件为 sessions/YYYY/MM/DD/rollout-*.jsonl（扫描端 collect_jsonl_files 仅收 .jsonl）
            path.extension().and_then(|e| e.to_str()) == Some("jsonl")
        }
        CliKind::Gemini => {
            // Gemini 会话文件为 tmp/<hash>/chats/session-*.jsonl
            path.extension().and_then(|e| e.to_str()) == Some("jsonl")
        }
        CliKind::WorkBuddy => {
            path.extension().and_then(|e| e.to_str()) == Some("jsonl")
        }
        CliKind::Dsh => {
            let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            file_name == "session.jsonl" || file_name == "session.jsonl.zstd"
        }
    }
}

fn spawn_refresh_worker(app: AppHandle, refresh_rx: Receiver<CliKind>) {
    thread::spawn(move || {
        let debounce = Duration::from_millis(SESSION_INDEX_WATCH_DEBOUNCE_MS);
        let mut pending: HashMap<CliKind, Instant> = HashMap::new();

        loop {
            let timeout = next_pending_timeout(&pending, debounce);
            match refresh_rx.recv_timeout(timeout) {
                Ok(kind) => {
                    pending.insert(kind, Instant::now());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }

            flush_ready_refreshes(&app, &mut pending, debounce);
        }
    });
}

fn next_pending_timeout(pending: &HashMap<CliKind, Instant>, debounce: Duration) -> Duration {
    pending
        .values()
        .map(|instant| debounce.saturating_sub(instant.elapsed()))
        .min()
        .unwrap_or(Duration::from_secs(60))
}

fn flush_ready_refreshes(
    app: &AppHandle,
    pending: &mut HashMap<CliKind, Instant>,
    debounce: Duration,
) {
    let ready: Vec<CliKind> = pending
        .iter()
        .filter_map(|(kind, instant)| (instant.elapsed() >= debounce).then_some(*kind))
        .collect();

    if ready.is_empty() {
        return;
    }

    if !is_main_window_visible(app) {
        let deferred_at = Instant::now();
        for kind in ready {
            pending.insert(kind, deferred_at);
        }
        return;
    }

    for kind in ready {
        pending.remove(&kind);
        match commands::scan_projects_inner_for_cli(kind, None, None, false) {
            Ok(_) => commands::emit_session_list_index_updated(app, kind),
            Err(err) => tracing::warn!(
                "后台自动刷新会话索引失败: cli={}, err={}",
                kind.id(),
                err
            ),
        }
    }
}

fn is_main_window_visible(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{CreateKind, ModifyKind, RemoveKind};
    use std::path::PathBuf;

    #[test]
    fn test_session_file_create_triggers_event() {
        let sessions_dir = PathBuf::from("/home/user/.gemini/antigravity-cli/brain");
        let session_file = PathBuf::from("/home/user/.gemini/antigravity-cli/brain/sess-1/.system_generated/logs/transcript.jsonl");

        let event = Event {
            kind: EventKind::Create(CreateKind::File),
            paths: vec![session_file],
            attrs: Default::default(),
        };

        assert!(is_relevant_session_index_event(&event, CliKind::Antigravity, &sessions_dir));
    }

    #[test]
    fn test_session_file_remove_triggers_event() {
        let sessions_dir = PathBuf::from("/home/user/.gemini/antigravity-cli/brain");
        let session_file = PathBuf::from("/home/user/.gemini/antigravity-cli/brain/sess-1/.system_generated/logs/transcript.jsonl");

        let event = Event {
            kind: EventKind::Remove(RemoveKind::File),
            paths: vec![session_file],
            attrs: Default::default(),
        };

        assert!(is_relevant_session_index_event(&event, CliKind::Antigravity, &sessions_dir));
    }

    #[test]
    fn test_session_directory_remove_triggers_event() {
        let sessions_dir = PathBuf::from("/home/user/.gemini/antigravity-cli/brain");
        let session_dir = PathBuf::from("/home/user/.gemini/antigravity-cli/brain/sess-1");

        let event = Event {
            kind: EventKind::Remove(RemoveKind::Folder),
            paths: vec![session_dir],
            attrs: Default::default(),
        };

        assert!(is_relevant_session_index_event(&event, CliKind::Antigravity, &sessions_dir));
    }

    #[test]
    fn test_history_file_modify_does_not_trigger_event() {
        let sessions_dir = PathBuf::from("/home/user/.gemini/antigravity-cli/brain");
        let history_path = PathBuf::from("/home/user/.gemini/antigravity-cli/history.jsonl");

        let event = Event {
            kind: EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Content)),
            paths: vec![history_path],
            attrs: Default::default(),
        };

        // 持续对话导致的 history.jsonl 修改不再触发左侧列表重刷
        assert!(!is_relevant_session_index_event(&event, CliKind::Antigravity, &sessions_dir));
    }

    #[test]
    fn test_active_session_log_modify_does_not_trigger_event() {
        let sessions_dir = PathBuf::from("/home/user/.gemini/antigravity-cli/brain");
        let session_file = PathBuf::from("/home/user/.gemini/antigravity-cli/brain/sess-1/.system_generated/logs/transcript.jsonl");

        let event = Event {
            kind: EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Content)),
            paths: vec![session_file],
            attrs: Default::default(),
        };

        // 持续打字/工具调用的日志修改不触发会话树重载
        assert!(!is_relevant_session_index_event(&event, CliKind::Antigravity, &sessions_dir));
    }

    #[test]
    fn test_task_log_create_does_not_trigger_event() {
        let sessions_dir = PathBuf::from("/home/user/.gemini/antigravity-cli/brain");
        let task_log = PathBuf::from("/home/user/.gemini/antigravity-cli/brain/sess-1/.system_generated/tasks/task-1.log");

        let event = Event {
            kind: EventKind::Create(CreateKind::File),
            paths: vec![task_log],
            attrs: Default::default(),
        };

        // 工具运行生成的 task log 不触发会话树重载
        assert!(!is_relevant_session_index_event(&event, CliKind::Antigravity, &sessions_dir));
    }

    #[test]
    fn test_codex_rollout_create_triggers_event() {
        let sessions_dir = PathBuf::from("/home/user/.codex/sessions");
        let session_file = PathBuf::from("/home/user/.codex/sessions/2026/09/02/rollout-abc.jsonl");

        let event = Event {
            kind: EventKind::Create(CreateKind::File),
            paths: vec![session_file],
            attrs: Default::default(),
        };

        // Codex 会话文件为 .jsonl，新建必须触发会话树重拉
        assert!(is_relevant_session_index_event(&event, CliKind::Codex, &sessions_dir));
    }

    #[test]
    fn test_gemini_chat_create_triggers_event() {
        let sessions_dir = PathBuf::from("/home/user/.gemini/tmp");
        let session_file = PathBuf::from("/home/user/.gemini/tmp/abc123/chats/session-1.jsonl");

        let event = Event {
            kind: EventKind::Create(CreateKind::File),
            paths: vec![session_file],
            attrs: Default::default(),
        };

        // Gemini 会话文件为 .jsonl，新建必须触发会话树重拉
        assert!(is_relevant_session_index_event(&event, CliKind::Gemini, &sessions_dir));
    }

    #[test]
    fn test_other_irrelevant_file_does_not_trigger_event() {
        let sessions_dir = PathBuf::from("/home/user/.gemini/antigravity-cli/brain");
        let other_file = PathBuf::from("/home/user/other/unrelated.txt");

        let event = Event {
            kind: EventKind::Create(CreateKind::File),
            paths: vec![other_file],
            attrs: Default::default(),
        };

        assert!(!is_relevant_session_index_event(&event, CliKind::Antigravity, &sessions_dir));
    }
}
