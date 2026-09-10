use crate::app_db;
use crate::cli::{self, CliKind};
use crate::error::{AppError, AppResult};
use crate::history;
use crate::parser;
use crate::session as session_mod;
use crate::session::{PaginatedProjects, ProjectInfo, ProjectSessionChunkItem, SessionInfo};
use crate::streaming;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

const SESSION_LIST_INDEX_UPDATED_EVENT: &str = "session-list-index-updated";
const SEARCH_INDEX_WRITE_BATCH_SIZE: usize = 20;
const SEARCH_INDEX_CONFIRM_SESSION_THRESHOLD: usize = 10;
const SEARCH_INDEX_CONFIRM_BYTES_THRESHOLD: u64 = 50 * 1024 * 1024;

static ONGOING_INDEX_BUILDS: LazyLock<Mutex<HashSet<CliKind>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionListIndexUpdatedPayload {
    cli_id: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchIndexStatus {
    pub(crate) ready: bool,
    requires_confirmation: bool,
    pub(crate) missing_sessions: usize,
    pub(crate) total_sessions: usize,
    missing_bytes: u64,
    total_bytes: u64,
    pub(crate) physical_index_ready: bool,
}

struct RawSession {
    session: SessionInfo,
    encoded_dir: String,
    original_path: String,
}

fn file_modified_ms(metadata: &fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or_default()
}

fn cached_session_list_metadata(
    cached: &HashMap<String, app_db::SessionListIndexRecord>,
    file_path: &str,
    file_size: u64,
    modified_ms: i64,
) -> Option<parser::SessionListMetadata> {
    if parser::is_subagent_session(file_path) {
        return None;
    }
    let record = cached.get(file_path)?;
    if record.file_size != file_size || record.modified_ms != modified_ms {
        return None;
    }
    if record.session_id.trim().is_empty() {
        return None;
    }

    Some(parser::SessionListMetadata {
        session_id: record.session_id.clone(),
        project_path: record.project_path.clone(),
        title: record.title.clone(),
        first_user_message: record.first_user_message.clone(),
        first_timestamp: record.first_timestamp.clone(),
        last_timestamp: record.last_timestamp.clone(),
        git_branch: record.git_branch.clone(),
        file_size: record.file_size,
    })
}

fn build_session_list_index_record(
    file_path: &str,
    modified_ms: i64,
    metadata: &parser::SessionListMetadata,
) -> app_db::SessionListIndexRecord {
    app_db::SessionListIndexRecord {
        session_path: file_path.to_string(),
        session_id: metadata.session_id.clone(),
        project_path: metadata.project_path.clone(),
        title: metadata.title.clone(),
        first_user_message: metadata.first_user_message.clone(),
        first_timestamp: metadata.first_timestamp.clone(),
        last_timestamp: metadata.last_timestamp.clone(),
        git_branch: metadata.git_branch.clone(),
        file_size: metadata.file_size,
        modified_ms,
        has_archive_snapshot: false,
        is_archived: false,
    }
}

fn build_session_search_doc_batch(
    file_path: &str,
    modified_ms: i64,
    docs: &[parser::SearchDocument],
) -> app_db::SessionSearchDocBatch {
    app_db::SessionSearchDocBatch {
        session_path: file_path.to_string(),
        modified_ms,
        docs: docs
            .iter()
            .map(|doc| app_db::SessionSearchDocRecord {
                message_index: doc.message_index,
                search_text: doc.search_text.clone(),
            })
            .collect(),
    }
}

fn scan_session_search_docs_for_index<F>(
    kind: CliKind,
    path: &str,
    record: &app_db::SessionListIndexRecord,
    on_progress: F,
) -> Option<Vec<parser::SearchDocument>>
where
    F: FnMut(parser::SearchScanProgress),
{
    let session_path = Path::new(path);
    if session_path.exists() {
        return parser::scan_session_search_docs_with_progress(session_path, on_progress);
    }

    match app_db::read_archived_session_content(kind.id(), path) {
        Ok(Some(content)) => {
            let docs = parser::scan_session_search_docs_from_bytes(path, &content);
            if docs.is_some() {
                tracing::info!(
                    "搜索索引: 从归档内容构建索引, cli={}, path={}",
                    kind.id(),
                    path
                );
            }
            docs
        }
        Ok(None) => {
            tracing::info!(
                "搜索索引: 会话源文件和归档内容均不存在, cli={}, path={}, archived={}",
                kind.id(),
                path,
                record.is_archived
            );
            None
        }
        Err(err) => {
            tracing::warn!(
                "搜索索引: 读取归档内容失败, cli={}, path={}, err={}",
                kind.id(),
                path,
                err
            );
            None
        }
    }
}

fn emit_search_index_progress(
    app: &AppHandle,
    kind: CliKind,
    phase: &str,
    current: usize,
    total: usize,
    processed_bytes: u64,
    total_bytes: u64,
    current_path: Option<&str>,
    current_file_bytes: u64,
    current_file_size: u64,
) {
    let _ = app.emit(
        "search-index-progress",
        serde_json::json!({
            "cliId": kind.id(),
            "phase": phase,
            "current": current,
            "total": total,
            "processedBytes": processed_bytes,
            "totalBytes": total_bytes,
            "currentPath": current_path,
            "currentFileBytes": current_file_bytes,
            "currentFileSize": current_file_size,
        }),
    );
}

/// 循环级索引进度事件节流门：同一相位 250ms 内最多放行一次，
/// 但相位切换与最后一个会话（终态帧）始终放行，保证前端不丢
/// "committed"、最终计数等关键状态。"written" 归入 "writing" 相位组，
/// 避免每个会话 scanning→writing→written 的微相位循环令节流失效。
struct IndexProgressEmitGate {
    last_emit: Instant,
    last_phase: Option<String>,
}

impl IndexProgressEmitGate {
    fn new() -> Self {
        Self {
            last_emit: Instant::now()
                .checked_sub(Duration::from_millis(250))
                .unwrap_or_else(Instant::now),
            last_phase: None,
        }
    }

    fn gate_phase(phase: &str) -> &str {
        match phase {
            "scanning" | "writing" | "written" => "processing",
            other => other,
        }
    }

    fn should_emit(&mut self, phase: &str, force: bool) -> bool {
        let now = Instant::now();
        let gated = Self::gate_phase(phase);
        let phase_changed = self.last_phase.as_deref() != Some(gated);
        if force
            || phase_changed
            || now.duration_since(self.last_emit) >= Duration::from_millis(250)
        {
            self.last_emit = now;
            self.last_phase = Some(gated.to_string());
            true
        } else {
            false
        }
    }
}

fn load_session_list_index(kind: CliKind) -> HashMap<String, app_db::SessionListIndexRecord> {
    let _ = app_db::restore_missing_archived_index_rows(kind);
    match app_db::read_session_list_index(kind) {
        Ok(index) => index,
        Err(err) => {
            tracing::warn!(
                "读取会话列表索引失败，回退实时扫描: cli={}, err={}",
                kind.id(),
                err
            );
            HashMap::new()
        }
    }
}

fn load_session_list_index_page(
    kind: CliKind,
    page: usize,
    page_size: usize,
) -> AppResult<Option<(Vec<app_db::SessionListIndexRecord>, usize, bool)>> {
    let _ = app_db::restore_missing_archived_index_rows(kind);
    let total_sessions = app_db::count_session_list_index(kind)?;
    if total_sessions == 0 {
        return Ok(None);
    }

    let start = page.saturating_mul(page_size);
    let records = app_db::read_session_list_index_page(kind, start, page_size)?;
    let has_more = start + records.len() < total_sessions;
    Ok(Some((records, total_sessions, has_more)))
}

fn build_paginated_projects_from_sessions(
    selected_sessions: Vec<RawSession>,
    total_sessions: usize,
    has_more: bool,
) -> PaginatedProjects {
    let mut project_map_ordered: Vec<(String, String, Vec<SessionInfo>)> = Vec::new();
    let mut project_index: HashMap<String, usize> = HashMap::new();

    for raw in selected_sessions {
        if let Some(&idx) = project_index.get(&raw.encoded_dir) {
            project_map_ordered[idx].2.push(raw.session);
        } else {
            let idx = project_map_ordered.len();
            project_index.insert(raw.encoded_dir.clone(), idx);
            project_map_ordered.push((raw.encoded_dir, raw.original_path, vec![raw.session]));
        }
    }

    let projects = project_map_ordered
        .into_iter()
        .map(|(encoded_dir, original_path, sessions)| ProjectInfo {
            encoded_dir,
            original_path,
            sessions,
        })
        .collect();

    PaginatedProjects {
        projects,
        has_more,
        total_sessions,
    }
}

fn load_claude_projects_snapshot(
    custom_names: &HashMap<String, String>,
    page: usize,
    page_size: usize,
) -> AppResult<Option<PaginatedProjects>> {
    let Some((records, total_sessions, has_more)) =
        load_session_list_index_page(CliKind::Claude, page, page_size)?
    else {
        return Ok(None);
    };

    let history_path = cli::history_path(CliKind::Claude)?;
    let (session_map, project_map) = history::parse_history(history_path.to_str().unwrap_or(""));
    let mut selected_sessions = Vec::with_capacity(records.len());

    for record in records {
        let file_path = record.session_path;
        if parser::is_subagent_session(&file_path) {
            continue;
        }
        let encoded_dir = Path::new(&file_path)
            .parent()
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("unknown")
            .to_string();
        let original_path = session_mod::resolve_project_path(
            &encoded_dir,
            record.project_path.as_deref(),
            Some(&project_map),
        );
        let session_id = record.session_id;
        let display_name = crate::db::title_resolver::resolve_display_name(
            super::session::custom_session_name(custom_names, CliKind::Claude, &file_path),
            record.title.as_deref(),
            record.first_user_message.as_deref(),
            session_map
                .get(&session_id)
                .map(|h| h.display.as_str())
                .filter(|d| !d.is_empty()),
            &session_id,
        );
        let timestamp = record
            .last_timestamp
            .or(record.first_timestamp)
            .unwrap_or_default();

        selected_sessions.push(RawSession {
            session: SessionInfo {
                session_id,
                file_path,
                display_name,
                timestamp,
                file_size: record.file_size,
                git_branch: record.git_branch,
                has_archive_snapshot: record.has_archive_snapshot,
                is_archived: record.is_archived,
                cli_id: CliKind::Claude.id().to_string(),
            },
            encoded_dir,
            original_path,
        });
    }

    Ok(Some(build_paginated_projects_from_sessions(
        selected_sessions,
        total_sessions,
        has_more,
    )))
}

fn load_codex_projects_snapshot(
    custom_names: &HashMap<String, String>,
    page: usize,
    page_size: usize,
) -> AppResult<Option<PaginatedProjects>> {
    let Some((records, total_sessions, has_more)) =
        load_session_list_index_page(CliKind::Codex, page, page_size)?
    else {
        return Ok(None);
    };

    let history_path = cli::history_path(CliKind::Codex)?;
    let session_map = history::parse_codex_history(history_path.to_str().unwrap_or(""));
    // 单次读取 Codex 索引 thread_name Map（规范 §3.1），整页查询 O(1) 命中
    let codex_titles = parser::load_codex_index_titles();
    let mut selected_sessions = Vec::with_capacity(records.len());

    for record in records {
        let file_path = record.session_path;
        if parser::is_subagent_session(&file_path) {
            continue;
        }
        let session_id = record.session_id;
        let original_path = record
            .project_path
            .filter(|path| !path.trim().is_empty())
            .unwrap_or_else(|| "未知项目".to_string());
        let encoded_dir = original_path.clone();
        let stem = Path::new(&file_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let title = record
            .title
            .as_deref()
            .or_else(|| codex_titles.get(stem).map(String::as_str))
            .or_else(|| codex_titles.get(&session_id).map(String::as_str));
        let display_name = crate::db::title_resolver::resolve_display_name(
            super::session::custom_session_name(custom_names, CliKind::Codex, &file_path),
            title,
            record.first_user_message.as_deref(),
            session_map
                .get(&session_id)
                .map(|h| h.display.as_str())
                .filter(|d| !d.is_empty()),
            &session_id,
        );
        let timestamp = record
            .last_timestamp
            .or(record.first_timestamp)
            .unwrap_or_default();

        selected_sessions.push(RawSession {
            session: SessionInfo {
                session_id,
                file_path,
                display_name,
                timestamp,
                file_size: record.file_size,
                git_branch: record.git_branch,
                has_archive_snapshot: record.has_archive_snapshot,
                is_archived: record.is_archived,
                cli_id: CliKind::Codex.id().to_string(),
            },
            encoded_dir,
            original_path,
        });
    }

    Ok(Some(build_paginated_projects_from_sessions(
        selected_sessions,
        total_sessions,
        has_more,
    )))
}

fn load_gemini_projects_snapshot(
    custom_names: &HashMap<String, String>,
    page: usize,
    page_size: usize,
) -> AppResult<Option<PaginatedProjects>> {
    let Some((records, total_sessions, has_more)) =
        load_session_list_index_page(CliKind::Gemini, page, page_size)?
    else {
        return Ok(None);
    };

    let mut selected_sessions = Vec::with_capacity(records.len());

    for record in records {
        let file_path = record.session_path;
        if parser::is_subagent_session(&file_path) {
            continue;
        }
        let session_id = record.session_id;
        let original_path = record
            .project_path
            .filter(|path| !path.trim().is_empty())
            .unwrap_or_else(|| session_id.clone());
        let encoded_dir = original_path.clone();
        let display_name = crate::db::title_resolver::resolve_display_name(
            super::session::custom_session_name(custom_names, CliKind::Gemini, &file_path),
            record.title.as_deref(),
            record.first_user_message.as_deref(),
            None,
            &session_id,
        );
        let timestamp = record
            .last_timestamp
            .or(record.first_timestamp)
            .unwrap_or_default();

        selected_sessions.push(RawSession {
            session: SessionInfo {
                session_id,
                file_path,
                display_name,
                timestamp,
                file_size: record.file_size,
                git_branch: record.git_branch,
                has_archive_snapshot: record.has_archive_snapshot,
                is_archived: record.is_archived,
                cli_id: CliKind::Gemini.id().to_string(),
            },
            encoded_dir,
            original_path,
        });
    }

    Ok(Some(build_paginated_projects_from_sessions(
        selected_sessions,
        total_sessions,
        has_more,
    )))
}

fn load_workbuddy_projects_snapshot(
    custom_names: &HashMap<String, String>,
    page: usize,
    page_size: usize,
) -> AppResult<Option<PaginatedProjects>> {
    let Some((records, total_sessions, has_more)) =
        load_session_list_index_page(CliKind::WorkBuddy, page, page_size)?
    else {
        return Ok(None);
    };

    // 单次读取 WorkBuddy 用户重命名 Map（workbuddy.db 的 sessions.custom_title），整页 O(1) 命中
    let wb_titles = parser::load_workbuddy_custom_titles();
    let mut selected_sessions = Vec::with_capacity(records.len());

    for record in records {
        let file_path = record.session_path;
        if parser::is_subagent_session(&file_path) {
            continue;
        }
        let session_id = record.session_id;
        let original_path = record
            .project_path
            .filter(|path| !path.trim().is_empty())
            .unwrap_or_else(|| session_id.clone());
        let encoded_dir = original_path.clone();
        let display_name = crate::db::title_resolver::resolve_display_name(
            super::session::custom_session_name(custom_names, CliKind::WorkBuddy, &file_path),
            wb_titles
                .get(&session_id)
                .map(String::as_str)
                .or(record.title.as_deref()),
            record.first_user_message.as_deref(),
            None,
            &session_id,
        );
        let timestamp = record
            .last_timestamp
            .or(record.first_timestamp)
            .unwrap_or_default();

        selected_sessions.push(RawSession {
            session: SessionInfo {
                session_id,
                file_path,
                display_name,
                timestamp,
                file_size: record.file_size,
                git_branch: record.git_branch,
                has_archive_snapshot: record.has_archive_snapshot,
                is_archived: record.is_archived,
                cli_id: CliKind::WorkBuddy.id().to_string(),
            },
            encoded_dir,
            original_path,
        });
    }

    Ok(Some(build_paginated_projects_from_sessions(
        selected_sessions,
        total_sessions,
        has_more,
    )))
}

fn load_dsh_projects_snapshot(
    custom_names: &HashMap<String, String>,
    page: usize,
    page_size: usize,
) -> AppResult<Option<PaginatedProjects>> {
    let Some((records, total_sessions, has_more)) =
        load_session_list_index_page(CliKind::Dsh, page, page_size)?
    else {
        return Ok(None);
    };

    let mut selected_sessions = Vec::with_capacity(records.len());

    for record in records {
        let file_path = record.session_path;
        if parser::is_subagent_session(&file_path) {
            continue;
        }
        let session_id = record.session_id;
        let original_path = record
            .project_path
            .filter(|path| !path.trim().is_empty())
            .unwrap_or_else(|| session_id.clone());
        let encoded_dir = original_path.clone();
        let display_name = crate::db::title_resolver::resolve_display_name(
            super::session::custom_session_name(custom_names, CliKind::Dsh, &file_path),
            record.title.as_deref(),
            record.first_user_message.as_deref(),
            None,
            &session_id,
        );
        let timestamp = record
            .last_timestamp
            .or(record.first_timestamp)
            .unwrap_or_default();

        selected_sessions.push(RawSession {
            session: SessionInfo {
                session_id,
                file_path,
                display_name,
                timestamp,
                file_size: record.file_size,
                git_branch: String::new(),
                has_archive_snapshot: record.has_archive_snapshot,
                is_archived: record.is_archived,
                cli_id: CliKind::Dsh.id().to_string(),
            },
            encoded_dir,
            original_path,
        });
    }

    Ok(Some(build_paginated_projects_from_sessions(
        selected_sessions,
        total_sessions,
        has_more,
    )))
}

fn load_antigravity_projects_snapshot(
    custom_names: &HashMap<String, String>,
    page: usize,
    page_size: usize,
) -> AppResult<Option<PaginatedProjects>> {
    let Some((records, total_sessions, has_more)) =
        load_session_list_index_page(CliKind::Antigravity, page, page_size)?
    else {
        return Ok(None);
    };

    let mut selected_sessions = Vec::with_capacity(records.len());

    for record in records {
        let file_path = record.session_path;
        if parser::is_subagent_session(&file_path) {
            continue;
        }
        let session_id = record.session_id;
        let original_path = record
            .project_path
            .filter(|path| !path.trim().is_empty())
            .unwrap_or_else(|| "未知项目".to_string());
        let encoded_dir = original_path.clone();
        let display_name = crate::db::title_resolver::resolve_display_name(
            super::session::custom_session_name(custom_names, CliKind::Antigravity, &file_path),
            record.title.as_deref(),
            record.first_user_message.as_deref(),
            None,
            &session_id,
        );
        let timestamp = record
            .last_timestamp
            .or(record.first_timestamp)
            .unwrap_or_default();

        selected_sessions.push(RawSession {
            session: SessionInfo {
                session_id,
                file_path,
                display_name,
                timestamp,
                file_size: record.file_size,
                git_branch: String::new(),
                has_archive_snapshot: record.has_archive_snapshot,
                is_archived: record.is_archived,
                cli_id: CliKind::Antigravity.id().to_string(),
            },
            encoded_dir,
            original_path,
        });
    }

    Ok(Some(build_paginated_projects_from_sessions(
        selected_sessions,
        total_sessions,
        has_more,
    )))
}

pub(crate) fn emit_session_list_index_updated(app: &tauri::AppHandle, kind: CliKind) {
    if let Err(err) = app.emit(
        SESSION_LIST_INDEX_UPDATED_EVENT,
        SessionListIndexUpdatedPayload {
            cli_id: kind.id().to_string(),
        },
    ) {
        tracing::warn!("发送会话索引刷新事件失败: cli={}, err={}", kind.id(), err);
    }
}

fn collect_missing_search_paths(
    list_index: &HashMap<String, app_db::SessionListIndexRecord>,
    search_index: &HashMap<String, app_db::SessionSearchIndexRecord>,
    physical_index_ready: bool,
) -> Vec<String> {
    let mut paths: Vec<String> = list_index
        .iter()
        .filter(|(path, record)| {
            if !physical_index_ready {
                return true;
            }
            match search_index.get(*path) {
                // modified_ms 匹配时无论 doc_count 是否为 0 均视为已索引：
                // doc_count == 0 表示该会话无可搜索的文本内容（如纯工具调用会话），
                // 不应重复触发索引构建。
                Some(sr) => sr.modified_ms != record.modified_ms,
                None => true,
            }
        })
        .map(|(path, _)| path.clone())
        .collect();
    paths.sort();
    paths
}

pub(crate) fn resolve_search_index_status(kind: CliKind) -> AppResult<SearchIndexStatus> {
    let list_index = app_db::read_session_list_index(kind)?;
    let total_sessions = list_index.len();
    let total_bytes: u64 = list_index.values().map(|record| record.file_size).sum();
    let physical_index_ready = app_db::tantivy_search::is_physical_index_ready()?;
    let search_index = if physical_index_ready {
        app_db::read_session_search_index(kind)?
    } else {
        HashMap::new()
    };
    let missing_paths = collect_missing_search_paths(
        &list_index,
        &search_index,
        physical_index_ready,
    );
    let missing_bytes: u64 = missing_paths
        .iter()
        .filter_map(|path| list_index.get(path).map(|record| record.file_size))
        .sum();
    let missing_sessions = missing_paths.len();
    let ready = missing_sessions == 0;
    let requires_confirmation = !ready
        && (!physical_index_ready
            || missing_sessions >= SEARCH_INDEX_CONFIRM_SESSION_THRESHOLD
            || missing_bytes >= SEARCH_INDEX_CONFIRM_BYTES_THRESHOLD);

    Ok(SearchIndexStatus {
        ready,
        requires_confirmation,
        missing_sessions,
        total_sessions,
        missing_bytes,
        total_bytes,
        physical_index_ready,
    })
}

fn resolve_session_index_payload(
    session_path: &Path,
    file_path: &str,
    file_metadata: &fs::Metadata,
    cached_list: &HashMap<String, app_db::SessionListIndexRecord>,
    list_updates: &mut Vec<app_db::SessionListIndexRecord>,
    invalid_paths: &mut HashSet<String>,
) -> Option<parser::SessionListMetadata> {
    let file_size = file_metadata.len();
    let modified_ms = file_modified_ms(file_metadata);
    let cached_list_metadata =
        cached_session_list_metadata(cached_list, file_path, file_size, modified_ms);

    if let Some(metadata) = cached_list_metadata {
        return Some(metadata);
    }

    match parser::scan_session_metadata_only(session_path) {
        Some(metadata) => {
            list_updates.push(build_session_list_index_record(
                file_path,
                modified_ms,
                &metadata,
            ));
            Some(metadata)
        }
        None => {
            invalid_paths.insert(file_path.to_string());
            None
        }
    }
}

fn collect_stale_paths(
    cached: &HashMap<String, app_db::SessionListIndexRecord>,
    seen_paths: &HashSet<String>,
    invalid_paths: &HashSet<String>,
) -> Vec<String> {
    // 有归档快照的会话即使源文件删除也保留索引行（与 purge_orphan_session_index 行为一致）
    let mut stale_paths: Vec<String> = cached
        .iter()
        .filter(|(path, record)| !seen_paths.contains(*path) && !record.has_archive_snapshot)
        .map(|(path, _)| path.clone())
        .collect();
    stale_paths.extend(invalid_paths.iter().cloned());
    stale_paths.sort();
    stale_paths.dedup();
    stale_paths
}

fn persist_session_list_index_changes(
    kind: CliKind,
    cached: &HashMap<String, app_db::SessionListIndexRecord>,
    seen_paths: &HashSet<String>,
    updates: Vec<app_db::SessionListIndexRecord>,
    invalid_paths: &HashSet<String>,
) {
    if let Err(err) = app_db::upsert_session_list_index(kind, &updates) {
        tracing::warn!("写入会话列表索引失败: cli={}, err={}", kind.id(), err);
    }

    let stale_paths = collect_stale_paths(cached, seen_paths, invalid_paths);
    if !stale_paths.is_empty() {
        if let Err(err) = app_db::delete_session_list_index_paths(kind, &stale_paths) {
            tracing::warn!("清理失效会话列表索引失败: cli={}, err={}", kind.id(), err);
        }
        if let Err(err) = app_db::delete_session_search_paths(kind, &stale_paths) {
            tracing::warn!("清理失效会话全文索引失败: cli={}, err={}", kind.id(), err);
        }
    }
}

#[tauri::command]
pub async fn ensure_search_index_ready(
    app: tauri::AppHandle,
    cli_id: Option<String>,
) -> AppResult<()> {
    let kind = CliKind::from_id(cli_id.as_deref())?;

    {
        let ongoing = ONGOING_INDEX_BUILDS
            .lock()
            .map_err(|e| AppError::business(e.to_string()))?;
        if ongoing.contains(&kind) {
            return Ok(());
        }
    }

    // 先清理纯搜索孤儿（物理索引里有、源文件与归档都没有的路径），
    // 避免重建后仍残留"可搜不可开"的陈旧命中。
    if let Err(err) = app_db::purge_search_orphans(Some(kind.id())) {
        tracing::warn!("重建索引前清理搜索孤儿失败: err={}", err);
    }

    let list_index = app_db::read_session_list_index(kind)?;
    let physical_index_ready = app_db::tantivy_search::is_physical_index_ready()?;
    if !physical_index_ready {
        tracing::warn!(
            "搜索索引: 物理索引缺失或不可打开，将清空全部状态并重建, cli={}",
            kind.id()
        );
        app_db::clear_all_session_search_index_state()?;
        app_db::tantivy_search::reset_physical_index()?;
    }
    let search_index = if physical_index_ready {
        app_db::read_session_search_index(kind)?
    } else {
        HashMap::new()
    };

    let missing_paths = collect_missing_search_paths(
        &list_index,
        &search_index,
        physical_index_ready,
    );

    if missing_paths.is_empty() {
        return Ok(());
    }

    {
        let mut ongoing = ONGOING_INDEX_BUILDS
            .lock()
            .map_err(|e| AppError::business(e.to_string()))?;
        ongoing.insert(kind);
    }

    let app_clone = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let total = missing_paths.len();
        let total_bytes: u64 = missing_paths
            .iter()
            .filter_map(|path| list_index.get(path).map(|record| record.file_size))
            .sum();
        tracing::info!(
            "搜索索引: 开始构建 {} 个会话的搜索索引, cli={}",
            total,
            kind.id()
        );

        emit_search_index_progress(
            &app_clone,
            kind,
            "scanning",
            0,
            total,
            0,
            total_bytes,
            None,
            0,
            0,
        );

        let single_commit_rebuild = total > SEARCH_INDEX_WRITE_BATCH_SIZE;
        let mut batches = Vec::new();
        let mut state_updates = Vec::new();
        let mut processed_complete_bytes = 0u64;
        let mut loop_emit_gate = IndexProgressEmitGate::new();
        for (i, path) in missing_paths.iter().enumerate() {
            let Some(record) = list_index.get(path) else {
                continue;
            };
            let current_file_size = record.file_size;
            let mut last_emit = Instant::now()
                .checked_sub(Duration::from_millis(250))
                .unwrap_or_else(Instant::now);
            let docs = scan_session_search_docs_for_index(kind, path, record, |progress| {
                let now = Instant::now();
                let is_complete = progress.bytes_read >= progress.file_size;
                if !is_complete && now.duration_since(last_emit) < Duration::from_millis(250) {
                    return;
                }
                last_emit = now;
                emit_search_index_progress(
                    &app_clone,
                    kind,
                    "scanning",
                    i,
                    total,
                    processed_complete_bytes.saturating_add(progress.bytes_read),
                    total_bytes,
                    Some(path),
                    progress.bytes_read,
                    progress.file_size,
                );
            });

            processed_complete_bytes = processed_complete_bytes.saturating_add(current_file_size);

            if let Some(docs) = docs {
                let batch = build_session_search_doc_batch(path, record.modified_ms, &docs);
                if single_commit_rebuild {
                    if loop_emit_gate.should_emit("writing", i == total - 1) {
                        emit_search_index_progress(
                            &app_clone,
                            kind,
                            "writing",
                            i + 1,
                            total,
                            processed_complete_bytes,
                            total_bytes,
                            Some(path),
                            0,
                            0,
                        );
                    }
                    let update = match app_db::write_session_search_docs_uncommitted(kind, &batch) {
                        Ok(update) => update,
                        Err(err) => {
                            // Discard any docs queued by earlier iterations so
                            // they are not flushed by a later unrelated commit.
                            app_db::tantivy_search::rollback_writes();
                            return Err(err);
                        }
                    };
                    state_updates.push(update);
                    if loop_emit_gate.should_emit("written", i == total - 1) {
                        emit_search_index_progress(
                            &app_clone,
                            kind,
                            "written",
                            i + 1,
                            total,
                            processed_complete_bytes,
                            total_bytes,
                            Some(path),
                            0,
                            0,
                        );
                    }
                } else {
                    batches.push(batch);
                }
            } else {
                // 文件存在但无可搜索文本（如纯工具调用会话）：
                // 写入 doc_count=0 的状态记录，标记该路径已处理完毕，
                // 避免 get_search_index_status 因找不到记录而反复计为 missing。
                let empty_update = app_db::SessionSearchIndexStateUpdate {
                    session_path: path.clone(),
                    modified_ms: record.modified_ms,
                    doc_count: 0,
                };
                if single_commit_rebuild {
                    state_updates.push(empty_update);
                } else {
                    // 小批量路径：用空 batch 触发 replace_session_search_docs_with_progress
                    // 实现同样效果 —— 直接将 empty_update 加入 state_updates 统一在结尾写入
                    state_updates.push(empty_update);
                }
            }

            if !single_commit_rebuild && batches.len() >= SEARCH_INDEX_WRITE_BATCH_SIZE {
                if loop_emit_gate.should_emit("writing", i == total - 1) {
                    emit_search_index_progress(
                        &app_clone,
                        kind,
                        "writing",
                        i + 1,
                        total,
                        processed_complete_bytes,
                        total_bytes,
                        Some(path),
                        current_file_size,
                        current_file_size,
                    );
                }
                let batch_base = i + 1 - batches.len();
                app_db::replace_session_search_docs_with_progress(kind, &batches, |progress| {
                    let current = batch_base + progress.written_sessions;
                    if loop_emit_gate.should_emit(progress.phase, current + 1 >= total) {
                        emit_search_index_progress(
                            &app_clone,
                            kind,
                            progress.phase,
                            current,
                            total,
                            processed_complete_bytes,
                            total_bytes,
                            progress.current_path.as_deref(),
                            0,
                            0,
                        );
                    }
                })?;
                batches.clear();
            }

            if loop_emit_gate.should_emit("scanning", i == total - 1) {
                emit_search_index_progress(
                    &app_clone,
                    kind,
                    "scanning",
                    i + 1,
                    total,
                    processed_complete_bytes,
                    total_bytes,
                    Some(path),
                    current_file_size,
                    current_file_size,
                );
            }
        }

        if single_commit_rebuild {
            if !state_updates.is_empty() {
                emit_search_index_progress(
                    &app_clone,
                    kind,
                    "committing",
                    total,
                    total,
                    processed_complete_bytes,
                    total_bytes,
                    None,
                    0,
                    0,
                );
                if let Err(err) = app_db::commit_session_search_writes() {
                    app_db::tantivy_search::rollback_writes();
                    return Err(err);
                }
                emit_search_index_progress(
                    &app_clone,
                    kind,
                    "committed",
                    total,
                    total,
                    processed_complete_bytes,
                    total_bytes,
                    None,
                    0,
                    0,
                );
                app_db::replace_session_search_index_state(kind, &state_updates)?;
            }
        } else {
            if !batches.is_empty() {
                emit_search_index_progress(
                    &app_clone,
                    kind,
                    "writing",
                    total,
                    total,
                    processed_complete_bytes,
                    total_bytes,
                    None,
                    0,
                    0,
                );
                let batch_base = total - batches.len();
                app_db::replace_session_search_docs_with_progress(kind, &batches, |progress| {
                    let current = batch_base + progress.written_sessions;
                    if loop_emit_gate.should_emit(progress.phase, current + 1 >= total) {
                        emit_search_index_progress(
                            &app_clone,
                            kind,
                            progress.phase,
                            current,
                            total,
                            processed_complete_bytes,
                            total_bytes,
                            progress.current_path.as_deref(),
                            0,
                            0,
                        );
                    }
                })?;
            }
            // 小批量路径：为无可搜索文本的会话单独写入 doc_count=0 的状态记录，
            // 防止 get_search_index_status 因找不到该路径记录而反复报告 missing。
            if !state_updates.is_empty() {
                app_db::replace_session_search_index_state(kind, &state_updates)?;
            }
        }

        tracing::info!("搜索索引: 构建完成, cli={}", kind.id());
        emit_search_index_progress(
            &app_clone,
            kind,
            "done",
            total,
            total,
            total_bytes,
            total_bytes,
            None,
            0,
            0,
        );
        Ok::<(), AppError>(())
    })
    .await;

    {
        let mut ongoing = ONGOING_INDEX_BUILDS
            .lock()
            .map_err(|e| AppError::business(e.to_string()))?;
        ongoing.remove(&kind);
    }

    // On ANY build failure (including a panicked blocking task), emit a
    // terminal "error" frame before propagating so the frontend clears its
    // progress indicator instead of being stuck (progress stays non-null and
    // blocks the search debounce). Single emit point: all error returns from
    // the build closure funnel through here.
    let build_result = match result {
        Ok(inner) => inner,
        Err(join_err) => Err(AppError::business(join_err.to_string())),
    };
    if let Err(ref err) = build_result {
        tracing::warn!("搜索索引: 构建失败, cli={}, err={}", kind.id(), err);
        emit_search_index_progress(&app, kind, "error", 0, 0, 0, 0, None, 0, 0);
    }
    build_result
}

