use crate::error::{AppError, AppResult};
use super::format_timestamp;
use crate::app_db;
use crate::cli::{self, CliKind};
use crate::history;
use crate::parser;
use crate::session::{self, ChatMessage, ContentPart, SearchResult, UsageRecord};
use crate::streaming;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::Path;
use tauri::AppHandle;

#[tauri::command]
pub async fn load_session(file_path: String, cli_id: Option<String>, skip_sidechain_filter: Option<bool>) -> AppResult<session::SessionLoadResult> {
    tauri::async_runtime::spawn_blocking(move || {
        let skip_sidechain = skip_sidechain_filter.unwrap_or(false);

        if Path::new(&file_path).exists() {
            return parser::parse_session_file_with_offset(&file_path, skip_sidechain).map_err(AppError::from);
        }

        // File missing — try archived content
        let kind = CliKind::from_id(cli_id.as_deref()).unwrap_or(CliKind::Claude);
        if let Ok(Some(data)) = app_db::read_archived_session_content(kind.id(), &file_path) {
            let content = String::from_utf8(data)
                .map_err(|e| AppError::business(e.to_string()))?;
            let messages = parser::parse_session_content_by_kind(&file_path, &content);
            return Ok(session::SessionLoadResult { messages, offset: 0, subagent_map: std::collections::HashMap::new() });
        }

        Err(AppError::business(format!("会话文件不存在: {}", file_path)))
    })
    .await
    .unwrap_or_else(|_| Err(AppError::business("任务执行失败")))
}

#[tauri::command]
pub async fn load_session_incremental(
    file_path: String,
    offset: u64,
    skip_sidechain_filter: Option<bool>,
) -> AppResult<session::SessionLoadResult> {
    tauri::async_runtime::spawn_blocking(move || {
        let skip_sidechain = skip_sidechain_filter.unwrap_or(false);
        parser::parse_session_incremental(&file_path, offset, skip_sidechain).map_err(AppError::from)
    })
    .await
    .unwrap_or_else(|_| Err(AppError::business("任务执行失败")))
}

#[tauri::command]
pub fn resolve_session_id(cli_id: Option<String>, file_path: String) -> AppResult<Option<String>> {
    // cli_id 仅用于保持命令 ABI 兼容；subagents 歧义守卫必须与 CLI 类型无关，
    // 否则切换 CLI 后对旧 Tab 的重新解析会绕过守卫。
    let _ = &cli_id;

    let is_subagent_file = Path::new(&file_path)
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        == Some("subagents")
        || parser::is_subagent_session(&file_path);
    if is_subagent_file {
        return Ok(None);
    }

    Ok(parser::read_session_id(&file_path).filter(|id| !id.trim().is_empty()))
}

const SESSION_STREAM_TOPIC: &str = "session";
const SESSION_STREAM_BATCH_SIZE: usize = 50;

#[derive(Debug, Clone, Serialize)]
struct SessionStreamDone {
    offset: u64,
    subagent_map: HashMap<String, session::SubagentInfo>,
}

/// 流式加载会话：通过 `session:<request_id>:chunk` / `:done` / `:error` 事件向前端推送。
/// 命令本身**立刻返回 Ok(())**——真正的工作在 `spawn_blocking` 里跑。
#[tauri::command]
pub async fn load_session_stream(
    app: AppHandle,
    request_id: String,
    file_path: String,
    cli_id: Option<String>,
    skip_sidechain_filter: Option<bool>,
) -> AppResult<()> {
    let skip_sidechain = skip_sidechain_filter.unwrap_or(false);

    streaming::spawn_streaming_task(app, SESSION_STREAM_TOPIC, request_id, move |app, topic, request_id| {
        // 文件不存在 -> 走归档兜底（与 load_session 等价）
        if !Path::new(&file_path).exists() {
            let kind = CliKind::from_id(cli_id.as_deref()).unwrap_or(CliKind::Claude);
            match app_db::read_archived_session_content(kind.id(), &file_path) {
                Ok(Some(data)) => {
                    let content = match String::from_utf8(data) {
                        Ok(s) => s,
                        Err(e) => {
                            streaming::emit_error(app, topic, request_id, e.to_string());
                            return;
                        }
                    };
                    let messages = parser::parse_session_content_by_kind(&file_path, &content);
                    if !messages.is_empty() {
                        streaming::emit_chunk(app, topic, request_id, &messages);
                    }
                    streaming::emit_done(app, topic, request_id, &SessionStreamDone {
                        offset: 0,
                        subagent_map: HashMap::new(),
                    });
                    return;
                }
                Ok(None) => {
                    streaming::emit_error(app, topic, request_id,
                        format!("会话文件不存在: {}", file_path));
                    return;
                }
                Err(e) => {
                    streaming::emit_error(app, topic, request_id, e.to_string());
                    return;
                }
            }
        }

        // 文件存在 -> 流式解析 + 分批 emit
        let result = parser::parse_session_file_streaming(
            &file_path,
            skip_sidechain,
            SESSION_STREAM_BATCH_SIZE,
            |batch| {
                streaming::emit_chunk(app, topic, request_id, &batch);
                true
            },
        );

        match result {
            Ok((offset, subagent_map)) => {
                streaming::emit_done(app, topic, request_id, &SessionStreamDone {
                    offset,
                    subagent_map,
                });
            }
            Err(e) => streaming::emit_error(app, topic, request_id, e),
        }
    });

    Ok(())
}

