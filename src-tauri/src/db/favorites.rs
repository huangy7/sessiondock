use super::{conn, now_rfc3339};
use crate::error::AppResult;
use rusqlite::{params, Connection};
use std::collections::HashSet;

/// 旧接口（read_favorites/write_favorites 与 favorites.json 迁移）归属的默认 CLI
const DEFAULT_FAVORITE_CLI_ID: &str = "claude";

/// 复合身份条目；字段名与前端 SessionIdentity 的映射由 TS 侧处理
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct FavoriteEntry {
    pub cli_id: String,
    pub path: String,
}

pub(crate) fn normalize_favorite_entries(entries: &[FavoriteEntry]) -> Vec<FavoriteEntry> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for entry in entries {
        let cli_id = entry.cli_id.trim();
        let path = entry.path.trim();
        if cli_id.is_empty() || path.is_empty() {
            continue;
        }
        let key = (cli_id.to_string(), path.to_string());
        if seen.insert(key.clone()) {
            normalized.push(FavoriteEntry {
                cli_id: key.0,
                path: key.1,
            });
        }
    }
    normalized
}

pub(crate) fn read_favorite_entries() -> AppResult<Vec<FavoriteEntry>> {
    let conn = conn()?;
    read_favorite_entries_inner(&conn)
}

pub(crate) fn write_favorite_entries(entries: &[FavoriteEntry]) -> AppResult<()> {
    let conn = conn()?;
    write_favorite_entries_inner(&conn, entries)
}

pub(super) fn read_favorite_entries_inner(conn: &Connection) -> AppResult<Vec<FavoriteEntry>> {
    let mut stmt =
        conn.prepare("SELECT cli_id, path FROM favorites ORDER BY position ASC, cli_id ASC, path ASC")?;
    let rows = stmt.query_map([], |row| {
        Ok(FavoriteEntry {
            cli_id: row.get::<_, String>(0)?,
            path: row.get::<_, String>(1)?,
        })
    })?;

    let mut entries = Vec::new();
    for row in rows {
        entries.push(row?);
    }
    Ok(entries)
}

pub(super) fn write_favorite_entries_inner(
    conn: &Connection,
    entries: &[FavoriteEntry],
) -> AppResult<()> {
    let normalized = normalize_favorite_entries(entries);
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM favorites", [])?;

    let now = now_rfc3339();
    for (index, entry) in normalized.iter().enumerate() {
        tx.execute(
            "INSERT INTO favorites (cli_id, path, position, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![entry.cli_id, entry.path, index as i64, now],
        )?;
    }

    tx.commit()?;
    Ok(())
}

pub(crate) fn normalize_favorites(paths: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for path in paths {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_string()) {
            normalized.push(trimmed.to_string());
        }
    }
    normalized
}

/// 旧接口兼容：只读默认 CLI（claude）的星标路径
pub(crate) fn read_favorites() -> AppResult<Vec<String>> {
    let conn = conn()?;
    read_favorites_inner(&conn)
}

/// 旧接口兼容：只覆盖默认 CLI（claude）的星标行，其它 CLI 的行保留
pub(crate) fn write_favorites(paths: &[String]) -> AppResult<()> {
    let conn = conn()?;
    write_favorites_inner(&conn, paths)
}

pub(super) fn read_favorites_inner(conn: &Connection) -> AppResult<Vec<String>> {
    Ok(read_favorite_entries_inner(conn)?
        .into_iter()
        .filter(|entry| entry.cli_id == DEFAULT_FAVORITE_CLI_ID)
        .map(|entry| entry.path)
        .collect())
}