#[tauri::command]
pub fn get_search_index_status(cli_id: Option<String>) -> AppResult<SearchIndexStatus> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    resolve_search_index_status(kind)
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexStats {
    pub session_count: usize,
    pub db_size_bytes: u64,
    pub search_doc_count: usize,
    pub search_index_bytes: u64,
    pub last_updated_ms: Option<i64>,
}

#[tauri::command]
pub fn get_index_stats(cli_id: Option<String>) -> AppResult<IndexStats> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    let session_count = app_db::count_session_list_index(kind)?;

    // DB 大小 = 主库 + WAL + SHM（WAL 模式下数据可能还在 wal 文件里）
    let mut db_size_bytes = 0u64;
    if let Ok(db_path) = app_db::app_db_path() {
        for suffix in ["", "-wal", "-shm"] {
            let path = if suffix.is_empty() {
                db_path.clone()
            } else {
                std::path::PathBuf::from(format!("{}{}", db_path.display(), suffix))
            };
            if let Ok(meta) = std::fs::metadata(&path) {
                db_size_bytes += meta.len();
            }
        }
    }

    let search_doc_count = app_db::count_session_search_indexed_docs(kind).unwrap_or(0);

    let search_index_bytes = {
        let mut total = 0u64;
        if let Ok(app_data_dir) = app_db::data_dir() {
            let search_dir = app_data_dir.join("search_index");
            if let Ok(entries) = std::fs::read_dir(&search_dir) {
                for entry in entries.flatten() {
                    if let Ok(meta) = entry.metadata() {
                        if meta.is_file() {
                            total += meta.len();
                        }
                    }
                }
            }
        }
        total
    };

    let last_updated_ms = app_db::read_index_last_updated(kind.id())?;

    Ok(IndexStats {
        session_count,
        db_size_bytes,
        search_doc_count,
        search_index_bytes,
        last_updated_ms,
    })
}

