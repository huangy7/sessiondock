use crate::error::{AppError, AppResult};
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};

#[derive(Debug, serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MigrationProgress {
    pub step: String,
    pub current: usize,
    pub total: usize,
}

fn legacy_db_path() -> AppResult<PathBuf> {
    Ok(super::app_data_dir()?.join("app_legacy.db"))
}

pub(crate) fn archive_file_path(cli_id: &str, session_path: &str) -> AppResult<PathBuf> {
    let mut hasher = Sha256::new();
    hasher.update(session_path.as_bytes());
    let hash = format!("{:x}", hasher.finalize());
    let data_dir = super::data_dir()?;
    Ok(data_dir
        .join("archives")
        .join(cli_id)
        .join(format!("{}.jsonl.gz", hash)))
}

/// Phase 1: Pre-pool synchronous migration.
/// Called BEFORE the connection pool is created.
/// Uses standalone rusqlite::Connection (not pooled) to avoid lock conflicts.
///
/// If app.db has user_version < 8:
///   1. Create a fresh app_v8.db with v8 schema
///   2. ATTACH old app.db and copy metadata tables
///   3. Rename: app.db -> app_legacy.db, app_v8.db -> app.db
///
/// This takes < 5 seconds because we only copy small metadata tables (~10-20MB),
/// not the 600-700MB FTS tables or 100MB BLOB data.
pub(crate) fn pre_pool_v8_migration() -> AppResult<()> {
    pre_pool_v8_migration_inner()
}

fn pre_pool_v8_migration_inner() -> AppResult<()> {
    let db_path = super::app_db_path()?;
    let legacy_path = legacy_db_path()?;
    let v8_path = db_path.with_file_name("app_v8.db");

    if !db_path.exists() {
        if v8_path.exists() && legacy_path.exists() {
            tracing::warn!("v8 pre-pool migration: completing interrupted DB swap");
            rename_db_files(&v8_path, &db_path)?;
        } else if legacy_path.exists() {
            tracing::warn!("v8 pre-pool migration: restoring interrupted legacy DB swap");
            rename_db_files(&legacy_path, &db_path)?;
        } else {
            return Ok(()); // Fresh install, no migration needed
        }
    }

    // If app_v8.db already exists from a previous interrupted migration, clean it up
    if v8_path.exists() {
        let _ = fs::remove_file(&v8_path);
    }

    // Check old DB version
    let old_version = {
        let conn = Connection::open(&db_path)
            .map_err(|e| AppError::business(format!("打开旧数据库失败: {}", e)))?;
        conn.pragma_query_value(None, "user_version", |row| row.get::<_, i32>(0))
            .unwrap_or(0)
    };

    if old_version >= 8 {
        // Already migrated. Legacy cleanup must run after the pool is ready;
        // pre-pool code must not call settings helpers that depend on GLOBAL_POOL.
        return Ok(());
    }

    tracing::info!(
        "v8 pre-pool migration: old version={}, creating new DB",
        old_version
    );

    // Create new v8 DB with fresh schema
    {
        let new_conn = Connection::open(&v8_path)
            .map_err(|e| AppError::business(format!("创建新数据库失败: {}", e)))?;

        new_conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;",
        )?;

        // Create v8 schema (no FTS tables, no jsonl_content column in archived_session_content)
        create_v8_schema(&new_conn)?;

        // Attach old DB and copy data
        let old_path_str = db_path.to_string_lossy();
        new_conn.execute_batch(&format!(
            "ATTACH DATABASE '{}' AS old_db;",
            old_path_str.replace('\'', "''")
        ))?;

        copy_metadata_tables(&new_conn)?;

        // Mark blob export as pending
        let now = chrono::Utc::now().to_rfc3339();
        new_conn.execute(
            "INSERT OR REPLACE INTO app_settings (key, value_json, updated_at) VALUES (?1, ?2, ?3)",
            params!["migration.v8_blob_export_pending", "true", &now],
        )?;

        new_conn.pragma_update(None, "user_version", 8)?;
        new_conn.execute_batch("DETACH DATABASE old_db;")?;
    }

    // Atomic file swap
    // 1. Move old DB to legacy
    rename_db_files(&db_path, &legacy_path)?;
    // 2. Move new DB to app.db
    rename_db_files(&v8_path, &db_path)?;

    tracing::info!("v8 pre-pool migration complete. New DB at {:?}", db_path);
    Ok(())
}

