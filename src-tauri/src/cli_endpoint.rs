use crate::cli::{self, CliKind};
use serde_json::Value;
use std::env;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedEndpoint {
    pub base_url: String,
    pub api_key: String,
}



fn settings_path_for(kind: CliKind) -> Result<PathBuf, String> {
    match kind {
        CliKind::Claude => Ok(cli::data_dir(kind)?.join("settings.json")),
        CliKind::Codex => Ok(cli::data_dir(kind)?.join("config.toml")),
        CliKind::Gemini => Err("Gemini 不支持端点配置".to_string()),
        CliKind::WorkBuddy => Err("WorkBuddy 不支持端点配置".to_string()),
        CliKind::Dsh => Err("DSH 不支持端点配置".to_string()),
        CliKind::Antigravity => Err("Antigravity 不支持端点配置".to_string()),
    }
}

fn codex_auth_path() -> Result<PathBuf, String> {
    Ok(cli::data_dir(CliKind::Codex)?.join("auth.json"))
}

fn read_json_file(path: &PathBuf, default_content: &str) -> Result<Value, String> {
    if !path.exists() {
        return serde_json::from_str(default_content).map_err(|e| format!("默认 JSON 无效: {}", e));
    }
    let raw = fs::read_to_string(path).map_err(|e| format!("读取配置失败: {}", e))?;
    serde_json::from_str(&raw).map_err(|e| format!("解析 JSON 失败: {}", e))
}

fn read_toml_file(path: &PathBuf) -> Result<toml::Value, String> {
    if !path.exists() {
        return Ok(toml::Value::Table(toml::map::Map::new()));
    }

    let raw = fs::read_to_string(path).map_err(|e| format!("读取配置失败: {}", e))?;
    if raw.trim().is_empty() {
        return Ok(toml::Value::Table(toml::map::Map::new()));
    }

    raw.parse::<toml::Value>()
        .map_err(|e| format!("解析 TOML 失败: {}", e))
}

fn normalize_base_url(url: &str, fallback: &str) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.trim_end_matches('/').to_string()
    }
}

fn first_non_empty(candidates: &[Option<String>]) -> Option<String> {
    candidates
        .iter()
        .flatten()
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty())
}

fn json_pointer_string(value: &Value, pointer: &str) -> Option<String> {
    value.pointer(pointer).and_then(|item| item.as_str()).map(|item| item.to_string())
}

fn resolve_claude_endpoint_from_settings() -> Result<ResolvedEndpoint, String> {
    let settings = read_json_file(&settings_path_for(CliKind::Claude)?, "{}")?;

    let base_url = first_non_empty(&[
        json_pointer_string(&settings, "/env/ANTHROPIC_BASE_URL"),
        json_pointer_string(&settings, "/baseUrl"),
    ])
    .unwrap_or_else(|| "https://api.anthropic.com".to_string());

    let api_key = first_non_empty(&[
        json_pointer_string(&settings, "/env/ANTHROPIC_API_KEY"),
        json_pointer_string(&settings, "/env/ANTHROPIC_AUTH_TOKEN"),
        json_pointer_string(&settings, "/apiKey"),
        json_pointer_string(&settings, "/oauth/token"),
        json_pointer_string(&settings, "/oauth/accessToken"),
    ])
    .ok_or_else(|| "未检测到 Claude CLI 配置，请先运行 `claude /login` 或设置 ANTHROPIC_API_KEY".to_string())?;

    Ok(ResolvedEndpoint {
        base_url: normalize_base_url(&base_url, "https://api.anthropic.com"),
        api_key,
    })
}

fn resolve_codex_endpoint_from_settings() -> Result<ResolvedEndpoint, String> {
    let config = read_toml_file(&settings_path_for(CliKind::Codex)?)?;
    let auth = read_json_file(&codex_auth_path()?, "{}")?;

    let provider_id = config
        .get("model_provider")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("openai-chat-completions");

    let provider_table = config
        .get("model_providers")
        .and_then(|value| value.as_table())
        .and_then(|providers| providers.get(provider_id))
        .and_then(|value| value.as_table());

    let base_url = provider_table
        .and_then(|provider| provider.get("base_url"))
        .and_then(|value| value.as_str())
        .map(|value| normalize_base_url(value, "https://api.openai.com/v1"))
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());

    let api_key = auth
        .get("OPENAI_API_KEY")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .ok_or_else(|| "未检测到 Codex CLI 配置，请先设置 OPENAI_API_KEY。".to_string())?;

    Ok(ResolvedEndpoint { base_url, api_key })
}

pub fn resolve_endpoint(cli_family: &str) -> Result<ResolvedEndpoint, String> {
    match cli_family.trim() {
        "claude" => {
            let env_base = env::var("ANTHROPIC_BASE_URL").ok();
            let env_key = env::var("ANTHROPIC_API_KEY")
                .ok()
                .or_else(|| env::var("ANTHROPIC_AUTH_TOKEN").ok());
            if let (Some(base_url), Some(api_key)) = (env_base, env_key) {
                let normalized_key = api_key.trim().to_string();
                if !normalized_key.is_empty() {
                    return Ok(ResolvedEndpoint {
                        base_url: normalize_base_url(&base_url, "https://api.anthropic.com"),
                        api_key: normalized_key,
                    });
                }
            }
            resolve_claude_endpoint_from_settings()
        }
        "codex" => {
            let env_base = env::var("OPENAI_BASE_URL").ok();
            let env_key = env::var("OPENAI_API_KEY").ok();
            if let (Some(base_url), Some(api_key)) = (env_base, env_key) {
                let normalized_key = api_key.trim().to_string();
                if !normalized_key.is_empty() {
                    return Ok(ResolvedEndpoint {
                        base_url: normalize_base_url(&base_url, "https://api.openai.com/v1"),
                        api_key: normalized_key,
                    });
                }
            }
            resolve_codex_endpoint_from_settings()
        }
        other => Err(format!("不支持的模型族: {}", other)),
    }
}