#[tauri::command]
pub fn export_session(
    cli_id: String,
    file_path: String,
    save_path: String,
    format: String,
    selected_indexes: Option<Vec<usize>>,
) -> AppResult<String> {
    CliKind::from_id(Some(cli_id.as_str()))?;
    let mut messages = parser::parse_session_file(&file_path)
        .map_err(|_| "无法加载会话内容。".to_string())?;
    
    if let Some(indexes) = selected_indexes {
        let mut index_set = std::collections::HashSet::new();
        for i in indexes { index_set.insert(i); }
        let mut filtered = Vec::new();
        for (i, msg) in messages.into_iter().enumerate() {
            if index_set.contains(&i) {
                filtered.push(msg);
            }
        }
        messages = filtered;
    }

    let output = match format.as_str() {
        "markdown" => format_as_markdown(&messages),
        "json" => format_as_json(&messages),
        "jsonl" => {
            let mut s = String::new();
            for m in &messages {
                if let Ok(line) = serde_json::to_string(m) {
                    s.push_str(&line);
                    s.push('\n');
                }
            }
            s
        },
        _ => format_as_text(&messages),
    };

    // Ensure the save path has the correct file extension
    let ext = match format.as_str() {
        "markdown" => ".md",
        "json" => ".json",
        "jsonl" => ".jsonl",
        _ => ".txt",
    };
    let final_path = if save_path.ends_with(ext) {
        save_path
    } else {
        format!("{}{}", save_path, ext)
    };

    // Validate the parent directory exists
    let final_path_buf = std::path::Path::new(&final_path);
    if let Some(parent) = final_path_buf.parent() {
        if !parent.exists() {
            return Err(AppError::business(format!("目录不存在: {}", parent.display())));
        }
    }

    let mut file = fs::File::create(&final_path).map_err(|_| "无法创建文件。".to_string())?;
    file.write_all(output.as_bytes())
        .map_err(|_| "无法创建文件。".to_string())?;

    Ok("会话已导出成功。".to_string())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchExportSessionIdentity {
    cli_id: String,
    file_path: String,
}

#[tauri::command]
pub fn batch_export_sessions(
    sessions: Vec<BatchExportSessionIdentity>,
    save_path: String,
    format: String,
    mode: String,
) -> AppResult<String> {
    let file_paths = sessions
        .into_iter()
        .map(|identity| {
            CliKind::from_id(Some(identity.cli_id.as_str()))?;
            if identity.file_path.trim().is_empty() {
                return Err(AppError::business("会话路径不能为空"));
            }
            Ok(identity.file_path)
        })
        .collect::<AppResult<Vec<_>>>()?;
    let ext = match format.as_str() {
        "markdown" => ".md",
        "json" => ".json",
        _ => ".txt",
    };

    if mode == "merged" {
        let mut all_messages: Vec<ChatMessage> = Vec::new();
        for fp in &file_paths {
            let msgs = parser::parse_session_file(fp)
                .map_err(|_| format!("无法加载会话: {}", fp))?;
            all_messages.extend(msgs);
        }

        let output = match format.as_str() {
            "markdown" => format_as_markdown(&all_messages),
            "json" => format_as_json(&all_messages),
            _ => format_as_text(&all_messages),
        };

        let final_path = if save_path.ends_with(ext) {
            save_path
        } else {
            format!("{}{}", save_path, ext)
        };

        let mut file = fs::File::create(&final_path).map_err(|_| "无法创建文件。".to_string())?;
        file.write_all(output.as_bytes())
            .map_err(|_| "写入文件失败。".to_string())?;

        Ok(format!("已合并导出 {} 个会话。", file_paths.len()))
    } else {
        let base = if save_path.ends_with(ext) {
            save_path[..save_path.len() - ext.len()].to_string()
        } else {
            save_path.clone()
        };

        for (i, fp) in file_paths.iter().enumerate() {
            let msgs = parser::parse_session_file(fp)
                .map_err(|_| format!("无法加载会话: {}", fp))?;

            let output = match format.as_str() {
                "markdown" => format_as_markdown(&msgs),
                "json" => format_as_json(&msgs),
                _ => format_as_text(&msgs),
            };

            let final_path = format!("{}_{}{}", base, i + 1, ext);
            let mut file =
                fs::File::create(&final_path).map_err(|_| "无法创建文件。".to_string())?;
            file.write_all(output.as_bytes())
                .map_err(|_| "写入文件失败。".to_string())?;
        }

        Ok(format!("已分别导出 {} 个会话。", file_paths.len()))
    }
}

/// Fork a session at a specific transcript entry (identified by its uuid).
/// Copies the original JSONL up to and including the line carrying `anchor_uuid`,
/// rewrites every kept line's `sessionId` to a fresh UUID, writes a new file next
/// to the original, and returns the new session ID.
/// 递归复制目录（file-history 快照用，目标已存在时跳过整体拷贝）
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Fork 核心（纯函数，便于单测）：保留到 anchor_uuid 所在行（含），此后全部丢弃；
/// 所有保留行的 sessionId 重写为 new_id。anchor 未出现时报错。
fn fork_truncate_lines(
    lines: impl IntoIterator<Item = String>,
    anchor_uuid: &str,
    new_id: &str,
) -> Result<Vec<String>, String> {
    let mut kept: Vec<String> = Vec::new();
    let mut found = false;

    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            kept.push(line);
            continue;
        }

        let entry: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => {
                kept.push(line);
                continue;
            }
        };

        let is_anchor = entry
            .get("uuid")
            .or_else(|| entry.get("id"))
            .or_else(|| entry.get("callId"))
            .and_then(|u| u.as_str())
            == Some(anchor_uuid);

        // Rewrite sessionId so the forked transcript owns a fresh session identity.
        let new_line = if entry.get("sessionId").is_some() {
            let mut obj = entry;
            obj["sessionId"] = serde_json::Value::String(new_id.to_string());
            serde_json::to_string(&obj).unwrap_or(line)
        } else {
            line
        };

        kept.push(new_line);
        if is_anchor {
            found = true;
            break;
        }
    }

    if !found {
        return Err("目标消息未找到，会话可能已更新".to_string());
    }
    Ok(kept)
}

fn sync_workbuddy_forked_session_to_db(
    db_path: &Path,
    old_id: &str,
    new_id: &str,
) -> Result<(), rusqlite::Error> {
    let conn = rusqlite::Connection::open(db_path)?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let rows_affected = conn.execute(
        r#"
        INSERT INTO sessions (
            id, cwd, user_id, title, custom_title, status, created_at, updated_at, last_activity_at, deleted_at,
            is_playground, source_mode, is_background_automation, mode, model, expert_id, expert_locale,
            expert_runtime_identity, expert_marketplace, permission_mode, use_sandbox_cli, project_id,
            plugin_context_json, last_user_prompt_expert_selection
        )
        SELECT 
            ?1, cwd, user_id,
            CASE 
                WHEN title IS NOT NULL AND title != '' THEN title || ' (分支)' 
                ELSE '新建分支会话' 
            END,
            custom_title, 'completed',
            ?2, ?2, ?2, NULL,
            is_playground, source_mode, is_background_automation, mode, model, expert_id, expert_locale,
            expert_runtime_identity, expert_marketplace, permission_mode, use_sandbox_cli, project_id,
            plugin_context_json, last_user_prompt_expert_selection
        FROM sessions WHERE id = ?3;
        "#,
        rusqlite::params![new_id, now_ms, old_id],
    )?;

    if rows_affected == 0 {
        let _ = conn.execute(
            r#"
            INSERT INTO sessions (
                id, cwd, user_id, title, status, created_at, updated_at, last_activity_at, is_playground
            ) VALUES (?1, '', 'default', '新建分支会话', 'completed', ?2, ?2, ?2, 0);
            "#,
            rusqlite::params![new_id, now_ms],
        );
    }

    Ok(())
}

#[tauri::command]
pub fn fork_session(
    cli_id: Option<String>,
    file_path: String,
    anchor_uuid: String,
) -> AppResult<String> {
    use std::io::{BufRead, BufReader};
    use uuid::Uuid;

    let kind = CliKind::from_id(cli_id.as_deref())?;
    if kind != CliKind::Claude && kind != CliKind::WorkBuddy {
        return Err(AppError::business(format!("{} 暂不支持从任意消息位置继续会话。", kind.name())));
    }

    // Validate file_path is within the CLI's sessions directory
    let sessions_root = cli::sessions_dir(kind).map_err(AppError::business)?;
    let file_path_buf = Path::new(&file_path);
    if !file_path_buf.starts_with(&sessions_root) {
        return Err(AppError::business("会话文件路径无效"));
    }

    let file = fs::File::open(&file_path).map_err(|e| AppError::business(e.to_string()))?;
    let reader = BufReader::new(file);

    let new_id = Uuid::new_v4().to_string();
    let kept_lines = fork_truncate_lines(reader.lines().map_while(Result::ok), &anchor_uuid, &new_id)
        .map_err(AppError::business)?;

    // Write new file next to the original
    let original_path = Path::new(&file_path);
    let parent = original_path.parent().ok_or("无法获取会话目录")?;
    let new_file_path = parent.join(format!("{}.jsonl", new_id));

    let mut writer =
        fs::File::create(&new_file_path).map_err(|e| AppError::business(e.to_string()))?;
    for line in &kept_lines {
        writeln!(writer, "{}", line)?;
    }

    if kind == CliKind::Claude {
        // 同步复制 /rewind 代码回滚快照（~/.claude/file-history/<旧id>/ → <新id>/），
        // 让 fork 出的会话也能回滚代码；目录不存在或拷贝失败不阻断 fork
        if let (Some(old_id), Ok(data_dir)) = (
            original_path.file_stem().and_then(|s| s.to_str()),
            cli::data_dir(kind),
        ) {
            let src_history = data_dir.join("file-history").join(old_id);
            if src_history.is_dir() {
                let dst_history = data_dir.join("file-history").join(&new_id);
                if let Err(e) = copy_dir_recursive(&src_history, &dst_history) {
                    eprintln!("[fork_session] file-history copy failed: {}", e);
                }
            }
        }
    } else if kind == CliKind::WorkBuddy {
        // 同步注册到 WorkBuddy 本地 SQLite 数据库（~/.workbuddy/workbuddy.db）
        if let (Some(old_id), Ok(data_dir)) = (
            original_path.file_stem().and_then(|s| s.to_str()),
            cli::data_dir(kind),
        ) {
            let db_path = data_dir.join("workbuddy.db");
            if db_path.is_file() {
                if let Err(e) = sync_workbuddy_forked_session_to_db(&db_path, old_id, &new_id) {
                    eprintln!("[fork_session] WorkBuddy SQLite 注册失败: {}", e);
                }
            }
        }
    }

    Ok(new_id)
}