#[tauri::command]
pub async fn clear_session_index(
    app: tauri::AppHandle,
    cli_id: Option<String>,
) -> AppResult<bool> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    let app_clone = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> AppResult<()> {
        let removed_paths = {
            let conn = app_db::conn()?;
            app_db::clear_session_index_inner(&conn, kind.id())?
        };
        // tantivy 物理索引同步删除（失败不阻塞）
        if !removed_paths.is_empty() {
            let _ = app_db::tantivy_search::delete_session_docs(&removed_paths);
        }
        let now_ms = chrono::Utc::now().timestamp_millis();
        if let Ok(conn) = app_db::conn() {
            let _ = app_db::write_index_last_updated(&conn, kind.id(), now_ms);
        }
        Ok(())
    })
    .await
    .map_err(|e| AppError::business(e.to_string()))??;

    emit_session_list_index_updated(&app_clone, kind);
    Ok(true)
}

fn requested_kinds_with_data_dir<F>(
    cli_ids: Option<&[String]>,
    front_cli_id: Option<&str>,
    mut data_dir_exists: F,
) -> AppResult<Vec<CliKind>>
where
    F: FnMut(CliKind) -> bool,
{
    let mut seen = HashSet::new();
    let mut kinds = match cli_ids {
        Some(ids) => {
            let mut parsed = Vec::with_capacity(ids.len());
            for id in ids {
                let kind = CliKind::from_id(Some(id.as_str()))?;
                if seen.insert(kind) {
                    parsed.push(kind);
                }
            }
            parsed
        }
        None => CliKind::all().to_vec(),
    };

    let front = front_cli_id
        .map(|id| CliKind::from_id(Some(id)))
        .transpose()?;
    kinds.retain(|kind| data_dir_exists(*kind));

    if let Some(front) = front {
        if let Some(index) = kinds.iter().position(|kind| *kind == front) {
            kinds.remove(index);
            kinds.insert(0, front);
        }
    }

    Ok(kinds)
}

