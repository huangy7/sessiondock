//! 多 CLI 会话解析。按 CLI 拆分到独立模块，本模块负责种类判定与薄分派。

pub(crate) mod antigravity;
pub(crate) mod claude;
pub(crate) mod codex;
pub(crate) mod dsh;
pub(crate) mod gemini;
pub(crate) mod shared;
pub(crate) mod workbuddy;

use crate::parser::shared::{
    file_content_lines, scan_search_docs_from_text, scan_search_docs_with_extractor,
};
use crate::session::{self, ChatMessage, SessionLoadResult, UsageRecord};
use serde_json::Value;
use std::path::Path;

pub use crate::parser::shared::{SearchDocument, SearchScanProgress};
pub(crate) use crate::parser::shared::truncate_transcript;
pub(crate) use codex::load_codex_index_titles;
pub(crate) use workbuddy::load_workbuddy_custom_titles;
pub use crate::session::SessionListMetadata;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionKind {
    Claude,
    Codex,
    Dsh,
    Gemini,
    WorkBuddy,
    Antigravity,
}

/// 按文件路径判定会话种类（与解析源文件/归档时使用同一判定）。
pub fn detect_kind(file_path: &str) -> SessionKind {
    if file_path.contains("/.gemini/antigravity-cli/")
        || file_path.contains("\\.gemini\\antigravity-cli\\")
    {
        SessionKind::Antigravity
    } else if file_path.contains("/.dsh/") || file_path.contains("\\.dsh\\") {
        SessionKind::Dsh
    } else if file_path.contains("/.gemini/") || file_path.contains("\\.gemini\\") {
        SessionKind::Gemini
    } else if file_path.contains("/.codex/") || file_path.contains("\\.codex\\") {
        SessionKind::Codex
    } else if file_path.contains("/.workbuddy/") || file_path.contains("\\.workbuddy\\") {
        SessionKind::WorkBuddy
    } else {
        SessionKind::Claude
    }
}

pub(crate) fn parse_session_file(file_path: &str) -> Result<Vec<ChatMessage>, String> {
    match detect_kind(file_path) {
        SessionKind::Claude => claude::parse_session_file(file_path),
        SessionKind::Codex => {
            codex::parse_codex_session_file(file_path).map(|result| result.messages)
        }
        SessionKind::Dsh => dsh::parse_dsh_session_file(file_path),
        SessionKind::Gemini => {
            gemini::parse_gemini_session_file(file_path).map(|result| result.messages)
        }
        SessionKind::WorkBuddy => {
            workbuddy::parse_workbuddy_session_file(file_path).map(|result| result.messages)
        }
        SessionKind::Antigravity => antigravity::parse_session_file(file_path),
    }
}

pub(crate) fn parse_session_file_with_offset(
    file_path: &str,
    skip_sidechain: bool,
) -> Result<SessionLoadResult, String> {
    match detect_kind(file_path) {
        SessionKind::Claude => claude::parse_session_file_with_offset(file_path, skip_sidechain),
        SessionKind::Codex => codex::parse_codex_session_file(file_path),
        SessionKind::Dsh => dsh::parse_dsh_session_file_with_offset(file_path),
        SessionKind::Gemini => gemini::parse_gemini_session_file(file_path),
        SessionKind::WorkBuddy => workbuddy::parse_workbuddy_session_file(file_path),
        SessionKind::Antigravity => antigravity::parse_session_file_with_offset(file_path, skip_sidechain),
    }
}

pub(crate) fn parse_session_file_streaming<F>(
    file_path: &str,
    skip_sidechain: bool,
    batch_size: usize,
    on_batch: F,
) -> Result<
    (
        u64,
        std::collections::HashMap<String, session::SubagentInfo>,
    ),
    String,
>
where
    F: FnMut(Vec<session::ChatMessage>) -> bool,
{
    match detect_kind(file_path) {
        SessionKind::Claude => {
            claude::parse_session_file_streaming(file_path, skip_sidechain, batch_size, on_batch)
        }
        SessionKind::Codex => {
            codex::parse_codex_session_file_streaming(file_path, batch_size, on_batch)
        }
        SessionKind::Dsh => dsh::parse_dsh_session_file_streaming(file_path, batch_size, on_batch),
        SessionKind::Gemini => {
            gemini::parse_gemini_session_file_streaming(file_path, batch_size, on_batch)
        }
        SessionKind::WorkBuddy => {
            workbuddy::parse_workbuddy_session_file_streaming(file_path, batch_size, on_batch)
        }
        SessionKind::Antigravity => {
            antigravity::parse_session_file_streaming(file_path, skip_sidechain, batch_size, on_batch)
        }
    }
}

