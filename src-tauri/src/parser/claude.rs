//! Claude 会话元数据扫描、可搜索文本提取辅助，以及 Claude JSONL 解析器。
//! 依赖方向：parser → session / parser → parser::shared。

use crate::parser::shared::{read_last_timestamp_from_file, read_typed_titles_from_tail};
use crate::session::{
    clean_user_message_text, tool_use_summary, ChatMessage, ContentPart, SessionListMetadata,
    SessionLoadResult, SubagentInfo, TokenUsage, UsageRecord,
};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

pub(crate) fn scan_claude_metadata_only(file_path: &Path) -> Option<SessionListMetadata> {
    let mut file = File::open(file_path).ok()?;
    let file_size = file.seek(SeekFrom::End(0)).ok()?;
    file.seek(SeekFrom::Start(0)).ok()?;

    let reader = BufReader::new(&file);
    let mut has_chat_messages = false;
    let mut raw_first_user_text = None;
    let mut head_custom_title = None;
    let mut head_ai_title = None;
    let mut first_timestamp = None;
    let mut project_path = None;
    let mut git_branch = String::new();

    // 头部窗口：只读前 50 行提取元数据与首条用户消息，
    // 大文件的 I/O 与逐行分配被严格限制在窗口内
    const HEAD_LINES: usize = 50;
    for (idx, line) in reader.lines().take(HEAD_LINES).enumerate() {
        let line = match line {
            Ok(line) => line,
            Err(_) => continue,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // 跳过异常巨大的行（如内嵌超大工具输出的条目），避免 serde_json 卡死
        if trimmed.len() > 256 * 1024 {
            continue;
        }

        let entry: Value = match serde_json::from_str(trimmed) {
            Ok(entry) => entry,
            Err(_) => continue,
        };

        if first_timestamp.is_none() {
            if let Some(ts) = entry.get("timestamp").and_then(|v| v.as_str()) {
                first_timestamp = Some(ts.to_string());
            }
        }

        if project_path.is_none() {
            project_path = extract_claude_project_path(&entry);
        }

        if git_branch.is_empty() && idx < 20 {
            if let Some(branch) = entry.get("gitBranch").and_then(|v| v.as_str()) {
                let branch = branch.trim();
                if !branch.is_empty() && branch != "HEAD" {
                    git_branch = branch.to_string();
                }
            }
        }

        let entry_type = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match entry_type {
            // 头部偶尔出现的标题行作为种子（尾部窗口的更新值会覆盖）
            "custom-title" => {
                head_custom_title =
                    crate::db::title_resolver::extract_typed_title(&entry, "customTitle", "custom_title");
            }
            "ai-title" => {
                head_ai_title =
                    crate::db::title_resolver::extract_typed_title(&entry, "aiTitle", "ai_title");
            }
            _ => {
                if !has_chat_messages {
                    if let Some((role, _)) = extract_claude_role_text(&entry) {
                        has_chat_messages = true;
                        if role == "user" {
                            raw_first_user_text = extract_claude_user_prompt_text(&entry);
                        }
                    }
                }
            }
        }
    }

    // 尾部窗口：custom-title/ai-title 由 Claude Code 追加在文件末尾，
    // 倒序扫描尾部 2MB（首个命中即最新，last-wins），避免全文件扫描
    let (tail_custom_title, tail_ai_title) = read_typed_titles_from_tail(&mut file, file_size);
    let custom_title = tail_custom_title.or(head_custom_title);
    let ai_title = tail_ai_title.or(head_ai_title);

    if !has_chat_messages && custom_title.is_none() && ai_title.is_none() {
        return None;
    }

    let last_timestamp = read_last_timestamp_from_file(&mut file, file_size);

    Some(SessionListMetadata {
        session_id: file_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("")
            .to_string(),
        project_path,
        title: custom_title.or(ai_title),
        first_user_message: raw_first_user_text
            .as_deref()
            .and_then(crate::db::title_resolver::clean_fallback_user_text),
        first_timestamp: first_timestamp.clone(),
        last_timestamp: last_timestamp.or(first_timestamp),
        git_branch,
        file_size,
    })
}

pub(crate) fn extract_claude_project_path(entry: &Value) -> Option<String> {
    entry
        .get("cwd")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
}

pub(crate) fn extract_claude_role_text(entry: &Value) -> Option<(String, String)> {
    let msg_type = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if msg_type != "user" && msg_type != "assistant" {
        return None;
    }

    if entry
        .get("isSidechain")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return None;
    }

    let message = entry.get("message")?;
    let role = message
        .get("role")
        .and_then(|v| v.as_str())
        .unwrap_or(msg_type)
        .to_string();
    let text = extract_claude_text(message.get("content")?)?;
    Some((role, text))
}