pub(crate) fn requested_kinds(
    cli_ids: Option<&[String]>,
    front_cli_id: Option<&str>,
) -> AppResult<Vec<CliKind>> {
    requested_kinds_with_data_dir(cli_ids, front_cli_id, |kind| {
        cli::data_dir(kind).map(|dir| dir.exists()).unwrap_or(false)
    })
}

pub(crate) fn effective_cli_ids(
    cli_ids: Option<Vec<String>>,
    legacy_cli_id: Option<String>,
) -> Option<Vec<String>> {
    cli_ids.or_else(|| match legacy_cli_id {
        Some(id) if id.trim() == "all" => None,
        Some(id) => Some(vec![id]),
        None => None,
    })
}

pub(crate) struct CliFanoutOutcome<T> {
    pub(crate) kind: CliKind,
    pub(crate) result: Result<T, String>,
}

pub(crate) fn run_cli_fanout<T, F, H>(
    kinds: &[CliKind],
    mut operation: F,
    mut handle_outcome: H,
)
where
    F: FnMut(CliKind) -> AppResult<T>,
    H: FnMut(CliFanoutOutcome<T>),
{
    for kind in kinds.iter().copied() {
        handle_outcome(CliFanoutOutcome {
            kind,
            result: operation(kind).map_err(|error| error.to_string()),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requested_kinds_resolves_supplied_subset_and_deduplicates_ids() {
        let kinds = requested_kinds_with_data_dir(
            Some(&[
                "codex".to_string(),
                "claude".to_string(),
                "codex".to_string(),
            ]),
            None,
            |_| true,
        )
        .unwrap();

        assert_eq!(kinds, vec![CliKind::Codex, CliKind::Claude]);
    }

    #[test]
    fn requested_kinds_rejects_invalid_ids_before_filtering_absent_directories() {
        let result = requested_kinds_with_data_dir(
            Some(&["not-a-cli".to_string()]),
            None,
            |_| false,
        );

        assert!(result.is_err());
    }

    #[test]
    fn requested_kinds_moves_included_front_cli_first() {
        let kinds = requested_kinds_with_data_dir(
            Some(&[
                "claude".to_string(),
                "codex".to_string(),
                "gemini".to_string(),
            ]),
            Some("gemini"),
            |_| true,
        )
        .unwrap();

        assert_eq!(
            kinds,
            vec![CliKind::Gemini, CliKind::Claude, CliKind::Codex]
        );
    }

    #[test]
    fn requested_kinds_skips_absent_data_directories_and_allows_empty_subset() {
        let kinds = requested_kinds_with_data_dir(
            Some(&["claude".to_string(), "codex".to_string()]),
            Some("claude"),
            |kind| kind == CliKind::Codex,
        )
        .unwrap();
        let empty = requested_kinds_with_data_dir(Some(&[]), None, |_| true).unwrap();

        assert_eq!(kinds, vec![CliKind::Codex]);
        assert!(empty.is_empty());
    }

    #[test]
    fn explicit_cli_ids_take_precedence_over_legacy_cli_id() {
        let ids = effective_cli_ids(
            Some(vec!["codex".to_string(), "gemini".to_string()]),
            Some("claude".to_string()),
        );

        assert_eq!(ids, Some(vec!["codex".to_string(), "gemini".to_string()]));
    }

    #[test]
    fn legacy_cli_id_preserves_single_source_and_all_semantics() {
        assert_eq!(
            effective_cli_ids(None, Some("claude".to_string())),
            Some(vec!["claude".to_string()])
        );
        assert_eq!(effective_cli_ids(None, Some("all".to_string())), None);
        assert_eq!(effective_cli_ids(None, None), None);
    }

    #[test]
    fn cli_fanout_continues_after_one_cli_fails() {
        let mut outcomes = Vec::new();
        run_cli_fanout(
            &[CliKind::Claude, CliKind::Codex],
            |kind| {
                if kind == CliKind::Claude {
                    Err(AppError::business("broken source"))
                } else {
                    Ok(kind.id().to_string())
                }
            },
            |outcome| outcomes.push(outcome),
        );

        assert_eq!(outcomes.len(), 2);
        assert!(outcomes[0].result.is_err());
        assert_eq!(outcomes[1].result.as_deref(), Ok("codex"));
    }

    fn list_record(path: &str, modified_ms: i64) -> app_db::SessionListIndexRecord {
        app_db::SessionListIndexRecord {
            session_path: path.to_string(),
            session_id: path.to_string(),
            project_path: None,
            title: None,
            first_user_message: None,
            first_timestamp: None,
            last_timestamp: None,
            git_branch: String::new(),
            file_size: 1,
            modified_ms,
            has_archive_snapshot: false,
            is_archived: false,
        }
    }

    fn search_record(
        path: &str,
        modified_ms: i64,
        doc_count: usize,
    ) -> app_db::SessionSearchIndexRecord {
        app_db::SessionSearchIndexRecord {
            session_path: path.to_string(),
            modified_ms,
            doc_count,
        }
    }

    /// 两个 DSH 扫描测试共享真实 app DB（Dsh 路径覆盖 + Dsh 索引），并行会互相覆盖，
    /// 串行化避免 flaky。
    static DSH_SCAN_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    /// 防御性清理：真实 app DB 跨进程共享，先前中断/并发运行的冒烟可能残留 DSH 索引行
    /// （曾致快照断言看到 4 个项目而非 1 个）。每个 DSH 测试开始前清一次，
    /// 保证起始状态干净（结束后再清一次恢复原状）。
    fn clear_dsh_index_rows() {
        if let Ok(conn) = app_db::conn() {
            let _ = app_db::clear_session_index_inner(&conn, "dsh");
        }
    }

    #[test]
    fn collect_missing_search_paths_rebuilds_all_when_physical_index_is_not_ready() {
        let list_index = HashMap::from([
            ("a.jsonl".to_string(), list_record("a.jsonl", 10)),
            ("b.jsonl".to_string(), list_record("b.jsonl", 20)),
        ]);
        let search_index = HashMap::from([
            ("a.jsonl".to_string(), search_record("a.jsonl", 10, 3)),
            ("b.jsonl".to_string(), search_record("b.jsonl", 20, 2)),
        ]);

        let missing = collect_missing_search_paths(&list_index, &search_index, false);

        assert_eq!(missing, vec!["a.jsonl".to_string(), "b.jsonl".to_string()]);
    }

    #[test]
    fn collect_missing_search_paths_ignores_txt_cache_gap() {
        let list_index = HashMap::from([
            ("a.jsonl".to_string(), list_record("a.jsonl", 10)),
            ("b.jsonl".to_string(), list_record("b.jsonl", 20)),
        ]);
        let search_index = HashMap::from([
            ("a.jsonl".to_string(), search_record("a.jsonl", 10, 3)),
            ("b.jsonl".to_string(), search_record("b.jsonl", 20, 2)),
        ]);

        // txt 缺失不再计入 missing：record 与 search_index 一致时为空
        let missing = collect_missing_search_paths(&list_index, &search_index, true);

        assert!(missing.is_empty(), "txt 缺口不应再触发重扫");
    }

    #[test]
    fn collect_missing_search_paths_treats_zero_doc_count_with_matching_ts_as_indexed() {
        // 回归：tool_use/tool_result 内容从索引剔除后，纯工具调用会话的 doc_count == 0，
        // 但 modified_ms 匹配说明已经尝试过索引、只是无可搜索文本，不应再次触发索引构建。
        let list_index = HashMap::from([
            ("tool_only.jsonl".to_string(), list_record("tool_only.jsonl", 100)),
            ("has_text.jsonl".to_string(), list_record("has_text.jsonl", 200)),
            ("stale.jsonl".to_string(), list_record("stale.jsonl", 300)),
        ]);
        let search_index = HashMap::from([
            // doc_count == 0 但 modified_ms 匹配 → 已索引（无文本内容），不算 missing
            ("tool_only.jsonl".to_string(), search_record("tool_only.jsonl", 100, 0)),
            // doc_count > 0，modified_ms 匹配 → 正常已索引
            ("has_text.jsonl".to_string(), search_record("has_text.jsonl", 200, 5)),
            // modified_ms 不匹配（文件有更新）→ 需要重新索引
            ("stale.jsonl".to_string(), search_record("stale.jsonl", 999, 3)),
        ]);

        let missing = collect_missing_search_paths(&list_index, &search_index, true);

        // 只有 stale.jsonl（ts 不匹配）是 missing，tool_only 和 has_text 均已索引
        assert_eq!(missing, vec!["stale.jsonl".to_string()]);
    }

    #[test]
    fn collect_stale_paths_keeps_sessions_with_archive_snapshot() {
        let mut archived_record = list_record("/archived.jsonl", 10);
        archived_record.has_archive_snapshot = true;
        let cached = HashMap::from([
            ("/archived.jsonl".to_string(), archived_record),
            ("/gone.jsonl".to_string(), list_record("/gone.jsonl", 20)),
        ]);
        let seen_paths = HashSet::new();
        let invalid_paths = HashSet::new();

        let stale = collect_stale_paths(&cached, &seen_paths, &invalid_paths);

        assert_eq!(stale, vec!["/gone.jsonl".to_string()]);
    }

    #[test]
    fn collect_stale_paths_keeps_invalid_paths_regardless_of_archive() {
        let mut archived_record = list_record("/archived.jsonl", 10);
        archived_record.has_archive_snapshot = true;
        let cached = HashMap::from([("/archived.jsonl".to_string(), archived_record)]);
        let seen_paths = HashSet::from(["/archived.jsonl".to_string()]);
        let invalid_paths = HashSet::from(["/broken.jsonl".to_string()]);

        let stale = collect_stale_paths(&cached, &seen_paths, &invalid_paths);

        assert_eq!(stale, vec!["/broken.jsonl".to_string()]);
    }

    /// 临时目录构造 zstd 会话夹具 → `scan_projects_inner_for_cli(Dsh,..)` 返回 1 项目/1 会话，
    /// `file_path` 指向 zstd、`original_path` 取 header cwd。
    #[test]
    fn dsh_scan_indexes_zstd_session_fixture() {
        let _guard = DSH_SCAN_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_dsh_index_rows();
        let root = tempfile::tempdir().unwrap();
        let session_file = root
            .path()
            .join("sessions")
            .join("--Users-x-proj--")
            .join("s1")
            .join("session.jsonl.zstd");
        fs::create_dir_all(session_file.parent().unwrap()).unwrap();
        let content = concat!(
            "{\"type\":\"session\",\"version\":0,\"id\":\"s1\",\"createdAt\":1700000000000,\"cwd\":\"/Users/x/proj\"}\n",
            "{\"type\":\"user/message\",\"seq\":1,\"time\":1700000000000,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n",
        );
        let compressed = zstd::encode_all(std::io::Cursor::new(content.as_bytes()), 3).unwrap();
        fs::write(&session_file, compressed).unwrap();

        // 快照前置的 Dsh 路径覆盖状态，测后恢复；并清理测试写入的 Dsh 索引行
        let prev_override = app_db::read_cli_path_overrides()
            .ok()
            .and_then(|overrides| overrides.get("dsh").cloned());
        cli::set_cli_data_dir_override(
            CliKind::Dsh,
            Some(root.path().to_str().unwrap().to_string()),
        )
        .unwrap();

        let result = scan_projects_inner_for_cli(CliKind::Dsh, None, None, true).unwrap();

        match prev_override {
            Some(path) => cli::set_cli_data_dir_override(CliKind::Dsh, Some(path)).unwrap(),
            None => cli::set_cli_data_dir_override(CliKind::Dsh, None).unwrap(),
        }
        if let Ok(conn) = app_db::conn() {
            let _ = app_db::clear_session_index_inner(&conn, "dsh");
        }

        assert_eq!(result.total_sessions, 1);
        assert_eq!(result.projects.len(), 1);
        let project = &result.projects[0];
        assert_eq!(project.original_path, "/Users/x/proj");
        assert_eq!(project.encoded_dir, "/Users/x/proj");
        assert_eq!(project.sessions.len(), 1);
        let session = &project.sessions[0];
        assert!(session.file_path.ends_with(".zstd"));
        assert_eq!(session.session_id, "s1");
        assert!(!session.timestamp.is_empty());
        // 标题对齐：title 恒 None，展示名回退到清洗后的首条用户消息（不再是目录名）
        assert_eq!(session.display_name, "hi");
    }

    /// 空会话（只有 header + 策略事件、无 surface 消息）不进列表，
    /// 对齐 sessionview 的"无 surfaced 消息即跳过"。
    #[test]
    fn dsh_scan_skips_session_without_surface_messages() {
        let _guard = DSH_SCAN_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_dsh_index_rows();
        let root = tempfile::tempdir().unwrap();
        let session_file = root
            .path()
            .join("sessions")
            .join("--Users-x-proj--")
            .join("s-empty")
            .join("session.jsonl.zstd");
        fs::create_dir_all(session_file.parent().unwrap()).unwrap();
        // 真实空会话形态（323B 案例）：header + permission/sandbox/approval，无聊天消息
        let content = concat!(
            "{\"type\":\"session\",\"version\":0,\"id\":\"s-empty\",\"createdAt\":1700000000000,\"cwd\":\"/Users/x/proj\"}\n",
            "{\"type\":\"permission/preset\",\"seq\":0,\"time\":1700000000001,\"data\":{\"preset\":\"danger-full-access\"}}\n",
            "{\"type\":\"sandbox/mode\",\"seq\":1,\"time\":1700000000002,\"data\":{\"mode\":\"danger-full-access\"}}\n",
            "{\"type\":\"approval/policy\",\"seq\":2,\"time\":1700000000003,\"data\":{\"policy\":\"never\"}}\n",
        );
        let compressed = zstd::encode_all(std::io::Cursor::new(content.as_bytes()), 3).unwrap();
        fs::write(&session_file, compressed).unwrap();

        let prev_override = app_db::read_cli_path_overrides()
            .ok()
            .and_then(|overrides| overrides.get("dsh").cloned());
        cli::set_cli_data_dir_override(
            CliKind::Dsh,
            Some(root.path().to_str().unwrap().to_string()),
        )
        .unwrap();

        let result = scan_projects_inner_for_cli(CliKind::Dsh, None, None, true).unwrap();

        match prev_override {
            Some(path) => cli::set_cli_data_dir_override(CliKind::Dsh, Some(path)).unwrap(),
            None => cli::set_cli_data_dir_override(CliKind::Dsh, None).unwrap(),
        }
        if let Ok(conn) = app_db::conn() {
            let _ = app_db::clear_session_index_inner(&conn, "dsh");
        }

        assert_eq!(
            result.total_sessions, 0,
            "空会话（无 surface 消息）不应进列表"
        );
    }

    /// cwd-less header 回归：header 缺 `cwd` 时扫描按目录名解码兜底项目路径，
    /// 并回写持久化索引，快照加载（读索引）必须与实时扫描返回同一项目 key。
    #[test]
    fn dsh_scan_and_snapshot_agree_on_cwdless_header() {
        let _guard = DSH_SCAN_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_dsh_index_rows();
        let root = tempfile::tempdir().unwrap();
        let session_file = root
            .path()
            .join("sessions")
            .join("--Users-x-proj--")
            .join("s1")
            .join("session.jsonl.zstd");
        fs::create_dir_all(session_file.parent().unwrap()).unwrap();
        // 头部无 cwd：权威项目路径缺失，只能按目录名解码兜底
        let content = concat!(
            "{\"type\":\"session\",\"version\":0,\"id\":\"s1\",\"createdAt\":1700000000000}\n",
            "{\"type\":\"user/message\",\"seq\":1,\"time\":1700000000000,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n",
        );
        let compressed = zstd::encode_all(std::io::Cursor::new(content.as_bytes()), 3).unwrap();
        fs::write(&session_file, compressed).unwrap();

        let prev_override = app_db::read_cli_path_overrides()
            .ok()
            .and_then(|overrides| overrides.get("dsh").cloned());
        cli::set_cli_data_dir_override(
            CliKind::Dsh,
            Some(root.path().to_str().unwrap().to_string()),
        )
        .unwrap();

        let scan_result = scan_projects_inner_for_cli(CliKind::Dsh, None, None, true).unwrap();
        let snapshot_result =
            load_dsh_projects_snapshot(&HashMap::new(), 0, 100).unwrap().expect("索引应有该行");

        match prev_override {
            Some(path) => cli::set_cli_data_dir_override(CliKind::Dsh, Some(path)).unwrap(),
            None => cli::set_cli_data_dir_override(CliKind::Dsh, None).unwrap(),
        }
        if let Ok(conn) = app_db::conn() {
            let _ = app_db::clear_session_index_inner(&conn, "dsh");
        }

        // 目录名 --Users-x-proj-- 解码兜底（decode 保留 -- 产生的双斜杠）
        let expected_key = session_mod::decode_project_dir("--Users-x-proj--");
        assert_eq!(scan_result.projects.len(), 1);
        assert_eq!(scan_result.projects[0].encoded_dir, expected_key);
        assert_eq!(scan_result.projects[0].original_path, expected_key);

        assert_eq!(snapshot_result.projects.len(), 1);
        assert_eq!(
            snapshot_result.projects[0].encoded_dir, expected_key,
            "快照（读索引）必须回读到扫描兜底的项目 key"
        );
        assert_eq!(
            snapshot_result.projects[0].original_path, expected_key,
            "快照项目路径必须与扫描一致"
        );
    }

    /// 真实 `~/.dsh/sessions` 冒烟：扫描不 panic、项目路径正确、可见会话数与磁盘一致。
    /// `--Users-...--` key 目录解码（去前后缀双斜杠）后的路径应能在结果 original_path 命中。
    /// 计数口径（与 scan_dsh_projects 同步）：header 可解析 && origin != "subagent"（子会话
    /// 不进侧边栏）&& 含 surface 消息（空会话过滤）。计数断言防止"静默丢会话测试全绿"。
    #[test]
    fn dsh_scan_smokes_against_real_sessions_dir() {
        let _guard = DSH_SCAN_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_dsh_index_rows();
        let sessions_dir = match cli::sessions_dir(CliKind::Dsh) {
            Ok(dir) if dir.exists() => dir,
            _ => return, // 机器无 ~/.dsh/sessions 时跳过
        };

        let result = scan_projects_inner_for_cli(CliKind::Dsh, None, None, true).unwrap();
        let session_count: usize = result
            .projects
            .iter()
            .map(|project| project.sessions.len())
            .sum();
        assert_eq!(result.total_sessions, session_count);

        for project in &result.projects {
            assert!(
                Path::new(&project.original_path).is_absolute(),
                "original_path 应为绝对路径: {}",
                project.original_path
            );
        }

        // 遍历磁盘会话文件，按扫描同口径算出期望可见集合（排除 subagent 与空会话）
        let mut expected_visible: HashMap<String, Vec<String>> = HashMap::new();
        let mut skipped_subagent = 0usize;
        let mut skipped_empty = 0usize;
        let mut skipped_bad_header = 0usize;
        for project_entry in fs::read_dir(&sessions_dir)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
        {
            let project_dir = project_entry.path();
            if !project_dir.is_dir() {
                continue;
            }
            let key = project_entry
                .file_name()
                .to_str()
                .unwrap_or("")
                .to_string();
            for session_entry in fs::read_dir(&project_dir)
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
            {
                let session_dir = session_entry.path();
                if !session_dir.is_dir() {
                    continue;
                }
                let session_file = if session_dir.join("session.jsonl.zstd").is_file() {
                    session_dir.join("session.jsonl.zstd")
                } else {
                    session_dir.join("session.jsonl")
                };
                if !session_file.is_file() {
                    continue;
                }
                let Some(header) = crate::parser::dsh::read_header_line(&session_file) else {
                    skipped_bad_header += 1;
                    continue;
                };
                if header.get("origin").and_then(Value::as_str) == Some("subagent") {
                    skipped_subagent += 1;
                    continue;
                }
                if !crate::parser::dsh::has_surface_messages(&session_file) {
                    skipped_empty += 1;
                    continue;
                }
                expected_visible
                    .entry(key.clone())
                    .or_default()
                    .push(session_file.to_string_lossy().to_string());
            }
        }
        let expected_total: usize = expected_visible.values().map(Vec::len).sum();
        eprintln!(
            "[dsh smoke] visible={session_count} expected={expected_total} \
             skipped_subagent={skipped_subagent} skipped_empty={skipped_empty} \
             skipped_bad_header={skipped_bad_header}"
        );
        for project in &result.projects {
            eprintln!(
                "[dsh smoke]   {} -> {} 个会话",
                project.original_path,
                project.sessions.len()
            );
        }
        assert_eq!(
            session_count, expected_total,
            "可见会话数必须等于磁盘上非 subagent 且含 surface 消息的会话数"
        );

        // 对照磁盘 key：含期望可见会话的 key 解码后应命中结果 original_path；
        // 只有 subagent/空会话的 key 可以不在结果中
        let found_paths: Vec<String> = result
            .projects
            .iter()
            .map(|project| project.original_path.trim_matches('/').to_string())
            .collect();
        for (key, files) in &expected_visible {
            if files.is_empty() {
                continue;
            }
            let normalized = session_mod::decode_project_dir(key)
                .trim_matches('/')
                .to_string();
            assert!(
                found_paths.contains(&normalized),
                "含可见会话的 key {key} 应解码命中 original_path {normalized}"
            );
        }

        // 清理冒烟扫描写入的 Dsh 索引行（Dsh 索引在接入前应为空）
        if let Ok(conn) = app_db::conn() {
            let _ = app_db::clear_session_index_inner(&conn, "dsh");
        }
    }
}

const SCAN_PROJECTS_TOPIC: &str = "scan_projects";
const SCAN_PROJECTS_BATCH_SIZE: usize = 50;
const SCAN_PROJECTS_SNAPSHOT_LIMIT: usize = 100_000;

#[derive(Debug, Clone, serde::Serialize)]
struct ScanProjectsStreamDone {
    total_sessions: usize,
    cli_results: Vec<ScanProjectsCliResult>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct ScanProjectsCliResult {
    cli_id: String,
    total_sessions: usize,
    error: Option<String>,
}

fn load_projects_stream_items(
    kind: CliKind,
    custom_names: &HashMap<String, String>,
) -> AppResult<(Vec<ProjectSessionChunkItem>, usize)> {
    let _ = crate::db::purge_subagents_from_db();
    let snapshot = match kind {
        CliKind::Claude => {
            load_claude_projects_snapshot(custom_names, 0, SCAN_PROJECTS_SNAPSHOT_LIMIT)
        }
        CliKind::Codex => {
            load_codex_projects_snapshot(custom_names, 0, SCAN_PROJECTS_SNAPSHOT_LIMIT)
        }
        CliKind::Gemini => {
            load_gemini_projects_snapshot(custom_names, 0, SCAN_PROJECTS_SNAPSHOT_LIMIT)
        }
        CliKind::WorkBuddy => {
            load_workbuddy_projects_snapshot(custom_names, 0, SCAN_PROJECTS_SNAPSHOT_LIMIT)
        }
        CliKind::Dsh => {
            load_dsh_projects_snapshot(custom_names, 0, SCAN_PROJECTS_SNAPSHOT_LIMIT)
        }
        CliKind::Antigravity => {
            load_antigravity_projects_snapshot(custom_names, 0, SCAN_PROJECTS_SNAPSHOT_LIMIT)
        }
    };
    if let Ok(Some(paginated)) = snapshot {
        let mut items = Vec::new();
        for project in paginated.projects {
            for session in project.sessions {
                items.push(ProjectSessionChunkItem {
                    session,
                    encoded_dir: project.encoded_dir.clone(),
                    original_path: project.original_path.clone(),
                });
            }
        }
        return Ok((items, paginated.total_sessions));
    }

    let mut all_sessions = match kind {
        CliKind::Claude => scan_claude_projects(custom_names, false)?,
        CliKind::Codex => scan_codex_projects(custom_names, false)?,
        CliKind::Gemini => scan_gemini_projects(custom_names, false)?,
        CliKind::WorkBuddy => scan_workbuddy_projects(custom_names, false)?,
        CliKind::Dsh => scan_dsh_projects(custom_names, false)?,
        CliKind::Antigravity => scan_antigravity_projects(custom_names, false)?,
    };
    all_sessions.sort_by(|a, b| b.session.timestamp.cmp(&a.session.timestamp));
    let total = all_sessions.len();
    let items = all_sessions
        .into_iter()
        .map(|raw| ProjectSessionChunkItem {
            session: raw.session,
            encoded_dir: raw.encoded_dir,
            original_path: raw.original_path,
        })
        .collect();
    Ok((items, total))
}

#[tauri::command]
pub async fn scan_projects_stream(
    app: AppHandle,
    request_id: String,
    cli_ids: Option<Vec<String>>,
    cli_id: Option<String>,
) -> AppResult<()> {
    let effective_ids = effective_cli_ids(cli_ids, cli_id);
    let kinds = requested_kinds(effective_ids.as_deref(), None)?;

    streaming::spawn_streaming_task(
        app,
        SCAN_PROJECTS_TOPIC,
        request_id,
        move |app, topic, request_id| {
            let custom_names = super::session::load_session_names().unwrap_or_default();
            let mut total_sessions = 0usize;
            let mut cli_results = Vec::with_capacity(kinds.len());
            run_cli_fanout(
                &kinds,
                |kind| load_projects_stream_items(kind, &custom_names),
                |outcome| match outcome.result {
                    Ok((items, total)) => {
                        total_sessions += total;
                        for batch in items.chunks(SCAN_PROJECTS_BATCH_SIZE) {
                            streaming::emit_chunk(app, topic, request_id, &batch.to_vec());
                        }
                        cli_results.push(ScanProjectsCliResult {
                            cli_id: outcome.kind.id().to_string(),
                            total_sessions: total,
                            error: None,
                        });
                    }
                    Err(error) => cli_results.push(ScanProjectsCliResult {
                        cli_id: outcome.kind.id().to_string(),
                        total_sessions: 0,
                        error: Some(error),
                    }),
                },
            );

            streaming::emit_done(
                app,
                topic,
                request_id,
                &ScanProjectsStreamDone {
                    total_sessions,
                    cli_results,
                },
            );
        },
    );

    Ok(())
}

#[tauri::command]
pub async fn refresh_session_list_index(
    app: tauri::AppHandle,
    cli_id: Option<String>,
    notify: Option<bool>,
    force: Option<bool>,
) -> AppResult<bool> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    let force = force.unwrap_or(false);
    let app_clone = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _ = crate::db::purge_subagents_from_db();
        scan_projects_inner_for_cli(kind, None, None, force)?;
        Ok::<(), AppError>(())
    })
    .await
    .map_err(|e| AppError::business(e.to_string()))??;
    if notify.unwrap_or(false) {
        emit_session_list_index_updated(&app_clone, kind);
    }
    Ok(true)
}

pub(crate) fn scan_projects_inner_for_cli(
    kind: CliKind,
    page: Option<usize>,
    page_size: Option<usize>,
    force: bool,
) -> AppResult<PaginatedProjects> {
    let custom_names = super::session::load_session_names().unwrap_or_default();
    let all_sessions = match kind {
        CliKind::Claude => scan_claude_projects(&custom_names, force)?,
        CliKind::Codex => scan_codex_projects(&custom_names, force)?,
        CliKind::Gemini => scan_gemini_projects(&custom_names, force)?,
        CliKind::WorkBuddy => scan_workbuddy_projects(&custom_names, force)?,
        CliKind::Dsh => scan_dsh_projects(&custom_names, force)?,
        CliKind::Antigravity => scan_antigravity_projects(&custom_names, force)?,
    };

    Ok(paginate_sessions(all_sessions, page, page_size))
}

fn scan_claude_projects(
    custom_names: &HashMap<String, String>,
    force: bool,
) -> AppResult<Vec<RawSession>> {
    let projects_dir = cli::sessions_dir(CliKind::Claude)?;
    let history_path = cli::history_path(CliKind::Claude)?;
    let (session_map, project_map) = history::parse_history(history_path.to_str().unwrap_or(""));

    if !projects_dir.exists() {
        return Ok(Vec::new());
    }

    // force 模式下不使用缓存索引，所有文件重新解析
    let cached_index = if force {
        HashMap::new()
    } else {
        load_session_list_index(CliKind::Claude)
    };
    let mut index_updates = Vec::new();
    let mut seen_paths = HashSet::new();
    let mut invalid_index_paths = HashSet::new();
    let mut all_sessions = Vec::new();
    let entries = fs::read_dir(&projects_dir)?;

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let encoded_dir = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let fallback_project_path =
            session_mod::resolve_project_path(&encoded_dir, None, Some(&project_map));

        let session_entries = match fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for session_entry in session_entries {
            let session_entry = match session_entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let session_path = session_entry.path();
            let file_name = session_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            if !file_name.ends_with(".jsonl") {
                continue;
            }

            let file_metadata = match fs::metadata(&session_path) {
                Ok(metadata) if metadata.is_file() => metadata,
                _ => continue,
            };
            let file_path = session_path.to_str().unwrap_or("").to_string();
            seen_paths.insert(file_path.clone());
            let Some(mut metadata) = resolve_session_index_payload(
                &session_path,
                &file_path,
                &file_metadata,
                &cached_index,
                &mut index_updates,
                &mut invalid_index_paths,
            ) else {
                continue;
            };

            let resolved_project_path = parser::read_project_path(&file_path)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .or_else(|| {
                    session_mod::find_project_path_in_map(&encoded_dir, &project_map)
                        .map(|value| value.to_string())
                })
                .or_else(|| {
                    metadata
                        .project_path
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty() && *value != "未知项目")
                        .map(|value| value.to_string())
                });

            if metadata.project_path != resolved_project_path {
                metadata.project_path = resolved_project_path.clone();
                index_updates.push(build_session_list_index_record(
                    &file_path,
                    file_modified_ms(&file_metadata),
                    &metadata,
                ));
            }

            let original_path = metadata
                .project_path
                .clone()
                .unwrap_or_else(|| fallback_project_path.clone());
            let session_id = metadata.session_id;

            let display_name = crate::db::title_resolver::resolve_display_name(
                super::session::custom_session_name(custom_names, CliKind::Claude, &file_path),
                metadata.title.as_deref(),
                metadata.first_user_message.as_deref(),
                session_map
                    .get(&session_id)
                    .map(|h| h.display.as_str())
                    .filter(|d| !d.is_empty()),
                &session_id,
            );

            let timestamp = metadata
                .last_timestamp
                .or(metadata.first_timestamp)
                .unwrap_or_default();
            let file_size = metadata.file_size;
            let git_branch = metadata.git_branch;
            let cached_record = cached_index.get(&file_path);
            let has_archive_snapshot = cached_record
                .map(|record| record.has_archive_snapshot)
                .unwrap_or(false);
            let is_archived = cached_record
                .map(|record| record.is_archived)
                .unwrap_or(false);

            all_sessions.push(RawSession {
                session: SessionInfo {
                    session_id,
                    file_path,
                    display_name,
                    timestamp,
                    file_size,
                    git_branch,
                    has_archive_snapshot,
                    is_archived,
                    cli_id: CliKind::Claude.id().to_string(),
                },
                encoded_dir: encoded_dir.clone(),
                original_path: original_path.clone(),
            });
        }
    }

    persist_session_list_index_changes(
        CliKind::Claude,
        &cached_index,
        &seen_paths,
        index_updates,
        &invalid_index_paths,
    );

    Ok(all_sessions)
}

