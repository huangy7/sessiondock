use crate::error::{AppError, AppResult};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, Occur, TermQuery};
use tantivy::schema::*;
use tantivy::tokenizer::NgramTokenizer;
use tantivy::{DocAddress, Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term};

const NGRAM_MIN: usize = 2;
const NGRAM_MAX: usize = 4;
const NGRAM_TOKENIZER_NAME: &str = "ngram_2_4";
const SEARCH_HEAP_SIZE: usize = 50_000_000; // 50MB

/// Schema field accessors
pub(crate) struct SearchSchema {
    pub cli_id: Field,
    pub session_path: Field,
    pub message_index: Field,
    pub search_text: Field,
    pub modified_ms: Field,
}

/// Global index context — lazily initialized singleton
pub(crate) struct IndexContext {
    pub schema: SearchSchema,
    pub index: Index,
    pub reader: IndexReader,
    pub writer: Arc<Mutex<IndexWriter>>,
    /// Generation captured when this context was created. A borrowed writer
    /// whose generation no longer matches `INDEX_GENERATION` is stale (the
    /// physical index was reset underneath it) and must not commit.
    pub generation: u64,
}

static INDEX_CONTEXT: LazyLock<Mutex<Option<IndexContext>>> = LazyLock::new(|| Mutex::new(None));

/// Bumped by `reset_physical_index` while holding the writer mutex, so any
/// writer Arc borrowed before the reset detects the mismatch and aborts
/// instead of committing pre-reset docs into the recreated directory.
static INDEX_GENERATION: AtomicU64 = AtomicU64::new(0);

fn search_index_dir() -> AppResult<PathBuf> {
    let dir = super::data_dir()?.join("search_index");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub(crate) fn is_physical_index_ready() -> AppResult<bool> {
    let index_dir = super::data_dir()?.join("search_index");
    if !index_dir.join("meta.json").exists() {
        return Ok(false);
    }
    let (expected_schema, _) = build_schema();
    Ok(Index::open_in_dir(&index_dir)
        .map(|index| index.schema() == expected_schema)
        .unwrap_or(false))
}

pub(crate) fn reset_physical_index() -> AppResult<()> {
    // Hold the global context guard for the whole reset so no new context can
    // be initialized on the directory while it is being recreated.
    let mut guard = INDEX_CONTEXT
        .lock()
        .map_err(|e| AppError::business(format!("Index context lock poisoned: {}", e)))?;

    // Serialize against any in-flight commit/delete that already borrowed the
    // writer: locking the writer mutex here waits for it to finish and blocks
    // it from writing while the directory is removed. Clone the Arc first so
    // the guard does not borrow from the context guard we are about to clear.
    let writer = guard.as_ref().map(|ctx| Arc::clone(&ctx.writer));
    let writer_guard = match &writer {
        Some(w) => Some(
            w.lock()
                .map_err(|e| AppError::business(format!("Writer lock poisoned: {}", e)))?,
        ),
        None => None,
    };

    // Invalidate writers borrowed before this point that are still waiting on
    // the writer mutex: they will observe the generation mismatch and abort
    // (rollback) instead of committing pre-reset docs into the fresh dir.
    INDEX_GENERATION.fetch_add(1, Ordering::SeqCst);
    *guard = None;

    let index_dir = super::data_dir()?.join("search_index");
    if index_dir.exists() {
        std::fs::remove_dir_all(&index_dir)?;
    }
    std::fs::create_dir_all(&index_dir)?;

    drop(writer_guard);
    Ok(())
}

fn build_schema() -> (Schema, SearchSchema) {
    let mut builder = Schema::builder();

    let cli_id = builder.add_text_field("cli_id", STRING | STORED);
    let session_path = builder.add_text_field("session_path", STRING | STORED);
    let message_index = builder.add_u64_field("message_index", INDEXED | STORED);

    // search_text uses ngram tokenizer for substring matching
    let text_options = TextOptions::default()
        .set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer(NGRAM_TOKENIZER_NAME)
                .set_index_option(IndexRecordOption::WithFreqsAndPositions),
        )
        .set_stored();
    let search_text = builder.add_text_field("search_text", text_options);

    let modified_ms = builder.add_u64_field("modified_ms", STORED);

    let schema_fields = SearchSchema {
        cli_id,
        session_path,
        message_index,
        search_text,
        modified_ms,
    };

    (builder.build(), schema_fields)
}