pub(crate) fn parse_session_incremental(
    file_path: &str,
    offset: u64,
    skip_sidechain: bool,
) -> Result<SessionLoadResult, String> {
    match detect_kind(file_path) {
        SessionKind::Claude => claude::parse_session_incremental(file_path, offset, skip_sidechain),
        SessionKind::Codex => codex::parse_codex_session_incremental(file_path, offset),
        SessionKind::Dsh => dsh::parse_dsh_session_incremental(file_path, offset),
        SessionKind::Gemini => gemini::parse_gemini_session_incremental(file_path, offset),
        SessionKind::WorkBuddy => workbuddy::parse_workbuddy_session_incremental(file_path, offset),
        SessionKind::Antigravity => antigravity::parse_session_incremental(file_path, offset),
    }
}

/// 从内存内容按会话类型解析消息（归档兜底用）。
/// 与源文件路径解析保持一致：按 detect_kind 分派对应 CLI 的解析器，
/// 避免归档兜底误用通用 Claude 解析器导致 Codex/Gemini/WorkBuddy/Antigravity 归档解析出 0 条。
pub(crate) fn parse_session_content_by_kind(
    file_path: &str,
    content: &str,
) -> Vec<session::ChatMessage> {
    match detect_kind(file_path) {
        SessionKind::Claude => claude::parse_session_from_string(content).unwrap_or_default(),
        SessionKind::Codex => codex::parse_codex_session_from_string(content),
        SessionKind::Dsh => dsh::parse_dsh_session_from_string(content),
        SessionKind::Gemini => gemini::parse_gemini_session_from_string(content),
        SessionKind::WorkBuddy => workbuddy::parse_workbuddy_session_from_string(content),
        SessionKind::Antigravity => antigravity::parse_antigravity_session_from_string(content),
    }
}

pub(crate) fn has_chat_messages(file_path: &str) -> bool {
    match detect_kind(file_path) {
        SessionKind::Claude => claude::has_chat_messages(file_path),
        SessionKind::Codex => codex::extract_searchable_messages(file_path)
            .into_iter()
            .next()
            .is_some(),
        SessionKind::Dsh => dsh::has_chat_messages(file_path),
        SessionKind::Gemini => scan_search_docs_with_extractor(
            Path::new(file_path),
            gemini::extract_gemini_role_text,
            |_| {},
        )
        .is_some(),
        SessionKind::WorkBuddy => scan_search_docs_with_extractor(
            Path::new(file_path),
            workbuddy::extract_workbuddy_role_text,
            |_| {},
        )
        .is_some(),
        SessionKind::Antigravity => antigravity::has_chat_messages(file_path),
    }
}

pub(crate) fn read_first_user_message(file_path: &str) -> Option<String> {
    match detect_kind(file_path) {
        SessionKind::Claude => claude::read_first_user_message(file_path),
        SessionKind::Codex => codex::read_codex_first_user_message(file_path),
        SessionKind::Dsh => dsh::read_first_user_message(file_path),
        SessionKind::Gemini => gemini::read_gemini_first_user_message(file_path),
        SessionKind::WorkBuddy => workbuddy::read_workbuddy_first_user_message(file_path),
        SessionKind::Antigravity => antigravity::read_first_user_message(file_path),
    }
}

pub(crate) fn read_session_id(file_path: &str) -> Option<String> {
    match detect_kind(file_path) {
        SessionKind::Claude => Path::new(file_path)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(|stem| stem.to_string()),
        SessionKind::Codex => codex::read_codex_session_meta_field(file_path, |payload| {
            payload
                .get("id")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string())
        })
        .or_else(|| {
            Path::new(file_path)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .and_then(|stem| stem.rsplit('-').next())
                .map(|stem| stem.to_string())
        }),
        SessionKind::Dsh => dsh::read_session_id(file_path),
        SessionKind::Gemini => gemini::read_gemini_header_field(file_path, "sessionId"),
        SessionKind::WorkBuddy => {
            workbuddy::read_workbuddy_header_field(file_path, "sessionId").or_else(|| {
                Path::new(file_path)
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .map(|stem| stem.to_string())
            })
        }
        SessionKind::Antigravity => antigravity::read_session_id(file_path),
    }
}