fn scan_codex_projects(
    custom_names: &HashMap<String, String>,
    force: bool,
) -> AppResult<Vec<RawSession>> {
    let sessions_dir = cli::sessions_dir(CliKind::Codex)?;
    let history_path = cli::history_path(CliKind::Codex)?;
    let session_map = history::parse_codex_history(history_path.to_str().unwrap_or(""));

    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }

    // force 模式下不使用缓存索引，所有文件重新解析
    let cached_index = if force {
        HashMap::new()
    } else {
        load_session_list_index(CliKind::Codex)
    };
    let mut index_updates = Vec::new();
    let mut seen_paths = HashSet::new();
    let mut invalid_index_paths = HashSet::new();
    let mut session_files = Vec::new();
    collect_jsonl_files(&sessions_dir, &mut session_files)?;
    session_files.sort();

    let mut all_sessions = Vec::new();
    // 单次读取 Codex 索引 thread_name Map（规范 §3.1），整批查询 O(1) 命中
    let codex_titles = parser::load_codex_index_titles();
    for session_path in session_files {
        let file_metadata = match fs::metadata(&session_path) {
            Ok(metadata) if metadata.is_file() => metadata,
            _ => continue,
        };
        let file_path = session_path.to_str().unwrap_or("").to_string();
        seen_paths.insert(file_path.clone());
        let Some(metadata) = resolve_session_index_payload(
            &session_path,
            &file_path,
            &file_metadata,
            &cached_index,
            &mut index_updates,
            &mut invalid_index_paths,
        ) else {
            continue;
        };

        let session_id = metadata.session_id;
        let original_path = metadata
            .project_path
            .filter(|path| !path.trim().is_empty())
            .unwrap_or_else(|| "未知项目".to_string());
        let encoded_dir = original_path.clone();
        let title = metadata
            .title
            .as_deref()
            .or_else(|| codex_titles.get(&session_id).map(String::as_str))
            .or_else(|| {
                session_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(|stem| codex_titles.get(stem).map(String::as_str))
            });
        let display_name = crate::db::title_resolver::resolve_display_name(
            super::session::custom_session_name(custom_names, CliKind::Codex, &file_path),
            title,
            metadata.first_user_message.as_deref(),
            session_map
                .get(&session_id)
                .map(|h| h.display.as_str())
                .filter(|d| !d.is_empty()),
            &session_id,
        );
        let timestamp = metadata
            .last_timestamp
            .or(metadata.first_timestamp)
            .unwrap_or_default();
        let file_size = metadata.file_size;
        let git_branch = metadata.git_branch;
        let cached_record = cached_index.get(&file_path);
        let has_archive_snapshot = cached_record
            .map(|record| record.has_archive_snapshot)
            .unwrap_or(false);
        let is_archived = cached_record
            .map(|record| record.is_archived)
            .unwrap_or(false);

        all_sessions.push(RawSession {
            session: SessionInfo {
                session_id,
                file_path,
                display_name,
                timestamp,
                file_size,
                git_branch,
                has_archive_snapshot,
                is_archived,
                cli_id: CliKind::Codex.id().to_string(),
            },
            encoded_dir,
            original_path,
        });
    }

    persist_session_list_index_changes(
        CliKind::Codex,
        &cached_index,
        &seen_paths,
        index_updates,
        &invalid_index_paths,
    );

    Ok(all_sessions)
}

