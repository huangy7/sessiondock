use serde::Serialize;

/// 一轮的 token 用量（来自 stream-json result 行的 usage 字段；旧版 CLI 可能缺失）
#[derive(Debug, Clone, Copy, Serialize, Default, PartialEq)]
pub struct TurnUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum AgentStreamEvent {
    Init { session_id: String, model: Option<String> },
    AssistantText { text: String },
    ToolUse { name: String, summary: String },
    Result {
        ok: bool,
        duration_ms: Option<u64>,
        cost_usd: Option<f64>,
        usage: Option<TurnUsage>,
        error: Option<String>,
    },
}

/// 解析单行 stream-json（claude -p --output-format stream-json 的输出）。
/// 非 JSON 行或不关心的类型返回 None。
pub fn parse_stream_line(line: &str) -> Option<AgentStreamEvent> {
    let parsed: serde_json::Value = serde_json::from_str(line).ok()?;
    let event_type = parsed.get("type")?.as_str()?;

    match event_type {
        "system" => {
            if parsed.get("subtype")?.as_str()? != "init" {
                return None;
            }
            let session_id = parsed.get("session_id")?.as_str()?.to_string();
            let model = parsed
                .get("model")
                .and_then(|m| m.as_str())
                .map(String::from);
            Some(AgentStreamEvent::Init { session_id, model })
        }
        "assistant" => {
            let content = parsed.get("message")?.get("content")?.as_array()?;
            for part in content {
                match part.get("type").and_then(|t| t.as_str()) {
                    Some("text") => {
                        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                            if !text.trim().is_empty() {
                                return Some(AgentStreamEvent::AssistantText {
                                    text: text.to_string(),
                                });
                            }
                        }
                    }
                    Some("tool_use") => {
                        let Some(name) = part.get("name").and_then(|n| n.as_str()) else {
                            continue;
                        };
                        let name = name.to_string();
                        let summary = part
                            .get("input")
                            .and_then(|i| i.get("command"))
                            .and_then(|c| c.as_str())
                            .map(|c| c.chars().take(120).collect())
                            .unwrap_or_default();
                        return Some(AgentStreamEvent::ToolUse { name, summary });
                    }
                    _ => continue,
                }
            }
            None
        }
        "result" => {
            let ok = parsed.get("subtype").and_then(|s| s.as_str()) == Some("success");
            let usage = parsed.get("usage").map(|u| TurnUsage {
                input_tokens: u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                output_tokens: u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                cache_read_input_tokens: u.get("cache_read_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                cache_creation_input_tokens: u.get("cache_creation_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
            });
            Some(AgentStreamEvent::Result {
                ok,
                duration_ms: parsed.get("duration_ms").and_then(|d| d.as_u64()),
                cost_usd: parsed.get("total_cost_usd").and_then(|c| c.as_f64()),
                usage,
                error: if ok {
                    None
                } else {
                    parsed
                        .get("result")
                        .and_then(|r| r.as_str())
                        .map(String::from)
                },
            })
        }
        _ => None,
    }
}

use crate::error::{AppError, AppResult};
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, ExitStatus, Stdio};

/// 截断字符串尾部，保留最多 max 字节（UTF-8 边界安全）。
fn truncate_tail(s: &mut String, max: usize) {
    if s.len() > max {
        let keep_from = (s.len() - max..=s.len())
            .find(|&i| s.is_char_boundary(i))
            .unwrap_or(s.len());
        *s = s.split_off(keep_from);
    }
}

/// pump_stream 的返回结果。
pub struct PumpOutcome {
    pub stderr_tail: String,
    pub status: ExitStatus,
}

/// proxy 二进制名（unix 无扩展名，Windows 带 .exe）。allow 规则与提示词用裸名，
/// 避免 Windows 路径反斜杠导致 Bash 规则匹配失败（claude-code 权限 glob 把 `\` 当转义符）。
/// 按 `/` 和 `\` 都切分，保证跨平台取到最后一个路径段。
fn proxy_binary_name(proxy_abs: &str) -> String {
    proxy_abs
        .rsplit(['/', '\\'])
        .next()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| proxy_abs.to_string())
}

