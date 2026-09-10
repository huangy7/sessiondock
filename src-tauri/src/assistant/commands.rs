use crate::cli::CliKind;
use crate::app_db::SessionListIndexRecord;
use crate::error::{AppError, AppResult};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use tauri::Emitter;

#[derive(Default)]
pub struct TurnState {
    running: HashSet<String>, // conversation_id 集合
}

impl TurnState {
    pub fn try_start(&mut self, conversation_id: &str) -> bool {
        self.running.insert(conversation_id.to_string())
    }
    pub fn finish(&mut self, conversation_id: &str) {
        self.running.remove(conversation_id);
    }
}

pub static TURN_STATE: Mutex<Option<TurnState>> = Mutex::new(None);

fn with_turn_state<R>(f: impl FnOnce(&mut TurnState) -> R) -> R {
    let mut guard = TURN_STATE.lock().unwrap_or_else(|e| e.into_inner());
    let state = guard.get_or_insert_with(TurnState::default);
    f(state)
}

#[derive(Default)]
pub struct CancelRegistry {
    pids: HashMap<String, u32>, // conversation_id -> child pid
    cancelled: HashSet<String>, // 用户主动取消的 conversation_id
}

impl CancelRegistry {
    pub fn register(&mut self, conversation_id: &str, pid: u32) {
        self.pids.insert(conversation_id.to_string(), pid);
    }
    pub fn take(&mut self, conversation_id: &str) -> Option<u32> {
        self.pids.remove(conversation_id)
    }
    /// 用户主动取消时打标（仅在实际 kill 了进程时调用）
    pub fn mark_cancelled(&mut self, conversation_id: &str) {
        self.cancelled.insert(conversation_id.to_string());
    }
    /// 取出并清除取消标记；turn 结束时必须消费，避免残留
    pub fn take_cancelled(&mut self, conversation_id: &str) -> bool {
        self.cancelled.remove(conversation_id)
    }
}

pub static CANCEL_REGISTRY: Mutex<Option<CancelRegistry>> = Mutex::new(None);

fn with_cancel_registry<R>(f: impl FnOnce(&mut CancelRegistry) -> R) -> R {
    let mut guard = CANCEL_REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    let state = guard.get_or_insert_with(CancelRegistry::default);
    f(state)
}

#[tauri::command]
pub fn assistant_cancel(conversation_id: String) -> AppResult<()> {
    let pid = with_cancel_registry(|r| r.take(&conversation_id));
    if let Some(pid) = pid {
        // 先打标后 kill：避免 kill 与打标之间被调度走，run_turn_inner 提前消费不到标记
        with_cancel_registry(|r| r.mark_cancelled(&conversation_id));
        #[cfg(unix)]
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .output();
        }
    }
    Ok(())
}

/// 引用跳转目标会话（前缀解析 + tooltip 标题 + 打开参数）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRef {
    pub title: String,
    pub file_path: String,
    pub cli_id: String,
}

/// 纯函数：在一组索引记录里按 session_id 前缀找唯一目标；歧义取 modified_ms 最大者。
fn resolve_one_prefix(
    records: &[SessionListIndexRecord],
    cli_id: &str,
    prefix: &str,
) -> Option<SessionRef> {
    let mut best: Option<&SessionListIndexRecord> = None;
    for r in records {
        if !r.session_id.starts_with(prefix) {
            continue;
        }
        best = Some(match best {
            None => r,
            Some(b) if r.modified_ms > b.modified_ms => r,
            Some(b) => b,
        });
    }
    let r = best?;
    let title = r
        .title
        .clone()
        .or_else(|| r.first_user_message.clone())
        .unwrap_or_else(|| "未命名会话".to_string());
    Some(SessionRef {
        title,
        file_path: r.session_path.clone(),
        cli_id: cli_id.to_string(),
    })
}