/// 仅供标题提取使用：取 user 条目中的真实用户文本。
/// 与 extract_claude_role_text 的区别：数组内容只保留 type=="text" 的项，
/// 按项跳过 tool_result（工具输出不应成为会话标题），混合条目中的真实文本不丢失。
pub(crate) fn extract_claude_user_prompt_text(entry: &Value) -> Option<String> {
    let msg_type = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if msg_type != "user" {
        return None;
    }
    if entry
        .get("isSidechain")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return None;
    }
    let content = entry.get("message")?.get("content")?;
    if let Some(text) = content.as_str() {
        let trimmed = text.trim();
        return if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
    }
    let items = content.as_array()?;
    let mut texts = Vec::new();
    for item in items {
        if item.get("type").and_then(|v| v.as_str()) == Some("text") {
            collect_json_string_value(item.get("text"), &mut texts);
        }
    }
    if texts.is_empty() {
        None
    } else {
        Some(texts.join("\n"))
    }
}

pub(crate) fn extract_claude_text(content: &Value) -> Option<String> {
    if let Some(text) = content.as_str() {
        let trimmed = text.trim();
        return if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
    }

    let items = content.as_array()?;
    let mut texts = Vec::new();
    for item in items {
        if item.get("type").and_then(|v| v.as_str()) == Some("text") {
            collect_json_string_value(item.get("text"), &mut texts);
        }
    }

    if texts.is_empty() {
        None
    } else {
        Some(texts.join("\n"))
    }
}

pub(crate) fn collect_json_string_value(value: Option<&Value>, texts: &mut Vec<String>) {
    let Some(text) = value.and_then(|value| value.as_str()) else {
        return;
    };
    let trimmed = text.trim();
    if !trimmed.is_empty() {
        texts.push(trimmed.to_string());
    }
}

#[allow(dead_code)]
pub(crate) fn collect_json_string_values(value: Option<&Value>, texts: &mut Vec<String>) {
    collect_json_string_values_inner(value, texts, 64);
}

#[allow(dead_code)]
pub(crate) fn collect_json_string_values_inner(
    value: Option<&Value>,
    texts: &mut Vec<String>,
    max_depth: usize,
) {
    if max_depth == 0 {
        return;
    }
    let Some(value) = value else {
        return;
    };

    match value {
        Value::String(_) => collect_json_string_value(Some(value), texts),
        Value::Array(items) => {
            for item in items {
                collect_json_string_values_inner(Some(item), texts, max_depth - 1);
            }
        }
        Value::Object(map) => {
            for value in map.values() {
                collect_json_string_values_inner(Some(value), texts, max_depth - 1);
            }
        }
        _ => {}
    }
}