pub fn build_system_prompt(proxy_abs: &str) -> String {
    let name = proxy_binary_name(proxy_abs);
    [
        "你是 SessionDock 助手，帮用户查询和理解他们的 AI 编程会话历史。",
        &format!("你唯一的工具是 sessiondock-proxy（已在白名单内）：先跑 `{} --json-help` 了解全部命令。", name),
        "常用流程：list 按时间/项目筛选会话 → grep 定位内容 → show 读正文（用 --from/--to 取段，别一次读整个长会话）。",
        "show 的 id 只给前 8 位前缀即可（不要复述完整 UUID，容易出错）；只要正文时加 --format text。",
        "grep 结果带 coverage 字段，partial 时要说明只搜了部分会话。",
        "show 会自动按需提取会话内容；返回空 text 表示该会话无可提取内容（或源文件已清理），跳过并注明即可，不要反复重试。",
        "不要给 proxy 命令加管道或重定向（如 2>/dev/null、| head、| python3）：想读部分正文用 show 自带的 --from/--to/--max-chars 参数。",
        "【必须标注来源】回答中只要提到某次会话的内容、结论或细节，句末必须跟来源标记，格式 `⟦N:会话id前8位⟧`（双方括号，N 从 1 递增，同来源复用同一 N）。",
        "id 前 8 位必须照抄你本轮 show/grep 输出里看到的 id 字段前 8 个字符，一个字符都不能改，更不要编造。",
        "示例：你 show 了 id 为 90259911-... 的会话，就写『那轮会话把缓存层拆了 ⟦1:90259911⟧』；又 show 了 3f8a9c2d-...，写『另一处优化了启动 ⟦2:3f8a9c2d⟧』。",
        "克制标注：同一段落里连续几句都来自同一来源时，只在该来源内容最后一次出现的句末标一次，不要每句都重复标。",
        "【动态追问建议】在每次回答的最末尾，根据当前对话的具体上下文和回答内容，另起一行附带 2~3 个紧密相关的后续追问或操作建议，格式严格为 `«FOLLOWUPS: 建议一 | 建议二 | 建议三»`（每个建议 10 字左右，简短清晰，由你根据上下文自适应生成；如果无需后续追问则不要输出此标签）。",
        "用中文回答，简洁直接；数字说明口径；查不到就说查不到，不编造。",
    ].join("\n")
}

pub fn build_spawn_args(
    proxy_abs: &str,
    prompt: &str,
    settings_file: &str,
    system_prompt: &str,
    resume_session_id: Option<&str>,
    model: Option<&str>,
) -> Vec<String> {
    let mut args = vec![
        "-p".to_string(),
        prompt.to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--verbose".to_string(),
        "--allowedTools".to_string(),
        format!("Bash({}:*)", proxy_binary_name(proxy_abs)),
        "--settings".to_string(),
        settings_file.to_string(),
        "--append-system-prompt".to_string(),
        system_prompt.to_string(),
    ];
    if let Some(id) = resume_session_id {
        args.push("--resume".to_string());
        args.push(id.to_string());
    }
    if let Some(m) = model.filter(|m| !m.is_empty()) {
        args.push("--model".to_string());
        args.push(m.to_string());
    }
    args
}

pub struct RunningTurn {
    pub child: Child,
}