#[derive(Debug, Clone)]
struct AggregatedSessionSearch {
    session_path: String,
    snippet: String,
    match_count: usize,
    matched_doc_count: usize,
    first_match_message_index: Option<usize>,
    rank: f64,
}

fn count_case_insensitive_occurrences(text: &str, query_lower: &str) -> usize {
    if text.is_empty() || query_lower.is_empty() {
        return 0;
    }

    text.to_lowercase().matches(query_lower).count()
}

fn ensure_session_search_index_ready(kind: CliKind) -> AppResult<()> {
    if app_db::count_session_search_indexed_sessions(kind)? == 0 {
        super::scan_projects_inner_for_cli(kind, None, None, false)?;
    }
    Ok(())
}

fn aggregate_session_searches(
    query: &str,
    candidates: Vec<app_db::SessionSearchCandidate>,
    matched_docs: Vec<app_db::SessionSearchMatchedDoc>,
) -> Vec<AggregatedSessionSearch> {
    let query_lower = query.to_lowercase();
    let mut docs_by_session: HashMap<String, Vec<app_db::SessionSearchMatchedDoc>> = HashMap::new();
    for doc in matched_docs {
        docs_by_session
            .entry(doc.session_path.clone())
            .or_default()
            .push(doc);
    }

    let mut aggregated = Vec::new();
    for candidate in candidates {
        let docs = docs_by_session.remove(&candidate.session_path).unwrap_or_default();
        if docs.is_empty() {
            continue;
        }

        let mut match_count = 0usize;
        let mut snippet = String::new();
        let mut first_match_message_index = candidate.first_match_message_index;

        for doc in docs {
            let current_count = count_case_insensitive_occurrences(&doc.search_text, &query_lower);
            if current_count == 0 {
                continue;
            }
            match_count += current_count;
            if snippet.is_empty() {
                snippet = session::extract_snippet(&doc.search_text, query, 100);
                first_match_message_index = Some(doc.message_index);
            }
        }

        if match_count == 0 {
            continue;
        }

        aggregated.push(AggregatedSessionSearch {
            session_path: candidate.session_path,
            snippet,
            match_count,
            matched_doc_count: candidate.matched_doc_count,
            first_match_message_index,
            rank: candidate.rank,
        });
    }

    aggregated.sort_by(|a, b| {
        b.match_count
            .cmp(&a.match_count)
            .then_with(|| b.matched_doc_count.cmp(&a.matched_doc_count))
            .then_with(|| a.rank.partial_cmp(&b.rank).unwrap_or(Ordering::Equal))
            .then_with(|| a.first_match_message_index.cmp(&b.first_match_message_index))
            .then_with(|| a.session_path.cmp(&b.session_path))
    });
    aggregated.truncate(50);
    aggregated
}

fn merge_session_id_search_matches(
    _query: &str,
    mut candidates: Vec<app_db::SessionSearchCandidate>,
    matched_docs: Vec<app_db::SessionSearchMatchedDoc>,
    session_id_matches: Vec<app_db::SessionListIndexRecord>,
) -> (Vec<app_db::SessionSearchCandidate>, Vec<app_db::SessionSearchMatchedDoc>) {
    let mut candidate_paths: HashSet<String> = candidates
        .iter()
        .map(|candidate| candidate.session_path.clone())
        .collect();
    let mut session_id_docs = Vec::new();

    for record in session_id_matches {
        let session_path = record.session_path;
        let session_id_text = format!("Session ID: {}", record.session_id);

        if candidate_paths.insert(session_path.clone()) {
            candidates.push(app_db::SessionSearchCandidate {
                session_path: session_path.clone(),
                first_match_message_index: Some(0),
                matched_doc_count: 1,
                rank: -1.0,
            });
        }

        session_id_docs.push(app_db::SessionSearchMatchedDoc {
            session_path,
            message_index: 0,
            search_text: session_id_text,
        });
    }

    session_id_docs.extend(matched_docs);
    let matched_docs = session_id_docs;
    (candidates, matched_docs)
}

fn fallback_session_id_from_path(file_path: &str) -> String {
    Path::new(file_path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("")
        .to_string()
}

fn build_claude_search_results(
    aggregated: Vec<AggregatedSessionSearch>,
    records: &HashMap<String, app_db::SessionListIndexRecord>,
    custom_names: &HashMap<String, String>,
    session_map: &HashMap<String, history::HistoryEntry>,
    project_map: &HashMap<String, String>,
) -> Vec<SearchResult> {
    aggregated
        .into_iter()
        .map(|entry| {
            let record = records.get(&entry.session_path);
            let session_id = record
                .map(|value| value.session_id.clone())
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| fallback_session_id_from_path(&entry.session_path));
            let encoded_dir = Path::new(&entry.session_path)
                .parent()
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str())
                .filter(|value| !value.is_empty())
                .unwrap_or("unknown")
                .to_string();
            let project_path = session::resolve_project_path(
                &encoded_dir,
                record.and_then(|value| value.project_path.as_deref()),
                Some(project_map),
            );
            let display_name = crate::db::title_resolver::resolve_display_name(
                custom_session_name(custom_names, CliKind::Claude, &entry.session_path),
                record.and_then(|value| value.title.as_deref()),
                record.and_then(|value| value.first_user_message.as_deref()),
                session_map
                    .get(&session_id)
                    .map(|history| history.display.as_str())
                    .filter(|value| !value.is_empty()),
                &session_id,
            );

            SearchResult {
                session_id,
                file_path: entry.session_path,
                display_name,
                project_path,
                snippet: entry.snippet,
                match_count: entry.match_count,
                first_match_message_index: entry.first_match_message_index,
                cli_id: String::new(),
            }
        })
        .collect()
}