pub(super) fn write_favorites_inner(conn: &Connection, paths: &[String]) -> AppResult<()> {
    let mut entries: Vec<FavoriteEntry> = normalize_favorites(paths)
        .into_iter()
        .map(|path| FavoriteEntry {
            cli_id: DEFAULT_FAVORITE_CLI_ID.to_string(),
            path,
        })
        .collect();
    entries.extend(
        read_favorite_entries_inner(conn)?
            .into_iter()
            .filter(|entry| entry.cli_id != DEFAULT_FAVORITE_CLI_ID),
    );
    write_favorite_entries_inner(conn, &entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate_to_v18_favorites_composite_identity;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE favorites (
                cli_id TEXT NOT NULL DEFAULT 'claude',
                path TEXT NOT NULL,
                position INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                PRIMARY KEY (cli_id, path)
            );
            "#,
        )
        .unwrap();
        conn
    }

    #[test]
    fn same_path_different_cli_coexists() {
        let conn = test_conn();
        write_favorite_entries_inner(
            &conn,
            &[
                FavoriteEntry { cli_id: "claude".into(), path: "/a.jsonl".into() },
                FavoriteEntry { cli_id: "codex".into(), path: "/a.jsonl".into() },
            ],
        )
        .unwrap();
        assert_eq!(read_favorite_entries_inner(&conn).unwrap().len(), 2);
    }

    #[test]
    fn write_then_read_preserves_position_order() {
        let conn = test_conn();
        write_favorite_entries_inner(
            &conn,
            &[
                FavoriteEntry { cli_id: "codex".into(), path: "/z.jsonl".into() },
                FavoriteEntry { cli_id: "claude".into(), path: "/a.jsonl".into() },
                FavoriteEntry { cli_id: "claude".into(), path: "/m.jsonl".into() },
            ],
        )
        .unwrap();
        let entries = read_favorite_entries_inner(&conn).unwrap();
        assert_eq!(
            entries,
            vec![
                FavoriteEntry { cli_id: "codex".into(), path: "/z.jsonl".into() },
                FavoriteEntry { cli_id: "claude".into(), path: "/a.jsonl".into() },
                FavoriteEntry { cli_id: "claude".into(), path: "/m.jsonl".into() },
            ],
            "读出顺序应与写入 position 一致"
        );

        // 全量覆盖写：再写一次只剩新条目
        write_favorite_entries_inner(
            &conn,
            &[FavoriteEntry { cli_id: "claude".into(), path: "/only.jsonl".into() }],
        )
        .unwrap();
        assert_eq!(read_favorite_entries_inner(&conn).unwrap().len(), 1);
    }

    #[test]
    fn normalize_dedupes_composite_key_and_drops_empty() {
        let normalized = normalize_favorite_entries(&[
            FavoriteEntry { cli_id: "claude".into(), path: "/a.jsonl".into() },
            FavoriteEntry { cli_id: "claude".into(), path: " /a.jsonl ".into() },
            FavoriteEntry { cli_id: "codex".into(), path: "/a.jsonl".into() },
            FavoriteEntry { cli_id: "claude".into(), path: "   ".into() },
            FavoriteEntry { cli_id: " ".into(), path: "/b.jsonl".into() },
        ]);
        assert_eq!(
            normalized,
            vec![
                FavoriteEntry { cli_id: "claude".into(), path: "/a.jsonl".into() },
                FavoriteEntry { cli_id: "codex".into(), path: "/a.jsonl".into() },
            ],
            "复合键去重、去空白、去空 cli_id/path"
        );
    }

    #[test]
    fn migration_assigns_claude_to_legacy_rows_and_keeps_position() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE favorites (
                path TEXT PRIMARY KEY,
                position INTEGER NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE INDEX idx_favorites_position ON favorites (position);
            INSERT INTO favorites (path, position, created_at) VALUES ('/b.jsonl', 1, '2026-01-01T00:00:00Z');
            INSERT INTO favorites (path, position, created_at) VALUES ('/a.jsonl', 0, '2026-01-01T00:00:00Z');
            "#,
        )
        .unwrap();

        migrate_to_v18_favorites_composite_identity(&conn).unwrap();

        let entries = read_favorite_entries_inner(&conn).unwrap();
        assert_eq!(
            entries,
            vec![
                FavoriteEntry { cli_id: "claude".into(), path: "/a.jsonl".into() },
                FavoriteEntry { cli_id: "claude".into(), path: "/b.jsonl".into() },
            ],
            "旧行应归入 claude 且保持 position 顺序"
        );

        // 迁移后索引仍存在
        let has_index: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type = 'index' AND name = 'idx_favorites_position'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(has_index, "idx_favorites_position 应被重建");

        // 幂等：重复执行不报错、数据不变
        migrate_to_v18_favorites_composite_identity(&conn).unwrap();
        assert_eq!(read_favorite_entries_inner(&conn).unwrap().len(), 2);
    }

    /// 崩溃恢复：上次迁移在 DROP legacy 之前崩溃，favorites（旧形状）与
    /// favorites_legacy 残留共存——迁移应仍能成功，且以 favorites 数据为准。
    #[test]
    fn migration_succeeds_with_stale_legacy_table() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE favorites (
                path TEXT PRIMARY KEY,
                position INTEGER NOT NULL,
                created_at TEXT NOT NULL
            );
            INSERT INTO favorites (path, position, created_at) VALUES ('/a.jsonl', 0, '2026-01-01T00:00:00Z');
            CREATE TABLE favorites_legacy (
                path TEXT PRIMARY KEY,
                position INTEGER NOT NULL,
                created_at TEXT NOT NULL
            );
            INSERT INTO favorites_legacy (path, position, created_at) VALUES ('/stale.jsonl', 0, '2026-01-01T00:00:00Z');
            "#,
        )
        .unwrap();

        migrate_to_v18_favorites_composite_identity(&conn).unwrap();

        assert_eq!(
            read_favorite_entries_inner(&conn).unwrap(),
            vec![FavoriteEntry { cli_id: "claude".into(), path: "/a.jsonl".into() }],
            "应以 favorites 数据为准，stale legacy 被清理"
        );
        let legacy_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = 'favorites_legacy'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!legacy_exists, "favorites_legacy 残留应被清理");
    }

    /// 崩溃恢复：上次迁移在 RENAME 之后、CREATE 之前崩溃，favorites 表不存在、
    /// 数据只在 favorites_legacy 中——迁移应先还原再重建，数据不丢。
    #[test]
    fn migration_recovers_when_only_legacy_table_survives() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE favorites_legacy (
                path TEXT PRIMARY KEY,
                position INTEGER NOT NULL,
                created_at TEXT NOT NULL
            );
            INSERT INTO favorites_legacy (path, position, created_at) VALUES ('/a.jsonl', 0, '2026-01-01T00:00:00Z');
            INSERT INTO favorites_legacy (path, position, created_at) VALUES ('/b.jsonl', 1, '2026-01-01T00:00:00Z');
            "#,
        )
        .unwrap();

        migrate_to_v18_favorites_composite_identity(&conn).unwrap();

        assert_eq!(
            read_favorite_entries_inner(&conn).unwrap(),
            vec![
                FavoriteEntry { cli_id: "claude".into(), path: "/a.jsonl".into() },
                FavoriteEntry { cli_id: "claude".into(), path: "/b.jsonl".into() },
            ],
            "legacy 中的数据应被还原并迁移，不丢行"
        );
    }

    /// 已是新形状但 legacy 残留（旧版在 DROP 前崩溃）：早退路径顺带清理。
    #[test]
    fn migration_cleans_stale_legacy_when_already_migrated() {
        let conn = test_conn();
        write_favorite_entries_inner(
            &conn,
            &[FavoriteEntry { cli_id: "codex".into(), path: "/x.jsonl".into() }],
        )
        .unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE favorites_legacy (
                path TEXT PRIMARY KEY,
                position INTEGER NOT NULL,
                created_at TEXT NOT NULL
            );
            "#,
        )
        .unwrap();

        migrate_to_v18_favorites_composite_identity(&conn).unwrap();

        assert_eq!(
            read_favorite_entries_inner(&conn).unwrap(),
            vec![FavoriteEntry { cli_id: "codex".into(), path: "/x.jsonl".into() }],
            "已有新数据不受影响"
        );
        let legacy_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = 'favorites_legacy'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!legacy_exists, "favorites_legacy 残留应被清理");
    }

    #[test]
    fn legacy_write_favorites_preserves_other_cli_rows() {
        let conn = test_conn();
        write_favorite_entries_inner(
            &conn,
            &[
                FavoriteEntry { cli_id: "claude".into(), path: "/a.jsonl".into() },
                FavoriteEntry { cli_id: "codex".into(), path: "/x.jsonl".into() },
            ],
        )
        .unwrap();

        write_favorites_inner(&conn, &["/b.jsonl".to_string()]).unwrap();

        let entries = read_favorite_entries_inner(&conn).unwrap();
        assert_eq!(
            entries,
            vec![
                FavoriteEntry { cli_id: "claude".into(), path: "/b.jsonl".into() },
                FavoriteEntry { cli_id: "codex".into(), path: "/x.jsonl".into() },
            ],
            "旧接口只覆盖 claude 行，其它 CLI 星标保留"
        );

        let legacy = read_favorites_inner(&conn).unwrap();
        assert_eq!(legacy, vec!["/b.jsonl".to_string()], "旧读接口只返回 claude 行");
    }
}
