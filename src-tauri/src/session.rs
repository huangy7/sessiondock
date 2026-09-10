use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub encoded_dir: String,
    pub original_path: String,
    pub sessions: Vec<SessionInfo>,
}

/// Paginated response for scan_projects.
#[derive(Debug, Clone, Serialize)]
pub struct PaginatedProjects {
    pub projects: Vec<ProjectInfo>,
    pub has_more: bool,
    pub total_sessions: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub file_path: String,
    pub display_name: String,
    pub timestamp: String,
    pub file_size: u64,
    pub git_branch: String,
    pub has_archive_snapshot: bool,
    pub is_archived: bool,
    /// 会话归属的 CLI（CliKind::id()），前端 Tab 据此绑定操作上下文
    pub cli_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub timestamp: String,
    pub model: Option<String>,
    pub token_usage: Option<TokenUsage>,
    pub content_parts: Vec<ContentPart>,
    #[serde(default)]
    pub is_meta: bool,
    /// Transcript entry uuid (Claude only) — stable anchor for fork-from-here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        summary: String,
        #[serde(default)]
        tool_name: String,
        #[serde(default)]
        input: String,
        tool_use_id: Option<String>,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        summary: String,
        content: String,
        is_error: bool,
    },
    #[serde(rename = "thinking")]
    Thinking { thinking: String },
    #[serde(rename = "image")]
    Image { media_type: String, data: String },
    #[serde(rename = "image_ref")]
    ImageRef { path: String },
    #[serde(rename = "image_meta")]
    ImageMeta,
}

/// A single usage record extracted from an assistant turn.
#[derive(Debug, Clone, Serialize)]
pub struct UsageRecord {
    pub date: String,          // YYYY-MM-DD (local time)
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub duration_ms: Option<u64>,
    pub project: String,
}

#[derive(Debug, Clone)]
pub struct SessionListMetadata {
    pub session_id: String,
    pub project_path: Option<String>,
    /// Provider 原生标题（Claude custom-title/ai-title；Codex 由调用侧经索引 Map 注入）
    pub title: Option<String>,
    /// 清洗后的首条用户消息（标题兜底）
    pub first_user_message: Option<String>,
    pub first_timestamp: Option<String>,
    pub last_timestamp: Option<String>,
    pub git_branch: String,
    pub file_size: u64,
}

/// Decode an encoded project directory name back to a path.
///
/// 注意：Claude 的目录编码会把路径分隔符编码为 `-`，而真实目录名里的 `-`
/// 也会原样保留，因此单靠目录名反解在 Windows 下存在歧义（例如 `go-timer`）。
/// 这个函数只适合作为最后的兜底；正常情况下应优先使用会话文件里的
/// 真实 `cwd`，其次才是 `history.jsonl` 中的原始项目路径映射。
pub fn decode_project_dir(encoded: &str) -> String {
    if encoded.starts_with('-') {
        encoded.replace('-', "/")
    } else if encoded.len() >= 2 && encoded.as_bytes()[0].is_ascii_alphabetic() && encoded.as_bytes()[1] == b'-' {
        let drive = encoded.as_bytes()[0] as char;
        let rest = &encoded[2..];
        format!("{}:\\{}", drive, rest.replace('-', "\\"))
    } else {
        encoded.replace('-', "/")
    }
}

pub fn find_project_path_in_map<'a>(
    encoded: &str,
    project_map: &'a HashMap<String, String>,
) -> Option<&'a str> {
    project_map
        .get(encoded)
        .or_else(|| {
            if encoded.len() >= 2
                && encoded.as_bytes()[0].is_ascii_alphabetic()
                && encoded.as_bytes()[1] == b'-'
            {
                project_map
                    .iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case(encoded))
                    .map(|(_, value)| value)
            } else {
                None
            }
        })
        .map(|value| value.as_str())
}

pub fn resolve_project_path(
    encoded: &str,
    cached_path: Option<&str>,
    project_map: Option<&HashMap<String, String>>,
) -> String {
    if encoded.trim().is_empty() || encoded == "unknown" {
        return "未知项目".to_string();
    }

    if let Some(path) = project_map
        .and_then(|map| find_project_path_in_map(encoded, map))
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return path.to_string();
    }

    if let Some(path) = cached_path
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "未知项目")
    {
        return path.to_string();
    }

    decode_project_dir(encoded)
}