fn create_v8_schema(conn: &Connection) -> AppResult<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS profiles (
            cli_id TEXT NOT NULL,
            name TEXT NOT NULL,
            normalized_name TEXT NOT NULL,
            content_json TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (cli_id, normalized_name)
        );

        CREATE TABLE IF NOT EXISTS active_profiles (
            cli_id TEXT PRIMARY KEY,
            profile_name TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS favorites (
            path TEXT PRIMARY KEY,
            position INTEGER NOT NULL,
            created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS bookmarks (
            cli_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            message_index INTEGER NOT NULL,
            note TEXT,
            created_at TEXT NOT NULL,
            PRIMARY KEY (cli_id, session_id, message_index)
        );

        CREATE TABLE IF NOT EXISTS session_names (
            session_path TEXT PRIMARY KEY,
            display_name TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS session_list_index (
            cli_id TEXT NOT NULL,
            session_path TEXT NOT NULL,
            session_id TEXT NOT NULL,
            project_path TEXT,
            title TEXT,
            first_user_message TEXT,
            first_timestamp TEXT,
            last_timestamp TEXT,
            git_branch TEXT NOT NULL,
            file_size INTEGER NOT NULL,
            modified_ms INTEGER NOT NULL,
            indexed_at TEXT NOT NULL,
            PRIMARY KEY (cli_id, session_path)
        );

        CREATE TABLE IF NOT EXISTS app_settings (
            key TEXT PRIMARY KEY,
            value_json TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS report_models (
            id TEXT PRIMARY KEY,
            label TEXT NOT NULL,
            cli_family TEXT NOT NULL,
            api_model_id TEXT NOT NULL,
            sort_order INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS report_narrative_cache (
            content_hash TEXT NOT NULL,
            model_id TEXT NOT NULL,
            narrative TEXT NOT NULL,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            PRIMARY KEY (content_hash, model_id)
        );

        -- v8: archived_session_content WITHOUT jsonl_content column
        CREATE TABLE IF NOT EXISTS archived_session_content (
            cli_id TEXT NOT NULL,
            session_path TEXT NOT NULL,
            snapshot_modified_ms INTEGER NOT NULL,
            archived_at TEXT NOT NULL,
            retention_policy TEXT NOT NULL DEFAULT 'default',
            pinned_at TEXT,
            PRIMARY KEY (cli_id, session_path)
        );

        -- v8: Tantivy index sync state
        CREATE TABLE IF NOT EXISTS session_search_index_state (
            cli_id TEXT NOT NULL,
            session_path TEXT NOT NULL,
            modified_ms INTEGER NOT NULL,
            doc_count INTEGER NOT NULL,
            indexed_at TEXT NOT NULL,
            PRIMARY KEY (cli_id, session_path)
        );

        -- Indexes
        CREATE INDEX IF NOT EXISTS idx_profiles_cli_name ON profiles (cli_id, name);
        CREATE INDEX IF NOT EXISTS idx_favorites_position ON favorites (position);
        CREATE INDEX IF NOT EXISTS idx_bookmarks_cli_created ON bookmarks (cli_id, created_at);
        CREATE INDEX IF NOT EXISTS idx_session_list_index_cli_last_timestamp
            ON session_list_index (cli_id, last_timestamp DESC, first_timestamp DESC, session_path ASC);
        CREATE INDEX IF NOT EXISTS idx_report_narrative_cache_created
            ON report_narrative_cache(created_at);
        "#,
    )?;

    // Add v7 bookmark columns (some old DBs may have v4-v6 data without these)
    for col_def in &[
        "message_role TEXT",
        "message_text TEXT",
        "message_timestamp TEXT",
        "session_display_name TEXT",
    ] {
        let col_name = col_def.split_whitespace().next().unwrap();
        let columns: Vec<String> = conn
            .prepare("PRAGMA table_info(bookmarks)")
            .and_then(|mut stmt| {
                stmt.query_map([], |row| row.get::<_, String>(1))
                    .and_then(|rows| rows.collect())
            })
            .unwrap_or_default();

        if !columns.iter().any(|c| c == col_name) {
            let _ = conn.execute_batch(&format!("ALTER TABLE bookmarks ADD COLUMN {};", col_def));
        }
    }

    // archived_at column for session_list_index (from v4 migration)
    {
        let columns: Vec<String> = conn
            .prepare("PRAGMA table_info(session_list_index)")
            .and_then(|mut stmt| {
                stmt.query_map([], |row| row.get::<_, String>(1))
                    .and_then(|rows| rows.collect())
            })
            .unwrap_or_default();

        if !columns.iter().any(|c| c == "archived_at") {
            let _ =
                conn.execute_batch("ALTER TABLE session_list_index ADD COLUMN archived_at TEXT;");
        }
    }

    Ok(())
}

fn copy_metadata_tables(conn: &Connection) -> AppResult<()> {
    // Check which tables exist in the old DB
    let old_tables: Vec<String> = conn
        .prepare("SELECT name FROM old_db.sqlite_master WHERE type='table'")?
        .query_map([], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    let table_columns = |schema: &str, table: &str| -> AppResult<Vec<String>> {
        let pragma = format!("PRAGMA {schema}.table_info({table})");
        Ok(conn
            .prepare(&pragma)?
            .query_map([], |row| row.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .collect())
    };

    fn quote_ident(ident: &str) -> String {
        format!("\"{}\"", ident.replace('"', "\"\""))
    }

    let copy_table = |table: &str, excluded_cols: &[&str]| -> AppResult<()> {
        if !old_tables.iter().any(|t| t == table) {
            tracing::info!(
                "v8 migration: table {} not found in old DB, skipping",
                table
            );
            return Ok(());
        }

        let old_cols = table_columns("old_db", table)?;
        let main_cols = table_columns("main", table)?;
        let copy_cols: Vec<String> = main_cols
            .into_iter()
            .filter(|col| old_cols.iter().any(|old_col| old_col == col))
            .filter(|col| !excluded_cols.iter().any(|excluded| *excluded == col))
            .collect();

        if copy_cols.is_empty() {
            return Err(AppError::business(format!(
                "v8 migration: table {} has no compatible columns to copy",
                table
            )));
        }

        let cols = copy_cols
            .iter()
            .map(|col| quote_ident(col))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "INSERT OR IGNORE INTO main.{table} ({cols}) SELECT {cols} FROM old_db.{table}",
            table = quote_ident(table),
            cols = cols
        );
        conn.execute_batch(&sql).map_err(|e| {
            AppError::business(format!(
                "v8 migration: failed to copy table {}: {}",
                table, e
            ))
        })?;

        tracing::info!(
            "v8 migration: copied table {} ({} columns)",
            table,
            copy_cols.len()
        );
        Ok(())
    };

    copy_table("profiles", &[])?;
    copy_table("active_profiles", &[])?;
    copy_table("favorites", &[])?;
    copy_table("bookmarks", &[])?;
    copy_table("session_names", &[])?;
    copy_table("session_list_index", &[])?;
    copy_table("app_settings", &[])?;
    copy_table("report_models", &[])?;
    copy_table("report_narrative_cache", &[])?;
    // v8 moves archived jsonl_content BLOBs to files later; only copy metadata here.
    copy_table("archived_session_content", &["jsonl_content"])?;

    Ok(())
}

fn rename_db_files(from: &Path, to: &Path) -> AppResult<()> {
    if from.exists() {
        fs::rename(from, to).map_err(|e| {
            AppError::business(format!(
                "重命名数据库失败: {} -> {}: {}",
                from.display(),
                to.display(),
                e
            ))
        })?;
    }

    // Also move WAL and SHM files
    for ext in &["-wal", "-shm"] {
        let from_extra = from.with_extension(
            from.extension()
                .map(|e| format!("{}{}", e.to_string_lossy(), ext))
                .unwrap_or_else(|| ext[1..].to_string()),
        );
        let to_extra = to.with_extension(
            to.extension()
                .map(|e| format!("{}{}", e.to_string_lossy(), ext))
                .unwrap_or_else(|| ext[1..].to_string()),
        );
        if from_extra.exists() {
            let _ = fs::rename(&from_extra, &to_extra);
        }
    }

    Ok(())
}

/// Phase 2: Async BLOB export from legacy DB.
/// Called in setup hook via spawn_blocking. Emits progress events.
/// User can use the app normally during this phase.
pub(crate) fn run_v8_blob_export(app: &AppHandle) -> AppResult<()> {
    let pending = super::settings::read_setting_json::<bool>("migration.v8_blob_export_pending")?
        .unwrap_or(false);
    if !pending {
        return Ok(());
    }

    let legacy_path = legacy_db_path()?;
    if !legacy_path.exists() {
        tracing::info!("v8 blob export: legacy DB not found, marking done");
        super::settings::write_setting_json("migration.v8_blob_export_pending", &false)?;
        return Ok(());
    }

    tracing::info!("v8 blob export: starting from {:?}", legacy_path);

    let legacy_conn = Connection::open(&legacy_path)
        .map_err(|e| AppError::business(format!("打开 legacy 数据库失败: {}", e)))?;

    // Check if jsonl_content column exists
    let has_blob_col = legacy_conn
        .prepare("PRAGMA table_info(archived_session_content)")
        .and_then(|mut stmt| {
            stmt.query_map([], |row| row.get::<_, String>(1))
                .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        })
        .unwrap_or_default()
        .iter()
        .any(|c| c == "jsonl_content");

    if !has_blob_col {
        tracing::info!("v8 blob export: no jsonl_content column in legacy DB");
        super::settings::write_setting_json("migration.v8_blob_export_pending", &false)?;
        return Ok(());
    }

    // Count records with BLOB data
    let total: usize = legacy_conn
        .query_row(
            "SELECT COUNT(*) FROM archived_session_content WHERE length(jsonl_content) > 0",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    if total == 0 {
        tracing::info!("v8 blob export: no BLOBs to export");
        super::settings::write_setting_json("migration.v8_blob_export_pending", &false)?;
        return Ok(());
    }

    let mut stmt = legacy_conn.prepare(
        "SELECT cli_id, session_path, jsonl_content, retention_policy, pinned_at, archived_at, snapshot_modified_ms FROM archived_session_content WHERE length(jsonl_content) > 0",
    )?;

    struct RowData {
        cli_id: String,
        session_path: String,
        content: Vec<u8>,
        retention_policy: String,
        pinned_at: Option<String>,
        archived_at: String,
        snapshot_modified_ms: i64,
    }

    let rows = stmt.query_map([], |row| {
        Ok(RowData {
            cli_id: row.get(0)?,
            session_path: row.get(1)?,
            content: row.get(2)?,
            retention_policy: row.get(3)?,
            pinned_at: row.get(4)?,
            archived_at: row.get(5)?,
            snapshot_modified_ms: row.get(6)?,
        })
    })?;

    let mut current = 0;
    let mut failures = 0usize;
    for row_res in rows {
        let row = match row_res {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("v8 blob export: failed to read row: {}", e);
                failures += 1;
                continue;
            }
        };

        let dest = archive_file_path(&row.cli_id, &row.session_path)?;
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(AppError::from)?;
        }
        if !row.content.is_empty() {
            fs::write(&dest, &row.content).map_err(AppError::from)?;

            // sidecar 灾备副本：pin/归档时间随快照落盘（best-effort）
            let sidecar = super::archive_sidecar::ArchiveSidecar {
                cli_id: row.cli_id.clone(),
                session_path: row.session_path.clone(),
                retention_policy: row.retention_policy.clone(),
                pinned_at: row.pinned_at.clone(),
                archived_at: row.archived_at.clone(),
                snapshot_modified_ms: row.snapshot_modified_ms,
            };
            if let Err(e) = super::archive_sidecar::write_archive_sidecar(&dest, &sidecar) {
                tracing::warn!("v8 blob export: 写归档 sidecar 失败: {}", e);
            }
        }

        current += 1;
        if current % 10 == 0 || current == total {
            let _ = app.emit(
                "db-migration-progress",
                MigrationProgress {
                    step: "export_archives".to_string(),
                    current,
                    total,
                },
            );
        }
    }

    if failures > 0 || current != total {
        return Err(AppError::business(format!(
            "v8 blob export incomplete: exported={}, expected={}, failures={}",
            current, total, failures
        )));
    }

    // Mark done
    super::settings::write_setting_json("migration.v8_blob_export_pending", &false)?;

    let _ = app.emit(
        "db-migration-progress",
        MigrationProgress {
            step: "done".to_string(),
            current: 1,
            total: 1,
        },
    );

    tracing::info!("v8 blob export: complete ({} records)", current);
    Ok(())
}

/// Clean up legacy DB if blob export is complete.
fn cleanup_legacy_db_if_ready() {
    let pending = super::settings::read_setting_json::<bool>("migration.v8_blob_export_pending")
        .ok()
        .flatten()
        .unwrap_or(true);

    if pending {
        return; // Blob export not done yet
    }

    if let Ok(legacy_path) = legacy_db_path() {
        if legacy_path.exists() {
            tracing::info!("Cleaning up legacy DB: {:?}", legacy_path);
            let _ = fs::remove_file(&legacy_path);
            // Clean WAL/SHM too
            for ext in &["db-wal", "db-shm"] {
                let extra = legacy_path.with_extension(ext);
                if extra.exists() {
                    let _ = fs::remove_file(&extra);
                }
            }
        }
    }
}

/// Public entry for cleanup from lib.rs
pub(crate) fn cleanup_legacy_db() {
    cleanup_legacy_db_if_ready();
}

/// Read archived content from legacy DB (fallback during migration).
pub(crate) fn read_from_legacy_db(cli_id: &str, session_path: &str) -> AppResult<Option<Vec<u8>>> {
    use flate2::read::GzDecoder;
    use std::io::Read;

    let legacy_path = legacy_db_path()?;
    if !legacy_path.exists() {
        return Ok(None);
    }

    let conn = match Connection::open(&legacy_path) {
        Ok(c) => c,
        Err(_) => return Ok(None),
    };

    let compressed: Option<Vec<u8>> = conn
        .query_row(
            "SELECT jsonl_content FROM archived_session_content WHERE cli_id = ?1 AND session_path = ?2",
            params![cli_id, session_path],
            |row| row.get(0),
        )
        .ok();

    match compressed {
        Some(data) if !data.is_empty() => {
            let mut decoder = GzDecoder::new(&data[..]);
            let mut decompressed = Vec::new();
            decoder.read_to_end(&mut decompressed)?;
            Ok(Some(decompressed))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_metadata_tables_preserves_rows_when_old_schema_lacks_added_columns() {
        let conn = Connection::open_in_memory().unwrap();
        create_v8_schema(&conn).unwrap();
        conn.execute_batch(
            r#"
            ATTACH DATABASE ':memory:' AS old_db;

            CREATE TABLE old_db.bookmarks (
                cli_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                message_index INTEGER NOT NULL,
                note TEXT,
                created_at TEXT NOT NULL,
                PRIMARY KEY (cli_id, session_id, message_index)
            );
            INSERT INTO old_db.bookmarks
                (cli_id, session_id, message_index, note, created_at)
            VALUES
                ('claude', 'session-1', 3, 'note', '2026-06-05T00:00:00Z');

            CREATE TABLE old_db.session_list_index (
                cli_id TEXT NOT NULL,
                session_path TEXT NOT NULL,
                session_id TEXT NOT NULL,
                project_path TEXT,
                first_user_message TEXT,
                first_timestamp TEXT,
                last_timestamp TEXT,
                git_branch TEXT NOT NULL,
                file_size INTEGER NOT NULL,
                modified_ms INTEGER NOT NULL,
                indexed_at TEXT NOT NULL,
                PRIMARY KEY (cli_id, session_path)
            );
            INSERT INTO old_db.session_list_index
                (cli_id, session_path, session_id, project_path, first_user_message,
                 first_timestamp, last_timestamp, git_branch, file_size, modified_ms, indexed_at)
            VALUES
                ('claude', '/tmp/session.jsonl', 'session-1', '/tmp', 'hello',
                 '2026-06-05T00:00:00Z', '2026-06-05T00:01:00Z', 'main', 42, 1000,
                 '2026-06-05T00:02:00Z');
            "#,
        )
        .unwrap();

        copy_metadata_tables(&conn).unwrap();

        let bookmark_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM bookmarks", [], |row| row.get(0))
            .unwrap();
        let session_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM session_list_index", [], |row| {
                row.get(0)
            })
            .unwrap();
        let bookmark_extra_cols: (Option<String>, Option<String>, Option<String>, Option<String>) =
            conn.query_row(
                "SELECT message_role, message_text, message_timestamp, session_display_name FROM bookmarks",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        let archived_at: Option<String> = conn
            .query_row("SELECT archived_at FROM session_list_index", [], |row| {
                row.get(0)
            })
            .unwrap();

        assert_eq!(bookmark_count, 1);
        assert_eq!(session_count, 1);
        assert_eq!(bookmark_extra_cols, (None, None, None, None));
        assert_eq!(archived_at, None);
    }
}