fn scan_gemini_projects(
    custom_names: &HashMap<String, String>,
    force: bool,
) -> AppResult<Vec<RawSession>> {
    let sessions_dir = cli::sessions_dir(CliKind::Gemini)?;
    let data_dir = cli::data_dir(CliKind::Gemini)?;

    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }

    let history_dir = data_dir.join("history");
    let mut project_paths: HashMap<String, String> = HashMap::new();
    if history_dir.exists() {
        if let Ok(entries) = fs::read_dir(&history_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let project_name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                let root_file = path.join(".project_root");
                if let Ok(root) = fs::read_to_string(&root_file) {
                    let root = root.trim().to_string();
                    if !root.is_empty() {
                        project_paths.insert(project_name, root);
                    }
                }
            }
        }
    }

    let projects_json_path = data_dir.join("projects.json");
    if projects_json_path.exists() {
        if let Ok(content) = fs::read_to_string(&projects_json_path) {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(projects) = parsed.get("projects").and_then(|v| v.as_object()) {
                    for (path, name) in projects {
                        if let Some(name_str) = name.as_str() {
                            project_paths
                                .entry(name_str.to_string())
                                .or_insert(path.clone());
                        }
                    }
                }
            }
        }
    }

    // force 模式下不使用缓存索引，所有文件重新解析
    let cached_index = if force {
        HashMap::new()
    } else {
        load_session_list_index(CliKind::Gemini)
    };
    let mut index_updates = Vec::new();
    let mut seen_paths = HashSet::new();
    let mut invalid_index_paths = HashSet::new();
    let mut all_sessions = Vec::new();

    let entries = fs::read_dir(&sessions_dir)?;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let project_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let chats_dir = path.join("chats");
        if !chats_dir.exists() {
            continue;
        }

        let original_path = project_paths
            .get(&project_name)
            .cloned()
            .unwrap_or_else(|| project_name.clone());
        let encoded_dir = original_path.clone();

        let chat_entries = match fs::read_dir(&chats_dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for session_entry in chat_entries {
            let session_entry = match session_entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let session_path = session_entry.path();
            let file_name = session_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            if !file_name.ends_with(".jsonl") {
                continue;
            }

            let file_metadata = match fs::metadata(&session_path) {
                Ok(metadata) if metadata.is_file() => metadata,
                _ => continue,
            };
            let file_path = session_path.to_str().unwrap_or("").to_string();
            seen_paths.insert(file_path.clone());
            let Some(mut metadata) = resolve_session_index_payload(
                &session_path,
                &file_path,
                &file_metadata,
                &cached_index,
                &mut index_updates,
                &mut invalid_index_paths,
            ) else {
                continue;
            };

            let resolved_project_path = Some(original_path.clone());
            if metadata.project_path != resolved_project_path {
                metadata.project_path = resolved_project_path;
                index_updates.push(build_session_list_index_record(
                    &file_path,
                    file_modified_ms(&file_metadata),
                    &metadata,
                ));
            }

            let session_id = metadata.session_id;
            let display_name = crate::db::title_resolver::resolve_display_name(
                super::session::custom_session_name(custom_names, CliKind::Gemini, &file_path),
                metadata.title.as_deref(),
                metadata.first_user_message.as_deref(),
                None,
                &session_id,
            );
            let timestamp = metadata
                .last_timestamp
                .or(metadata.first_timestamp)
                .unwrap_or_default();
            let file_size = metadata.file_size;
            let cached_record = cached_index.get(&file_path);
            let has_archive_snapshot = cached_record
                .map(|record| record.has_archive_snapshot)
                .unwrap_or(false);
            let is_archived = cached_record
                .map(|record| record.is_archived)
                .unwrap_or(false);

            all_sessions.push(RawSession {
                session: SessionInfo {
                    session_id,
                    file_path,
                    display_name,
                    timestamp,
                    file_size,
                    git_branch: String::new(),
                    has_archive_snapshot,
                    is_archived,
                    cli_id: CliKind::Gemini.id().to_string(),
                },
                encoded_dir: encoded_dir.clone(),
                original_path: original_path.clone(),
            });
        }
    }

    persist_session_list_index_changes(
        CliKind::Gemini,
        &cached_index,
        &seen_paths,
        index_updates,
        &invalid_index_paths,
    );

    Ok(all_sessions)
}