fn build_codex_search_results(
    aggregated: Vec<AggregatedSessionSearch>,
    records: &HashMap<String, app_db::SessionListIndexRecord>,
    custom_names: &HashMap<String, String>,
    session_map: &HashMap<String, history::HistoryEntry>,
    kind: CliKind,
) -> Vec<SearchResult> {
    // 单次读取 Codex 索引 thread_name Map，整批搜索结果 O(1) 命中
    let codex_titles = parser::load_codex_index_titles();
    // WorkBuddy 用户重命名在 workbuddy.db（sessions.custom_title），覆盖原生 AI 标题
    let wb_titles = if kind == CliKind::WorkBuddy {
        parser::load_workbuddy_custom_titles()
    } else {
        HashMap::new()
    };
    aggregated
        .into_iter()
        .map(|entry| {
            let record = records.get(&entry.session_path);
            let session_id = record
                .map(|value| value.session_id.clone())
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| {
                    Path::new(&entry.session_path)
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .and_then(|stem| stem.rsplit('-').next())
                        .unwrap_or("")
                        .to_string()
                });
            let project_path = record
                .and_then(|value| value.project_path.clone())
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "未知项目".to_string());
            let title = record
                .and_then(|value| value.title.as_deref())
                .or_else(|| codex_titles.get(&session_id).map(String::as_str))
                .or_else(|| {
                    Path::new(&entry.session_path)
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .and_then(|stem| codex_titles.get(stem).map(String::as_str))
                });
            let title = wb_titles
                .get(&session_id)
                .or_else(|| {
                    Path::new(&entry.session_path)
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .and_then(|stem| wb_titles.get(stem))
                })
                .map(String::as_str)
                .or(title);
            let display_name = crate::db::title_resolver::resolve_display_name(
                custom_session_name(custom_names, kind, &entry.session_path),
                title,
                record.and_then(|value| value.first_user_message.as_deref()),
                session_map
                    .get(&session_id)
                    .map(|history| history.display.as_str())
                    .filter(|value| !value.is_empty()),
                &session_id,
            );

            SearchResult {
                session_id,
                file_path: entry.session_path,
                display_name,
                project_path,
                snippet: entry.snippet,
                match_count: entry.match_count,
                first_match_message_index: entry.first_match_message_index,
                cli_id: String::new(),
            }
        })
        .collect()
}

/// 内部 helper：执行 search_sessions 的完整 pipeline。
/// 已假定 `trimmed` 非空、`kind` 已解析、search index 已 ready。
/// 抽出供 search_sessions（sync 命令）与 search_sessions_stream（P3）共用。
fn run_search_sessions_pipeline(kind: CliKind, trimmed: &str) -> AppResult<Vec<SearchResult>> {
    let candidates = app_db::search_session_candidates(kind, trimmed, 200)?;
    let session_id_matches = app_db::search_session_ids(kind, trimmed, 50)?;
    if candidates.is_empty() {
        if session_id_matches.is_empty() {
            return Ok(Vec::new());
        }
    }

    let session_paths: Vec<String> = candidates
        .iter()
        .map(|candidate| candidate.session_path.clone())
        .collect();
    let matched_docs = app_db::read_session_search_docs_for_paths(kind, trimmed, &session_paths)?;
    let (candidates, matched_docs) =
        merge_session_id_search_matches(trimmed, candidates, matched_docs, session_id_matches);
    let aggregated = aggregate_session_searches(trimmed, candidates, matched_docs);
    if aggregated.is_empty() {
        return Ok(Vec::new());
    }

    let resolved_paths: Vec<String> = aggregated
        .iter()
        .map(|entry| entry.session_path.clone())
        .collect();
    let records = app_db::read_session_list_index_for_paths(kind, &resolved_paths)?;
    let custom_names = load_session_names().unwrap_or_default();

    let mut results = match kind {
        CliKind::Claude => {
            let history_path = cli::history_path(CliKind::Claude)?;
            let (session_map, project_map) = history::parse_history(history_path.to_str().unwrap_or(""));
            build_claude_search_results(
                aggregated,
                &records,
                &custom_names,
                &session_map,
                &project_map,
            )
        }
        CliKind::Codex => {
            let history_path = cli::history_path(CliKind::Codex)?;
            let session_map = history::parse_codex_history(history_path.to_str().unwrap_or(""));
            build_codex_search_results(
                aggregated,
                &records,
                &custom_names,
                &session_map,
                CliKind::Codex,
            )
        }
        CliKind::Gemini => build_codex_search_results(
            aggregated,
            &records,
            &custom_names,
            &HashMap::new(),
            CliKind::Gemini,
        ),
        CliKind::WorkBuddy => build_codex_search_results(
            aggregated,
            &records,
            &custom_names,
            &HashMap::new(),
            CliKind::WorkBuddy,
        ),
        CliKind::Dsh => build_codex_search_results(
            aggregated,
            &records,
            &custom_names,
            &HashMap::new(),
            CliKind::Dsh,
        ),
        CliKind::Antigravity => build_codex_search_results(
            aggregated,
            &records,
            &custom_names,
            &HashMap::new(),
            CliKind::Antigravity,
        ),
    };
    stamp_results_cli_id(&mut results, kind);
    Ok(results)
}

/// 为搜索结果盖戳来源 CLI。抽成小函数以便单元测试直接断言盖戳行为
/// （完整 pipeline 依赖全局 DB 与 tantivy 物理索引,无测试替身,见 cross_cli_tests）。
fn stamp_results_cli_id(results: &mut [SearchResult], kind: CliKind) {
    for r in results.iter_mut() {
        r.cli_id = kind.id().to_string();
    }
}

#[tauri::command]
pub fn search_sessions(cli_id: Option<String>, query: String) -> AppResult<Vec<SearchResult>> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let kind = CliKind::from_id(cli_id.as_deref())?;
    ensure_session_search_index_ready(kind)?;
    run_search_sessions_pipeline(kind, trimmed)
}

const SEARCH_SESSIONS_TOPIC: &str = "search_sessions";
const SEARCH_SESSIONS_BATCH_SIZE: usize = 20;

#[derive(Debug, Clone, serde::Serialize)]
struct SearchSessionsStreamDone {
    total: usize,
    query: String,
    /// 所选范围内从未建立内容索引、本次未参与搜索的 CLI
    pending_cli_ids: Vec<String>,
    /// 所选范围内参与了搜索但索引有增量未同步的 CLI(结果可能缺少最新内容)
    #[serde(default)]
    stale_cli_ids: Vec<String>,
    #[serde(default)]
    cli_errors: Vec<SearchStreamCliError>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct SearchStreamCliError {
    cli_id: String,
    message: String,
}

/// 多 CLI fan-out 中单个 CLI 的处置决策(纯函数,便于测试)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AllSearchDisposition {
    /// 无数据目录/无会话:不参与,也不提示
    Skip,
    /// 从未建立过内容索引(或物理索引缺失):进 pending 引导
    Pending,
    /// 参与搜索;stale = 有会话更新尚未同步进索引(仍搜索已有内容)
    Search { stale: bool },
}

fn all_search_disposition(
    data_dir_exists: bool,
    physical_index_ready: bool,
    missing_sessions: usize,
    total_sessions: usize,
) -> AllSearchDisposition {
    if !data_dir_exists {
        return AllSearchDisposition::Skip;
    }
    if !physical_index_ready {
        // 物理索引缺失:有会话才值得引导重建
        return if total_sessions > 0 {
            AllSearchDisposition::Pending
        } else {
            AllSearchDisposition::Skip
        };
    }
    if total_sessions > 0 && missing_sessions >= total_sessions {
        // 一条都没索引过 → 尚未建立
        return AllSearchDisposition::Pending;
    }
    AllSearchDisposition::Search {
        stale: missing_sessions > 0,
    }
}

