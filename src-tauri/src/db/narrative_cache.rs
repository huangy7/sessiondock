use super::conn;
use crate::error::{AppError, AppResult};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedNarrative {
    pub content_hash: String,
    pub model_id: String,
    pub narrative: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub created_at: String,
}

pub(crate) fn get_cached_narrative(
    content_hash: &str,
    model_id: &str,
) -> AppResult<Option<CachedNarrative>> {
    let conn = conn()?;
    conn.query_row(
        r#"
        SELECT content_hash, model_id, narrative, input_tokens, output_tokens, created_at
        FROM report_narrative_cache
        WHERE content_hash = ?1 AND model_id = ?2
        "#,
        params![content_hash.trim(), model_id.trim()],
        |row| {
            Ok(CachedNarrative {
                content_hash: row.get(0)?,
                model_id: row.get(1)?,
                narrative: row.get(2)?,
                input_tokens: row.get(3)?,
                output_tokens: row.get(4)?,
                created_at: row.get(5)?,
            })
        },
    )
    .optional()
    .map_err(AppError::from)
}

pub(crate) fn put_cached_narrative(entry: &CachedNarrative) -> AppResult<()> {
    let conn = conn()?;
    conn.execute(
        r#"
        INSERT OR REPLACE INTO report_narrative_cache (
            content_hash,
            model_id,
            narrative,
            input_tokens,
            output_tokens,
            created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
        params![
            entry.content_hash.trim(),
            entry.model_id.trim(),
            entry.narrative,
            entry.input_tokens,
            entry.output_tokens,
            entry.created_at,
        ],
    )?;
    Ok(())
}

pub(crate) fn count_narrative_cache() -> AppResult<i64> {
    let conn = conn()?;
    conn.query_row("SELECT COUNT(*) FROM report_narrative_cache", [], |row| row.get(0))
        .map_err(AppError::from)
}

pub(crate) fn purge_narrative_cache(older_than_days: Option<i64>) -> AppResult<usize> {
    let conn = conn()?;
    let deleted = match older_than_days {
        None => conn.execute("DELETE FROM report_narrative_cache", [])?,
        Some(days) => {
            if days <= 0 {
                return Err(AppError::business("清理天数必须大于 0"));
            }
            let cutoff = chrono::Utc::now() - chrono::Duration::days(days);
            let cutoff_str = cutoff.to_rfc3339();
            conn.execute(
                "DELETE FROM report_narrative_cache WHERE created_at < ?1",
                rusqlite::params![cutoff_str],
            )?
        }
    };
    Ok(deleted)
}