fn init_index_context(create_if_missing: bool) -> AppResult<IndexContext> {
    let index_dir = search_index_dir()?;
    let (schema, schema_fields) = build_schema();

    // Open or create index. Existing broken indexes must be repaired by the
    // explicit rebuild path so SQLite sync state can be invalidated together.
    let index = if index_dir.join("meta.json").exists() {
        match Index::open_in_dir(&index_dir) {
            Ok(idx) if idx.schema() == schema => idx,
            Ok(_) => return Err(AppError::business("Tantivy index schema mismatch")),
            Err(e) => {
                return Err(AppError::business(format!(
                    "Tantivy index cannot be opened: {}",
                    e
                )))
            }
        }
    } else if create_if_missing {
        Index::create_in_dir(&index_dir, schema.clone())?
    } else {
        return Err(AppError::business("Tantivy index is missing"));
    };

    // Register ngram tokenizer
    index.tokenizers().register(
        NGRAM_TOKENIZER_NAME,
        NgramTokenizer::new(NGRAM_MIN, NGRAM_MAX, false)
            .map_err(|e| AppError::business(format!("Failed to create NgramTokenizer: {:?}", e)))?,
    );

    let reader = index
        .reader_builder()
        .reload_policy(ReloadPolicy::OnCommitWithDelay)
        .try_into()
        .map_err(|e| AppError::business(format!("Failed to create index reader: {}", e)))?;

    let writer = index.writer(SEARCH_HEAP_SIZE)?;

    Ok(IndexContext {
        schema: schema_fields,
        index,
        reader,
        writer: Arc::new(Mutex::new(writer)),
        generation: INDEX_GENERATION.load(Ordering::SeqCst),
    })
}

/// Open the global index context without creating a missing physical index.
pub(crate) fn get_index_context() -> AppResult<MutexGuard<'static, Option<IndexContext>>> {
    get_index_context_inner(false)
}

fn get_or_create_index_context() -> AppResult<MutexGuard<'static, Option<IndexContext>>> {
    get_index_context_inner(true)
}

fn get_index_context_inner(
    create_if_missing: bool,
) -> AppResult<MutexGuard<'static, Option<IndexContext>>> {
    let mut guard = INDEX_CONTEXT
        .lock()
        .map_err(|e| AppError::business(format!("Index context lock poisoned: {}", e)))?;

    if guard.is_none() {
        let ctx = init_index_context(create_if_missing)
            .map_err(|e| AppError::business(format!("Search index init failed: {}", e)))?;
        *guard = Some(ctx);
    }

    Ok(guard)
}

/// Add documents for a session (delete-then-insert pattern).
pub(crate) fn write_session_docs(
    cli_id: &str,
    session_path: &str,
    modified_ms: u64,
    docs: &[(usize, String)], // (message_index, search_text)
) -> AppResult<()> {
    let ctx_guard = get_or_create_index_context()?;
    let ctx = ctx_guard
        .as_ref()
        .ok_or_else(|| AppError::business("Search index context is unavailable"))?;
    let mut writer = ctx
        .writer
        .lock()
        .map_err(|e| AppError::business(format!("Writer lock poisoned: {}", e)))?;

    // Delete existing docs for this session_path
    let path_term = Term::from_field_text(ctx.schema.session_path, session_path);
    writer.delete_term(path_term);

    // Add new docs. On failure, roll back the writer buffer so the queued
    // delete + partial adds are not flushed by a later unrelated commit
    // (this also discards earlier successfully-queued batches — safe because
    // re-indexing is delete-then-insert and therefore idempotent).
    for (msg_idx, text) in docs {
        let mut doc = TantivyDocument::default();
        doc.add_text(ctx.schema.cli_id, cli_id);
        doc.add_text(ctx.schema.session_path, session_path);
        doc.add_u64(ctx.schema.message_index, *msg_idx as u64);
        doc.add_text(ctx.schema.search_text, text);
        doc.add_u64(ctx.schema.modified_ms, modified_ms);
        if let Err(e) = writer.add_document(doc) {
            let _ = writer.rollback();
            return Err(e.into());
        }
    }

    Ok(())
}

/// Discard any queued (uncommitted) writes in the shared writer buffer.
///
/// Best-effort helper for error paths: callers that fail between queueing
/// docs and committing them invoke this so the leftover delete+partial adds
/// cannot be flushed by a later unrelated `commit_writes` /
/// `delete_session_docs`. Lock/rollback failures are ignored because the
/// caller is already propagating a primary error.
pub(crate) fn rollback_writes() {
    let writer = match INDEX_CONTEXT.lock() {
        Ok(guard) => guard.as_ref().map(|ctx| Arc::clone(&ctx.writer)),
        Err(_) => None,
    };
    if let Some(writer) = writer {
        if let Ok(mut writer) = writer.lock() {
            let _ = writer.rollback();
        }
    }
}