/// Parse a single JSONL line into a ChatMessage if it's a user or assistant message.
pub(crate) fn parse_jsonl_line(
    line: &str,
    skip_sidechain: bool,
    subagent_map: Option<&mut HashMap<String, SubagentInfo>>,
    session_file_path: Option<&Path>,
) -> Option<ChatMessage> {
    let entry: serde_json::Value = serde_json::from_str(line).ok()?;

    let msg_type = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if msg_type != "user" && msg_type != "assistant" {
        return None;
    }

    // Skip sidechain messages unless skip_sidechain is true
    if !skip_sidechain && entry
        .get("isSidechain")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return None;
    }

    let timestamp = entry
        .get("timestamp")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let message = entry.get("message")?;

    let role = message
        .get("role")
        .and_then(|v| v.as_str())
        .unwrap_or(msg_type)
        .to_string();

    let model = message
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let content_parts = parse_content(message.get("content"));

    if let (Some(map), Some(file_path)) = (subagent_map, session_file_path) {
        if role == "user" {
            if let Some(agent_id) = entry.get("toolUseResult").and_then(|v| v.get("agentId")).and_then(|v| v.as_str()) {
                if let Some(content) = message.get("content").and_then(|c| c.as_array()) {
                    if let Some(first) = content.first() {
                        if let Some(tool_use_id) = first.get("tool_use_id").and_then(|v| v.as_str()) {
                            if let (Some(parent), Some(stem)) = (file_path.parent(), file_path.file_stem()) {
                                let subagent_file = parent.join(stem).join("subagents").join(format!("agent-{}.jsonl", agent_id));
                                if subagent_file.exists() {
                                    map.insert(tool_use_id.to_string(), SubagentInfo {
                                        file_path: subagent_file.to_string_lossy().to_string(),
                                        label: format!("Subagent {}", &agent_id[..6]),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if content_parts.is_empty() {
        return None;
    }

    let token_usage = message.get("usage").map(|u| TokenUsage {
        input_tokens: u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
        output_tokens: u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
        cache_creation_input_tokens: u.get("cache_creation_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
        cache_read_input_tokens: u.get("cache_read_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
    });

    Some(ChatMessage {
        role,
        timestamp,
        model,
        token_usage,
        content_parts,
        is_meta: entry.get("isMeta").and_then(|v| v.as_bool()).unwrap_or(false),
        uuid: entry
            .get("uuid")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    })
}

pub(crate) fn has_chat_type_marker(text: &str) -> bool {
    text.contains(r#""type":"user""#)
        || text.contains(r#""type": "user""#)
        || text.contains(r#""type":"assistant""#)
        || text.contains(r#""type": "assistant""#)
}

pub(crate) fn chat_line_candidate(line: &str) -> bool {
    // Ensure prefix + suffix windows overlap by using at least half the line length.
    let window = std::cmp::max(1024, line.len() / 2 + 128);
    let mut prefix_end = std::cmp::min(window, line.len());
    while prefix_end > 0 && !line.is_char_boundary(prefix_end) {
        prefix_end -= 1;
    }
    if prefix_end > 0 && has_chat_type_marker(&line[..prefix_end]) {
        return true;
    }
    if line.len() <= window {
        return false;
    }
    let mut suffix_start = line.len().saturating_sub(window);
    while suffix_start < line.len() && !line.is_char_boundary(suffix_start) {
        suffix_start += 1;
    }
    if suffix_start < line.len() && suffix_start < prefix_end + 1024 {
        has_chat_type_marker(&line[suffix_start..])
    } else {
        false
    }
}

pub(crate) fn parse_session_file(file_path: &str) -> Result<Vec<ChatMessage>, String> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(file_path).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(file);
    let mut messages = Vec::new();
    let mut buf = String::new();

    loop {
        buf.clear();
        match reader.read_line(&mut buf) {
            Ok(0) => break,
            Err(_) => continue,
            Ok(_) => {}
        }
        let line = buf.trim();
        if line.is_empty() {
            continue;
        }
        if !chat_line_candidate(line) {
            continue;
        }
        if let Some(msg) = parse_jsonl_line(line, false, None, None) {
            messages.push(msg);
        }
    }

    Ok(messages)
}

pub(crate) fn parse_session_from_string(content: &str) -> Result<Vec<ChatMessage>, String> {
    let mut messages = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(msg) = parse_jsonl_line(line, false, None, None) {
            messages.push(msg);
        }
    }
    Ok(messages)
}

/// Parse a session JSONL file and return messages + file offset for incremental loading.
pub(crate) fn parse_session_file_with_offset(file_path: &str, skip_sidechain: bool) -> Result<SessionLoadResult, String> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(file_path).map_err(|e| e.to_string())?;
    let file_size = file.metadata().map_err(|e| e.to_string())?.len();
    let mut reader = BufReader::new(file);
    let mut messages = Vec::new();
    let mut subagent_map = HashMap::new();
    let session_path = std::path::Path::new(file_path);
    let mut buf = String::new();

    loop {
        buf.clear();
        match reader.read_line(&mut buf) {
            Ok(0) => break,
            Err(_) => continue,
            Ok(_) => {}
        }
        let line = buf.trim();
        if line.is_empty() {
            continue;
        }
        if !chat_line_candidate(line) {
            continue;
        }
        if let Some(msg) = parse_jsonl_line(line, skip_sidechain, Some(&mut subagent_map), Some(session_path)) {
            messages.push(msg);
        }
    }

    Ok(SessionLoadResult {
        messages,
        offset: file_size,
        subagent_map,
    })
}

/// Parse incremental content from a session file starting at the given byte offset.
pub(crate) fn parse_session_incremental(file_path: &str, offset: u64, skip_sidechain: bool) -> Result<SessionLoadResult, String> {
    use std::io::{BufRead, BufReader, Seek, SeekFrom};
    let mut file = std::fs::File::open(file_path).map_err(|e| e.to_string())?;
    let file_size = file.metadata().map_err(|e| e.to_string())?.len();

    if file_size <= offset {
        return Ok(SessionLoadResult {
            messages: vec![],
            offset,
            subagent_map: HashMap::new(),
        });
    }

    file.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(file);
    let mut messages = Vec::new();
    let mut subagent_map = HashMap::new();
    let session_path = std::path::Path::new(file_path);
    let mut buf = String::new();

    loop {
        buf.clear();
        match reader.read_line(&mut buf) {
            Ok(0) => break,
            Err(_) => continue,
            Ok(_) => {}
        }
        let line = buf.trim();
        if line.is_empty() {
            continue;
        }
        if !chat_line_candidate(line) {
            continue;
        }
        if let Some(msg) = parse_jsonl_line(line, skip_sidechain, Some(&mut subagent_map), Some(session_path)) {
            messages.push(msg);
        }
    }

    Ok(SessionLoadResult {
        messages,
        offset: file_size,
        subagent_map,
    })
}

/// 流式解析回调签名：每收到一批消息就调用一次（batch 长度由调用者用 `batch_size` 控制）。
/// 返回 `false` 表示调用方希望中止（目前未使用，留作未来取消支持）。
pub(crate) fn parse_session_file_streaming<F>(
    file_path: &str,
    skip_sidechain: bool,
    batch_size: usize,
    mut on_batch: F,
) -> Result<(u64, HashMap<String, SubagentInfo>), String>
where
    F: FnMut(Vec<ChatMessage>) -> bool,
{
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(file_path).map_err(|e| e.to_string())?;
    let file_size = file.metadata().map_err(|e| e.to_string())?.len();
    let mut reader = BufReader::new(file);
    let mut subagent_map = HashMap::new();
    let session_path = std::path::Path::new(file_path);
    let mut buf = String::new();
    let mut batch: Vec<ChatMessage> = Vec::with_capacity(batch_size);

    loop {
        buf.clear();
        match reader.read_line(&mut buf) {
            Ok(0) => break,
            Err(_) => continue,
            Ok(_) => {}
        }
        let line = buf.trim();
        if line.is_empty() {
            continue;
        }
        if !chat_line_candidate(line)
        {
            continue;
        }
        if let Some(msg) = parse_jsonl_line(line, skip_sidechain, Some(&mut subagent_map), Some(session_path)) {
            batch.push(msg);
            if batch.len() >= batch_size {
                let take = std::mem::replace(&mut batch, Vec::with_capacity(batch_size));
                if !on_batch(take) {
                    return Ok((file_size, subagent_map));
                }
            }
        }
    }

    if !batch.is_empty() {
        on_batch(batch);
    }

    Ok((file_size, subagent_map))
}

/// Detect skill content or system caveats and return a collapsible ToolResult.
pub(crate) fn try_parse_collapsible(text: &str) -> Option<ContentPart> {
    // Skill content: "Base directory for this skill: /path/to/skill-name\n..."
    if text.starts_with("Base directory for this skill:") {
        let summary = text
            .lines()
            .find(|l| l.starts_with("# "))
            .map(|l| l.trim_start_matches("# ").to_string())
            .unwrap_or_else(|| {
                text.lines()
                    .next()
                    .unwrap_or("")
                    .rsplit('/')
                    .next()
                    .unwrap_or("Skill")
                    .to_string()
            });
        return Some(ContentPart::ToolResult {
            summary: format!("[Skill: {}]", summary),
            content: text.to_string(),
            is_error: false,
        });
    }

    // Local command caveat: "<local-command-caveat>..."
    if text.starts_with("<local-command-caveat>") {
        return Some(ContentPart::ToolResult {
            summary: "[System caveat]".to_string(),
            content: text.to_string(),
            is_error: false,
        });
    }

    // CLI 粘贴图片时生成的 meta 引用行: "[Image: source: /path/to/img.png]"
    if text.starts_with("[Image: source: ") && text.trim_end().ends_with(']') {
        let trimmed = text.trim_end();
        let path = trimmed["[Image: source: ".len()..trimmed.len() - 1].trim();
        if !path.is_empty() {
            return Some(ContentPart::ImageRef {
                path: path.to_string(),
            });
        }
        return Some(ContentPart::ImageMeta);
    }

    None
}

pub(crate) fn parse_content(content: Option<&serde_json::Value>) -> Vec<ContentPart> {
    let content = match content {
        Some(c) => c,
        None => return Vec::new(),
    };

    if let Some(text) = content.as_str() {
        if let Some(part) = try_parse_collapsible(text) {
            return vec![part];
        }
        return vec![ContentPart::Text {
            text: text.to_string(),
        }];
    }

    if let Some(arr) = content.as_array() {
        let mut parts = Vec::new();
        for item in arr {
            let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match item_type {
                "text" => {
                    if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                        if !text.is_empty() {
                            if let Some(part) = try_parse_collapsible(text) {
                                parts.push(part);
                            } else {
                                parts.push(ContentPart::Text {
                                    text: text.to_string(),
                                });
                            }
                        }
                    }
                }
                "tool_use" => {
                    let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("Tool");
                    let tool_use_id = item.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());
                    let input_val = item
                        .get("input")
                        .cloned()
                        .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
                    let summary = tool_use_summary(name, &input_val);
                    let input_str = serde_json::to_string_pretty(&input_val)
                        .unwrap_or_default();
                    parts.push(ContentPart::ToolUse {
                        summary,
                        tool_name: name.to_string(),
                        input: input_str,
                        tool_use_id,
                    });
                }
                "thinking" => {
                    if let Some(thinking) = item.get("thinking").and_then(|v| v.as_str()) {
                        if !thinking.is_empty() {
                            parts.push(ContentPart::Thinking {
                                thinking: thinking.to_string(),
                            });
                        }
                    }
                }
                "image" => {
                    if let Some(source) = item.get("source") {
                        let source_type = source.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        if source_type == "base64" {
                            let media_type = source.get("media_type").and_then(|v| v.as_str()).unwrap_or("image/png");
                            if let Some(data) = source.get("data").and_then(|v| v.as_str()) {
                                parts.push(ContentPart::Image {
                                    media_type: media_type.to_string(),
                                    data: data.to_string(),
                                });
                            }
                        }
                    }
                }
                "tool_result" => {
                    let is_error = item.get("is_error").and_then(|v| v.as_bool()).unwrap_or(false);
                    let content_val = item.get("content");

                    let mut content_str = String::new();
                    if let Some(arr) = content_val.and_then(|v| v.as_array()) {
                        for sub in arr {
                            if let Some(text) = sub.get("text").and_then(|v| v.as_str()) {
                                content_str.push_str(text);
                                content_str.push('\n');
                            }
                        }
                    } else if let Some(text) = content_val.and_then(|v| v.as_str()) {
                        content_str = text.to_string();
                    }

                    if !content_str.is_empty() {
                        const MAX_LEN: usize = 50_000;
                        if content_str.len() > MAX_LEN {
                            let mut cut = MAX_LEN;
                            while cut > 0 && !content_str.is_char_boundary(cut) {
                                cut -= 1;
                            }
                            content_str = format!(
                                "{}...\n\n[内容超长，已被系统截断以避免界面卡死]",
                                &content_str[..cut]
                            );
                        }
                        let summary = if is_error { "[Tool Error]" } else { "[Tool Result]" }.to_string();
                        parts.push(ContentPart::ToolResult {
                            summary,
                            content: content_str,
                            is_error,
                        });
                    }
                }
                _ => {}
            }
        }
        return parts;
    }

    Vec::new()
}

/// Check if a JSONL file contains at least one user or assistant message.
pub(crate) fn has_chat_messages(file_path: &str) -> bool {
    use std::io::{BufRead, BufReader};
    let file = match std::fs::File::open(file_path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut reader = BufReader::new(file);
    let mut buf = String::new();
    loop {
        buf.clear();
        match reader.read_line(&mut buf) {
            Ok(0) => break,
            Err(_) => continue,
            Ok(_) => {}
        }
        let line = buf.trim();
        if line.is_empty() {
            continue;
        }
        if !chat_line_candidate(line) {
            continue;
        }
        let entry: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let msg_type = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if msg_type == "user" || msg_type == "assistant" {
            return true;
        }
    }
    false
}

/// Read the first user message from a JSONL file (first 30 chars) for display name fallback.
pub(crate) fn read_first_user_message(file_path: &str) -> Option<String> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(file_path).ok()?;
    let reader = BufReader::new(file);
    // Only check the first 50 lines to avoid reading huge files
    for line in reader.lines().take(50) {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        // 跳过异常巨大的行（如内嵌数百 MB 工具输出的 sidechain 条目），避免 serde_json 卡死
        if line.len() > 256 * 1024 {
            continue;
        }
        let entry: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let msg_type = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if msg_type != "user" {
            continue;
        }
        let message = entry.get("message")?;
        let content = message.get("content")?;
        let text = if let Some(s) = content.as_str() {
            s.to_string()
        } else if let Some(arr) = content.as_array() {
            arr.iter()
                .find_map(|item| {
                    if item.get("type").and_then(|v| v.as_str()) == Some("text") {
                        item.get("text").and_then(|v| v.as_str()).map(|s| s.to_string())
                    } else {
                        None
                    }
                })
                .unwrap_or_default()
        } else {
            continue;
        };
        let cleaned = match clean_user_message_text(&text) {
            Some(c) => c,
            None => continue,
        };
        let truncated: String = cleaned.chars().take(30).collect();
        let display = if cleaned.chars().count() > 30 {
            format!("{}...", truncated)
        } else {
            truncated
        };
        return Some(display);
    }
    None
}

/// Read the first timestamp from a JSONL file for display.
pub(crate) fn read_first_timestamp(file_path: &str) -> Option<String> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(file_path).ok()?;
    let reader = BufReader::new(file);
    for line in reader.lines().take(10) {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.len() > 256 * 1024 {
            continue;
        }
        let entry: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(ts) = entry.get("timestamp").and_then(|v| v.as_str()) {
            return Some(ts.to_string());
        }
    }
    None
}

/// Read the last timestamp from a JSONL file for sorting (most recent activity).
pub(crate) fn read_last_timestamp(file_path: &str) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(file_path).ok()?;
    let file_size = file.seek(SeekFrom::End(0)).ok()?;
    if file_size == 0 {
        return None;
    }

    let chunk_size: u64 = 65536;
    let max_search: u64 = 2 * 1024 * 1024; // 搜索至多 2MB
    let mut offset = file_size;
    let mut overlap = String::new();

    while offset > 0 && file_size.saturating_sub(offset) < max_search {
        let read_size = std::cmp::min(offset, chunk_size);
        offset -= read_size;

        if file.seek(SeekFrom::Start(offset)).is_err() {
            break;
        }

        let mut buf = vec![0u8; read_size as usize];
        let mut total_read = 0;
        while total_read < read_size as usize {
            match file.read(&mut buf[total_read..]) {
                Ok(0) => break,
                Ok(n) => total_read += n,
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return None,
            }
        }

        let chunk_text = String::from_utf8_lossy(&buf[..total_read]);
        let combined = format!("{}{}", chunk_text, overlap);

        let mut lines: Vec<&str> = combined.lines().collect();
        // 倒序读取时，块最前面的第一行极大概率是被截断的，需要保存给下一个块补全
        if offset > 0 && !combined.starts_with('\n') && !lines.is_empty() {
            overlap = lines.remove(0).to_string();
        } else {
            overlap.clear();
        }

        for line in lines.into_iter().rev() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // 快速前缀过滤：跳过不包含 timestamp 的元数据行（如 file-history-snapshot）
            if !trimmed.contains("\"timestamp\"") {
                continue;
            }

            let entry: serde_json::Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if let Some(ts) = entry.get("timestamp").and_then(|v| v.as_str()) {
                return Some(ts.to_string());
            }
        }
    }
    None
}

/// Read the git branch from a JSONL file (from early entries that have gitBranch field).
pub(crate) fn read_git_branch(file_path: &str) -> String {
    use std::io::{BufRead, BufReader};
    let file = match std::fs::File::open(file_path) {
        Ok(f) => f,
        Err(_) => return String::new(),
    };
    let reader = BufReader::new(file);
    for line in reader.lines().take(20) {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.len() > 256 * 1024 {
            continue;
        }
        let entry: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(branch) = entry.get("gitBranch").and_then(|v| v.as_str()) {
            let b = branch.trim();
            if !b.is_empty() && b != "HEAD" {
                return b.to_string();
            }
        }
    }
    String::new()
}

/// Extract usage records from a JSONL session file.
/// Reads assistant entries for token usage and system/turn_duration for duration.
pub(crate) fn extract_usage_records(file_path: &str, project: &str) -> Vec<UsageRecord> {
    use std::io::{BufRead, BufReader};
    let file = match std::fs::File::open(file_path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let reader = BufReader::new(file);
    let mut records: Vec<UsageRecord> = Vec::new();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        let entry: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let msg_type = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");

        // Extract turn duration from system entries
        if msg_type == "system" {
            let subtype = entry.get("subtype").and_then(|v| v.as_str()).unwrap_or("");
            if subtype == "turn_duration" {
                if let Some(dur) = entry.get("durationMs").and_then(|v| v.as_u64()) {
                    // Attach duration to the last record (the assistant turn it follows)
                    if let Some(last) = records.last_mut() {
                        last.duration_ms = Some(dur);
                    }
                }
            }
            continue;
        }

        if msg_type != "assistant" {
            continue;
        }

        let message = match entry.get("message") {
            Some(m) => m,
            None => continue,
        };

        let usage = match message.get("usage") {
            Some(u) => u,
            None => continue,
        };

        let input_tokens = usage.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        let output_tokens = usage.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        let cache_creation_tokens = usage.get("cache_creation_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        let cache_read_tokens = usage.get("cache_read_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);

        // Skip entries with zero usage
        if input_tokens == 0 && output_tokens == 0 && cache_creation_tokens == 0 && cache_read_tokens == 0 {
            continue;
        }

        let model = message
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let timestamp = entry
            .get("timestamp")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // Convert ISO timestamp to local date YYYY-MM-DD
        let date = timestamp_to_local_date(timestamp);

        records.push(UsageRecord {
            date,
            model,
            input_tokens,
            output_tokens,
            cache_creation_tokens,
            cache_read_tokens,
            duration_ms: None,
            project: project.to_string(),
        });
    }

    records
}

/// Convert ISO 8601 timestamp to local date string YYYY-MM-DD.
pub(crate) fn timestamp_to_local_date(ts: &str) -> String {
    use chrono::{DateTime, Local, Utc};
    match ts.parse::<DateTime<Utc>>() {
        Ok(utc) => {
            let local: DateTime<Local> = utc.with_timezone(&Local);
            local.format("%Y-%m-%d").to_string()
        }
        Err(_) => "unknown".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn claude_search_text_excludes_tool_use_only_entries() {
        // 优化：tool_use 条目会产生巨型 N-gram token，已从搜索索引中剔除。
        // 纯 tool_use（无 text 项）的 assistant 消息不再被索引。
        let entry = json!({
            "type": "assistant",
            "message": {
                "role": "assistant",
                "content": [
                    {
                        "type": "tool_use",
                        "name": "Write",
                        "input": {
                            "file_path": "plan.md",
                            "content": "Step 1: 扩展 OpenTab 接口，增加滚动状态字段"
                        }
                    }
                ]
            }
        });
        // 纯 tool_use 无 text 项 → None（不进入索引，减少 N-gram 爆炸）
        assert!(extract_claude_role_text(&entry).is_none());

        // 但 assistant 回答中同时含 text 的情况，text 部分仍会被索引
        let entry_with_text = json!({
            "type": "assistant",
            "message": {
                "role": "assistant",
                "content": [
                    {"type": "text", "text": "好的，我来帮你扩展接口"},
                    {"type": "tool_use", "name": "Write", "input": {"content": "噪声数据"}}
                ]
            }
        });
        let (_role, text) = extract_claude_role_text(&entry_with_text).expect("text item should be indexed");
        assert!(text.contains("好的，我来帮你扩展接口"));
        assert!(!text.contains("噪声数据"));
    }

    #[test]
    fn claude_tool_result_entry_excluded_from_search_index() {
        // 优化：tool_result 输出（命令行日志、文件内容等）可达数 MB，
        // 经 Ngram(2..4) 分词后会产生数百万 token，严重拖慢全量索引构建速度。
        // 现在只索引 type=="text" 的用户/助手正文，tool_result 已从索引中剔除。
        let entry = json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": [
                    {"type": "tool_result", "content": "error: linker command failed"}
                ]
            }
        });
        // 纯 tool_result 消息 → None，不占用索引空间
        assert!(extract_claude_role_text(&entry).is_none());

        // 混合消息（tool_result + text）→ 只有 text 部分被索引
        let mixed = json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": [
                    {"type": "tool_result", "content": "error: linker command failed"},
                    {"type": "text", "text": "请帮我修复上面的链接错误"}
                ]
            }
        });
        let (_role, text) = extract_claude_role_text(&mixed).expect("text item should be indexed");
        assert!(text.contains("请帮我修复上面的链接错误"));
        assert!(!text.contains("linker command failed"));
    }

    #[test]
    fn claude_user_prompt_text_skips_tool_result_keeps_text_items() {
        // 混合条目：[tool_result, text] —— 标题路径按项过滤，真实用户文本不丢失
        let entry = json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": [
                    {"type": "tool_result", "content": "tool output noise"},
                    {"type": "text", "text": "真正的问题"}
                ]
            }
        });

        let text = extract_claude_user_prompt_text(&entry).expect("mixed entry should yield text item");
        assert_eq!(text, "真正的问题");

        // 纯 tool_result 条目对标题不可见
        let tool_only = json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": [{"type": "tool_result", "content": "noise"}]
            }
        });
        assert_eq!(extract_claude_user_prompt_text(&tool_only), None);
    }

    #[test]
    fn scan_claude_metadata_finds_late_title_via_tail_window() {
        // 回归：标题行追加在文件末尾（超过 64KB 多个 chunk），尾部窗口必须命中，且只读头+尾
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("sessiondock-tail-title-{unique}.jsonl"));

        let mut content = String::from(
            "{\"type\":\"user\",\"timestamp\":\"2026-07-27T10:00:00Z\",\"cwd\":\"/p\",\"message\":{\"role\":\"user\",\"content\":\"帮我看下这个问题\"}}\n",
        );
        // 填充超过 64KB 的中间内容，确保标题行落在尾部第二个 chunk 之前
        let filler_line = format!(
            "{{\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"content\":\"{}\"}}}}\n",
            "x".repeat(1024)
        );
        while content.len() < 200 * 1024 {
            content.push_str(&filler_line);
        }
        content.push_str("{\"type\":\"ai-title\",\"aiTitle\":\"尾部 AI 标题\"}\n");
        content.push_str("{\"type\":\"custom-title\",\"customTitle\":\"尾部自定义标题\"}\n");
        fs::write(&path, &content).unwrap();

        let metadata = scan_claude_metadata_only(&path).expect("should scan metadata");
        let _ = fs::remove_file(&path);

        // last-wins：文件末尾的 custom-title 覆盖 ai-title
        assert_eq!(metadata.title.as_deref(), Some("尾部自定义标题"));
        assert_eq!(metadata.first_user_message.as_deref(), Some("帮我看下这个问题"));
    }

    #[test]
    fn scan_claude_metadata_reads_only_head_and_tail_of_large_file() {
        // 性能回归：头部窗口外的畸形行（非 JSON、超大行）不得导致扫描失败或全量解析
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("sessiondock-head-scan-{unique}.jsonl"));

        let mut content = String::from(
            "{\"type\":\"user\",\"timestamp\":\"2026-07-27T10:00:00Z\",\"message\":{\"role\":\"user\",\"content\":\"first\"}}\n",
        );
        while content.len() < 100 * 1024 {
            content.push_str(&format!("not json at all {}\n", "y".repeat(512)));
        }
        fs::write(&path, &content).unwrap();

        let metadata = scan_claude_metadata_only(&path).expect("malformed middle lines must not fail scan");
        let _ = fs::remove_file(&path);

        assert_eq!(metadata.title, None);
        assert_eq!(metadata.first_user_message.as_deref(), Some("first"));
    }

    #[test]
    fn chat_line_candidate_finds_type_in_prefix() {
        let line = r#"{"parentUuid":"x","isSidechain":false,"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hi"}]}}"#;
        assert!(chat_line_candidate(line));
    }

    #[test]
    fn chat_line_candidate_finds_type_in_suffix_long_line() {
        let padding = "x".repeat(4_000);
        let line = format!(
            r#"{{"parentUuid":"x","isSidechain":false,"message":{{"content":[{{"type":"tool_use","name":"Agent","input":{{"prompt":"{}"}}}}],"role":"assistant"}},"type":"assistant"}}"#,
            padding
        );
        let type_pos = line.find(r#""type":"assistant""#).unwrap();
        assert!(type_pos > 2048, "type must be beyond mid-window gap zone");
        assert!(chat_line_candidate(&line));
    }

    #[test]
    fn chat_line_candidate_finds_type_in_gap_zone() {
        // Line ~2500 bytes, type at ~1200 — captured by new overlapping windows
        let padding = "x".repeat(1_200);
        let line = format!(
            r#"{{"parentUuid":"x","isSidechain":false,"message":{{"content":[{{"type":"text","text":"{}"}}],"role":"user"}},"type":"user"}}"#,
            padding
        );
        let type_pos = line.find(r#""type":"user""#).unwrap();
        assert!(type_pos > 1024, "type should be beyond old 1024 prefix window");
        // Old algorithm missed this (type in gap between prefix and suffix),
        // new overlapping windows find it.
        assert!(chat_line_candidate(&line));
    }

    #[test]
    fn chat_line_candidate_rejects_non_chat_line() {
        let line = r#"{"type":"system","message":"internal event"}"#;
        assert!(!chat_line_candidate(line));
    }

    #[test]
    fn parse_jsonl_line_marks_is_meta_and_skips_image_reference() {
        let line = r#"{"parentUuid":"x","isSidechain":false,"isMeta":true,"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Image: source: /Users/x/.claude/image-cache/abc/1.png]"}]}}"#;
        let msg = parse_jsonl_line(line, false, None, None).expect("should parse");
        assert!(msg.is_meta);
        assert_eq!(msg.content_parts.len(), 1);
        assert!(matches!(msg.content_parts[0], ContentPart::ImageRef { .. } | ContentPart::ImageMeta));
    }

    #[test]
    fn parse_jsonl_line_is_meta_defaults_to_false() {
        let line = r#"{"parentUuid":"x","isSidechain":false,"type":"user","message":{"role":"user","content":[{"type":"text","text":"hello"}]}}"#;
        let msg = parse_jsonl_line(line, false, None, None).expect("should parse");
        assert!(!msg.is_meta);
    }
}