fn scan_workbuddy_projects(
    custom_names: &HashMap<String, String>,
    force: bool,
) -> AppResult<Vec<RawSession>> {
    let sessions_dir = cli::sessions_dir(CliKind::WorkBuddy)?;

    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }

    // force 模式下不使用缓存索引，所有文件重新解析
    let cached_index = if force {
        HashMap::new()
    } else {
        load_session_list_index(CliKind::WorkBuddy)
    };
    let mut index_updates = Vec::new();
    let mut seen_paths = HashSet::new();
    let mut invalid_index_paths = HashSet::new();
    let mut all_sessions = Vec::new();
    // 单次读取 WorkBuddy 用户重命名 Map（workbuddy.db 的 sessions.custom_title），整批 O(1) 命中
    let wb_titles = parser::load_workbuddy_custom_titles();

    let entries = fs::read_dir(&sessions_dir)?;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let project_slug = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        let session_entries = match fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for session_entry in session_entries {
            let session_entry = match session_entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let session_path = session_entry.path();
            let file_name = session_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            if !file_name.ends_with(".jsonl") {
                continue;
            }

            let file_metadata = match fs::metadata(&session_path) {
                Ok(metadata) if metadata.is_file() => metadata,
                _ => continue,
            };
            let file_path = session_path.to_str().unwrap_or("").to_string();
            seen_paths.insert(file_path.clone());
            let Some(mut metadata) = resolve_session_index_payload(
                &session_path,
                &file_path,
                &file_metadata,
                &cached_index,
                &mut index_updates,
                &mut invalid_index_paths,
            ) else {
                continue;
            };

            // 权威项目路径来自每行 JSONL 的 cwd；兜底按 slug 解码
            let original_path = metadata
                .project_path
                .clone()
                .filter(|p| !p.trim().is_empty())
                .unwrap_or_else(|| {
                    session_mod::resolve_project_path(&project_slug, None, None)
                });
            let encoded_dir = original_path.clone();

            if metadata.project_path.as_deref() != Some(original_path.as_str()) {
                metadata.project_path = Some(original_path.clone());
                index_updates.push(build_session_list_index_record(
                    &file_path,
                    file_modified_ms(&file_metadata),
                    &metadata,
                ));
            }

            let session_id = metadata.session_id;
            let display_name = crate::db::title_resolver::resolve_display_name(
                super::session::custom_session_name(custom_names, CliKind::WorkBuddy, &file_path),
                wb_titles
                    .get(&session_id)
                    .map(String::as_str)
                    .or(metadata.title.as_deref()),
                metadata.first_user_message.as_deref(),
                None,
                &session_id,
            );
            let timestamp = metadata
                .last_timestamp
                .or(metadata.first_timestamp)
                .unwrap_or_default();
            let file_size = metadata.file_size;
            let cached_record = cached_index.get(&file_path);
            let has_archive_snapshot = cached_record
                .map(|record| record.has_archive_snapshot)
                .unwrap_or(false);
            let is_archived = cached_record
                .map(|record| record.is_archived)
                .unwrap_or(false);

            all_sessions.push(RawSession {
                session: SessionInfo {
                    session_id,
                    file_path,
                    display_name,
                    timestamp,
                    file_size,
                    git_branch: String::new(),
                    has_archive_snapshot,
                    is_archived,
                    cli_id: CliKind::WorkBuddy.id().to_string(),
                },
                encoded_dir: encoded_dir.clone(),
                original_path: original_path.clone(),
            });
        }
    }

    persist_session_list_index_changes(
        CliKind::WorkBuddy,
        &cached_index,
        &seen_paths,
        index_updates,
        &invalid_index_paths,
    );

    Ok(all_sessions)
}