/// Generate a tool_use summary from the tool name and input JSON.
pub fn tool_use_summary(name: &str, input: &serde_json::Value) -> String {
    match name {
        "Read" | "Write" | "Edit" => {
            let file_path = input
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let filename = Path::new(file_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(file_path);
            format!("[{}: {}]", name, filename)
        }
        "Bash" => {
            let cmd = input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let truncated = if cmd.chars().count() > 40 {
                let s: String = cmd.chars().take(40).collect();
                format!("{}...", s)
            } else {
                cmd.to_string()
            };
            format!("[Bash: {}]", truncated)
        }
        "Grep" => {
            let pattern = input
                .get("pattern")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("[Grep: {}]", pattern)
        }
        "Glob" => {
            let pattern = input
                .get("pattern")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("[Glob: {}]", pattern)
        }
        "Task" => {
            let desc = input
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("[Task: {}]", desc)
        }
        "show_widget" => {
            let title = input
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if title.is_empty() {
                "[图表]".to_string()
            } else {
                format!("[图表: {}]", title)
            }
        }
        _ => format!("[{}]", name),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentInfo {
    pub file_path: String,
    pub label: String,
}

/// Result of incremental session loading.
#[derive(Debug, Clone, Serialize)]
pub struct SessionLoadResult {
    pub messages: Vec<ChatMessage>,
    pub offset: u64,
    pub subagent_map: HashMap<String, SubagentInfo>,
}

/// Clean user prompt text by stripping system caveat blocks (e.g. <local-command-caveat>) and XML tags.
pub fn clean_user_message_text(text: &str) -> Option<String> {
    let mut s = text.trim();
    if s.is_empty() {
        return None;
    }
    // 1. 如果包含 </local-command-caveat>，截取其后面的有效内容
    if let Some(idx) = s.find("</local-command-caveat>") {
        s = s[idx + "</local-command-caveat>".len()..].trim();
    }
    // 2. 如果包含 <command-message>，提取里面的内容
    if let (Some(start), Some(end)) = (s.find("<command-message>"), s.find("</command-message>")) {
        if end > start + "<command-message>".len() {
            let cmd_msg = s[start + "<command-message>".len()..end].trim();
            if !cmd_msg.is_empty() {
                return Some(cmd_msg.to_string());
            }
        }
    }
    // 3. 如果包含 XML 标签控制块，提取内部标签外的文本
    let cleaned_str = if s.starts_with('<') && s.contains('>') {
        let mut in_tag = false;
        let mut clean = String::new();
        for c in s.chars() {
            if c == '<' {
                in_tag = true;
            } else if c == '>' {
                in_tag = false;
            } else if !in_tag {
                clean.push(c);
            }
        }
        clean.trim().to_string()
    } else {
        s.to_string()
    };

    let res = cleaned_str.trim();
    if res.is_empty() {
        None
    } else {
        Some(res.to_string())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub session_id: String,
    pub file_path: String,
    pub display_name: String,
    pub project_path: String,
    pub snippet: String,
    pub match_count: usize,
    pub first_match_message_index: Option<usize>,
    /// 结果所属 CLI("claude"/"codex"/"gemini"/"workbuddy"),由 pipeline 盖戳
    pub cli_id: String,
}

/// Extract a snippet of approximately `context_chars` characters around the first
/// case-insensitive occurrence of `query` in `text`. Adds "..." at the boundaries
/// if the snippet is truncated.
pub fn extract_snippet(text: &str, query: &str, context_chars: usize) -> String {
    let text_lower = text.to_lowercase();
    let query_lower = query.to_lowercase();

    let pos = match text_lower.find(&query_lower) {
        Some(p) => p,
        None => return String::new(),
    };

    let start = if pos > context_chars {
        pos - context_chars
    } else {
        0
    };
    let end = std::cmp::min(text.len(), pos + query.len() + context_chars);

    // Ensure we don't split multi-byte characters
    let safe_start = if start == 0 {
        0
    } else {
        // Find a valid char boundary at or after `start`
        let mut s = start;
        while s < text.len() && !text.is_char_boundary(s) {
            s += 1;
        }
        s
    };
    let safe_end = if end >= text.len() {
        text.len()
    } else {
        let mut e = end;
        while e < text.len() && !text.is_char_boundary(e) {
            e += 1;
        }
        e
    };

    let slice = &text[safe_start..safe_end];
    // Replace newlines with spaces for a cleaner snippet
    let snippet = slice.replace('\n', " ").replace('\r', " ");

    let prefix = if safe_start > 0 { "..." } else { "" };
    let suffix = if safe_end < text.len() { "..." } else { "" };

    format!("{}{}{}", prefix, snippet.trim(), suffix)
}

/// 流式扫描时每个 chunk 元素：一条会话 + 它所属项目的元信息。
/// 前端拿到 chunk 后按 `encoded_dir` 增量合并到 ProjectInfo 列表。
#[derive(Debug, Clone, Serialize)]
pub struct ProjectSessionChunkItem {
    pub session: SessionInfo,
    pub encoded_dir: String,
    pub original_path: String,
}
