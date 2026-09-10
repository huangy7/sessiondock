use super::conn;
use crate::cli::CliKind;
use crate::error::AppResult;
use rusqlite::params;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub(crate) struct SessionSearchIndexRecord {
    pub session_path: String,
    pub modified_ms: i64,
    #[allow(dead_code)]
    pub doc_count: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct SessionSearchDocRecord {
    pub message_index: usize,
    pub search_text: String,
}

#[derive(Debug, Clone)]
pub(crate) struct SessionSearchDocBatch {
    pub session_path: String,
    pub modified_ms: i64,
    pub docs: Vec<SessionSearchDocRecord>,
}

#[derive(Debug, Clone)]
pub(crate) struct SessionSearchWriteProgress {
    pub phase: &'static str,
    pub written_sessions: usize,
    pub current_path: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct SessionSearchIndexStateUpdate {
    pub session_path: String,
    pub modified_ms: i64,
    pub doc_count: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct SessionSearchCandidate {
    pub session_path: String,
    pub first_match_message_index: Option<usize>,
    pub matched_doc_count: usize,
    pub rank: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct SessionSearchMatchedDoc {
    pub session_path: String,
    pub message_index: usize,
    pub search_text: String,
}

fn map_session_search_index_record(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<SessionSearchIndexRecord> {
    Ok(SessionSearchIndexRecord {
        session_path: row.get(0)?,
        modified_ms: row.get(1)?,
        doc_count: row.get::<_, i64>(2)?.max(0) as usize,
    })
}

pub(crate) fn count_session_search_indexed_sessions(kind: CliKind) -> AppResult<usize> {
    let conn = conn()?;
    let count: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT session_path) FROM session_search_index_state WHERE cli_id = ?1",
        params![kind.id()],
        |row| row.get(0),
    )?;
    Ok(count.max(0) as usize)
}

pub(crate) fn count_session_search_indexed_docs(kind: CliKind) -> AppResult<usize> {
    let conn = conn()?;
    let count: i64 = conn.query_row(
        "SELECT COALESCE(SUM(doc_count), 0) FROM session_search_index_state WHERE cli_id = ?1",
        params![kind.id()],
        |row| row.get(0),
    )?;
    Ok(count.max(0) as usize)
}

pub(crate) fn read_session_search_index(
    kind: CliKind,
) -> AppResult<HashMap<String, SessionSearchIndexRecord>> {
    let conn = conn()?;
    let mut stmt = conn.prepare(
        r#"
            SELECT session_path, modified_ms, doc_count
            FROM session_search_index_state
            WHERE cli_id = ?1
            "#,
    )?;
    let rows = stmt.query_map(params![kind.id()], map_session_search_index_record)?;

    let mut result = HashMap::new();
    for row in rows {
        let record = row?;
        result.insert(record.session_path.clone(), record);
    }
    Ok(result)
}

pub(crate) fn clear_all_session_search_index_state() -> AppResult<()> {
    let conn = conn()?;
    conn.execute("DELETE FROM session_search_index_state", [])?;
    Ok(())
}

pub(crate) fn replace_session_search_docs_with_progress<F>(
    kind: CliKind,
    batches: &[SessionSearchDocBatch],
    mut on_progress: F,
) -> AppResult<()>
where
    F: FnMut(SessionSearchWriteProgress),
{
    if batches.is_empty() {
        return Ok(());
    }

    let total_sessions = batches.len();

    // Phase 1: Tantivy writes + commit WITHOUT any SQLite transaction open.
    // ngram(2-4) tokenization and writer.commit() dominate the wall-clock time;
    // keeping them outside a SQLite transaction shrinks the database write-lock
    // window from seconds to milliseconds.
    //
    // On ANY error before the commit lands, roll back the writer buffer so the
    // queued delete+partial adds are not flushed by a later unrelated commit.
    let tantivy_stage = |on_progress: &mut F| -> AppResult<()> {
        for (idx, batch) in batches.iter().enumerate() {
            // Write to Tantivy index
            let tantivy_docs: Vec<(usize, String)> = batch
                .docs
                .iter()
                .map(|d| (d.message_index, d.search_text.clone()))
                .collect();
            super::tantivy_search::write_session_docs(
                kind.id(),
                &batch.session_path,
                batch.modified_ms as u64,
                &tantivy_docs,
            )?;

            on_progress(SessionSearchWriteProgress {
                phase: "writing",
                written_sessions: idx + 1,
                current_path: Some(batch.session_path.clone()),
            });
        }

        on_progress(SessionSearchWriteProgress {
            phase: "committing",
            written_sessions: total_sessions,
            current_path: None,
        });

        // Commit Tantivy changes FIRST. This ordering makes the dangerous
        // failure direction (SQLite claims indexed but Tantivy lacks the docs)
        // impossible: we only record state for docs that are already committed.
        super::tantivy_search::commit_writes()?;
        Ok(())
    };
    if let Err(err) = tantivy_stage(&mut on_progress) {
        super::tantivy_search::rollback_writes();
        return Err(err);
    }

    // Phase 2: bookkeeping rows in a short SQLite transaction AFTER the
    // Tantivy commit succeeded. If this fails, the sessions simply get
    // re-indexed on the next run (write_session_docs is delete-then-insert,
    // so re-indexing is idempotent) — a safe failure direction.
    let mut conn = conn()?;
    let tx = conn.transaction()?;
    let now = super::now_rfc3339();
    for batch in batches {
        tx.execute(
            r#"
            INSERT OR REPLACE INTO session_search_index_state (
                cli_id,
                session_path,
                modified_ms,
                doc_count,
                indexed_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            params![
                kind.id(),
                &batch.session_path,
                batch.modified_ms,
                batch.docs.len() as i64,
                &now,
            ],
        )?;
    }
    tx.commit()?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let _ = super::write_index_last_updated(&conn, kind.id(), now_ms);

    on_progress(SessionSearchWriteProgress {
        phase: "committed",
        written_sessions: total_sessions,
        current_path: None,
    });
    Ok(())
}

pub(crate) fn write_session_search_docs_uncommitted(
    kind: CliKind,
    batch: &SessionSearchDocBatch,
) -> AppResult<SessionSearchIndexStateUpdate> {
    let tantivy_docs: Vec<(usize, String)> = batch
        .docs
        .iter()
        .map(|d| (d.message_index, d.search_text.clone()))
        .collect();
    super::tantivy_search::write_session_docs(
        kind.id(),
        &batch.session_path,
        batch.modified_ms as u64,
        &tantivy_docs,
    )?;

    Ok(SessionSearchIndexStateUpdate {
        session_path: batch.session_path.clone(),
        modified_ms: batch.modified_ms,
        doc_count: batch.docs.len(),
    })
}

pub(crate) fn commit_session_search_writes() -> AppResult<()> {
    super::tantivy_search::commit_writes()
}

/// Number of `session_search_index_state` rows written per SQLite transaction.
/// Chunking keeps the write-lock window small during large rebuilds (a 10k
/// session rebuild no longer holds the lock in one giant transaction). On
/// failure, earlier chunks stay committed — acceptable because unrecorded
/// sessions are simply re-indexed on the next run (Tantivy writes are
/// delete-then-insert, hence idempotent).
const STATE_WRITE_CHUNK_SIZE: usize = 500;

pub(crate) fn replace_session_search_index_state(
    kind: CliKind,
    updates: &[SessionSearchIndexStateUpdate],
) -> AppResult<()> {
    if updates.is_empty() {
        return Ok(());
    }

    let mut conn = conn()?;
    let now = super::now_rfc3339();

    for chunk in updates.chunks(STATE_WRITE_CHUNK_SIZE) {
        let tx = conn.transaction()?;
        for update in chunk {
            tx.execute(
                r#"
            INSERT OR REPLACE INTO session_search_index_state (
                cli_id,
                session_path,
                modified_ms,
                doc_count,
                indexed_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
                params![
                    kind.id(),
                    &update.session_path,
                    update.modified_ms,
                    update.doc_count as i64,
                    &now,
                ],
            )?;
        }
        tx.commit()?;
    }

    Ok(())
}

pub(crate) fn delete_session_search_paths(
    kind: CliKind,
    session_paths: &[String],
) -> AppResult<()> {
    if session_paths.is_empty() {
        return Ok(());
    }

    // Delete from Tantivy
    super::tantivy_search::delete_session_docs(session_paths)?;

    // Delete from SQLite
    let mut conn = conn()?;
    let tx = conn.transaction()?;

    for session_path in session_paths {
        tx.execute(
            "DELETE FROM session_search_index_state WHERE cli_id = ?1 AND session_path = ?2",
            params![kind.id(), session_path],
        )?;
    }

    tx.commit()?;

    Ok(())
}

pub(crate) fn delete_session_records(kind: CliKind, session_paths: &[String]) -> AppResult<()> {
    if session_paths.is_empty() {
        return Ok(());
    }

    // Delete from Tantivy search index first. Best-effort: record deletion
    // must still proceed on failure, but log so stale search hits are
    // diagnosable instead of silently lingering.
    if let Err(err) = super::tantivy_search::delete_session_docs(session_paths) {
        tracing::warn!(
            "删除会话搜索索引文档失败（记录仍将被删除）: cli={}, paths={:?}, err={}",
            kind.id(),
            session_paths,
            err
        );
    }

    let mut conn = conn()?;
    let tx = conn.transaction()?;

    for session_path in session_paths {
        tx.execute(
            "DELETE FROM archived_session_content WHERE cli_id = ?1 AND session_path = ?2",
            params![kind.id(), session_path],
        )?;
        tx.execute(
            "DELETE FROM session_list_index WHERE cli_id = ?1 AND session_path = ?2",
            params![kind.id(), session_path],
        )?;
        tx.execute(
            "DELETE FROM session_search_index_state WHERE cli_id = ?1 AND session_path = ?2",
            params![kind.id(), session_path],
        )?;
        tx.execute(
            "DELETE FROM session_names WHERE session_path = ?1",
            params![session_path],
        )?;
    }

    tx.commit()?;

    Ok(())
}

/// 清理纯搜索孤儿：物理搜索索引中存在、但源文件与归档内容都不存在的会话路径。
///
/// 这类条目会被全局搜索命中，但打开必失败（load_session_stream 报"会话文件不存在"），
/// 且增量重建（collect_missing_search_paths 基于会话列表索引）无法发现它们，
/// 需要主动遍历物理索引逐一核对。`cli_id` 为 None 时清理全部 CLI，否则仅清理指定 CLI。
/// 返回清理的路径数。
pub(crate) fn purge_search_orphans(cli_id: Option<&str>) -> AppResult<usize> {
    let indexed = super::tantivy_search::all_indexed_session_paths()?;
    let mut orphans: Vec<String> = Vec::new();
    let mut pairs: Vec<(String, String)> = Vec::new();
    for (cli, path) in indexed {
        if let Some(filter) = cli_id {
            if cli != filter {
                continue;
            }
        }
        if std::path::Path::new(&path).exists() {
            continue;
        }
        match super::read_archived_session_content(&cli, &path) {
            // 归档内容可用 → 可从归档打开，保留
            Ok(Some(_)) => continue,
            // 归档不可用 → 孤儿；读取失败保守保留，避免误删
            Ok(None) => {}
            Err(_) => continue,
        }
        orphans.push(path.clone());
        pairs.push((cli, path));
    }
    if orphans.is_empty() {
        return Ok(0);
    }
    // tantivy 物理文档删除（best-effort，失败不阻断 SQLite 状态清理）
    if let Err(err) = super::tantivy_search::delete_session_docs(&orphans) {
        tracing::warn!("清理搜索孤儿文档失败: err={}", err);
    }
    // SQLite 搜索状态同步删除
    let mut conn = conn()?;
    let tx = conn.transaction()?;
    for (cli, path) in &pairs {
        tx.execute(
            "DELETE FROM session_search_index_state WHERE cli_id = ?1 AND session_path = ?2",
            params![cli, path],
        )?;
    }
    tx.commit()?;
    tracing::info!("搜索索引: 清理纯搜索孤儿 {} 个", orphans.len());
    Ok(orphans.len())
}

pub(crate) fn search_session_candidates(
    kind: CliKind,
    query: &str,
    limit: usize,
) -> AppResult<Vec<SessionSearchCandidate>> {
    let hits = super::tantivy_search::search_sessions(kind.id(), query, limit * 10)?;

    // Group and aggregate hits by session_path, since tantivy returns document-level hits
    let mut grouping: HashMap<String, (usize, usize, f64)> = HashMap::new(); // (first_match_msg_idx, matched_doc_count, max_score)

    for hit in hits {
        let entry = grouping
            .entry(hit.session_path)
            .or_insert((usize::MAX, 0, 0.0));
        if (hit.message_index as usize) < entry.0 {
            entry.0 = hit.message_index as usize;
        }
        entry.1 += 1;
        if (hit.score as f64) > entry.2 {
            entry.2 = hit.score as f64;
        }
    }

    let mut candidates: Vec<SessionSearchCandidate> = grouping
        .into_iter()
        .map(|(path, (first_msg, count, score))| SessionSearchCandidate {
            session_path: path,
            first_match_message_index: Some(first_msg),
            matched_doc_count: count,
            rank: score,
        })
        .collect();

    // Sort by matched_doc_count DESC, first_match_message_index ASC
    candidates.sort_by(|a, b| {
        b.matched_doc_count.cmp(&a.matched_doc_count).then_with(|| {
            a.first_match_message_index
                .cmp(&b.first_match_message_index)
        })
    });

    candidates.truncate(limit);
    Ok(candidates)
}

pub(crate) fn read_session_search_docs_for_paths(
    kind: CliKind,
    query: &str,
    session_paths: &[String],
) -> AppResult<Vec<SessionSearchMatchedDoc>> {
    let hits = super::tantivy_search::search_docs_for_paths(kind.id(), query, session_paths)?;

    let mut result: Vec<SessionSearchMatchedDoc> = hits
        .into_iter()
        .map(|hit| SessionSearchMatchedDoc {
            session_path: hit.session_path,
            message_index: hit.message_index as usize,
            search_text: hit.search_text,
        })
        .collect();

    // Sort by session_path ASC, message_index ASC
    result.sort_by(|a, b| {
        a.session_path
            .cmp(&b.session_path)
            .then_with(|| a.message_index.cmp(&b.message_index))
    });

    Ok(result)
}

pub(crate) fn read_session_search_doc_text(
    kind: CliKind,
    session_path: &str,
    message_index: usize,
) -> AppResult<Option<String>> {
    // Look up doc directly from SQLite using the list index fallback if needed
    // Actually, in the FTS implementation this was queried directly from FTS tables.
    // Let's get it from the Tantivy index via a specific term query on session_path and msg_index.
    let ctx_guard = super::tantivy_search::get_index_context()?;
    let ctx = ctx_guard
        .as_ref()
        .ok_or_else(|| crate::error::AppError::business("Search index context is unavailable"))?;
    let searcher = ctx.reader.searcher();

    use tantivy::query::{BooleanQuery, Occur, TermQuery};
    use tantivy::schema::{IndexRecordOption, Value};
    use tantivy::Term;

    let c_term = Term::from_field_text(ctx.schema.cli_id, kind.id());
    let c_query = Box::new(TermQuery::new(c_term, IndexRecordOption::Basic));

    let p_term = Term::from_field_text(ctx.schema.session_path, session_path);
    let p_query = Box::new(TermQuery::new(p_term, IndexRecordOption::Basic));

    let m_term = Term::from_field_u64(ctx.schema.message_index, message_index as u64);
    let m_query = Box::new(TermQuery::new(m_term, IndexRecordOption::Basic));

    let combined = BooleanQuery::new(vec![
        (Occur::Must, c_query),
        (Occur::Must, p_query),
        (Occur::Must, m_query),
    ]);

    let top_docs = searcher.search(&combined, &tantivy::collector::TopDocs::with_limit(1))?;

    if let Some((_, doc_addr)) = top_docs.first() {
        let doc: tantivy::TantivyDocument = searcher.doc(*doc_addr)?;
        if let Some(val) = doc
            .get_first(ctx.schema.search_text)
            .and_then(|v| v.as_str())
        {
            return Ok(Some(val.to_string()));
        }
    }

    Ok(None)
}

#[cfg(test)]
mod orphan_tests {
    use super::*;

    #[test]
    fn test_purge_removes_orphan_docs() {
        let fake = "/tmp/sessiondock-nonexistent-orphan-12345.jsonl".to_string();
        // 应用运行时 tantivy 索引被独占（LockBusy），跳过而非误报
        if crate::db::tantivy_search::write_session_docs("claude", &fake, 1, &[(0, "hello world orphan".to_string())]).is_err() {
            println!("搜索索引被应用占用，跳过孤儿清理测试");
            return;
        }
        if crate::db::tantivy_search::commit_writes().is_err() {
            println!("搜索索引被应用占用，跳过孤儿清理测试");
            return;
        }

        let before: Vec<(String, String)> = crate::db::tantivy_search::all_indexed_session_paths().unwrap()
            .into_iter().filter(|(_, p)| *p == fake).collect();
        assert_eq!(before.len(), 1, "孤儿应已写入索引");

        purge_search_orphans(Some("claude")).unwrap();

        let after: Vec<(String, String)> = crate::db::tantivy_search::all_indexed_session_paths().unwrap()
            .into_iter().filter(|(_, p)| *p == fake).collect();
        assert_eq!(after.len(), 0, "孤儿应已被清理");
    }
}
