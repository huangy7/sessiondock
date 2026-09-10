use crate::capture::TrafficRecord;
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;

pub struct Storage {
    conn: Mutex<Connection>,
}

impl Storage {
    pub fn new(db_path: &str) -> anyhow::Result<Self> {
        if let Some(parent) = Path::new(db_path).parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS traffic (
                id          TEXT PRIMARY KEY,
                timestamp   TEXT NOT NULL,
                method      TEXT NOT NULL,
                path        TEXT NOT NULL,
                req_headers TEXT,
                req_body    TEXT,
                req_size    INTEGER NOT NULL DEFAULT 0,
                status      INTEGER,
                res_headers TEXT,
                res_body    TEXT,
                res_size    INTEGER NOT NULL DEFAULT 0,
                duration_ms INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_traffic_timestamp ON traffic(timestamp);
            ",
        )?;

        let column_names = conn
            .prepare("PRAGMA table_info(traffic)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;

        if !column_names.iter().any(|name| name == "session_id") {
            conn.execute_batch("ALTER TABLE traffic ADD COLUMN session_id TEXT;")?;
        }
        if !column_names.iter().any(|name| name == "cli_id") {
            conn.execute_batch("ALTER TABLE traffic ADD COLUMN cli_id TEXT DEFAULT 'claude';")?;
            conn.execute("UPDATE traffic SET cli_id = 'claude' WHERE cli_id IS NULL", [])?;
        }

        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_traffic_session_timestamp ON traffic(session_id, timestamp);
             CREATE INDEX IF NOT EXISTS idx_traffic_cli_timestamp ON traffic(cli_id, timestamp);",
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn insert(&self, record: &TrafficRecord) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO traffic (id, timestamp, method, path, req_headers, req_body, req_size, status, res_headers, res_body, res_size, duration_ms, session_id, cli_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                record.id,
                record.timestamp,
                record.method,
                record.path,
                record.req_headers,
                record.req_body,
                record.req_size,
                record.status,
                record.res_headers,
                record.res_body,
                record.res_size,
                record.duration_ms,
                record.session_id,
                record.cli_id,
            ],
        )?;
        Ok(())
    }
}