pub(crate) fn read_project_path(file_path: &str) -> Option<String> {
    match detect_kind(file_path) {
        SessionKind::Claude => None,
        SessionKind::Codex => codex::read_codex_session_meta_field(file_path, |payload| {
            payload
                .get("cwd")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string())
        })
        .or_else(|| codex::read_codex_turn_context_cwd(file_path)),
        SessionKind::Dsh => dsh::read_project_path(file_path),
        SessionKind::Gemini => None,
        SessionKind::WorkBuddy => workbuddy::read_workbuddy_header_field(file_path, "cwd"),
        SessionKind::Antigravity => antigravity::read_project_path(file_path),
    }
}

pub(crate) fn read_first_timestamp(file_path: &str) -> Option<String> {
    match detect_kind(file_path) {
        // 其它 CLI 沿用 Claude 首时间戳读取（不变更既有行为）。
        SessionKind::Dsh => dsh::read_first_timestamp(file_path),
        SessionKind::Antigravity => antigravity::read_first_timestamp(file_path),
        _ => claude::read_first_timestamp(file_path),
    }
}

pub(crate) fn read_last_timestamp(file_path: &str) -> Option<String> {
    match detect_kind(file_path) {
        SessionKind::Dsh => dsh::read_last_timestamp(file_path),
        SessionKind::Antigravity => antigravity::read_last_timestamp(file_path),
        _ => claude::read_last_timestamp(file_path),
    }
}

pub(crate) fn read_git_branch(file_path: &str) -> String {
    match detect_kind(file_path) {
        SessionKind::Claude => claude::read_git_branch(file_path),
        SessionKind::Codex => String::new(),
        SessionKind::Dsh => String::new(),
        SessionKind::Gemini => String::new(),
        SessionKind::WorkBuddy => String::new(),
        SessionKind::Antigravity => antigravity::read_git_branch(file_path),
    }
}

pub(crate) fn extract_usage_records(file_path: &str, project: &str) -> Vec<UsageRecord> {
    match detect_kind(file_path) {
        SessionKind::Claude => claude::extract_usage_records(file_path, project),
        SessionKind::Codex => codex::extract_usage_records(file_path, project),
        SessionKind::Dsh => dsh::extract_usage_records(file_path, project),
        SessionKind::Gemini => gemini::extract_gemini_usage_records(file_path, project),
        SessionKind::Antigravity => antigravity::extract_usage_records(file_path, project),
        // WorkBuddy 用量统计暂不接入
        SessionKind::WorkBuddy => Vec::new(),
    }
}