/// 从 DSH 会话头行构建列表索引元数据。
/// 权威项目路径 = header `cwd`（缺失时不视为无效，由调用侧兜底解码目录名）；
/// `title` 恒为 None（扫描期不读日志尾部的 `session/title` LLM 标题），展示名经
/// title_resolver 回退到清洗后的首条用户消息（与 `scan_session_metadata_only` 一致，
/// 快照加载与实时扫描共用同一解析链）。
fn build_dsh_session_metadata(
    header: &Value,
    session_path: &Path,
    file_metadata: &fs::Metadata,
) -> Option<parser::SessionListMetadata> {
    let cwd = crate::parser::dsh::parse_session_header_value(header)
        .map(|(cwd, _, _)| cwd)
        .filter(|cwd| !cwd.trim().is_empty());
    let session_id = header
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            session_path
                .parent()
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })?;
    let timestamp = header
        .get("createdAt")
        .and_then(Value::as_i64)
        .or_else(|| Some(file_modified_ms(file_metadata)))
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|dt| dt.to_rfc3339());
    let first_user_message =
        crate::parser::dsh::read_first_user_message(&session_path.to_string_lossy())
            .and_then(|text| crate::db::title_resolver::clean_fallback_user_text(&text));
    Some(parser::SessionListMetadata {
        session_id,
        project_path: cwd,
        title: None,
        first_user_message,
        first_timestamp: timestamp.clone(),
        last_timestamp: timestamp,
        git_branch: String::new(),
        file_size: file_metadata.len(),
    })
}

fn scan_dsh_projects(
    custom_names: &HashMap<String, String>,
    force: bool,
) -> AppResult<Vec<RawSession>> {
    let sessions_dir = cli::sessions_dir(CliKind::Dsh)?;

    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }

    // force 模式下不使用缓存索引，所有文件重新解析
    let cached_index = if force {
        HashMap::new()
    } else {
        load_session_list_index(CliKind::Dsh)
    };
    let mut index_updates = Vec::new();
    let mut seen_paths = HashSet::new();
    let mut invalid_index_paths = HashSet::new();
    let mut all_sessions = Vec::new();

    let entries = fs::read_dir(&sessions_dir)?;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let project_path = entry.path();
        if !project_path.is_dir() {
            continue;
        }

        let encoded_dir_name = project_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_string();
        let fallback_project_path =
            session_mod::resolve_project_path(&encoded_dir_name, None, None);

        let session_entries = match fs::read_dir(&project_path) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for session_entry in session_entries {
            let session_entry = match session_entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let session_path = session_entry.path();
            if !session_path.is_dir() {
                continue;
            }

            // 每会话目录 zstd 优先于明文：目录可能瞬态同时存在两种副本，
            // 只挑一种避免同一 session id 被解析两次。
            let session_file = if session_path.join("session.jsonl.zstd").is_file() {
                session_path.join("session.jsonl.zstd")
            } else {
                session_path.join("session.jsonl")
            };
            if !session_file.is_file() {
                continue;
            }

            let file_metadata = match fs::metadata(&session_file) {
                Ok(metadata) if metadata.is_file() => metadata,
                _ => continue,
            };
            let file_path = session_file.to_str().unwrap_or("").to_string();
            seen_paths.insert(file_path.clone());
            let file_size = file_metadata.len();
            let modified_ms = file_modified_ms(&file_metadata);

            let mut metadata =
                match cached_session_list_metadata(&cached_index, &file_path, file_size, modified_ms)
                {
                    Some(metadata) => metadata,
                    None => {
                        let Some(header) = crate::parser::dsh::read_header_line(&session_file)
                        else {
                            invalid_index_paths.insert(file_path.clone());
                            continue;
                        };
                        // 子代理会话（origin=="subagent"）不进侧边栏列表：与 Claude
                        // subagents/ 目录、sessionview 一致；经父会话 subagent 工具调用的
                        // chip 导航打开（subagent_map 已在解析层接线）。
                        if header.get("origin").and_then(Value::as_str) == Some("subagent") {
                            continue;
                        }
                        // 无 surface 消息的空会话（从未开始，只有 header + 策略事件）
                        // 不进列表，对齐 sessionview 的"无 surfaced 消息即跳过"。
                        if !crate::parser::dsh::has_surface_messages(&session_file) {
                            continue;
                        }
                        let Some(metadata) =
                            build_dsh_session_metadata(&header, &session_file, &file_metadata)
                        else {
                            invalid_index_paths.insert(file_path.clone());
                            continue;
                        };
                        index_updates.push(build_session_list_index_record(
                            &file_path,
                            modified_ms,
                            &metadata,
                        ));
                        metadata
                    }
                };

            // 权威项目路径来自 header cwd；兜底按项目目录名解码
            let original_path = metadata
                .project_path
                .clone()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| fallback_project_path.clone());
            // header cwd 缺失（用目录名解码兜底）时，把解析出的项目路径回写进持久化索引，
            // 否则快照加载（读索引）会退回 session_id 作项目 key，与实时扫描分组分叉。
            if metadata.project_path.as_deref() != Some(original_path.as_str()) {
                metadata.project_path = Some(original_path.clone());
                index_updates.push(build_session_list_index_record(
                    &file_path,
                    modified_ms,
                    &metadata,
                ));
            }
            // 项目分组 key 用权威 cwd（与快照加载路径一致）
            let encoded_dir = original_path.clone();
            let session_id = metadata.session_id.clone();
            let display_name = crate::db::title_resolver::resolve_display_name(
                super::session::custom_session_name(custom_names, CliKind::Dsh, &file_path),
                metadata.title.as_deref(),
                metadata.first_user_message.as_deref(),
                None,
                &session_id,
            );
            let timestamp = metadata
                .last_timestamp
                .or(metadata.first_timestamp)
                .unwrap_or_default();
            let file_size = metadata.file_size;
            let cached_record = cached_index.get(&file_path);
            let has_archive_snapshot = cached_record
                .map(|record| record.has_archive_snapshot)
                .unwrap_or(false);
            let is_archived = cached_record
                .map(|record| record.is_archived)
                .unwrap_or(false);

            all_sessions.push(RawSession {
                session: SessionInfo {
                    session_id,
                    file_path,
                    display_name,
                    timestamp,
                    file_size,
                    git_branch: String::new(),
                    has_archive_snapshot,
                    is_archived,
                    cli_id: CliKind::Dsh.id().to_string(),
                },
                encoded_dir,
                original_path,
            });
        }
    }

    persist_session_list_index_changes(
        CliKind::Dsh,
        &cached_index,
        &seen_paths,
        index_updates,
        &invalid_index_paths,
    );

    Ok(all_sessions)
}

fn scan_antigravity_projects(
    custom_names: &HashMap<String, String>,
    force: bool,
) -> AppResult<Vec<RawSession>> {
    let brain_dir = cli::sessions_dir(CliKind::Antigravity)?;

    if !brain_dir.exists() {
        return Ok(Vec::new());
    }

    let cached_index = if force {
        HashMap::new()
    } else {
        load_session_list_index(CliKind::Antigravity)
    };
    let mut index_updates = Vec::new();
    let mut seen_paths = HashSet::new();
    let mut invalid_index_paths = HashSet::new();
    let mut all_sessions = Vec::new();

    let mut jsonl_files = Vec::new();
    collect_jsonl_files(&brain_dir, &mut jsonl_files)?;

    for session_path in jsonl_files {
        let file_name = session_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if file_name != "transcript.jsonl" {
            continue;
        }

        let file_metadata = match fs::metadata(&session_path) {
            Ok(metadata) if metadata.is_file() => metadata,
            _ => continue,
        };
        let file_path = session_path.to_str().unwrap_or("").to_string();

        if parser::is_subagent_session(&file_path) {
            continue;
        }

        seen_paths.insert(file_path.clone());

        let Some(mut metadata) = resolve_session_index_payload(
            &session_path,
            &file_path,
            &file_metadata,
            &cached_index,
            &mut index_updates,
            &mut invalid_index_paths,
        ) else {
            continue;
        };

        let original_path = metadata
            .project_path
            .clone()
            .filter(|p| !p.trim().is_empty())
            .unwrap_or_else(|| "未知项目".to_string());
        let encoded_dir = original_path.clone();

        if metadata.project_path.as_deref() != Some(original_path.as_str()) {
            metadata.project_path = Some(original_path.clone());
            index_updates.push(build_session_list_index_record(
                &file_path,
                file_modified_ms(&file_metadata),
                &metadata,
            ));
        }

        let session_id = metadata.session_id;
        let display_name = crate::db::title_resolver::resolve_display_name(
            super::session::custom_session_name(custom_names, CliKind::Antigravity, &file_path),
            metadata.title.as_deref(),
            metadata.first_user_message.as_deref(),
            None,
            &session_id,
        );
        let timestamp = metadata
            .last_timestamp
            .or(metadata.first_timestamp)
            .unwrap_or_default();
        let file_size = metadata.file_size;
        let cached_record = cached_index.get(&file_path);
        let has_archive_snapshot = cached_record
            .map(|record| record.has_archive_snapshot)
            .unwrap_or(false);
        let is_archived = cached_record
            .map(|record| record.is_archived)
            .unwrap_or(false);

        all_sessions.push(RawSession {
            session: SessionInfo {
                session_id,
                file_path,
                display_name,
                timestamp,
                file_size,
                git_branch: String::new(),
                has_archive_snapshot,
                is_archived,
                cli_id: CliKind::Antigravity.id().to_string(),
            },
            encoded_dir,
            original_path,
        });
    }

    persist_session_list_index_changes(
        CliKind::Antigravity,
        &cached_index,
        &seen_paths,
        index_updates,
        &invalid_index_paths,
    );

    Ok(all_sessions)
}

fn paginate_sessions(
    mut all_sessions: Vec<RawSession>,
    page: Option<usize>,
    page_size: Option<usize>,
) -> PaginatedProjects {
    let total_sessions = all_sessions.len();
    all_sessions.sort_by(|a, b| b.session.timestamp.cmp(&a.session.timestamp));

    let selected_sessions: Vec<RawSession> = match (page, page_size) {
        (Some(p), Some(ps)) => {
            let start = p * ps;
            if start >= all_sessions.len() {
                Vec::new()
            } else {
                let end = std::cmp::min(start + ps, all_sessions.len());
                all_sessions
                    .into_iter()
                    .skip(start)
                    .take(end - start)
                    .collect()
            }
        }
        _ => all_sessions,
    };

    let has_more = match (page, page_size) {
        (Some(p), Some(ps)) => (p + 1) * ps < total_sessions,
        _ => false,
    };

    build_paginated_projects_from_sessions(selected_sessions, total_sessions, has_more)
}

fn collect_jsonl_files(dir: &Path, files: &mut Vec<PathBuf>) -> AppResult<()> {
    let entries = fs::read_dir(dir)?;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl_files(&path, files)?;
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            files.push(path);
        }
    }
    Ok(())
}