/// Commit pending writes to the index.
pub(crate) fn commit_writes() -> AppResult<()> {
    // Clone the writer Arc and reader, then DROP the global context guard so
    // concurrent searches are not blocked for the duration of writer.commit()
    // (which can take seconds on large rebuilds). The single-writer invariant
    // is preserved: the writer itself stays behind its own Mutex, and cloning
    // the Arc never creates a second IndexWriter. IndexReader is an Arc
    // wrapper, so reload() on the clone reloads the shared reader state.
    let (writer, reader, generation) = {
        let ctx_guard = get_or_create_index_context()?;
        let ctx = ctx_guard
            .as_ref()
            .ok_or_else(|| AppError::business("Search index context is unavailable"))?;
        (
            Arc::clone(&ctx.writer),
            ctx.reader.clone(),
            ctx.generation,
        )
    };
    let mut writer = writer
        .lock()
        .map_err(|e| AppError::business(format!("Writer lock poisoned: {}", e)))?;
    let current_generation = INDEX_GENERATION.load(Ordering::SeqCst);
    if generation != current_generation {
        // The physical index was reset after this writer was borrowed:
        // discard the buffer instead of committing pre-reset docs into the
        // recreated directory. Returning an error (instead of a silent Ok)
        // prevents rebuild callers from persisting SQLite index-state rows
        // for documents that were never committed — which would cause
        // permanent silent search misses.
        tracing::warn!(
            "搜索索引: commit 检测到代际不匹配，丢弃缓冲写入 (borrowed_gen={}, current_gen={})",
            generation,
            current_generation
        );
        let _ = writer.rollback();
        return Err(AppError::business(format!(
            "搜索索引代际不匹配 ({} -> {})，写入已回滚，请重新构建索引",
            generation, current_generation
        )));
    }
    if let Err(e) = writer.commit() {
        // A failed commit can leave the buffer intact; discard it so partial
        // writes are not flushed by a later unrelated commit.
        let _ = writer.rollback();
        return Err(e.into());
    }
    drop(writer);
    reader.reload()?;
    Ok(())
}

/// Delete all documents for given session paths.
pub(crate) fn delete_session_docs(session_paths: &[String]) -> AppResult<()> {
    if !is_physical_index_ready()? {
        return Ok(());
    }

    // Same pattern as commit_writes: release the global context guard before
    // the (potentially slow) writer.commit() so searches stay responsive.
    let (writer, reader, session_path_field, generation) = {
        let ctx_guard = get_index_context()?;
        let ctx = ctx_guard
            .as_ref()
            .ok_or_else(|| AppError::business("Search index context is unavailable"))?;
        (
            Arc::clone(&ctx.writer),
            ctx.reader.clone(),
            ctx.schema.session_path,
            ctx.generation,
        )
    };
    let mut writer = writer
        .lock()
        .map_err(|e| AppError::business(format!("Writer lock poisoned: {}", e)))?;
    let current_generation = INDEX_GENERATION.load(Ordering::SeqCst);
    if generation != current_generation {
        // Stale writer (index was reset after borrowing): nothing to delete
        // in the recreated directory. Keeping Ok here is the safe direction
        // (pre-reset docs are gone with the reset), but log it for
        // observability.
        tracing::warn!(
            "搜索索引: delete 检测到代际不匹配，跳过删除 (borrowed_gen={}, current_gen={})",
            generation,
            current_generation
        );
        let _ = writer.rollback();
        return Ok(());
    }

    for path in session_paths {
        let term = Term::from_field_text(session_path_field, path);
        writer.delete_term(term);
    }
    if let Err(e) = writer.commit() {
        let _ = writer.rollback();
        return Err(e.into());
    }
    drop(writer);
    reader.reload()?;
    Ok(())
}

/// 返回物理索引中所有已索引的 (cli_id, session_path) 去重集合。
/// 供孤儿清理使用：逐一核对每个路径的源文件与归档内容是否仍然存在。
pub(crate) fn all_indexed_session_paths() -> AppResult<Vec<(String, String)>> {
    if !is_physical_index_ready()? {
        return Ok(Vec::new());
    }
    let ctx_guard = get_index_context()?;
    let ctx = ctx_guard
        .as_ref()
        .ok_or_else(|| AppError::business("Search index context is unavailable"))?;
    let searcher = ctx.reader.searcher();
    let schema = &ctx.schema;
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    for (ord, segment_reader) in searcher.segment_readers().iter().enumerate() {
        for doc_id in segment_reader.doc_ids_alive() {
            let addr = DocAddress::new(ord as u32, doc_id);
            if let Ok(doc) = searcher.doc::<TantivyDocument>(addr) {
                let cli = doc
                    .get_first(schema.cli_id)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let path = doc
                    .get_first(schema.session_path)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if !cli.is_empty() && !path.is_empty() {
                    seen.insert((cli, path));
                }
            }
        }
    }
    Ok(seen.into_iter().collect())
}