/// spawn 一轮 agent。主进程 PATH 已在启动时注入完整 shell 环境（pty_manager::
/// inject_shell_env_into_process），Command 直接继承，无需额外处理。
pub fn spawn_turn(
    claude_bin: &str,
    proxy_abs: &str,
    prompt: &str,
    settings_file: &str,
    resume_session_id: Option<&str>,
    model: Option<&str>,
    workspace: &std::path::Path,
) -> AppResult<RunningTurn> {
    let system_prompt = build_system_prompt(proxy_abs);
    let args = build_spawn_args(proxy_abs, prompt, settings_file, &system_prompt, resume_session_id, model);
    let mut cmd = Command::new(claude_bin);
    cmd.args(&args)
        .current_dir(workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // claude 是控制台子程序：Windows 上必须加 CREATE_NO_WINDOW，否则 GUI 主进程
    // 拉起 claude 时会额外弹一个控制台黑框（与 cli.rs / proxy.rs 同款处理）。
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    // allow 规则是裸名 Bash(sessiondock-proxy:*)，agent 按名调用才能命中：
    // 把 proxy 所在目录 prepend 进子进程 PATH（Windows 分隔符 ';'，unix 是 ':'）。
    if let Some(dir) = std::path::Path::new(proxy_abs).parent() {
        let sep = if cfg!(windows) { ";" } else { ":" };
        let existing = std::env::var("PATH").unwrap_or_default();
        let merged = if existing.is_empty() {
            dir.display().to_string()
        } else {
            format!("{}{}{}", dir.display(), sep, existing)
        };
        cmd.env("PATH", merged);
    }
    let child = cmd
        .spawn()
        .map_err(|e| AppError::business(format!("agent 启动失败: {}", e)))?;
    Ok(RunningTurn { child })
}

/// 读 stdout 逐行解析，回调事件；同时收集 stderr 尾部（错误诊断）。
/// 结束后 wait 回收子进程，避免僵尸。
pub fn pump_stream<F: FnMut(AgentStreamEvent)>(
    turn: &mut RunningTurn,
    mut on_event: F,
) -> AppResult<PumpOutcome> {
    let stdout = turn.child.stdout.take()
        .ok_or_else(|| AppError::business("agent stdout 不可用"))?;
    let stderr = turn.child.stderr.take()
        .ok_or_else(|| AppError::business("agent stderr 不可用"))?;

    // stderr 独立线程收集（尾部 400 字节用于诊断）
    let stderr_handle = std::thread::spawn(move || {
        let mut tail = String::new();
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            tail.push_str(&line);
            tail.push('\n');
            truncate_tail(&mut tail, 400);
        }
        tail
    });

    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        if let Some(event) = parse_stream_line(&line) {
            on_event(event);
        }
    }

    let tail = match stderr_handle.join() {
        Ok(v) => v,
        Err(_) => {
            tracing::warn!("agent stderr 收集线程异常");
            String::new()
        }
    };

    let status = turn
        .child
        .wait()
        .map_err(|e| AppError::business(format!("agent 等待退出失败: {}", e)))?;

    Ok(PumpOutcome {
        stderr_tail: tail,
        status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_init_event() {
        let line = r#"{"type":"system","subtype":"init","session_id":"sess-123","model":"claude-sonnet-4-6"}"#;
        let event = parse_stream_line(line).unwrap();
        match event {
            AgentStreamEvent::Init { session_id, model } => {
                assert_eq!(session_id, "sess-123");
                assert_eq!(model.as_deref(), Some("claude-sonnet-4-6"));
            }
            _ => panic!("expected init"),
        }
    }

    #[test]
    fn parses_assistant_text() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"你好"}]}}"#;
        let event = parse_stream_line(line).unwrap();
        match event {
            AgentStreamEvent::AssistantText { text } => assert_eq!(text, "你好"),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn parses_tool_use_with_command_summary() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"sessiondock-proxy list --days 7"}}]}}"#;
        let event = parse_stream_line(line).unwrap();
        match event {
            AgentStreamEvent::ToolUse { name, summary } => {
                assert_eq!(name, "Bash");
                assert!(summary.contains("sessiondock-proxy list"));
            }
            _ => panic!("expected tool_use"),
        }
    }

    #[test]
    fn parses_result_success_with_cost() {
        let line = r#"{"type":"result","subtype":"success","duration_ms":1234,"total_cost_usd":0.05}"#;
        let event = parse_stream_line(line).unwrap();
        match event {
            AgentStreamEvent::Result {
                ok,
                duration_ms,
                cost_usd,
                ..
            } => {
                assert!(ok);
                assert_eq!(duration_ms, Some(1234));
                assert_eq!(cost_usd, Some(0.05));
            }
            _ => panic!("expected result"),
        }
    }

    #[test]
    fn mixed_content_skips_malformed_text_part() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"text"},{"type":"tool_use","name":"Bash","input":{"command":"ls"}}]}}"#;
        assert!(matches!(parse_stream_line(line), Some(AgentStreamEvent::ToolUse { .. })));
    }

    #[test]
    fn non_json_lines_are_ignored() {
        assert!(parse_stream_line("not json at all").is_none());
        assert!(parse_stream_line("").is_none());
    }

    #[test]
    fn build_args_include_bare_proxy_allow_rule() {
        let args = super::build_spawn_args(
            "/abs/sessiondock-proxy", "提示词", "/tmp/settings.json",
            "系统提示", None, None,
        );
        let joined = args.join(" ");
        assert!(joined.contains("Bash(sessiondock-proxy:*)"));
        assert!(joined.contains("--output-format stream-json"));
        assert!(joined.contains("--settings /tmp/settings.json"));
        assert!(!joined.contains("--resume"));
    }

    #[test]
    fn build_args_with_resume_session() {
        let args = super::build_spawn_args(
            "/abs/sessiondock-proxy", "追问", "/tmp/s.json", "系统", Some("sess-1"), None,
        );
        let pos = args.iter().position(|a| a == "--resume").unwrap();
        assert_eq!(args[pos + 1], "sess-1");
    }

    #[test]
    fn system_prompt_contains_proxy_bare_name_and_contract() {
        let prompt = super::build_system_prompt("/abs/sessiondock-proxy");
        assert!(prompt.contains("sessiondock-proxy"));
        assert!(prompt.contains("--json-help"));
        assert!(prompt.contains("coverage"));
    }

    #[test]
    fn proxy_binary_name_keeps_windows_exe_extension() {
        assert_eq!(super::proxy_binary_name("/abs/sessiondock-proxy"), "sessiondock-proxy");
        assert_eq!(
            super::proxy_binary_name("D:\\Codes\\SessionDock\\target\\debug\\sessiondock-proxy.exe"),
            "sessiondock-proxy.exe"
        );
    }

    #[test]
    fn truncate_tail_handles_utf8_boundary() {
        // 134 个"中" = 402 字节，keep_from=2 非边界，旧代码会 panic
        let mut s = "中".repeat(134);
        truncate_tail(&mut s, 400);
        assert!(s.len() <= 400);
        assert!(s.ends_with("中"));
    }

    #[test]
    fn pump_stream_reaps_child_and_returns_status() {
        let child = Command::new("/bin/echo")
            .arg("hello")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut turn = RunningTurn { child };
        let outcome = pump_stream(&mut turn, |_| {}).unwrap();
        assert!(outcome.status.success());
        assert_eq!(outcome.stderr_tail, "");
    }

    #[test]
    fn parses_result_usage() {
        let line = r#"{"type":"result","subtype":"success","duration_ms":1234,"total_cost_usd":0.01,"usage":{"input_tokens":1200,"output_tokens":380,"cache_read_input_tokens":5000,"cache_creation_input_tokens":600}}"#;
        let event = parse_stream_line(line).unwrap();
        match event {
            AgentStreamEvent::Result { usage, .. } => {
                let u = usage.expect("usage 应被解析");
                assert_eq!(u.input_tokens, 1200);
                assert_eq!(u.output_tokens, 380);
                assert_eq!(u.cache_read_input_tokens, 5000);
                assert_eq!(u.cache_creation_input_tokens, 600);
            }
            _ => panic!("expected result"),
        }
    }

    #[test]
    fn result_without_usage_degrades_to_none() {
        let line = r#"{"type":"result","subtype":"success","duration_ms":100}"#;
        match parse_stream_line(line).unwrap() {
            AgentStreamEvent::Result { usage, .. } => assert!(usage.is_none()),
            _ => panic!("expected result"),
        }
    }

    #[test]
    fn result_usage_missing_cache_fields_defaults_zero() {
        let line = r#"{"type":"result","subtype":"success","usage":{"input_tokens":10,"output_tokens":5}}"#;
        match parse_stream_line(line).unwrap() {
            AgentStreamEvent::Result { usage, .. } => {
                let u = usage.unwrap();
                assert_eq!(u.input_tokens, 10);
                assert_eq!(u.output_tokens, 5);
                assert_eq!(u.cache_read_input_tokens, 0);
                assert_eq!(u.cache_creation_input_tokens, 0);
            }
            _ => panic!("expected result"),
        }
    }
}