pub(crate) fn build_clean_transcript(file_path: &str) -> String {
    let messages = match detect_kind(file_path) {
        SessionKind::Claude => file_content_lines(file_path)
            .into_iter()
            .filter_map(|line| {
                let entry: Value = serde_json::from_str(line.trim()).ok()?;
                claude::extract_claude_role_text(&entry)
            })
            .collect::<Vec<_>>(),
        SessionKind::Codex => file_content_lines(file_path)
            .into_iter()
            .filter_map(|line| {
                let entry: Value = serde_json::from_str(line.trim()).ok()?;
                codex::extract_codex_role_text(&entry)
            })
            .collect::<Vec<_>>(),
        SessionKind::Dsh => return dsh::build_clean_transcript(file_path),
        SessionKind::Gemini => file_content_lines(file_path)
            .into_iter()
            .filter_map(|line| {
                let entry: Value = serde_json::from_str(line.trim()).ok()?;
                gemini::extract_gemini_role_text(&entry)
            })
            .collect::<Vec<_>>(),
        SessionKind::WorkBuddy => file_content_lines(file_path)
            .into_iter()
            .filter_map(|line| {
                let entry: Value = serde_json::from_str(line.trim()).ok()?;
                workbuddy::extract_workbuddy_role_text(&entry)
            })
            .collect::<Vec<_>>(),
        SessionKind::Antigravity => return antigravity::build_clean_transcript(file_path),
    };

    messages
        .into_iter()
        .map(|(role, text)| format!("[{}]\n{}", role, text.trim()))
        .filter(|chunk| !chunk.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(crate) fn scan_session_metadata_only(file_path: &Path) -> Option<SessionListMetadata> {
    let path_str = file_path.to_string_lossy().to_string();
    match detect_kind(&path_str) {
        SessionKind::Claude => claude::scan_claude_metadata_only(file_path),
        SessionKind::Codex => codex::scan_codex_metadata_only(file_path),
        SessionKind::Dsh => dsh::scan_session_metadata_only(file_path),
        SessionKind::Gemini => gemini::scan_gemini_metadata_only(file_path),
        SessionKind::WorkBuddy => workbuddy::scan_workbuddy_metadata_only(file_path),
        SessionKind::Antigravity => antigravity::scan_session_metadata_only(file_path),
    }
}

pub(crate) fn is_subagent_session(file_path: &str) -> bool {
    let path = Path::new(file_path);
    if path
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        == Some("subagents")
    {
        return true;
    }
    match detect_kind(file_path) {
        SessionKind::Claude => false,
        SessionKind::Codex => codex::is_codex_subagent_file(file_path),
        SessionKind::Dsh => dsh::is_subagent_file(file_path),
        SessionKind::Antigravity => antigravity::is_subagent_session(file_path),
        _ => false,
    }
}

pub(crate) fn scan_session_search_docs_with_progress<F>(
    file_path: &Path,
    on_progress: F,
) -> Option<Vec<SearchDocument>>
where
    F: FnMut(SearchScanProgress),
{
    let path_str = file_path.to_string_lossy().to_string();
    match detect_kind(&path_str) {
        SessionKind::Claude => {
            scan_search_docs_with_extractor(file_path, claude::extract_claude_role_text, on_progress)
        }
        SessionKind::Codex => {
            scan_search_docs_with_extractor(file_path, codex::extract_codex_role_text, on_progress)
        }
        SessionKind::Dsh => dsh::scan_search_docs_with_progress(file_path, on_progress),
        SessionKind::Gemini => {
            scan_search_docs_with_extractor(file_path, gemini::extract_gemini_role_text, on_progress)
        }
        SessionKind::WorkBuddy => scan_search_docs_with_extractor(
            file_path,
            workbuddy::extract_workbuddy_role_text,
            on_progress,
        ),
        SessionKind::Antigravity => scan_search_docs_with_extractor(
            file_path,
            antigravity::extract_antigravity_role_text,
            on_progress,
        ),
    }
}

pub(crate) fn scan_session_search_docs_from_bytes(
    file_path: &str,
    content: &[u8],
) -> Option<Vec<SearchDocument>> {
    match detect_kind(file_path) {
        SessionKind::Claude => scan_search_docs_from_text(content, claude::extract_claude_role_text),
        SessionKind::Codex => scan_search_docs_from_text(content, codex::extract_codex_role_text),
        SessionKind::Dsh => dsh::scan_search_docs_from_bytes(content),
        SessionKind::Gemini => scan_search_docs_from_text(content, gemini::extract_gemini_role_text),
        SessionKind::WorkBuddy => {
            scan_search_docs_from_text(content, workbuddy::extract_workbuddy_role_text)
        }
        SessionKind::Antigravity => {
            scan_search_docs_from_text(content, antigravity::extract_antigravity_role_text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn detect_kind_routes_workbuddy_paths() {
        assert!(matches!(
            detect_kind("/Users/x/.workbuddy/projects/foo/sess.jsonl"),
            SessionKind::WorkBuddy
        ));
        assert!(matches!(
            detect_kind("/Users/x/.claude/projects/foo/sess.jsonl"),
            SessionKind::Claude
        ));
    }

    #[test]
    fn search_doc_scan_reports_file_byte_progress() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("sessiondock-search-progress-{unique}.jsonl"));
        let content = concat!(
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"first\"}}\n",
            "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":\"second\"}}\n"
        );
        fs::write(&path, content).unwrap();

        let mut progress = Vec::new();
        let docs = scan_session_search_docs_with_progress(&path, |p| progress.push(p)).unwrap();
        let _ = fs::remove_file(&path);

        assert_eq!(docs.len(), 2);
        assert!(!progress.is_empty());
        assert_eq!(progress.last().unwrap().bytes_read, content.len() as u64);
        assert_eq!(progress.last().unwrap().file_size, content.len() as u64);
    }

    #[test]
    fn search_document_carries_role_from_extractor() {
        // 用 scan_session_search_docs_from_bytes 走真实提取路径
        let content = concat!(
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"你好\"}}\n",
            "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"我在\"}]}}\n"
        );
        let docs = scan_session_search_docs_from_bytes(
            "/tmp/claude-session.jsonl",
            content.as_bytes(),
        )
        .unwrap();
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0].role, "user");
        assert_eq!(docs[1].role, "assistant");
    }

    #[test]
    fn search_doc_scan_from_bytes_indexes_archived_content() {
        let content = concat!(
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"archived needle\"}}\n",
            "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":\"archived answer\"}}\n"
        );

        let docs =
            scan_session_search_docs_from_bytes("/tmp/claude-session.jsonl", content.as_bytes())
                .expect("archived content should produce search docs");

        assert_eq!(docs.len(), 2);
        assert!(docs[0].search_text.contains("archived needle"));
        assert!(docs[1].search_text.contains("archived answer"));
    }
}

#[cfg(test)]
mod archive_dispatch_tests {
    use super::*;

    #[test]
    fn test_archive_content_parses_by_kind() {
        // Codex 归档：通用 Claude 解析器会解析出 0 条，分派后应解析出 1 条
        let codex = r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}}"#;
        let n = parse_session_content_by_kind("/Users/x/.codex/sessions/a.jsonl", codex);
        assert_eq!(n.len(), 1, "codex 归档应解析出 1 条");
        assert_eq!(n[0].role, "user");

        // WorkBuddy 归档
        let wb = r#"{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}"#;
        let n = parse_session_content_by_kind("/Users/x/.workbuddy/projects/p/a.jsonl", wb);
        assert_eq!(n.len(), 1, "workbuddy 归档应解析出 1 条");

        // Claude 归档保持原有行为
        let cl = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"hi"}]}}"#;
        let n = parse_session_content_by_kind("/Users/x/.claude/projects/p/a.jsonl", cl);
        assert_eq!(n.len(), 1, "claude 归档应解析出 1 条");
    }
}