/// 会话引用前缀合法性：小写十六进制 8 位（对齐前端 SESSION_REF_RE）
fn is_valid_ref_prefix(prefix: &str) -> bool {
    prefix.len() == 8
        && prefix
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// 文件存在校验：渲染与点击之间会话文件可能已被删除
fn session_file_exists(path: &str) -> bool {
    std::path::Path::new(path).exists()
}

/// 批量解析引用前缀 → 会话目标；未找到/已删 → None（前端据此抹掉无效引用）。
#[tauri::command]
pub fn resolve_session_refs(prefixes: Vec<String>) -> AppResult<Vec<Option<SessionRef>>> {
    // 引用可来自任一 CLI：预载所有 kind 的索引
    let mut by_kind: Vec<(String, Vec<SessionListIndexRecord>)> = Vec::new();
    for kind in CliKind::all() {
        let index = crate::app_db::read_session_list_index(kind)?;
        by_kind.push((kind.id().to_string(), index.into_values().collect()));
    }
    Ok(prefixes
        .iter()
        .map(|p| {
            if !is_valid_ref_prefix(p) {
                return None; // 畸形/空前缀直接判无效，保持输出与入参长度/顺序对应
            }
            by_kind
                .iter()
                .find_map(|(cli, records)| resolve_one_prefix(records, cli, p))
                .filter(|r| session_file_exists(&r.file_path)) // 已删 → None
        })
        .collect())
}

/// 点击引用角标：解析前缀 → 通知主窗口打开该会话（show+focus）。
#[tauri::command]
pub fn assistant_open_session(app: tauri::AppHandle, prefix: String) -> AppResult<()> {
    use tauri::{Emitter, Manager};

    if !is_valid_ref_prefix(&prefix) {
        return Err(AppError::business("无效的会话引用"));
    }

    // 跨全部 CLI 找目标（复用 Task 1 的纯函数）
    let mut target: Option<SessionRef> = None;
    for kind in CliKind::all() {
        let index = crate::app_db::read_session_list_index(kind)?;
        let records: Vec<SessionListIndexRecord> = index.into_values().collect();
        if let Some(r) = resolve_one_prefix(&records, kind.id(), &prefix) {
            target = Some(r);
            break;
        }
    }
    let target = target
        .filter(|r| session_file_exists(&r.file_path)) // 渲染与点击间被删 → 业务错误
        .ok_or_else(|| AppError::business("会话不存在或已删除"))?;

    let Some(main) = app.get_webview_window("main") else {
        return Err(AppError::business("主窗口不存在"));
    };
    let _ = main.show();
    let _ = main.set_focus();
    main.emit(
        "open-session-from-assistant",
        serde_json::json!({
            "cliId": target.cli_id,
            "filePath": target.file_path,
        }),
    )
    .map_err(|e| AppError::business(format!("通知主窗口失败: {}", e)))?;
    Ok(())
}

/// 发送一轮消息（conversation_id 为空 = 新对话）。
/// 流式事件经 `assistant-event` emit 到助手窗口。
#[tauri::command]
pub async fn assistant_send(
    app: tauri::AppHandle,
    conversation_id: Option<String>,
    prompt: String,
    profile: String,
    model: Option<String>,
) -> AppResult<serde_json::Value> {
    use super::conversations;

    let prompt = prompt.trim().chars().take(8000).collect::<String>();
    if prompt.is_empty() {
        return Err(AppError::business("消息不能为空"));
    }

    // 解析或创建对话
    let (conv_id, resume_id) = match &conversation_id {
        Some(id) => {
            let conv = conversations::get_conversation(id)?
                .ok_or_else(|| AppError::business("对话不存在"))?;
            if !with_turn_state(|s| s.try_start(id)) {
                return Err(AppError::business("上一轮还在进行中"));
            }
            // claude_session_id 为空字符串（首轮失败占位）时不传 --resume
            let resume = if conv.claude_session_id.is_empty() {
                None
            } else {
                Some(conv.claude_session_id)
            };
            (conv.id, resume)
        }
        None => {
            // 新对话：claude_session_id 占位，init 事件后回写真实 id
            let id = conversations::create_conversation(
                "",
                &profile,
                &conversations::title_from_prompt(&prompt),
            )?;
            if !with_turn_state(|s| s.try_start(&id)) {
                return Err(AppError::business("上一轮还在进行中"));
            }
            (id, None)
        }
    };

    // 引擎与配置
    let claude_bin = match crate::cli::find_cli_path(crate::cli::CliKind::Claude) {
        Some(p) => p,
        None => {
            with_turn_state(|s| s.finish(&conv_id));
            return Err(AppError::business("未检测到 claude CLI"));
        }
    };
    let proxy_abs = match crate::proxy::proxy_binary_path_pub() {
        Ok(p) => p.to_string_lossy().into_owned(),
        Err(e) => {
            with_turn_state(|s| s.finish(&conv_id));
            return Err(AppError::business(e));
        }
    };
    let settings_file = match crate::commands::profile::build_profile_settings_file(
        Some("claude".to_string()),
        profile.clone(),
        None,
    ) {
        Ok(f) => f,
        Err(e) => {
            with_turn_state(|s| s.finish(&conv_id));
            return Err(e);
        }
    };
    let workspace = crate::commands::data_dir()?.join("assistant").join("workspace");
    std::fs::create_dir_all(&workspace)?;

    let app_clone = app.clone();
    let conv_id_clone = conv_id.clone();
    tauri::async_runtime::spawn(async move {
        let conv_id_blocking = conv_id_clone.clone();
        let result = tauri::async_runtime::spawn_blocking(move || {
            run_turn_blocking(
                &app_clone,
                &conv_id_blocking,
                &claude_bin,
                &proxy_abs,
                &settings_file,
                &prompt,
                resume_id.as_deref(),
                model.as_deref(),
                &workspace,
            )
        })
        .await;
        with_turn_state(|s| s.finish(&conv_id_clone));
        match result {
            Err(e) => {
                let _ = app.emit(
                    "assistant-event",
                    serde_json::json!({
                        "conversationId": conv_id_clone,
                        "type": "error",
                        "error": e.to_string(),
                    }),
                );
            }
            Ok(Err(e)) => {
                let _ = app.emit(
                    "assistant-event",
                    serde_json::json!({
                        "conversationId": conv_id_clone,
                        "type": "error",
                        "error": e.to_string(),
                    }),
                );
            }
            Ok(Ok(())) => {}
        }
    });

    Ok(serde_json::json!({ "conversationId": conv_id }))
}

fn run_turn_blocking(
    app: &tauri::AppHandle,
    conv_id: &str,
    claude_bin: &str,
    proxy_abs: &str,
    settings_file: &str,
    prompt: &str,
    resume_id: Option<&str>,
    model: Option<&str>,
    workspace: &std::path::Path,
) -> AppResult<()> {
    let result = run_turn_inner(
        app,
        conv_id,
        claude_bin,
        proxy_abs,
        settings_file,
        prompt,
        resume_id,
        model,
        workspace,
    );
    // 成功/失败都清理临时 settings（pty.rs cleanup_settings_file_after_create_error 同款语义）
    if let Err(err) = std::fs::remove_file(settings_file) {
        if err.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!("清理临时 settings 失败: {}, err={}", settings_file, err);
        }
    }
    result
}

