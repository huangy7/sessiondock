//! Streaming IPC helpers.
//!
//! 所有需要分批推送大列表给前端的命令复用本模块的事件命名规则：
//! - `<topic>:<request_id>:chunk` — payload 是一批数据（具体类型由命令决定）
//! - `<topic>:<request_id>:done`  — payload 是完成元数据（如总偏移量）
//! - `<topic>:<request_id>:error` — payload 是 `StreamError { message }`

use serde::Serialize;
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, Serialize)]
pub struct StreamError {
    pub message: String,
}

/// 拼出 `<topic>:<request_id>:<event>` 形式的事件名。
pub fn event_name(topic: &str, request_id: &str, event: &str) -> String {
    format!("{}:{}:{}", topic, request_id, event)
}

/// emit chunk — 出错时记日志，返回 false 表示前端已断开，调用方可提前终止
pub fn emit_chunk<T: Serialize + Clone>(
    app: &AppHandle,
    topic: &str,
    request_id: &str,
    payload: &T,
) -> bool {
    let name = event_name(topic, request_id, "chunk");
    if let Err(e) = app.emit(&name, payload) {
        tracing::warn!("emit chunk failed: event={} err={}", name, e);
        return false;
    }
    true
}

pub fn emit_done<T: Serialize + Clone>(
    app: &AppHandle,
    topic: &str,
    request_id: &str,
    payload: &T,
) {
    let name = event_name(topic, request_id, "done");
    if let Err(e) = app.emit(&name, payload) {
        tracing::warn!("emit done failed: event={} err={}", name, e);
    }
}

pub fn emit_error(app: &AppHandle, topic: &str, request_id: &str, message: impl Into<String>) {
    let name = event_name(topic, request_id, "error");
    let err = StreamError { message: message.into() };
    if let Err(e) = app.emit(&name, &err) {
        tracing::warn!("emit error failed: event={} err={}", name, e);
    }
}

/// 启动一个流式后端任务的统一入口：spawn_blocking + catch_unwind + panic 兜底 emit_error。
///
/// 用法：
/// ```ignore
/// streaming::spawn_streaming_task(app, "session", request_id, |app, topic, request_id| {
///     // 业务核心：parse 文件 / 查 DB / emit chunk / emit done
/// });
/// ```
///
/// `work` 闭包内若 panic，会被 `catch_unwind` 接住，外层自动 emit `<topic>:<request_id>:error`，
/// 避免前端流式 UI 永远停在 loading 状态。
pub fn spawn_streaming_task<F>(
    app: AppHandle,
    topic: &'static str,
    request_id: String,
    work: F,
)
where
    F: FnOnce(&AppHandle, &str, &str) + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let app_for_panic = app.clone();
        let request_id_for_panic = request_id.clone();
        let topic_for_panic = topic;

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            work(&app, topic, &request_id);
        }));

        if let Err(payload) = result {
            let panic_msg = payload.downcast_ref::<&str>().copied()
                .or_else(|| payload.downcast_ref::<String>().map(|s| s.as_str()))
                .unwrap_or("<non-string panic>");
            tracing::error!("streaming task '{}' panicked: {}", topic_for_panic, panic_msg);
            emit_error(
                &app_for_panic,
                topic_for_panic,
                &request_id_for_panic,
                "操作发生内部错误（panic），请反馈给开发者",
            );
        }
    });
}