enum SearchKindStreamResult {
    Skip,
    Pending,
    Search {
        results: Vec<SearchResult>,
        stale: bool,
    },
}

fn search_kind_for_stream(kind: CliKind, trimmed: &str) -> AppResult<SearchKindStreamResult> {
    let status = super::session_index::resolve_search_index_status(kind)?;
    match all_search_disposition(
        true,
        status.physical_index_ready,
        status.missing_sessions,
        status.total_sessions,
    ) {
        AllSearchDisposition::Skip => Ok(SearchKindStreamResult::Skip),
        AllSearchDisposition::Pending => Ok(SearchKindStreamResult::Pending),
        AllSearchDisposition::Search { stale } => Ok(SearchKindStreamResult::Search {
            results: run_search_sessions_pipeline(kind, trimmed)?,
            stale,
        }),
    }
}

/// 流式全局搜索：通过 `search_sessions:<request_id>:chunk` / `:done` / `:error` 推送。
/// chunk payload = `Vec<SearchResult>`（每批 20 条），顺序与 search_sessions 同步命令一致。
/// 对 cli_ids 指定的 CLI fan-out，front_cli_id 在集合中时优先；索引未就绪的 CLI
/// 记入 done 事件的 pending_cli_ids，单来源失败则记入 cli_errors 并继续其他来源。
/// 命令本身立刻返回 Ok(())。
#[tauri::command]
pub async fn search_sessions_stream(
    app: AppHandle,
    request_id: String,
    cli_ids: Option<Vec<String>>,
    front_cli_id: Option<String>,
    query: String,
    cli_id: Option<String>,
) -> AppResult<()> {
    let trimmed = query.trim().to_string();
    let effective_ids = super::session_index::effective_cli_ids(cli_ids, cli_id);
    let kinds = super::session_index::requested_kinds(
        effective_ids.as_deref(),
        front_cli_id.as_deref(),
    )?;

    streaming::spawn_streaming_task(app, SEARCH_SESSIONS_TOPIC, request_id, move |app, topic, request_id| {
        if trimmed.is_empty() {
            // 空 query 快路径：不查 DB，直接 done(0)。前端 useStreamingCollection 会清空 items
            streaming::emit_done(
                app,
                topic,
                request_id,
                &SearchSessionsStreamDone {
                    total: 0,
                    query: trimmed.clone(),
                    pending_cli_ids: Vec::new(),
                    stale_cli_ids: Vec::new(),
                    cli_errors: Vec::new(),
                },
            );
            return;
        }

        let mut pending: Vec<String> = Vec::new();
        let mut stale: Vec<String> = Vec::new();
        let mut cli_errors = Vec::new();
        let mut total = 0usize;
        super::session_index::run_cli_fanout(
            &kinds,
            |kind| search_kind_for_stream(kind, &trimmed),
            |outcome| match outcome.result {
                Ok(SearchKindStreamResult::Skip) => {}
                Ok(SearchKindStreamResult::Pending) => {
                    pending.push(outcome.kind.id().to_string());
                }
                Ok(SearchKindStreamResult::Search { results, stale: is_stale }) => {
                    if is_stale {
                        stale.push(outcome.kind.id().to_string());
                    }
                    total += results.len();
                    for batch in results.chunks(SEARCH_SESSIONS_BATCH_SIZE) {
                        streaming::emit_chunk(app, topic, request_id, &batch.to_vec());
                    }
                }
                Err(message) => {
                    cli_errors.push(SearchStreamCliError {
                        cli_id: outcome.kind.id().to_string(),
                        message,
                    });
                }
            },
        );
        streaming::emit_done(
            app,
            topic,
            request_id,
            &SearchSessionsStreamDone {
                total,
                query: trimmed.clone(),
                pending_cli_ids: pending,
                stale_cli_ids: stale,
                cli_errors,
            },
        );
    });

    Ok(())
}

// ---- Cross-CLI Title Search ----

#[derive(Debug, Clone, serde::Serialize)]
pub struct CrossCliSessionHit {
    pub session_id: String,
    pub file_path: String,
    pub display_name: String,
    pub project_path: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CrossCliTitleGroup {
    pub cli_id: String,
    pub cli_name: String,
    pub hits: Vec<CrossCliSessionHit>,
}

/// 侧边栏跨 CLI 标题搜索:排除当前 CLI,查其余 CLI 的会话列表索引。
/// 未安装(数据目录不存在)的 CLI 静默跳过;不触发任何扫描/索引构建。
#[tauri::command]
pub fn search_cross_cli_titles(
    current_cli_id: Option<String>,
    query: String,
) -> AppResult<Vec<CrossCliTitleGroup>> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let current = CliKind::from_id(current_cli_id.as_deref())?;
    let custom_names = load_session_names().unwrap_or_default();

    let mut groups = Vec::new();
    for kind in CliKind::all() {
        if kind == current {
            continue;
        }
        // 用数据目录存在性代替 detect_cli(避免每次搜索 spawn login shell)
        let installed = crate::cli::data_dir(kind).map(|d| d.exists()).unwrap_or(false);
        if !installed {
            continue;
        }
        let records = app_db::search_session_titles(kind, trimmed, 20)?;
        if records.is_empty() {
            continue;
        }
        // WorkBuddy 用户重命名在 workbuddy.db（sessions.custom_title），覆盖原生 AI 标题
        let wb_titles = if kind == CliKind::WorkBuddy {
            parser::load_workbuddy_custom_titles()
        } else {
            HashMap::new()
        };
        let hits = records
            .into_iter()
            .map(|r| {
                let display_name = crate::db::title_resolver::resolve_display_name(
                    custom_session_name(&custom_names, kind, &r.session_path),
                    wb_titles
                        .get(&r.session_id)
                        .map(String::as_str)
                        .or(r.title.as_deref()),
                    r.first_user_message.as_deref(),
                    None,
                    &r.session_id,
                );
                CrossCliSessionHit {
                    session_id: r.session_id,
                    file_path: r.session_path,
                    display_name,
                    project_path: r.project_path.unwrap_or_default(),
                    timestamp: r.last_timestamp.or(r.first_timestamp).unwrap_or_default(),
                }
            })
            .collect();
        groups.push(CrossCliTitleGroup {
            cli_id: kind.id().to_string(),
            cli_name: kind.name().to_string(),
            hits,
        });
    }
    Ok(groups)
}

// ---- Session Stats ----

#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionStats {
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_creation_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub total_duration_ms: u64,
    pub turn_count: usize,
}

#[tauri::command]
pub async fn get_session_stats(
    cli_id: Option<String>,
    file_path: String,
) -> AppResult<SessionStats> {
    let _kind = CliKind::from_id(cli_id.as_deref())?;
    tauri::async_runtime::spawn_blocking(move || {
        let records = parser::extract_usage_records(&file_path, "");
        let mut stats = SessionStats {
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cache_creation_tokens: 0,
            total_cache_read_tokens: 0,
            total_duration_ms: 0,
            turn_count: records.len(),
        };
        for r in &records {
            stats.total_input_tokens += r.input_tokens;
            stats.total_output_tokens += r.output_tokens;
            stats.total_cache_creation_tokens += r.cache_creation_tokens;
            stats.total_cache_read_tokens += r.cache_read_tokens;
            if let Some(dur) = r.duration_ms {
                stats.total_duration_ms += dur;
            }
        }
        Ok(stats)
    })
    .await
    .unwrap_or_else(|_| Err(AppError::business("任务执行失败")))
}