fn run_turn_inner(
    app: &tauri::AppHandle,
    conv_id: &str,
    claude_bin: &str,
    proxy_abs: &str,
    settings_file: &str,
    prompt: &str,
    resume_id: Option<&str>,
    model: Option<&str>,
    workspace: &std::path::Path,
) -> AppResult<()> {
    use super::{agent, conversations};

    let mut turn = agent::spawn_turn(
        claude_bin,
        proxy_abs,
        prompt,
        settings_file,
        resume_id,
        model,
        workspace,
    )?;
    with_cancel_registry(|r| r.register(conv_id, turn.child.id()));

    let app2 = app.clone();
    let conv2 = conv_id.to_string();
    // 记录是否收到过 result 事件：未收到且进程非零退出时，stderr 尾部是唯一的
    // 失败线索，必须上屏（对齐 swob 的 sawResult 合成错误事件语义）
    let saw_result_flag = std::cell::Cell::new(false);
    let saw_result = &saw_result_flag;
    let outcome = match agent::pump_stream(&mut turn, move |event| {
        if let agent::AgentStreamEvent::Result { ok, duration_ms, cost_usd, .. } = event {
            saw_result.set(true);
            // 记录每轮结果用于诊断截断/失败（output 截断时 claude 仍报 success，需结合 UI 判断）
            tracing::info!("agent 轮次结束: ok={}, duration={:?}ms, cost={:?}", ok, duration_ms, cost_usd);
        }
        // init 事件：回写真实 claude_session_id（新对话首次）
        if let agent::AgentStreamEvent::Init { ref session_id, .. } = event {
            let _ = conversations::touch_conversation(&conv2, session_id);
        }
        let _ = app2.emit(
            "assistant-event",
            serde_json::json!({
                "conversationId": conv2,
                "event": event,
            }),
        );
    }) {
        Ok(o) => o,
        Err(e) => {
            // pump 失败提前返回也要清掉 registry 条目与取消标记，避免残留误判后续轮次
            with_cancel_registry(|r| {
                r.take(conv_id);
                r.take_cancelled(conv_id);
            });
            return Err(e);
        }
    };

    // 正常结束：take 掉 registry 条目，避免 cancel 命中已退出进程的陈旧 pid
    with_cancel_registry(|r| r.take(conv_id));

    // 消费取消标记（turn 结束必须取走，无论成败，避免残留影响下轮判定）
    let was_cancelled = with_cancel_registry(|r| r.take_cancelled(conv_id));

    let ok = outcome.status.success();
    if !ok {
        if was_cancelled {
            // 用户主动停止：保留已生成内容，不误报错误，通知前端打「已停止」标记
            let _ = app.emit(
                "assistant-event",
                serde_json::json!({
                    "conversationId": conv_id,
                    "type": "cancelled",
                }),
            );
        } else {
            tracing::warn!("agent 异常退出: status={}, stderr 尾部={}", outcome.status, outcome.stderr_tail.trim());
            if !saw_result_flag.get() {
                // 没有 result 事件可解释失败：把 stderr 尾部推给前端上屏
                let detail = outcome.stderr_tail.trim();
                let msg = if detail.is_empty() {
                    format!("agent 异常退出（{}），无错误输出", outcome.status)
                } else {
                    format!("agent 运行失败: {}", detail)
                };
                let _ = app.emit(
                    "assistant-event",
                    serde_json::json!({
                        "conversationId": conv_id,
                        "type": "error",
                        "error": msg,
                    }),
                );
            }
        }
    }

    let _ = app.emit(
        "assistant-event",
        serde_json::json!({
            "conversationId": conv_id,
            "type": "done",
            "ok": ok,
        }),
    );
    Ok(())
}