#[cfg(test)]
mod dispatch_tests {
    use super::*;

    fn claude_line() -> &'static str {
        r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"你好"}]}}"#
    }
    fn codex_line() -> &'static str {
        r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}}"#
    }
    fn gemini_line() -> &'static str {
        r#"{"type":"user","content":[{"text":"hello","role":"user"}]}"#
    }
    fn workbuddy_line() -> &'static str {
        r#"{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}"#
    }
    fn antigravity_line() -> &'static str {
        r#"{"step_index":1,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-08-30T12:00:00Z","content":"<USER_REQUEST>hello agy</USER_REQUEST>"}"#
    }

    #[test]
    fn detect_kind_routes_by_path() {
        assert_eq!(detect_kind("/Users/x/.claude/projects/a/b.jsonl"), SessionKind::Claude);
        assert_eq!(detect_kind("/Users/x/.codex/sessions/a.jsonl"), SessionKind::Codex);
        assert_eq!(detect_kind("/Users/x/.gemini/tmp/a.jsonl"), SessionKind::Gemini);
        assert_eq!(detect_kind("/Users/x/.workbuddy/projects/p/a.jsonl"), SessionKind::WorkBuddy);
        assert_eq!(detect_kind("/Users/x/.gemini/antigravity-cli/brain/s1/.system_generated/logs/transcript.jsonl"), SessionKind::Antigravity);
    }

    #[test]
    fn parse_content_by_kind_parses_each_format() {
        assert_eq!(parse_session_content_by_kind("/Users/x/.claude/projects/p/a.jsonl", claude_line()).len(), 1);
        assert_eq!(parse_session_content_by_kind("/Users/x/.codex/sessions/a.jsonl", codex_line()).len(), 1);
        assert_eq!(parse_session_content_by_kind("/Users/x/.gemini/tmp/a.jsonl", gemini_line()).len(), 1);
        assert_eq!(parse_session_content_by_kind("/Users/x/.workbuddy/projects/p/a.jsonl", workbuddy_line()).len(), 1);
        assert_eq!(parse_session_content_by_kind("/Users/x/.gemini/antigravity-cli/brain/s1/transcript.jsonl", antigravity_line()).len(), 1);
    }

    #[test]
    fn dsh_dispatch_arms_route_to_real_implementations() {
        let dir = tempfile::tempdir().unwrap();
        let session_dir = dir
            .path()
            .join(".dsh")
            .join("sessions")
            .join("--Users-x-proj--")
            .join("sess-1");
        std::fs::create_dir_all(&session_dir).unwrap();
        let path = session_dir.join("session.jsonl");
        let content = concat!(
            "{\"type\":\"session\",\"version\":0,\"id\":\"sess-1\",\"createdAt\":1700000000000,\"cwd\":\"/Users/x/proj\"}\n",
            "{\"type\":\"user/message\",\"seq\":1,\"time\":1700000000000,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"hello dsh\"}]}}\n",
            "{\"type\":\"assistant/message\",\"seq\":2,\"time\":1700000000001,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"hi there\"}]}}\n",
        );
        std::fs::write(&path, content).unwrap();
        let file_path = path.to_str().unwrap();

        assert_eq!(read_session_id(file_path).as_deref(), Some("sess-1"));
        assert_eq!(read_project_path(file_path).as_deref(), Some("/Users/x/proj"));
        assert_eq!(read_first_user_message(file_path).as_deref(), Some("hello dsh"));
        assert!(!read_first_timestamp(file_path).unwrap_or_default().is_empty());
        assert!(!read_last_timestamp(file_path).unwrap_or_default().is_empty());

        let transcript = build_clean_transcript(file_path);
        assert!(transcript.contains("hello dsh"));
        assert!(transcript.contains("hi there"));

        let docs = scan_session_search_docs_from_bytes(file_path, content.as_bytes()).unwrap();
        assert_eq!(docs.len(), 2);
        assert!(docs[0].search_text.contains("hello dsh"));

        let metadata = scan_session_metadata_only(&path).expect("metadata");
        assert_eq!(metadata.session_id, "sess-1");
        assert_eq!(metadata.project_path.as_deref(), Some("/Users/x/proj"));
    }

    #[test]
    fn antigravity_dispatch_arms_route_to_real_implementations() {
        let dir = tempfile::tempdir().unwrap();
        let session_dir = dir
            .path()
            .join(".gemini")
            .join("antigravity-cli")
            .join("brain")
            .join("agy-sess-1")
            .join(".system_generated")
            .join("logs");
        std::fs::create_dir_all(&session_dir).unwrap();
        let path = session_dir.join("transcript.jsonl");
        let content = concat!(
            "{\"step_index\":1,\"source\":\"USER_EXPLICIT\",\"type\":\"USER_INPUT\",\"status\":\"DONE\",\"created_at\":\"2026-08-30T12:00:00Z\",\"content\":\"<USER_REQUEST>hello antigravity</USER_REQUEST>\"}\n",
            "{\"step_index\":2,\"source\":\"MODEL\",\"type\":\"PLANNER_RESPONSE\",\"status\":\"DONE\",\"created_at\":\"2026-08-30T12:00:01Z\",\"content\":\"hi from agy\"}\n",
        );
        std::fs::write(&path, content).unwrap();
        let file_path = path.to_str().unwrap();

        assert_eq!(read_session_id(file_path).as_deref(), Some("agy-sess-1"));
        assert_eq!(read_first_user_message(file_path).as_deref(), Some("hello antigravity"));
        assert!(!read_first_timestamp(file_path).unwrap_or_default().is_empty());
        assert!(!read_last_timestamp(file_path).unwrap_or_default().is_empty());

        let transcript = build_clean_transcript(file_path);
        assert!(transcript.contains("hello antigravity"));
        assert!(transcript.contains("hi from agy"));

        let docs = scan_session_search_docs_from_bytes(file_path, content.as_bytes()).unwrap();
        assert_eq!(docs.len(), 2);
        assert!(docs[0].search_text.contains("hello antigravity"));

        let metadata = scan_session_metadata_only(&path).expect("metadata");
        assert_eq!(metadata.session_id, "agy-sess-1");
    }
}