#[tauri::command]
pub fn rename_session(cli_id: String, file_path: String, new_name: String) -> AppResult<()> {
    let kind = CliKind::from_id(Some(cli_id.as_str()))?;
    let mut names = load_session_names().unwrap_or_default();
    let key = session_name_key(kind, &file_path);
    // 一旦用户在新版本改名，移除旧路径键，避免它继续作为其他来源的兼容回退。
    names.remove(&file_path);
    if new_name.trim().is_empty() {
        names.remove(&key);
    } else {
        names.insert(key, new_name.trim().to_string());
    }
    app_db::write_session_names(&names)
}

#[tauri::command]
pub async fn get_usage_stats(cli_id: Option<String>) -> AppResult<Vec<UsageRecord>> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    tauri::async_runtime::spawn_blocking(move || get_usage_stats_sync(kind))
        .await
        .map_err(|e| AppError::business(e.to_string()))?
}

fn get_usage_stats_sync(kind: CliKind) -> AppResult<Vec<UsageRecord>> {
    // WorkBuddy 无用用量数据；其余 CLI（Claude/Codex/Gemini/DSH）均有 extract_usage_records 实现
    if kind == CliKind::WorkBuddy {
        return Ok(Vec::new());
    }

    let projects = super::scan_projects_inner_for_cli(kind, None, None, false)?.projects;
    let mut all_records: Vec<UsageRecord> = Vec::new();

    for project in projects {
        let project_name = project
            .original_path
            .replace('\\', "/")
            .split('/')
            .filter(|s| !s.is_empty())
            .last()
            .unwrap_or(&project.original_path)
            .to_string();

        for session_info in project.sessions {
            let records =
                parser::extract_usage_records(&session_info.file_path, &project_name);
            all_records.extend(records);
        }
    }

    Ok(all_records)
}

/// Load custom session names map
pub(crate) fn load_session_names() -> AppResult<HashMap<String, String>> {
    app_db::load_session_names()
}

/// 用户自定义对话名称的持久化键必须携带来源 CLI，避免不同来源的同路径互相覆盖。
pub(crate) fn session_name_key(kind: CliKind, file_path: &str) -> String {
    format!("{}\u{0}{}", kind.id(), file_path)
}

/// 读取时兼容旧版只按路径保存的名称；新写入始终使用复合键。
pub(crate) fn custom_session_name<'a>(
    names: &'a HashMap<String, String>,
    kind: CliKind,
    file_path: &str,
) -> Option<&'a str> {
    names
        .get(&session_name_key(kind, file_path))
        .or_else(|| names.get(file_path))
        .map(String::as_str)
}

/// 报告与书签消费者还支持历史上按 session ID 保存的自定义名称。
pub(crate) fn custom_session_name_or_legacy_id<'a>(
    names: &'a HashMap<String, String>,
    kind: CliKind,
    file_path: &str,
    session_id: &str,
) -> Option<&'a str> {
    custom_session_name(names, kind, file_path)
        .or_else(|| names.get(session_id).map(String::as_str))
}

// ---- Format helpers ----

fn format_as_text(messages: &[ChatMessage]) -> String {
    let mut output = String::new();
    output.push('\u{FEFF}');

    for msg in messages {
        let ts = format_timestamp(&msg.timestamp);
        match msg.role.as_str() {
            "user" => output.push_str(&format!("[user] {}\n", ts)),
            "assistant" => {
                if let Some(ref model) = msg.model {
                    output.push_str(&format!("[assistant] {} ({})\n", ts, model));
                } else {
                    output.push_str(&format!("[assistant] {}\n", ts));
                }
            }
            _ => output.push_str(&format!("[{}] {}\n", msg.role, ts)),
        }
        for part in &msg.content_parts {
            match part {
                ContentPart::Text { text } => {
                    output.push_str(text);
                    output.push('\n');
                }
                ContentPart::ToolUse { summary, .. } => {
                    output.push_str(summary);
                    output.push('\n');
                }
                ContentPart::ToolResult { summary, .. } => {
                    output.push_str(summary);
                    output.push('\n');
                }
                ContentPart::Thinking { .. } | ContentPart::Image { .. } | ContentPart::ImageRef { .. } | ContentPart::ImageMeta => {}
            }
        }
        output.push('\n');
    }
    output
}

fn format_as_markdown(messages: &[ChatMessage]) -> String {
    let mut output = String::new();

    for msg in messages {
        let ts = format_timestamp(&msg.timestamp);
        let role_label = match msg.role.as_str() {
            "user" => "User".to_string(),
            "assistant" => match &msg.model {
                Some(model) => format!("Assistant ({})", model),
                None => "Assistant".to_string(),
            },
            other => other.to_string(),
        };

        output.push_str(&format!("### {} — {}\n\n", role_label, ts));

        for part in &msg.content_parts {
            match part {
                ContentPart::Text { text } => {
                    output.push_str(text);
                    output.push_str("\n\n");
                }
                ContentPart::ToolUse {
                    tool_name, input, ..
                } => {
                    output.push_str(&format!(
                        "<details>\n<summary><code>{}</code></summary>\n\n```json\n{}\n```\n\n</details>\n\n",
                        tool_name, input
                    ));
                }
                ContentPart::ToolResult {
                    summary, content, ..
                } => {
                    output.push_str(&format!(
                        "<details>\n<summary>{}</summary>\n\n```\n{}\n```\n\n</details>\n\n",
                        summary, content
                    ));
                }
                ContentPart::Thinking { thinking } => {
                    output.push_str(&format!(
                        "<details>\n<summary><em>Thinking...</em></summary>\n\n{}\n\n</details>\n\n",
                        thinking
                    ));
                }
                ContentPart::Image { .. } | ContentPart::ImageRef { .. } => {
                    output.push_str("*[Image]*\n\n");
                }
                ContentPart::ImageMeta => {}
            }
        }

        output.push_str("---\n\n");
    }
    output
}

fn format_as_json(messages: &[ChatMessage]) -> String {
    serde_json::to_string_pretty(messages).unwrap_or_else(|_| "[]".to_string())
}

#[tauri::command]
pub fn get_archive_retention_days() -> AppResult<i64> {
    app_db::get_archive_retention_days()
}

#[tauri::command]
pub fn set_archive_retention_days(days: i64) -> AppResult<()> {
    app_db::set_archive_retention_days(days)
}

#[tauri::command]
pub fn get_session_archive_pinned(cli_id: Option<String>, file_path: String) -> AppResult<bool> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    app_db::is_session_archive_pinned(kind, &file_path)
}

#[tauri::command]
pub fn get_session_archive_status(
    cli_id: Option<String>,
    file_path: String,
) -> AppResult<app_db::SessionArchiveStatus> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    app_db::get_session_archive_status(kind, &file_path)
}

#[tauri::command]
pub async fn set_session_archive_pinned(
    cli_id: Option<String>,
    file_path: String,
    pinned: bool,
) -> AppResult<bool> {
    // spawn_blocking：pin 会对整个会话 jsonl 做 gzip，大会话同步执行会卡主线程
    tauri::async_runtime::spawn_blocking(move || {
        let kind = CliKind::from_id(cli_id.as_deref())?;
        app_db::set_session_archive_pinned(kind, &file_path, pinned)
    })
    .await
    .map_err(|e| AppError::business(format!("设置永久保留失败: {}", e)))?
}