fn join_text_parts(parts: &[serde_json::Value]) -> String {
    parts
        .iter()
        .filter(|p| p.get("type").and_then(|t| t.as_str()) == Some("text"))
        .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// 从 claude 会话 jsonl 行的 message.usage 提取 token 用量（缺字段补 0，无 usage 返回 None）
fn extract_usage(parsed: &serde_json::Value) -> Option<super::agent::TurnUsage> {
    let u = parsed.get("message")?.get("usage")?;
    Some(super::agent::TurnUsage {
        input_tokens: u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
        output_tokens: u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
        cache_read_input_tokens: u.get("cache_read_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
        cache_creation_input_tokens: u.get("cache_creation_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
    })
}

/// 从 claude 会话 jsonl 提取展示用消息：只保留真实用户输入与助手文本，
/// 跳过 tool_result/thinking/tool_use（对齐 swob loadSessionDetail 的 text-only 语义）
pub fn extract_display_messages(content: &str) -> Vec<(String, String, Option<super::agent::TurnUsage>)> {
    let mut out = Vec::new();
    for line in content.lines() {
        let Ok(parsed) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(event_type) = parsed.get("type").and_then(|v| v.as_str()) else {
            continue;
        };
        let content = parsed.get("message").and_then(|m| m.get("content"));
        match (event_type, content) {
            ("user", Some(serde_json::Value::String(s))) => {
                if !s.trim().is_empty() {
                    out.push(("user".to_string(), s.clone(), None));
                }
            }
            ("user", Some(serde_json::Value::Array(parts))) => {
                let text = join_text_parts(parts);
                if !text.is_empty() {
                    out.push(("user".to_string(), text, None));
                }
            }
            ("assistant", Some(serde_json::Value::Array(parts))) => {
                let text = join_text_parts(parts);
                if !text.is_empty() {
                    out.push(("assistant".to_string(), text, extract_usage(&parsed)));
                }
            }
            _ => {}
        }
    }
    out
}

/// 回放对话历史：直接解析 claude 会话 jsonl，提取干净的用户/助手文本
#[tauri::command]
pub fn assistant_conversation_messages(conversation_id: String) -> AppResult<Vec<serde_json::Value>> {
    use super::conversations;

    let conv = conversations::get_conversation(&conversation_id)?
        .ok_or_else(|| AppError::business("对话不存在"))?;
    let Some(session_path) = super::conversations::assistant_session_path(&conv.claude_session_id)? else {
        return Ok(vec![]);
    };
    if !session_path.exists() {
        return Ok(vec![]);
    }
    let content = std::fs::read_to_string(&session_path)?;
    let msgs = extract_display_messages(&content)
        .into_iter()
        .map(|(role, text, usage)| serde_json::json!({ "role": role, "text": text, "usage": usage }))
        .collect();
    Ok(msgs)
}

#[tauri::command]
pub fn assistant_list_conversations() -> AppResult<Vec<super::conversations::AssistantConversation>> {
    super::conversations::purge_orphan_conversations()?;
    super::conversations::list_conversations()
}

#[tauri::command]
pub fn assistant_delete_conversation(conversation_id: String) -> AppResult<()> {
    super::conversations::delete_conversation(&conversation_id)
}

#[tauri::command]
pub fn assistant_list_quick_phrases() -> AppResult<Vec<super::quick_phrases::QuickPhrase>> {
    super::quick_phrases::list_quick_phrases()
}

#[tauri::command]
pub fn assistant_save_quick_phrase(
    id: Option<String>,
    name: String,
    content: String,
) -> AppResult<String> {
    super::quick_phrases::save_quick_phrase(id.as_deref(), &name, &content)
}

#[tauri::command]
pub fn assistant_delete_quick_phrase(id: String) -> AppResult<()> {
    super::quick_phrases::delete_quick_phrase(&id)
}

#[tauri::command]
pub fn assistant_engine_status() -> AppResult<serde_json::Value> {
    let available = crate::cli::find_cli_path(crate::cli::CliKind::Claude).is_some();
    Ok(serde_json::json!({
        "available": available,
        "reasonCode": if available { serde_json::Value::Null } else { serde_json::json!("cli_not_found") },
    }))
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheStatus {
    pub total_sessions: usize,
    pub cached_sessions: usize,
    /// 从未提取过的会话数（无记录）。gone/empty 终态不计入——它们不是「待补齐」
    pub pending_sessions: usize,
    pub has_search_index: bool,
}

#[tauri::command]
pub fn assistant_cache_status() -> AppResult<CacheStatus> {
    let mut total = 0usize;
    let mut session_ids = std::collections::HashSet::new();
    for kind in [crate::cli::CliKind::Claude, crate::cli::CliKind::Codex, crate::cli::CliKind::Gemini] {
        let index = crate::app_db::read_session_list_index(kind)?;
        total += index.len();
        session_ids.extend(index.values().map(|r| r.session_id.clone()));
    }
    let db = crate::commands::data_dir()?.join("transcript_cache.db");
    let (cached, pending) = if db.exists() {
        match transcript_store::store::open(&db) {
            Ok(conn) => {
                let cached = transcript_store::store::count_cached(&conn).unwrap_or(0);
                let classified = transcript_store::store::all_records(&conn)
                    .map(|m| m.into_keys().collect::<std::collections::HashSet<_>>())
                    .unwrap_or_default();
                let pending = session_ids.iter().filter(|id| !classified.contains(*id)).count();
                (cached, pending)
            }
            Err(_) => (0, session_ids.len()),
        }
    } else {
        (0, session_ids.len())
    };
    let has_search_index = crate::app_db::tantivy_search::is_physical_index_ready()?;
    Ok(CacheStatus {
        total_sessions: total,
        cached_sessions: cached,
        pending_sessions: pending,
        has_search_index,
    })
}

#[tauri::command]
pub fn assistant_backfill_cache(app: tauri::AppHandle) -> AppResult<()> {
    super::backfill::start(app);
    Ok(())
}

#[tauri::command]
pub fn assistant_backfill_cancel() -> AppResult<()> {
    super::backfill::cancel();
    Ok(())
}

#[tauri::command]
pub fn assistant_new_conversation(profile: String) -> AppResult<String> {
    super::conversations::create_conversation("", &profile, "")
}

#[tauri::command]
pub async fn assistant_open_window(app: tauri::AppHandle) -> AppResult<()> {
    use tauri::Manager;
    if let Some(win) = app.get_webview_window("assistant") {
        let _ = win.show();
        let _ = win.set_focus();
        return Ok(());
    }
    build_assistant_window(&app)
}

fn build_assistant_window(app: &tauri::AppHandle) -> AppResult<()> {
    tauri::WebviewWindowBuilder::new(
        app,
        "assistant",
        tauri::WebviewUrl::App("src/assistant.html".into()),
    )
    .title("SessionDock 助手")
    .inner_size(560.0, 720.0)
    .min_inner_size(400.0, 500.0)
    .build()
    .map_err(|e| AppError::business(format!("打开助手窗口失败: {}", e)))?;
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantPendingPrompt {
    pub text: String,
    pub auto_send: bool,
}

/// Dashboard「问 SessionDock」待发送 prompt 或其他入口预填 prompt：
/// 窗口未开时由新窗口 mount 后取走，窗口已开时通过 assistant-dashboard-prompt 事件通知前端取走。
pub static PENDING_PROMPT: Mutex<Option<AssistantPendingPrompt>> = Mutex::new(None);

fn store_pending_prompt(prompt: String, auto_send: bool) {
    let mut guard = PENDING_PROMPT.lock().unwrap_or_else(|e| e.into_inner());
    *guard = Some(AssistantPendingPrompt {
        text: prompt,
        auto_send,
    });
}

fn take_pending_prompt() -> Option<AssistantPendingPrompt> {
    let mut guard = PENDING_PROMPT.lock().unwrap_or_else(|e| e.into_inner());
    guard.take()
}

/// Dashboard 提交「问 SessionDock」或预填 prompt：保证 prompt 在助手窗口启动过程中不丢失。
/// 窗口已存在时聚焦并 emit 事件；不存在时创建窗口，由前端 mount 后取 pending prompt。
#[tauri::command]
pub async fn assistant_open_with_prompt(
    app: tauri::AppHandle,
    prompt: String,
    auto_send: Option<bool>,
) -> AppResult<()> {
    use tauri::Manager;
    let prompt = prompt.trim().to_string();
    if prompt.is_empty() {
        return Err(AppError::business("消息不能为空"));
    }
    let auto_send = auto_send.unwrap_or(true);
    let existing = app.get_webview_window("assistant");
    store_pending_prompt(prompt, auto_send);
    if let Some(win) = existing {
        let _ = win.show();
        let _ = win.set_focus();
        win.emit("assistant-dashboard-prompt", ())
            .map_err(|e| AppError::business(format!("通知助手窗口失败: {}", e)))?;
        return Ok(());
    }
    build_assistant_window(&app)
}

/// 助手窗口取走待发送/预填 prompt（take 语义：每条 prompt 只被消费一次）。
#[tauri::command]
pub fn assistant_take_pending_prompt() -> AppResult<Option<AssistantPendingPrompt>> {
    Ok(take_pending_prompt())
}

#[tauri::command]
pub fn assistant_set_always_on_top(app: tauri::AppHandle, on_top: bool) -> AppResult<()> {
    use tauri::Manager;
    let win = app
        .get_webview_window("assistant")
        .ok_or_else(|| AppError::business("助手窗口不存在"))?;
    win.set_always_on_top(on_top)
        .map_err(|e| AppError::business(format!("设置置顶失败: {}", e)))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn pending_prompt_is_consumed_exactly_once() {
        // 单测合并在一个用例里：PENDING_PROMPT 是进程级静态，并行测试会互相干扰
        super::store_pending_prompt("first".to_string(), true);
        super::store_pending_prompt("second".to_string(), false);
        // 后写覆盖先写（窗口未开时用户连续提交，只保留最新一条）
        let p = super::take_pending_prompt().unwrap();
        assert_eq!(p.text, "second");
        assert!(!p.auto_send);
        // take 语义：消费后即空，不会重复发送
        assert_eq!(super::take_pending_prompt(), None);
    }

    #[test]
    fn extract_display_messages_keeps_only_user_and_assistant_text() {
        let jsonl = concat!(
            r#"{"type":"user","message":{"content":"生成一份周报"}}"#, "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"..."},{"type":"text","text":"好的"}]}}"#, "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"x"}}]}}"#, "\n",
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"[#1 user] 嵌套行"}]}}"#, "\n",
            r#"{"type":"user","message":{"content":[{"type":"text","text":"追问一下"}]}}"#, "\n",
            r#"{"type":"system","subtype":"init","session_id":"s"}"#, "\n",
        );
        let msgs = super::extract_display_messages(jsonl);
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0], ("user".to_string(), "生成一份周报".to_string(), None));
        assert_eq!(msgs[1], ("assistant".to_string(), "好的".to_string(), None));
        assert_eq!(msgs[2], ("user".to_string(), "追问一下".to_string(), None));
    }

    #[test]
    fn cancel_registry_tracks_running_children() {
        let mut reg = super::CancelRegistry::default();
        reg.register("conv-1", 12345);
        assert_eq!(reg.take("conv-1"), Some(12345));
        assert_eq!(reg.take("conv-1"), None);
    }

    #[test]
    fn busy_guard_rejects_second_turn() {
        let mut state = super::TurnState::default();
        assert!(state.try_start("conv-1"));
        assert!(!state.try_start("conv-1")); // 同对话进行中
        assert!(state.try_start("conv-2")); // 不同对话可以
        state.finish("conv-1");
        assert!(state.try_start("conv-1"));
    }

    #[test]
    fn cache_status_serializes_camel_case() {
        let status = super::CacheStatus {
            total_sessions: 10,
            cached_sessions: 4,
            pending_sessions: 3,
            has_search_index: true,
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["totalSessions"], 10);
        assert_eq!(json["cachedSessions"], 4);
        assert_eq!(json["pendingSessions"], 3);
        assert_eq!(json["hasSearchIndex"], true);
    }

    #[test]
    fn cancel_marks_and_turn_end_consumes_flag() {
        let mut reg = super::CancelRegistry::default();
        reg.register("conv-1", 12345);
        assert_eq!(reg.take("conv-1"), Some(12345));
        reg.mark_cancelled("conv-1");
        assert!(reg.take_cancelled("conv-1"));
        assert!(!reg.take_cancelled("conv-1")); // 已消费，不残留
    }

    #[test]
    fn take_cancelled_without_mark_is_false() {
        let mut reg = super::CancelRegistry::default();
        assert!(!reg.take_cancelled("conv-x"));
    }

    #[test]
    fn extract_display_messages_captures_assistant_usage() {
        let jsonl = concat!(
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"答"}],"usage":{"input_tokens":100,"output_tokens":20}}}"#, "\n",
            r#"{"type":"user","message":{"content":"问"}}"#, "\n",
        );
        let msgs = super::extract_display_messages(jsonl);
        assert_eq!(msgs.len(), 2);
        let u = msgs[0].2.expect("assistant 行应带 usage");
        assert_eq!(u.input_tokens, 100);
        assert_eq!(u.output_tokens, 20);
        assert_eq!(u.cache_read_input_tokens, 0);
        assert!(msgs[1].2.is_none()); // user 行无 usage
    }

    #[test]
    fn extract_display_messages_ignores_lines_without_usage() {
        let jsonl = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"答"}]}}"#;
        let msgs = super::extract_display_messages(jsonl);
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].2.is_none());
    }

    fn rec(session_id: &str, title: Option<&str>, fum: Option<&str>, path: &str, modified: i64) -> super::SessionListIndexRecord {
        super::SessionListIndexRecord {
            session_path: path.to_string(),
            session_id: session_id.to_string(),
            project_path: None,
            title: title.map(String::from),
            first_user_message: fum.map(String::from),
            first_timestamp: None,
            last_timestamp: None,
            git_branch: String::new(),
            file_size: 0,
            modified_ms: modified,
            has_archive_snapshot: false,
            is_archived: false,
        }
    }

    #[test]
    fn resolve_unique_prefix_returns_match() {
        let records = vec![
            rec("90259911-abcd-0000-0000-000000000000", Some("cores"), None, "/a.jsonl", 100),
            rec("3f8e5341-abcd-0000-0000-000000000000", Some("other"), None, "/b.jsonl", 200),
        ];
        let r = super::resolve_one_prefix(&records, "claude", "90259911").unwrap();
        assert_eq!(r.title, "cores");
        assert_eq!(r.file_path, "/a.jsonl");
    }

    #[test]
    fn resolve_ambiguous_prefix_picks_most_recent() {
        let records = vec![
            rec("90259911-aaaa-0000-0000-000000000000", Some("old"), None, "/old.jsonl", 100),
            rec("90259911-bbbb-0000-0000-000000000000", Some("new"), None, "/new.jsonl", 200),
        ];
        let r = super::resolve_one_prefix(&records, "claude", "90259911").unwrap();
        assert_eq!(r.title, "new"); // modified_ms 大者胜
        assert_eq!(r.file_path, "/new.jsonl");
    }

    #[test]
    fn resolve_unknown_prefix_returns_none() {
        let records = vec![rec("90259911-abcd-0000-0000-000000000000", None, None, "/a.jsonl", 100)];
        assert!(super::resolve_one_prefix(&records, "claude", "deadbeef").is_none());
    }

    #[test]
    fn resolve_title_falls_back_to_first_user_message_then_unnamed() {
        let with_title = vec![rec("aaaaaaaa-0000-0000-0000-000000000000", Some("标题"), Some("首条消息"), "/a.jsonl", 1)];
        assert_eq!(super::resolve_one_prefix(&with_title, "claude", "aaaaaaaa").unwrap().title, "标题");

        let with_fum = vec![rec("bbbbbbbb-0000-0000-0000-000000000000", None, Some("首条消息"), "/b.jsonl", 1)];
        assert_eq!(super::resolve_one_prefix(&with_fum, "claude", "bbbbbbbb").unwrap().title, "首条消息");

        let bare = vec![rec("cccccccc-0000-0000-0000-000000000000", None, None, "/c.jsonl", 1)];
        assert_eq!(super::resolve_one_prefix(&bare, "claude", "cccccccc").unwrap().title, "未命名会话");
    }

    #[test]
    fn session_file_exists_distinguishes_existing_and_deleted() {
        // 存在的文件：temp_dir 写唯一文件，测完清理
        let path = std::env::temp_dir().join(format!("sessiondock-ref-test-{}.jsonl", std::process::id()));
        std::fs::write(&path, "{}").unwrap();
        assert!(super::session_file_exists(&path.to_string_lossy()));
        std::fs::remove_file(&path).unwrap();
        // 已删/不存在 → false
        assert!(!super::session_file_exists(&path.to_string_lossy()));
    }

    #[test]
    fn resolve_ref_filter_keeps_existing_and_drops_deleted() {
        // 已存在的文件（当前测试二进制）→ filter 保留
        let existing = std::env::current_exe().unwrap().to_string_lossy().into_owned();
        let records = vec![rec("aaaaaaaa-0000-0000-0000-000000000000", Some("kept"), None, &existing, 100)];
        let r = super::resolve_one_prefix(&records, "claude", "aaaaaaaa")
            .filter(|r| super::session_file_exists(&r.file_path));
        assert!(r.is_some());

        // 不存在的文件路径 → filter 丢弃（对应「文件已删」状态）
        let records_missing = vec![rec(
            "bbbbbbbb-0000-0000-0000-000000000000",
            Some("gone"),
            None,
            "/definitely/not/exists.jsonl",
            100,
        )];
        let r = super::resolve_one_prefix(&records_missing, "claude", "bbbbbbbb")
            .filter(|r| super::session_file_exists(&r.file_path));
        assert!(r.is_none());
    }

    #[test]
    fn resolve_one_prefix_is_filesystem_independent() {
        // 纯函数不应检查文件系统：即使 file_path 不存在也照常返回解析结果
        let records = vec![rec(
            "90259911-abcd-0000-0000-000000000000",
            Some("cores"),
            None,
            "/definitely/not/exists.jsonl",
            100,
        )];
        let r = super::resolve_one_prefix(&records, "claude", "90259911").unwrap();
        assert_eq!(r.title, "cores");
        assert_eq!(r.file_path, "/definitely/not/exists.jsonl");
    }

    #[test]
    fn is_valid_ref_prefix_rejects_malformed_and_uppercase() {
        assert!(super::is_valid_ref_prefix("90259911"));
        assert!(!super::is_valid_ref_prefix("")); // 空
        assert!(!super::is_valid_ref_prefix("9025991")); // 太短
        assert!(!super::is_valid_ref_prefix("902599110")); // 太长
        assert!(!super::is_valid_ref_prefix("9025991g")); // 非 hex
        assert!(!super::is_valid_ref_prefix("9025991Z")); // 大写
        assert!(!super::is_valid_ref_prefix("90_59911")); // 非法字符
    }
}