/// Search result from Tantivy
#[derive(Debug, Clone)]
pub(crate) struct TantivySearchHit {
    pub session_path: String,
    pub message_index: u64,
    pub search_text: String,
    pub score: f32,
}

/// Search for sessions matching the query.
/// Uses PhraseQuery via quoted string to correctly match ngram substrings.
pub(crate) fn search_sessions(
    cli_id: &str,
    query: &str,
    limit: usize,
) -> AppResult<Vec<TantivySearchHit>> {
    let ctx_guard = get_index_context()?;
    let ctx = ctx_guard
        .as_ref()
        .ok_or_else(|| AppError::business("Search index context is unavailable"))?;
    let searcher = ctx.reader.searcher();

    // Build query: cli_id filter AND phrase query on search_text
    let cli_term = Term::from_field_text(ctx.schema.cli_id, cli_id);
    let cli_query = TermQuery::new(cli_term, IndexRecordOption::Basic);

    // For search_text: wrap in quotes to create a PhraseQuery
    // This ensures ngram tokens are matched in order (position-aware)
    // Escape backslashes to prevent breaking the quoted phrase; preserve user's double quotes
    let safe_query = query.replace('\\', "\\\\").replace('"', "\\\"");
    let phrase_str = format!("\"{}\"", safe_query);

    let query_parser =
        tantivy::query::QueryParser::for_index(&ctx.index, vec![ctx.schema.search_text]);
    let text_query = query_parser
        .parse_query(&phrase_str)
        .map_err(|e| AppError::business(format!("Query parse error: {}", e)))?;

    let combined = BooleanQuery::new(vec![
        (Occur::Must, Box::new(cli_query)),
        (Occur::Must, text_query),
    ]);

    let top_docs = searcher.search(&combined, &TopDocs::with_limit(limit * 10))?;

    let mut hits = Vec::new();
    for (score, doc_addr) in top_docs {
        let doc: TantivyDocument = searcher.doc(doc_addr)?;

        let session_path = doc
            .get_first(ctx.schema.session_path)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let message_index = doc
            .get_first(ctx.schema.message_index)
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let search_text = doc
            .get_first(ctx.schema.search_text)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        hits.push(TantivySearchHit {
            session_path,
            message_index,
            search_text,
            score,
        });
    }

    Ok(hits)
}

/// Search and return matched docs for specific session paths.
pub(crate) fn search_docs_for_paths(
    cli_id: &str,
    query: &str,
    session_paths: &[String],
) -> AppResult<Vec<TantivySearchHit>> {
    if session_paths.is_empty() || query.trim().is_empty() {
        return Ok(Vec::new());
    }

    let ctx_guard = get_index_context()?;
    let ctx = ctx_guard
        .as_ref()
        .ok_or_else(|| AppError::business("Search index context is unavailable"))?;
    let searcher = ctx.reader.searcher();

    // Build cli_id filter
    let cli_term = Term::from_field_text(ctx.schema.cli_id, cli_id);
    let cli_query = TermQuery::new(cli_term, IndexRecordOption::Basic);

    // Build session_path OR filter
    let path_queries: Vec<(Occur, Box<dyn tantivy::query::Query>)> = session_paths
        .iter()
        .map(|p| {
            let term = Term::from_field_text(ctx.schema.session_path, p);
            (
                Occur::Should,
                Box::new(TermQuery::new(term, IndexRecordOption::Basic))
                    as Box<dyn tantivy::query::Query>,
            )
        })
        .collect();
    let paths_query = BooleanQuery::new(path_queries);

    // Build text phrase query
    let safe_query = query.replace('\\', "\\\\").replace('"', "\\\"");
    let phrase_str = format!("\"{}\"", safe_query);
    let query_parser =
        tantivy::query::QueryParser::for_index(&ctx.index, vec![ctx.schema.search_text]);
    let text_query = query_parser
        .parse_query(&phrase_str)
        .map_err(|e| AppError::business(format!("Query parse error: {}", e)))?;

    let combined = BooleanQuery::new(vec![
        (Occur::Must, Box::new(cli_query)),
        (Occur::Must, Box::new(paths_query)),
        (Occur::Must, text_query),
    ]);

    let top_docs = searcher.search(&combined, &TopDocs::with_limit(10_000))?;

    let mut hits = Vec::new();
    for (score, doc_addr) in top_docs {
        let doc: TantivyDocument = searcher.doc(doc_addr)?;

        let session_path = doc
            .get_first(ctx.schema.session_path)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let message_index = doc
            .get_first(ctx.schema.message_index)
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let search_text = doc
            .get_first(ctx.schema.search_text)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        hits.push(TantivySearchHit {
            session_path,
            message_index,
            search_text,
            score,
        });
    }

    Ok(hits)
}