#[tauri::command]
pub async fn list_archived_sessions(
    cli_id: Option<String>,
) -> AppResult<Vec<app_db::ArchivedSessionEntry>> {
    // spawn_blocking：归档列表会触发退化行快照解析（gunzip），不能阻塞主线程
    tauri::async_runtime::spawn_blocking(move || {
        let kind = CliKind::from_id(cli_id.as_deref())?;
        // 救回历史 bug 误删索引行的归档会话（失败不阻塞列表）
        let _ = app_db::restore_missing_archived_index_rows(kind);
        app_db::list_archived_sessions(kind)
    })
    .await
    .map_err(|e| AppError::business(format!("加载归档列表失败: {}", e)))?
}

#[tauri::command]
pub fn delete_session_archive(cli_id: Option<String>, file_path: String) -> AppResult<bool> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    app_db::delete_session_archive(kind, &file_path)?;
    Ok(true)
}

#[tauri::command]
pub fn restore_session_to_disk(cli_id: Option<String>, file_path: String) -> AppResult<bool> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    app_db::restore_session_to_disk(kind, &file_path)?;
    Ok(true)
}

#[tauri::command]
pub async fn rebuild_archive_index_from_disk(
    app: AppHandle,
    cli_id: Option<String>,
) -> AppResult<app_db::RebuildArchiveResult> {
    // spawn_blocking：全量 gunzip + 元数据解析为重 IO/CPU 操作，同步命令会冻结主线程
    tauri::async_runtime::spawn_blocking(move || {
        app_db::rebuild_archive_index_from_disk(&app, cli_id.as_deref())
    })
    .await
    .map_err(|e| AppError::business(format!("重建归档任务执行失败: {}", e)))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_names_are_scoped_by_cli_for_same_path() {
        let path = "/shared/session.jsonl";
        let mut names = HashMap::new();
        names.insert(session_name_key(CliKind::Claude, path), "Claude 对话".to_string());
        names.insert(session_name_key(CliKind::Codex, path), "Codex 对话".to_string());

        assert_eq!(custom_session_name(&names, CliKind::Claude, path), Some("Claude 对话"));
        assert_eq!(custom_session_name(&names, CliKind::Codex, path), Some("Codex 对话"));
    }

    #[test]
    fn report_and_bookmark_readers_keep_cli_scoped_names_with_legacy_fallbacks() {
        let path = "/shared/session.jsonl";
        let mut names = HashMap::new();
        names.insert(session_name_key(CliKind::Claude, path), "Claude 对话".to_string());
        names.insert(session_name_key(CliKind::Codex, path), "Codex 对话".to_string());
        names.insert("legacy-session-id".to_string(), "旧对话".to_string());

        assert_eq!(
            custom_session_name_or_legacy_id(&names, CliKind::Claude, path, "legacy-session-id"),
            Some("Claude 对话"),
        );
        assert_eq!(
            custom_session_name_or_legacy_id(&names, CliKind::Codex, path, "legacy-session-id"),
            Some("Codex 对话"),
        );
        assert_eq!(
            custom_session_name_or_legacy_id(&HashMap::from([("legacy-session-id".to_string(), "旧对话".to_string())]), CliKind::Codex, path, "legacy-session-id"),
            Some("旧对话"),
        );
        assert_eq!(
            custom_session_name_or_legacy_id(
                &HashMap::from([(path.to_string(), "旧路径对话".to_string())]),
                CliKind::Claude,
                path,
                "different-session-id",
            ),
            Some("旧路径对话"),
        );
        assert_eq!(
            custom_session_name_or_legacy_id(
                &HashMap::from([(path.to_string(), "旧路径对话".to_string())]),
                CliKind::Codex,
                path,
                "different-session-id",
            ),
            Some("旧路径对话"),
        );
    }

    #[test]
    fn resolve_session_id_rejects_ambiguous_claude_subagent() {
        let resolved = resolve_session_id(
            Some("claude".to_string()),
            "/tmp/parent-session/subagents/agent-123.jsonl".to_string(),
        )
        .unwrap();

        assert_eq!(resolved, None);
    }

    #[test]
    fn fork_truncate_keeps_up_to_anchor_inclusive_and_rewrites_session_id() {
        let lines = vec![
            r#"{"type":"summary","summary":"s"}"#.to_string(),
            r#"{"type":"user","uuid":"u1","sessionId":"old","message":{"content":"hi"}}"#.to_string(),
            r#"{"type":"assistant","uuid":"u2","sessionId":"old","message":{"content":[]}}"#.to_string(),
            r#"{"type":"user","uuid":"u3","sessionId":"old","message":{"content":"later"}}"#.to_string(),
        ];

        let kept = fork_truncate_lines(lines, "u2", "new-id").unwrap();

        assert_eq!(kept.len(), 3);
        for line in &kept {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            if v.get("sessionId").is_some() {
                assert_eq!(v["sessionId"], "new-id");
            }
        }
        let last: serde_json::Value = serde_json::from_str(&kept[2]).unwrap();
        assert_eq!(last["uuid"], "u2");
    }

    #[test]
    fn fork_truncate_errors_when_anchor_missing() {
        let lines = vec![
            r#"{"type":"user","uuid":"u1","sessionId":"old"}"#.to_string(),
        ];
        assert!(fork_truncate_lines(lines, "nope", "new-id").is_err());
    }

    #[test]
    fn fork_truncate_ignores_uuid_in_dropped_tail() {
        let lines = vec![
            r#"{"type":"user","uuid":"u1","sessionId":"old"}"#.to_string(),
            r#"{"type":"user","uuid":"u1","sessionId":"old"}"#.to_string(),
        ];
        // 第一条 u1 即截断，重复的 uuid 也不会被越过
        let kept = fork_truncate_lines(lines, "u1", "new-id").unwrap();
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn resolve_session_id_rejects_subagent_regardless_of_cli() {
        let resolved = resolve_session_id(
            Some("codex".to_string()),
            "/tmp/parent-session/subagents/agent-123.jsonl".to_string(),
        )
        .unwrap();

        assert_eq!(resolved, None);
    }

    #[test]
    fn session_id_matches_are_added_as_search_candidates() {
        let candidates = Vec::new();
        let docs = Vec::new();
        let session_id_matches = vec![app_db::SessionListIndexRecord {
            session_path: "/tmp/session-a.jsonl".to_string(),
            session_id: "abc123-session".to_string(),
            project_path: Some("/tmp/project".to_string()),
            title: None,
            first_user_message: Some("first prompt".to_string()),
            first_timestamp: None,
            last_timestamp: None,
            git_branch: String::new(),
            file_size: 0,
            modified_ms: 0,
            has_archive_snapshot: false,
            is_archived: false,
        }];

        let (merged_candidates, merged_docs) =
            merge_session_id_search_matches("ABC123", candidates, docs, session_id_matches);

        assert_eq!(merged_candidates.len(), 1);
        assert_eq!(merged_candidates[0].session_path, "/tmp/session-a.jsonl");
        assert_eq!(merged_candidates[0].first_match_message_index, Some(0));
        assert_eq!(merged_docs.len(), 1);
        assert_eq!(merged_docs[0].search_text, "Session ID: abc123-session");
    }

    #[test]
    fn session_id_match_doc_is_prioritized_over_content_docs() {
        let candidates = vec![app_db::SessionSearchCandidate {
            session_path: "/tmp/session-a.jsonl".to_string(),
            first_match_message_index: Some(8),
            matched_doc_count: 1,
            rank: 0.0,
        }];
        let docs = vec![app_db::SessionSearchMatchedDoc {
            session_path: "/tmp/session-a.jsonl".to_string(),
            message_index: 8,
            search_text: "content also mentions abc123".to_string(),
        }];
        let session_id_matches = vec![app_db::SessionListIndexRecord {
            session_path: "/tmp/session-a.jsonl".to_string(),
            session_id: "abc123-session".to_string(),
            project_path: Some("/tmp/project".to_string()),
            title: None,
            first_user_message: Some("first prompt".to_string()),
            first_timestamp: None,
            last_timestamp: None,
            git_branch: String::new(),
            file_size: 0,
            modified_ms: 0,
            has_archive_snapshot: false,
            is_archived: false,
        }];

        let (merged_candidates, merged_docs) =
            merge_session_id_search_matches("abc123", candidates, docs, session_id_matches);

        assert_eq!(merged_candidates.len(), 1);
        assert_eq!(merged_docs[0].search_text, "Session ID: abc123-session");
        assert_eq!(merged_docs[1].search_text, "content also mentions abc123");
    }
}

#[cfg(test)]
mod cross_cli_tests {
    use crate::cli::CliKind;

    fn empty_result() -> crate::session::SearchResult {
        crate::session::SearchResult {
            session_id: "s1".into(),
            file_path: "/tmp/a.jsonl".into(),
            display_name: "t".into(),
            project_path: "/p".into(),
            snippet: String::new(),
            match_count: 1,
            first_match_message_index: None,
            cli_id: String::new(),
        }
    }

    // 注:不对 run_search_sessions_pipeline 做端到端断言 —— 它依赖全局
    // LazyLock DB 连接池(指向真实用户库,无测试替身)与 tantivy 物理索引,
    // 播种成本过高且有污染真实数据的风险。盖戳逻辑抽为 stamp_results_cli_id，
    // CLI 子集解析与故障隔离在 session_index 的纯函数测试中覆盖。

    #[test]
    fn stamp_results_cli_id_stamps_kind_id_on_every_result() {
        let mut results = vec![empty_result(), empty_result()];
        super::stamp_results_cli_id(&mut results, CliKind::WorkBuddy);
        assert!(results.iter().all(|r| r.cli_id == "workbuddy"));

        super::stamp_results_cli_id(&mut results, CliKind::Claude);
        assert!(results.iter().all(|r| r.cli_id == "claude"));
    }

    #[test]
    fn all_search_disposition_three_states() {
        use super::{all_search_disposition, AllSearchDisposition};
        // 无数据目录 → Skip
        assert_eq!(
            all_search_disposition(false, true, 0, 10),
            AllSearchDisposition::Skip
        );
        // 物理索引缺失:有会话 → Pending;无会话 → Skip
        assert_eq!(
            all_search_disposition(true, false, 0, 10),
            AllSearchDisposition::Pending
        );
        assert_eq!(
            all_search_disposition(true, false, 0, 0),
            AllSearchDisposition::Skip
        );
        // 从未索引过任何会话(missing == total)→ Pending(尚未建立)
        assert_eq!(
            all_search_disposition(true, true, 26, 26),
            AllSearchDisposition::Pending
        );
        // 已建索引但有增量(missing < total)→ 照常搜索 + stale
        assert_eq!(
            all_search_disposition(true, true, 3, 381),
            AllSearchDisposition::Search { stale: true }
        );
        // 完全新鲜 → Search 且不 stale
        assert_eq!(
            all_search_disposition(true, true, 0, 381),
            AllSearchDisposition::Search { stale: false }
        );
        // 没有任何会话的 CLI:不为它提示构建
        assert_eq!(
            all_search_disposition(true, true, 0, 0),
            AllSearchDisposition::Search { stale: false }
        );
    }

    #[test]
    fn test_fork_truncate_lines_claude_uuid() {
        let lines = vec![
            r#"{"uuid":"u1","sessionId":"old-s","text":"hello"}"#.to_string(),
            r#"{"uuid":"u2","sessionId":"old-s","text":"target"}"#.to_string(),
            r#"{"uuid":"u3","sessionId":"old-s","text":"after"}"#.to_string(),
        ];
        let res = super::fork_truncate_lines(lines, "u2", "new-s").unwrap();
        assert_eq!(res.len(), 2);
        assert!(res[0].contains(r#""sessionId":"new-s""#));
        assert!(res[1].contains(r#""sessionId":"new-s""#));
        assert!(res[1].contains(r#""uuid":"u2""#));
    }

    #[test]
    fn test_fork_truncate_lines_workbuddy_id() {
        let lines = vec![
            r#"{"id":"wb-1","sessionId":"old-wb","type":"message","role":"user"}"#.to_string(),
            r#"{"id":"wb-2","sessionId":"old-wb","type":"message","role":"assistant"}"#.to_string(),
            r#"{"id":"wb-3","sessionId":"old-wb","type":"message","role":"user"}"#.to_string(),
        ];
        let res = super::fork_truncate_lines(lines, "wb-2", "new-wb").unwrap();
        assert_eq!(res.len(), 2);
        assert!(res[0].contains(r#""sessionId":"new-wb""#));
        assert!(res[1].contains(r#""sessionId":"new-wb""#));
        assert!(res[1].contains(r#""id":"wb-2""#));
    }

    #[test]
    fn test_sync_workbuddy_forked_session_to_db() {
        use rusqlite::Connection;
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let db_path = std::env::temp_dir().join(format!("sessiondock-wb-fork-{unique}.db"));
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            r#"
            CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                cwd TEXT NOT NULL,
                user_id TEXT NOT NULL,
                title TEXT,
                custom_title TEXT,
                status TEXT NOT NULL DEFAULT 'Pending',
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                last_activity_at INTEGER,
                deleted_at INTEGER,
                is_playground INTEGER NOT NULL DEFAULT 0,
                source_mode TEXT,
                is_background_automation INTEGER,
                mode TEXT,
                model TEXT,
                expert_id TEXT,
                expert_locale TEXT,
                expert_runtime_identity TEXT,
                expert_marketplace TEXT,
                permission_mode TEXT,
                use_sandbox_cli INTEGER,
                project_id TEXT,
                plugin_context_json TEXT,
                last_user_prompt_expert_selection TEXT
            );
            "#,
            [],
        ).unwrap();

        conn.execute(
            r#"
            INSERT INTO sessions (id, cwd, user_id, title, status, created_at, updated_at)
            VALUES ('orig-wb', '/proj', 'user-1', '测试会话', 'completed', 1000, 1000);
            "#,
            [],
        ).unwrap();
        drop(conn);

        super::sync_workbuddy_forked_session_to_db(&db_path, "orig-wb", "forked-wb").unwrap();

        let conn = Connection::open(&db_path).unwrap();
        let mut stmt = conn.prepare("SELECT id, cwd, user_id, title, status FROM sessions WHERE id = 'forked-wb'").unwrap();
        let mut rows = stmt.query([]).unwrap();
        let row = rows.next().unwrap().unwrap();
        let id: String = row.get(0).unwrap();
        let cwd: String = row.get(1).unwrap();
        let user_id: String = row.get(2).unwrap();
        let title: String = row.get(3).unwrap();
        let status: String = row.get(4).unwrap();

        assert_eq!(id, "forked-wb");
        assert_eq!(cwd, "/proj");
        assert_eq!(user_id, "user-1");
        assert_eq!(title, "测试会话 (分支)");
        assert_eq!(status, "completed");

        let _ = std::fs::remove_file(&db_path);
    }
}
