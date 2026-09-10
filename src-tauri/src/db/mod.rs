use crate::cli::CliKind;
use crate::error::{AppError, AppResult};
use chrono::Utc;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, Connection};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

mod narrative_cache;
mod profiles;
mod favorites;
mod bookmarks;
mod session_index;
pub(crate) mod session_archive;
mod archive_sidecar;
mod session_search;
mod maintenance;
mod settings;
mod blocked_folders;
pub(crate) mod migration_v8;
pub(crate) mod tantivy_search;
pub(crate) mod title_resolver;

pub(crate) use blocked_folders::*;
pub(crate) use bookmarks::*;
pub(crate) use favorites::*;
pub(crate) use narrative_cache::*;
pub(crate) use profiles::*;
pub(crate) use session_archive::*;
pub(crate) use session_index::*;
pub(crate) use session_search::*;
pub(crate) use maintenance::*;
pub(crate) use settings::*;

const APP_DB_USER_VERSION: i32 = 18;

pub(crate) fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

static GLOBAL_POOL: LazyLock<Pool<SqliteConnectionManager>> = LazyLock::new(|| {
    // Phase 1: Pre-pool v8 migration (creates fresh DB if needed)
    migration_v8::pre_pool_v8_migration().expect("Pre-pool v8 migration failed");
    create_pool().expect("Failed to initialize app database pool")
});

fn create_pool() -> AppResult<Pool<SqliteConnectionManager>> {
    let path = app_db_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| AppError::business(format!("创建数据库目录失败: {}", e)))?;
    }

    let manager = SqliteConnectionManager::file(&path)
        .with_init(|conn| {
            conn.execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA busy_timeout = 5000;
                 PRAGMA synchronous = NORMAL;
                 PRAGMA foreign_keys = ON;"
            )?;
            Ok(())
        });

    let pool = Pool::builder()
        .max_size(8)
        .min_idle(Some(2))
        .build(manager)?;

    // Run schema init on a dedicated connection
    {
        let conn = pool.get()?;
        init_schema(&conn)?;
    }

    Ok(pool)
}

/// Get a connection from the pool. This never deadlocks because each call
/// gets its own connection — no nested locking.
pub(crate) fn conn() -> AppResult<r2d2::PooledConnection<SqliteConnectionManager>> {
    Ok(GLOBAL_POOL.get()?)
}

fn app_data_dir() -> AppResult<PathBuf> {
    let data = dirs::data_dir().ok_or_else(|| AppError::business("无法获取应用数据目录"))?;
    Ok(data.join("com.sessiondock.app"))
}

fn config_dir() -> AppResult<PathBuf> {
    Ok(app_data_dir()?.join("config"))
}

pub(crate) fn data_dir() -> AppResult<PathBuf> {
    Ok(app_data_dir()?.join("data"))
}

pub(crate) fn app_db_path() -> AppResult<PathBuf> {
    Ok(app_data_dir()?.join("app.db"))
}



fn init_schema(conn: &Connection) -> AppResult<()> {
    create_base_schema(conn)?;
    apply_schema_migrations(conn)
}

fn create_base_schema(conn: &Connection) -> AppResult<()> {
    conn.execute_batch(
        r#"
        PRAGMA foreign_keys = ON;

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
            cli_id TEXT NOT NULL DEFAULT 'claude',
            path TEXT NOT NULL,
            position INTEGER NOT NULL,
            created_at TEXT NOT NULL,
            PRIMARY KEY (cli_id, path)
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

        CREATE TABLE IF NOT EXISTS blocked_folders (
            path TEXT PRIMARY KEY,
            created_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_profiles_cli_name ON profiles (cli_id, name);
        CREATE INDEX IF NOT EXISTS idx_favorites_position ON favorites (position);
        CREATE INDEX IF NOT EXISTS idx_bookmarks_cli_created ON bookmarks (cli_id, created_at);
        CREATE INDEX IF NOT EXISTS idx_session_list_index_cli_last_timestamp
            ON session_list_index (cli_id, last_timestamp DESC, first_timestamp DESC, session_path ASC);
        CREATE INDEX IF NOT EXISTS idx_report_narrative_cache_created
            ON report_narrative_cache(created_at);
        "#,
    )?;

    Ok(())
}

fn current_db_user_version(conn: &Connection) -> AppResult<i32> {
    Ok(conn.pragma_query_value(None, "user_version", |row| row.get(0))?)
}

fn set_db_user_version(conn: &Connection, version: i32) -> AppResult<()> {
    Ok(conn.pragma_update(None, "user_version", version)?)
}

fn apply_schema_migrations(conn: &Connection) -> AppResult<()> {
    let mut version = current_db_user_version(conn)?;

    if version < 2 {
        version = 2;
        set_db_user_version(conn, version)?;
    }

    if version < 3 {
        migrate_to_v3_session_search_index(conn)?;
        version = 3;
        set_db_user_version(conn, version)?;
    }

    if version < 4 {
        migrate_to_v4_session_archive(conn)?;
        version = 4;
        set_db_user_version(conn, version)?;
    }

    if version < 5 {
        migrate_to_v5_archive_retention_policy(conn)?;
        version = 5;
        set_db_user_version(conn, version)?;
    }

    if version < 6 {
        migrate_to_v6_rebuild_session_search_index(conn)?;
        version = 6;
        set_db_user_version(conn, version)?;
    }

    if version < 7 {
        migrate_to_v7_bookmark_snapshot(conn)?;
        version = 7;
        set_db_user_version(conn, version)?;
    }

    if version < 8 {
        migrate_to_v8_tantivy_search_state(conn)?;
        version = 8;
        set_db_user_version(conn, version)?;
    }

    if version < 9 {
        migrate_to_v9_profile_scope_tabs(conn)?;
        version = 9;
        set_db_user_version(conn, version)?;
    }

    if version < 10 {
        migrate_to_v10_session_search_docs_repair(conn)?;
        version = 10;
        set_db_user_version(conn, version)?;
    }

    if version < 11 {
        migrate_to_v11_session_list_title(conn)?;
        version = 11;
        set_db_user_version(conn, version)?;
    }

    if version < 12 {
        migrate_to_v12_report_styles_and_conversations(conn)?;
        version = 12;
        set_db_user_version(conn, version)?;
    }

    if version < 13 {
        migrate_to_v13_quick_phrases(conn)?;
        version = 13;
        set_db_user_version(conn, version)?;
    }

    if version < 14 {
        migrate_to_v14_builtin_report_styles(conn)?;
        version = 14;
        set_db_user_version(conn, version)?;
    }

    if version < 15 {
        migrate_to_v15_builtin_template_content(conn)?;
        version = 15;
        set_db_user_version(conn, version)?;
    }

    if version < 16 {
        migrate_to_v16_profile_library_decoupling(conn)?;
        version = 16;
        set_db_user_version(conn, version)?;
    }

    if version < 17 {
        migrate_to_v17_dsh_index_rebuild(conn)?;
        version = 17;
        set_db_user_version(conn, version)?;
    }

    if version < 18 {
        migrate_to_v18_favorites_composite_identity(conn)?;
        version = 18;
        set_db_user_version(conn, version)?;
    }

    // 自愈检查：确保 profiles 表若存在则必须具备 scope 列（兼容中间开发测试库）
    let has_profiles_table: bool = conn.query_row(
        "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = 'profiles'",
        [],
        |row| row.get(0),
    )?;
    if has_profiles_table {
        let has_scope_col: bool = {
            let mut stmt = conn.prepare("PRAGMA table_info(profiles)")?;
            let names: Vec<String> = stmt
                .query_map([], |r| r.get::<_, String>(1))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            names.iter().any(|n| n == "scope")
        };
        if !has_scope_col {
            conn.execute_batch(
                r#"
                ALTER TABLE profiles ADD COLUMN scope TEXT NOT NULL DEFAULT 'global';
                CREATE INDEX IF NOT EXISTS idx_profiles_cli_scope_name ON profiles (cli_id, scope, name);
                "#,
            )?;
        }
    }

    if version != APP_DB_USER_VERSION {
        set_db_user_version(conn, APP_DB_USER_VERSION)?;
    }

    Ok(())
}

fn migrate_to_v3_session_search_index(conn: &Connection) -> AppResult<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS session_search_docs (
            cli_id TEXT NOT NULL,
            session_path TEXT NOT NULL,
            modified_ms INTEGER NOT NULL,
            message_index INTEGER NOT NULL,
            search_text TEXT NOT NULL,
            indexed_at TEXT NOT NULL,
            PRIMARY KEY (cli_id, session_path, message_index)
        );

        CREATE INDEX IF NOT EXISTS idx_session_search_docs_cli_session
            ON session_search_docs (cli_id, session_path, message_index ASC);
        CREATE INDEX IF NOT EXISTS idx_session_search_docs_cli_modified
            ON session_search_docs (cli_id, modified_ms DESC, session_path ASC);

        CREATE VIRTUAL TABLE IF NOT EXISTS session_search_docs_fts USING fts5(
            search_text,
            cli_id UNINDEXED,
            session_path UNINDEXED,
            message_index UNINDEXED,
            tokenize='trigram'
        );
        "#,
    )?;

    Ok(())
}

fn migrate_to_v4_session_archive(conn: &Connection) -> AppResult<()> {
    let _ = conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS archived_session_content (
            cli_id TEXT NOT NULL,
            session_path TEXT NOT NULL,
            jsonl_content BLOB NOT NULL,
            snapshot_modified_ms INTEGER NOT NULL,
            archived_at TEXT NOT NULL,
            retention_policy TEXT NOT NULL DEFAULT 'default',
            pinned_at TEXT,
            PRIMARY KEY (cli_id, session_path)
        );
        "#,
    );

    let _ = conn.execute_batch("ALTER TABLE session_list_index ADD COLUMN archived_at TEXT;");

    Ok(())
}

fn migrate_to_v5_archive_retention_policy(conn: &Connection) -> AppResult<()> {
    let _ = conn.execute_batch(
        "ALTER TABLE archived_session_content ADD COLUMN retention_policy TEXT NOT NULL DEFAULT 'default';",
    );
    let _ = conn.execute_batch(
        "ALTER TABLE archived_session_content ADD COLUMN pinned_at TEXT;",
    );
    Ok(())
}

fn migrate_to_v6_rebuild_session_search_index(conn: &Connection) -> AppResult<()> {
    conn.execute_batch(
        r#"
        DELETE FROM session_search_docs;
        DELETE FROM session_search_docs_fts;
        "#,
    )?;

    Ok(())
}

fn migrate_to_v7_bookmark_snapshot(conn: &Connection) -> AppResult<()> {
    let columns: Vec<String> = conn
        .prepare("PRAGMA table_info(bookmarks)")
        .and_then(|mut stmt| {
            stmt.query_map([], |row| row.get::<_, String>(1))
                .and_then(|rows| rows.collect())
        })
        .unwrap_or_default();

    let new_cols = [
        "message_role TEXT",
        "message_text TEXT",
        "message_timestamp TEXT",
        "session_display_name TEXT",
    ];
    for col_def in &new_cols {
        let col_name = col_def.split_whitespace().next().unwrap();
        if !columns.iter().any(|c| c == col_name) {
            conn.execute_batch(&format!("ALTER TABLE bookmarks ADD COLUMN {};", col_def))?;
        }
    }
    Ok(())
}

fn migrate_to_v8_tantivy_search_state(conn: &Connection) -> AppResult<()> {
    // Create session_search_index_state table for Tantivy index sync tracking
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS session_search_index_state (
            cli_id TEXT NOT NULL,
            session_path TEXT NOT NULL,
            modified_ms INTEGER NOT NULL,
            doc_count INTEGER NOT NULL,
            indexed_at TEXT NOT NULL,
            PRIMARY KEY (cli_id, session_path)
        );
        "#,
    )?;

    Ok(())
}

fn migrate_to_v9_profile_scope_tabs(conn: &Connection) -> AppResult<()> {
    conn.execute_batch(
        r#"
        -- 1. 创建新表 profile_tabs
        CREATE TABLE IF NOT EXISTS profile_tabs (
            id TEXT PRIMARY KEY,
            cli_id TEXT NOT NULL,
            name TEXT NOT NULL,
            sort_order INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        -- 2. 创建新表 profile_tab_dirs
        CREATE TABLE IF NOT EXISTS profile_tab_dirs (
            tab_id TEXT NOT NULL,
            dir_path TEXT NOT NULL,
            PRIMARY KEY (tab_id, dir_path),
            FOREIGN KEY (tab_id) REFERENCES profile_tabs(id) ON DELETE CASCADE
        );

        -- 3. 升级 profiles 表结构（添加 scope 列并更新主键约束）
        ALTER TABLE profiles RENAME TO temp_profiles;

        CREATE TABLE profiles (
            cli_id TEXT NOT NULL,
            name TEXT NOT NULL,
            normalized_name TEXT NOT NULL,
            content_json TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            scope TEXT NOT NULL DEFAULT 'global',
            PRIMARY KEY (cli_id, scope, normalized_name)
        );

        INSERT INTO profiles (cli_id, name, normalized_name, content_json, created_at, updated_at, scope)
        SELECT cli_id, name, normalized_name, content_json, created_at, updated_at, 'global'
        FROM temp_profiles;

        DROP TABLE temp_profiles;

        -- 4. 升级 active_profiles 表结构（添加 scope 列并更新主键约束）
        ALTER TABLE active_profiles RENAME TO temp_active_profiles;

        CREATE TABLE active_profiles (
            cli_id TEXT NOT NULL,
            profile_name TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            scope TEXT NOT NULL DEFAULT 'global',
            PRIMARY KEY (cli_id, scope)
        );

        INSERT INTO active_profiles (cli_id, profile_name, updated_at, scope)
        SELECT cli_id, profile_name, updated_at, 'global'
        FROM temp_active_profiles;

        DROP TABLE temp_active_profiles;

        -- 5. 重建 profiles 索引
        CREATE INDEX IF NOT EXISTS idx_profiles_cli_name ON profiles (cli_id, name);
        "#,
    )?;
    Ok(())
}

/// v8 pre-pool 迁移路径下新建的库会跳过 v3 迁移，导致 session_search_docs
/// 从未创建（统计、清除索引、搜索构建全部报 no such table）。幂等补建。
fn migrate_to_v10_session_search_docs_repair(conn: &Connection) -> AppResult<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS session_search_docs (
            cli_id TEXT NOT NULL,
            session_path TEXT NOT NULL,
            modified_ms INTEGER NOT NULL,
            message_index INTEGER NOT NULL,
            search_text TEXT NOT NULL,
            indexed_at TEXT NOT NULL,
            PRIMARY KEY (cli_id, session_path, message_index)
        );

        CREATE INDEX IF NOT EXISTS idx_session_search_docs_cli_session
            ON session_search_docs (cli_id, session_path, message_index ASC);
        CREATE INDEX IF NOT EXISTS idx_session_search_docs_cli_modified
            ON session_search_docs (cli_id, modified_ms DESC, session_path ASC);
        "#,
    )?;
    Ok(())
}

/// v11：session_list_index 新增 title 列（承载 Provider 原生标题，与
/// first_user_message 的「清洗后首条用户消息」语义分离）。
/// 同时清空列表索引与搜索索引，强制用新版扫描器全量重建一次：
/// 旧行的 30 字截断标题/未清洗文本格式已废弃，且 tool_result 过滤门禁
/// 移除后搜索文档需要按新提取器重建（tantivy 按 session_path 先删后加，无重复）。
fn migrate_to_v11_session_list_title(conn: &Connection) -> AppResult<()> {
    let columns: Vec<String> = conn
        .prepare("PRAGMA table_info(session_list_index)")
        .and_then(|mut stmt| {
            stmt.query_map([], |row| row.get::<_, String>(1))
                .and_then(|rows| rows.collect())
        })
        .unwrap_or_default();

    if !columns.iter().any(|c| c == "title") {
        conn.execute_batch("ALTER TABLE session_list_index ADD COLUMN title TEXT;")?;
    }

    conn.execute_batch(
        r#"
        DELETE FROM session_list_index;
        DELETE FROM session_search_docs;
        DELETE FROM session_search_index_state;
        "#,
    )?;
    Ok(())
}

/// v12：report_styles（周报风格库，助手窗口与旧对话框共用）
/// + assistant_conversations（助手对话历史，--resume 续聊）
pub(crate) fn migrate_to_v12_report_styles_and_conversations(conn: &Connection) -> AppResult<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS report_styles (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            content TEXT NOT NULL,
            is_default INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS assistant_conversations (
            id TEXT PRIMARY KEY,
            claude_session_id TEXT NOT NULL,
            profile TEXT NOT NULL DEFAULT '',
            title TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        "#,
    )?;
    Ok(())
}

/// v14：report_styles 加 is_builtin 列并 seed 内置模板（固定 id 幂等；不可删改）
pub(crate) fn migrate_to_v14_builtin_report_styles(conn: &Connection) -> AppResult<()> {
    // SQLite 无 ADD COLUMN IF NOT EXISTS：先查表结构
    let has_col = {
        let mut stmt = conn.prepare("PRAGMA table_info(report_styles)")?;
        let names: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        names.iter().any(|n| n == "is_builtin")
    };
    if !has_col {
        conn.execute_batch(
            "ALTER TABLE report_styles ADD COLUMN is_builtin INTEGER NOT NULL DEFAULT 0;",
        )?;
    }
    let now = now_rfc3339();
    // 固定 id + INSERT OR IGNORE：迁移重跑/重复执行不重复 seed
    conn.execute(
        "INSERT OR IGNORE INTO report_styles (id, name, content, is_default, is_builtin, created_at, updated_at) \
         VALUES ('builtin-standard', '内置模板', ?1, 0, 1, ?2, ?3)",
        params![BUILTIN_TEMPLATE_CONTENT, now, now],
    )?;
    // 尚无默认风格时，内置模板兜底为默认
    conn.execute(
        "UPDATE report_styles SET is_default = 1 WHERE id = 'builtin-standard' \
         AND NOT EXISTS (SELECT 1 FROM report_styles WHERE is_default = 1)",
        [],
    )?;
    Ok(())
}

/// 内置模板的完整内容（要求 + 输出结构；与前端 weekly-report-prompt.ts 的 REPORT_GUIDANCE 同文）
const BUILTIN_TEMPLATE_CONTENT: &str = "要求：
- 严格基于会话事实，不要编造
- 先写 1-5 句话的整体总括，再按项目分组整理，不要按时间顺序流水账
- 同一件事在多个会话里出现时合并表达，不要重复
- 每个项目下归并成 2-4 条真正值得汇报的工作项，每条写成\"做了什么 + 围绕什么问题/目标 + 结果/当前进展\"
- 保留因果链（为什么做 → 做了什么 → 结果/状态）
- 不要输出空的项目标题
- 优先把代码实现层表达上卷为事项层表达；不堆函数名、文件名、commit 数、评论条数、耗时分钟数
- 语言简洁、客观、专业，适合管理视角阅读
- 有明确的阻塞、风险、待验证项可自然写在对应工作项句尾；没有就不要硬凑\"问题\"章节

输出结构（严格遵守）：
## 工作概览
[1-5 句话，概括本周主要投入方向、关键进展与总体状态，不要简单重复下面细项]

## 主要工作

### [项目名]
- [工作项 1：1-2 句话，体现动作、对象、结果或状态]
- [工作项 2]";

/// v15：修正早期 v14 的 seed——「标准结构」（空内容）更新为「内置模板」（完整内容），删除「表格模板」
pub(crate) fn migrate_to_v15_builtin_template_content(conn: &Connection) -> AppResult<()> {
    let now = now_rfc3339();
    conn.execute(
        "UPDATE report_styles SET name = '内置模板', content = ?1, updated_at = ?2 \
         WHERE id = 'builtin-standard' AND is_builtin = 1",
        params![BUILTIN_TEMPLATE_CONTENT, now],
    )?;
    conn.execute(
        "DELETE FROM report_styles WHERE id = 'builtin-table' AND is_builtin = 1",
        [],
    )?;
    Ok(())
}

/// v16：Profile 库与 Tab 解耦
/// profiles 表移除 scope 列，改为 (cli_id, normalized_name) 全局唯一；
/// 跨 scope 重名 profile 进行去重与合并：相同内容合并为 1 条，不同内容重命名为 `name (来自 <tab_name>)`；
/// active_profiles 同步更新重命名后的 profile_name。
pub(crate) fn migrate_to_v16_profile_library_decoupling(conn: &Connection) -> AppResult<()> {
    conn.execute_batch("SAVEPOINT v16_profile_decoupling;")?;

    let res = (|| -> AppResult<()> {
        let has_profiles_table: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = 'profiles'",
            [],
            |row| row.get(0),
        )?;

        if !has_profiles_table {
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS profiles (
                    cli_id TEXT NOT NULL,
                    scope TEXT NOT NULL DEFAULT 'global',
                    name TEXT NOT NULL,
                    normalized_name TEXT NOT NULL,
                    content_json TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    PRIMARY KEY (cli_id, scope, normalized_name)
                );
                CREATE INDEX IF NOT EXISTS idx_profiles_cli_scope_name ON profiles (cli_id, scope, name);
                "#,
            )?;
            return Ok(());
        }

        let has_scope_col: bool = {
            let mut stmt = conn.prepare("PRAGMA table_info(profiles)")?;
            let names: Vec<String> = stmt
                .query_map([], |r| r.get::<_, String>(1))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            names.iter().any(|n| n == "scope")
        };

        if !has_scope_col {
            conn.execute_batch(
                r#"
                ALTER TABLE profiles ADD COLUMN scope TEXT NOT NULL DEFAULT 'global';
                CREATE INDEX IF NOT EXISTS idx_profiles_cli_scope_name ON profiles (cli_id, scope, name);
                "#,
            )?;
            return Ok(());
        }

        let has_tabs_table: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = 'profile_tabs'",
            [],
            |row| row.get(0),
        )?;

        let mut tab_names: HashMap<String, String> = HashMap::new();
        if has_tabs_table {
            let mut stmt = conn.prepare("SELECT id, name FROM profile_tabs")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for r in rows {
                let (id, name) = r?;
                tab_names.insert(id, name);
            }
        }

        struct OldProfile {
            cli_id: String,
            scope: String,
            name: String,
            normalized_name: String,
            content_json: String,
            created_at: String,
            updated_at: String,
        }

        let mut stmt = conn.prepare(
            "SELECT cli_id, scope, name, normalized_name, content_json, created_at, updated_at \
             FROM profiles \
             ORDER BY cli_id ASC, CASE WHEN scope = 'global' THEN 0 ELSE 1 END, created_at ASC, rowid ASC",
        )?;

        let old_profiles: Vec<OldProfile> = stmt
            .query_map([], |r| {
                Ok(OldProfile {
                    cli_id: r.get(0)?,
                    scope: r.get(1)?,
                    name: r.get(2)?,
                    normalized_name: r.get(3)?,
                    content_json: r.get(4)?,
                    created_at: r.get(5)?,
                    updated_at: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut saved_profiles: HashMap<(String, String), (String, String, String, String)> = HashMap::new();
        let mut active_remap: HashMap<(String, String, String), String> = HashMap::new();

        for row in old_profiles {
            let cli_id = row.cli_id;
            let scope = row.scope;
            let name = row.name;
            let norm = if row.normalized_name.is_empty() {
                name.trim().to_lowercase()
            } else {
                row.normalized_name
            };
            let content = row.content_json;
            let created_at = row.created_at;
            let updated_at = row.updated_at;

            let key = (cli_id.clone(), norm.clone());
            if let Some(existing) = saved_profiles.get(&key) {
                if values_equivalent(&existing.1, &content) {
                    active_remap.insert((cli_id.clone(), scope.clone(), norm.clone()), existing.0.clone());
                } else {
                    let tab_display = tab_names
                        .get(&scope)
                        .cloned()
                        .unwrap_or_else(|| if scope == "global" { "全局".to_string() } else { scope.clone() });

                    let mut candidate_name = format!("{} (来自 {})", name, tab_display);
                    let mut candidate_norm = candidate_name.trim().to_lowercase();
                    let mut counter = 2;

                    while let Some(existing_cand) = saved_profiles.get(&(cli_id.clone(), candidate_norm.clone())) {
                        if values_equivalent(&existing_cand.1, &content) {
                            break;
                        }
                        candidate_name = format!("{} (来自 {} {})", name, tab_display, counter);
                        candidate_norm = candidate_name.trim().to_lowercase();
                        counter += 1;
                    }

                    if let Some(existing_cand) = saved_profiles.get(&(cli_id.clone(), candidate_norm.clone())) {
                        active_remap.insert((cli_id.clone(), scope.clone(), norm.clone()), existing_cand.0.clone());
                    } else {
                        saved_profiles.insert(
                            (cli_id.clone(), candidate_norm.clone()),
                            (candidate_name.clone(), content.clone(), created_at.clone(), updated_at.clone()),
                        );
                        active_remap.insert((cli_id.clone(), scope.clone(), norm.clone()), candidate_name);
                    }
                }
            } else {
                saved_profiles.insert(
                    key,
                    (name.clone(), content.clone(), created_at.clone(), updated_at.clone()),
                );
                active_remap.insert((cli_id.clone(), scope.clone(), norm.clone()), name.clone());
            }
        }

        conn.execute_batch(
            r#"
            DROP TABLE IF EXISTS new_profiles;
            CREATE TABLE new_profiles (
                cli_id TEXT NOT NULL,
                scope TEXT NOT NULL DEFAULT 'global',
                name TEXT NOT NULL,
                normalized_name TEXT NOT NULL,
                content_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (cli_id, scope, normalized_name)
            );
            "#,
        )?;

        {
            let mut insert_stmt = conn.prepare(
                "INSERT INTO new_profiles (cli_id, scope, name, normalized_name, content_json, created_at, updated_at) \
                 VALUES (?1, 'global', ?2, ?3, ?4, ?5, ?6)",
            )?;

            let mut entries: Vec<((String, String), (String, String, String, String))> = saved_profiles.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));

            for ((cli_id, norm), (name, content, created_at, updated_at)) in entries {
                insert_stmt.execute(params![cli_id, name, norm, content, created_at, updated_at])?;
            }
        }

        conn.execute_batch(
            r#"
            DROP TABLE profiles;
            ALTER TABLE new_profiles RENAME TO profiles;
            CREATE INDEX IF NOT EXISTS idx_profiles_cli_scope_name ON profiles (cli_id, scope, name);
            "#,
        )?;

        let has_active_table: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = 'active_profiles'",
            [],
            |row| row.get(0),
        )?;

        if has_active_table {
            let has_scope_in_active: bool = {
                let mut stmt = conn.prepare("PRAGMA table_info(active_profiles)")?;
                let names: Vec<String> = stmt
                    .query_map([], |r| r.get::<_, String>(1))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                names.iter().any(|n| n == "scope")
            };

            if has_scope_in_active {
                let mut stmt = conn.prepare("SELECT cli_id, scope, profile_name FROM active_profiles")?;
                let rows: Vec<(String, String, String)> = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;

                let now = now_rfc3339();
                for (cli_id, scope, profile_name) in rows {
                    let norm = profile_name.trim().to_lowercase();
                    if let Some(new_name) = active_remap.get(&(cli_id.clone(), scope.clone(), norm)) {
                        if new_name != &profile_name {
                            conn.execute(
                                "UPDATE active_profiles SET profile_name = ?1, updated_at = ?2 WHERE cli_id = ?3 AND scope = ?4",
                                params![new_name, now, cli_id, scope],
                            )?;
                        }
                    }
                }
            }
        }

        Ok(())
    })();

    match res {
        Ok(()) => {
            conn.execute_batch("RELEASE SAVEPOINT v16_profile_decoupling;")?;
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute_batch(
                "ROLLBACK TO SAVEPOINT v16_profile_decoupling; RELEASE SAVEPOINT v16_profile_decoupling;",
            );
            Err(e)
        }
    }
}

/// v17：DSH 列表索引语义修正后强制重建一次——旧行带「目录名标题」与
/// 「无 surface 消息的空会话」残留（扫描改为 title=None + 首条用户消息，
/// 并过滤空会话）。仅清 DSH 行，不动其它 CLI 的索引。
fn migrate_to_v17_dsh_index_rebuild(conn: &Connection) -> AppResult<()> {
    conn.execute_batch("DELETE FROM session_list_index WHERE cli_id = 'dsh';")?;
    Ok(())
}

/// v18：favorites 星标升级为 (cli_id, path) 复合身份——SQLite 无法直接改主键，
/// 建新表搬移数据后 DROP/RENAME；旧行一律归 'claude'，position/created_at 保留。
/// 幂等：已含 cli_id 列（新库走 base schema）直接跳过。
/// 整个重建序列包裹在 SAVEPOINT 中保证原子性（同 v16 模式）。
/// 崩溃恢复：早期非原子版本若在 RENAME 后、CREATE 前崩溃，数据只在
/// favorites_legacy 中，先还原回 favorites 再重建；若已进入新形状但
/// legacy 残留（DROP 前崩溃），早退时顺带清理。
pub(crate) fn migrate_to_v18_favorites_composite_identity(conn: &Connection) -> AppResult<()> {
    let table_exists = |name: &str| -> AppResult<bool> {
        Ok(conn.query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |row| row.get(0),
        )?)
    };

    if !table_exists("favorites")? && table_exists("favorites_legacy")? {
        // 崩溃发生在 RENAME 之后、CREATE 之前：数据只在 legacy 表中，先还原
        conn.execute_batch("ALTER TABLE favorites_legacy RENAME TO favorites;")?;
    }

    let has_cli_id: bool = {
        let mut stmt = conn.prepare("PRAGMA table_info(favorites)")?;
        let names: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        names.iter().any(|n| n == "cli_id")
    };
    if has_cli_id {
        // 已是新形状：清理可能残留的 legacy 表（旧版在 DROP 前崩溃的场景）
        conn.execute_batch("DROP TABLE IF EXISTS favorites_legacy;")?;
        return Ok(());
    }

    conn.execute_batch("SAVEPOINT v18_favorites_rebuild;")?;

    let res = (|| -> AppResult<()> {
        conn.execute_batch(
            r#"
            DROP TABLE IF EXISTS favorites_legacy;
            ALTER TABLE favorites RENAME TO favorites_legacy;
            CREATE TABLE favorites (
                cli_id TEXT NOT NULL DEFAULT 'claude',
                path TEXT NOT NULL,
                position INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                PRIMARY KEY (cli_id, path)
            );
            INSERT INTO favorites (cli_id, path, position, created_at)
            SELECT 'claude', path, position, created_at FROM favorites_legacy;
            DROP TABLE favorites_legacy;
            CREATE INDEX IF NOT EXISTS idx_favorites_position ON favorites (position);
            "#,
        )?;
        Ok(())
    })();

    match res {
        Ok(()) => {
            conn.execute_batch("RELEASE SAVEPOINT v18_favorites_rebuild;")?;
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute_batch(
                "ROLLBACK TO SAVEPOINT v18_favorites_rebuild; RELEASE SAVEPOINT v18_favorites_rebuild;",
            );
            Err(e)
        }
    }
}
/// v13：助手快捷用语库 + seed 内置周报短语（seed 后即为普通数据，可改可删）
pub(crate) fn migrate_to_v13_quick_phrases(conn: &Connection) -> AppResult<()> {
    // 只在首次建表时 seed：用户删光内置短语后不得因迁移重跑而复活
    let existed: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='assistant_quick_phrases'",
        [],
        |r| r.get(0),
    )?;
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS assistant_quick_phrases (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            content TEXT NOT NULL,
            sort INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        "#,
    )?;
    if existed == 0 {
        crate::assistant::quick_phrases::seed_builtin_phrases(conn)?;
    }
    Ok(())
}

pub(crate) fn init_app_db() -> AppResult<()> {
    let _conn = conn()?;
    Ok(())
}

fn read_json_file<T: DeserializeOwned>(path: &Path) -> AppResult<T> {
    let raw = fs::read_to_string(path)?;
    serde_json::from_str(&raw).map_err(AppError::from)
}

fn remove_file_and_prune(path: &Path) -> AppResult<()> {
    if path.exists() {
        fs::remove_file(path)?;
        if let Some(parent) = path.parent() {
            prune_empty_dirs(parent)?;
        }
    }
    Ok(())
}

fn prune_empty_dirs(start: &Path) -> AppResult<()> {
    let root = app_data_dir()?;
    let mut current = Some(start.to_path_buf());
    while let Some(path) = current {
        if path == root || !path.starts_with(&root) || !path.exists() {
            break;
        }

        let is_empty = fs::read_dir(&path)
            ?
            .next()
            .is_none();
        if !is_empty {
            break;
        }

        let parent = path.parent().map(Path::to_path_buf);
        fs::remove_dir(&path)?;
        current = parent;
    }
    Ok(())
}

fn values_equivalent(left: &str, right: &str) -> bool {
    let left_json = serde_json::from_str::<Value>(left);
    let right_json = serde_json::from_str::<Value>(right);
    match (left_json, right_json) {
        (Ok(left), Ok(right)) => left == right,
        _ => left.trim() == right.trim(),
    }
}

fn migrate_profile_dir(kind: CliKind, dir: &Path) -> AppResult<()> {
    if !dir.exists() {
        return Ok(());
    }

    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        ?
        .filter_map(|entry| entry.ok().map(|item| item.path()))
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect();
    entries.sort();

    for path in entries {
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let name = stem.trim();
        if name.is_empty() {
            continue;
        }

        let content =
            fs::read_to_string(&path)?;
        serde_json::from_str::<Value>(&content)
            ?;

        match read_profile_record(kind, name)? {
            Some((existing_name, existing_content)) => {
                if existing_name != name || !values_equivalent(&existing_content, &content) {
                    tracing::warn!(
                        "发现重名 legacy profile，保留数据库版本: cli={}, name={}, source={}",
                        kind.id(),
                        name,
                        path.display()
                    );
                }
            }
            None => {
                save_profile(kind, name, &content)?;
            }
        }

        remove_file_and_prune(&path)?;
    }

    prune_empty_dirs(dir)?;
    Ok(())
}

fn migrate_active_profile_file(kind: CliKind, path: &Path) -> AppResult<()> {
    if !path.exists() {
        return Ok(());
    }

    let raw = fs::read_to_string(path)?;
    let name = raw.trim();
    if name.is_empty() {
        remove_file_and_prune(path)?;
        return Ok(());
    }

    let existing = get_active_profile(kind, None)?;
    if existing.is_empty() {
        if read_profile_record(kind, name)?.is_some() {
            set_active_profile(kind, name, None)?;
        } else {
            tracing::warn!(
                "legacy 活动配置引用了不存在的 profile，已跳过: cli={}, name={}, source={}",
                kind.id(),
                name,
                path.display()
            );
        }
    } else if existing != name {
        tracing::warn!(
            "发现冲突的 legacy 活动配置，保留数据库版本: cli={}, db={}, legacy={}, source={}",
            kind.id(),
            existing,
            name,
            path.display()
        );
    }

    remove_file_and_prune(path)?;
    Ok(())
}

fn migrate_favorites_file(path: &Path) -> AppResult<()> {
    if !path.exists() {
        return Ok(());
    }

    let legacy: Vec<String> = read_json_file(path)?;
    let normalized = normalize_favorites(&legacy);
    let existing = read_favorites()?;
    if existing.is_empty() {
        write_favorites(&normalized)?;
    } else if existing != normalized {
        tracing::warn!(
            "发现冲突的 legacy 收藏数据，保留数据库版本: {}",
            path.display()
        );
    }

    remove_file_and_prune(path)?;
    Ok(())
}

fn migrate_bookmarks_file(path: &Path) -> AppResult<()> {
    if !path.exists() {
        return Ok(());
    }

    let legacy: Vec<BookmarkRecord> = normalize_bookmarks(read_json_file(path)?);
    let existing = read_all_bookmarks()?;
    if existing.is_empty() {
        write_all_bookmarks(&legacy)?;
    } else if existing != legacy {
        tracing::warn!(
            "发现冲突的 legacy 书签数据，保留数据库版本: {}",
            path.display()
        );
    }

    remove_file_and_prune(path)?;
    Ok(())
}

fn migrate_session_names_file(path: &Path) -> AppResult<()> {
    if !path.exists() {
        return Ok(());
    }

    let legacy: HashMap<String, String> = read_json_file(path)?;
    let existing = load_session_names()?;
    if existing.is_empty() {
        write_session_names(&legacy)?;
    } else if existing != legacy {
        tracing::warn!(
            "发现冲突的 legacy 会话命名数据，保留数据库版本: {}",
            path.display()
        );
    }

    remove_file_and_prune(path)?;
    Ok(())
}

fn migrate_dock_visible_file(path: &Path) -> AppResult<()> {
    if !path.exists() {
        return Ok(());
    }

    let legacy =
        fs::read_to_string(path)?;
    let legacy_value = legacy.trim() != "false";
    let existing = read_setting_json::<bool>(APP_SETTING_DOCK_VISIBLE)?;
    match existing {
        None => write_dock_visible(legacy_value)?,
        Some(current) if current != legacy_value => {
            tracing::warn!(
                "发现冲突的 legacy Dock 设置，保留数据库版本: {}",
                path.display()
            );
        }
        _ => {}
    }

    remove_file_and_prune(path)?;
    Ok(())
}

fn migrate_cli_path_overrides_file(path: &Path) -> AppResult<()> {
    if !path.exists() {
        return Ok(());
    }

    let legacy: HashMap<String, String> = read_json_file(path)?;
    let existing = read_cli_path_overrides()?;
    if existing.is_empty() {
        write_cli_path_overrides(&legacy)?;
    } else if existing != legacy {
        tracing::warn!(
            "发现冲突的 legacy CLI 路径配置，保留数据库版本: {}",
            path.display()
        );
    }

    remove_file_and_prune(path)?;
    Ok(())
}

fn run_migration_step<F>(errors: &mut Vec<String>, label: &str, op: F)
where
    F: FnOnce() -> AppResult<()>,
{
    if let Err(err) = op() {
        tracing::warn!("{}: {}", label, err);
        errors.push(format!("{}: {}", label, err));
    }
}

pub(crate) fn migrate_legacy_metadata() -> AppResult<()> {
    init_app_db()?;

    let root = app_data_dir()?;
    let cfg = config_dir()?;
    let dat = data_dir()?;
    let mut errors = Vec::new();

    for (kind, dirs) in [
        (
            CliKind::Claude,
            vec![
                cfg.join("cli").join(CliKind::Claude.id()).join("profiles"),
                cfg.join("profiles"),
                root.join("profiles"),
            ],
        ),
        (
            CliKind::Codex,
            vec![cfg.join("cli").join(CliKind::Codex.id()).join("profiles")],
        ),
    ] {
        for dir in dirs {
            let label = format!("迁移 profile 目录 {} -> {}", kind.id(), dir.display());
            run_migration_step(&mut errors, &label, || migrate_profile_dir(kind, &dir));
        }
    }

    for (kind, files) in [
        (
            CliKind::Claude,
            vec![
                cfg.join("cli")
                    .join(CliKind::Claude.id())
                    .join("active_profile"),
                cfg.join("active_profile"),
                root.join("active_profile"),
            ],
        ),
        (
            CliKind::Codex,
            vec![cfg
                .join("cli")
                .join(CliKind::Codex.id())
                .join("active_profile")],
        ),
    ] {
        for file in files {
            let label = format!("迁移活动配置 {} -> {}", kind.id(), file.display());
            run_migration_step(&mut errors, &label, || {
                migrate_active_profile_file(kind, &file)
            });
        }
    }

    for file in [dat.join("favorites.json"), root.join("favorites.json")] {
        let label = format!("迁移收藏 -> {}", file.display());
        run_migration_step(&mut errors, &label, || migrate_favorites_file(&file));
    }

    for file in [dat.join("bookmarks.json"), root.join("bookmarks.json")] {
        let label = format!("迁移书签 -> {}", file.display());
        run_migration_step(&mut errors, &label, || migrate_bookmarks_file(&file));
    }

    for file in [
        dat.join("session_names.json"),
        root.join("session_names.json"),
    ] {
        let label = format!("迁移会话命名 -> {}", file.display());
        run_migration_step(&mut errors, &label, || migrate_session_names_file(&file));
    }

    let dock_visible = cfg.join("dock_visible");
    run_migration_step(&mut errors, "迁移 Dock 设置", || {
        migrate_dock_visible_file(&dock_visible)
    });

    let cli_paths = cfg.join("cli_paths.json");
    run_migration_step(&mut errors, "迁移 CLI 路径配置", || {
        migrate_cli_path_overrides_file(&cli_paths)
    });

    if errors.is_empty() {
        Ok(())
    } else {
        Err(AppError::business(errors.join(" | ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_to_v10_creates_session_search_docs_idempotently() {
        let conn = Connection::open_in_memory().unwrap();

        migrate_to_v10_session_search_docs_repair(&conn).unwrap();

        let table_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = 'session_search_docs'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(table_exists, "session_search_docs 表应被补建");

        conn.execute(
            "INSERT INTO session_search_docs (cli_id, session_path, modified_ms, message_index, search_text, indexed_at) VALUES ('claude', '/a.jsonl', 1, 0, 'hello', '2026-07-24T00:00:00Z')",
            [],
        )
        .unwrap();

        // 幂等：重复执行不报错
        migrate_to_v10_session_search_docs_repair(&conn).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM session_search_docs", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 1, "幂等重跑不应影响已有数据");
    }

    #[test]
    fn migrate_to_v11_adds_title_column_and_wipes_indexes() {
        let conn = Connection::open_in_memory().unwrap();
        // v10 形状的旧库：session_list_index 无 title 列，含旧格式缓存行
        conn.execute_batch(
            r#"
            CREATE TABLE session_list_index (
                cli_id TEXT NOT NULL,
                session_path TEXT NOT NULL,
                session_id TEXT NOT NULL,
                project_path TEXT,
                first_user_message TEXT,
                first_timestamp TEXT,
                last_timestamp TEXT,
                git_branch TEXT NOT NULL DEFAULT '',
                file_size INTEGER NOT NULL DEFAULT 0,
                modified_ms INTEGER NOT NULL DEFAULT 0,
                indexed_at TEXT NOT NULL DEFAULT '',
                archived_at TEXT,
                PRIMARY KEY (cli_id, session_path)
            );
            CREATE TABLE session_search_docs (
                cli_id TEXT NOT NULL,
                session_path TEXT NOT NULL,
                modified_ms INTEGER NOT NULL,
                message_index INTEGER NOT NULL,
                search_text TEXT NOT NULL,
                indexed_at TEXT NOT NULL DEFAULT '',
                PRIMARY KEY (cli_id, session_path, message_index)
            );
            CREATE TABLE session_search_index_state (
                cli_id TEXT NOT NULL,
                session_path TEXT NOT NULL,
                modified_ms INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (cli_id, session_path)
            );
            INSERT INTO session_list_index (cli_id, session_path, session_id, first_user_message)
                VALUES ('claude', '/a.jsonl', 'a', '旧格式截断标题...');
            INSERT INTO session_search_docs (cli_id, session_path, modified_ms, message_index, search_text)
                VALUES ('claude', '/a.jsonl', 1, 0, 'hello');
            INSERT INTO session_search_index_state (cli_id, session_path) VALUES ('claude', '/a.jsonl');
            "#,
        )
        .unwrap();

        migrate_to_v11_session_list_title(&conn).unwrap();

        // title 列已新增
        let has_title: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM pragma_table_info('session_list_index') WHERE name = 'title'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(has_title, "session_list_index 应有 title 列");
        // 旧缓存行与搜索状态被清空，强制全量重建
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM session_list_index", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0, "旧格式列表索引应被清空");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM session_search_docs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0, "搜索文档应被清空重建");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM session_search_index_state", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0, "搜索索引状态应被清空重建");

        // 幂等：重复执行不报错
        migrate_to_v11_session_list_title(&conn).unwrap();
    }

    #[test]
    fn migrate_to_v12_creates_styles_and_conversations_idempotently() {
        let conn = Connection::open_in_memory().unwrap();

        migrate_to_v12_report_styles_and_conversations(&conn).unwrap();

        for table in ["report_styles", "assistant_conversations"] {
            let table_exists: bool = conn
                .query_row(
                    "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(table_exists, "{} 表应被创建", table);
        }

        // 幂等：重复执行不报错
        migrate_to_v12_report_styles_and_conversations(&conn).unwrap();
    }

    #[test]
    fn migrate_to_v16_decouples_profile_library_and_deduplicates() {
        let conn = Connection::open_in_memory().unwrap();
        // 1. Create v15 tables (with profile_tabs, profiles with scope, active_profiles with scope)
        conn.execute_batch(
            r#"
            CREATE TABLE profile_tabs (
                id TEXT PRIMARY KEY,
                cli_id TEXT NOT NULL,
                name TEXT NOT NULL,
                sort_order INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE profile_tab_dirs (
                tab_id TEXT NOT NULL,
                dir_path TEXT NOT NULL,
                PRIMARY KEY (tab_id, dir_path),
                FOREIGN KEY (tab_id) REFERENCES profile_tabs(id) ON DELETE CASCADE
            );

            CREATE TABLE profiles (
                cli_id TEXT NOT NULL,
                name TEXT NOT NULL,
                normalized_name TEXT NOT NULL,
                content_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                scope TEXT NOT NULL DEFAULT 'global',
                PRIMARY KEY (cli_id, scope, normalized_name)
            );

            CREATE TABLE active_profiles (
                cli_id TEXT NOT NULL,
                profile_name TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                scope TEXT NOT NULL DEFAULT 'global',
                PRIMARY KEY (cli_id, scope)
            );
            "#,
        )
        .unwrap();

        // 2. Insert test data
        conn.execute(
            "INSERT INTO profile_tabs (id, cli_id, name, sort_order, created_at, updated_at) VALUES ('tab_work', 'claude', '工作项目', 0, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO profile_tabs (id, cli_id, name, sort_order, created_at, updated_at) VALUES ('tab_personal', 'claude', '个人项目', 1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();

        // Global scope: "Default" with content {"baseUrl": "https://api.anthropic.com"}
        conn.execute(
            "INSERT INTO profiles (cli_id, name, normalized_name, content_json, created_at, updated_at, scope) VALUES ('claude', 'Default', 'default', '{\"baseUrl\": \"https://api.anthropic.com\"}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 'global')",
            [],
        )
        .unwrap();
        // Global scope: "UniqueGlobal"
        conn.execute(
            "INSERT INTO profiles (cli_id, name, normalized_name, content_json, created_at, updated_at, scope) VALUES ('claude', 'UniqueGlobal', 'uniqueglobal', '{\"baseUrl\": \"https://global.anthropic.com\"}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 'global')",
            [],
        )
        .unwrap();

        // tab_work: "Default" with identical content to global -> should merge with global "Default"
        conn.execute(
            "INSERT INTO profiles (cli_id, name, normalized_name, content_json, created_at, updated_at, scope) VALUES ('claude', 'Default', 'default', '{\"baseUrl\": \"https://api.anthropic.com\"}', '2026-01-02T00:00:00Z', '2026-01-02T00:00:00Z', 'tab_work')",
            [],
        )
        .unwrap();
        // tab_work: "WorkOnly" -> unique, should keep name "WorkOnly"
        conn.execute(
            "INSERT INTO profiles (cli_id, name, normalized_name, content_json, created_at, updated_at, scope) VALUES ('claude', 'WorkOnly', 'workonly', '{\"baseUrl\": \"https://work.anthropic.com\"}', '2026-01-02T00:00:00Z', '2026-01-02T00:00:00Z', 'tab_work')",
            [],
        )
        .unwrap();

        // tab_personal: "Default" with DIFFERENT content -> should rename to "Default (来自 个人项目)"
        conn.execute(
            "INSERT INTO profiles (cli_id, name, normalized_name, content_json, created_at, updated_at, scope) VALUES ('claude', 'Default', 'default', '{\"baseUrl\": \"https://personal.anthropic.com\"}', '2026-01-03T00:00:00Z', '2026-01-03T00:00:00Z', 'tab_personal')",
            [],
        )
        .unwrap();

        // Active profiles
        conn.execute(
            "INSERT INTO active_profiles (cli_id, profile_name, updated_at, scope) VALUES ('claude', 'Default', '2026-01-01T00:00:00Z', 'global')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO active_profiles (cli_id, profile_name, updated_at, scope) VALUES ('claude', 'Default', '2026-01-02T00:00:00Z', 'tab_work')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO active_profiles (cli_id, profile_name, updated_at, scope) VALUES ('claude', 'Default', '2026-01-03T00:00:00Z', 'tab_personal')",
            [],
        )
        .unwrap();

        // 3. Execute v16 migration
        migrate_to_v16_profile_library_decoupling(&conn).unwrap();

        // 4. Assertions
        let has_scope: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM pragma_table_info('profiles') WHERE name = 'scope'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(has_scope, "profiles 表应保留 scope 列用于双向回退兼容");

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM profiles WHERE cli_id = 'claude' AND scope = 'global'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 4, "应有 4 个去重/重命名后的 profile 且均处于 global");

        let mut stmt = conn
            .prepare("SELECT name, normalized_name FROM profiles WHERE cli_id = 'claude' ORDER BY name ASC")
            .unwrap();
        let names: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        let name_list: Vec<String> = names.iter().map(|(n, _)| n.clone()).collect();
        assert!(name_list.contains(&"Default".to_string()));
        assert!(name_list.contains(&"UniqueGlobal".to_string()));
        assert!(name_list.contains(&"WorkOnly".to_string()));
        assert!(name_list.contains(&"Default (来自 个人项目)".to_string()));

        // Check active profiles
        let global_active: String = conn
            .query_row(
                "SELECT profile_name FROM active_profiles WHERE cli_id = 'claude' AND scope = 'global'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(global_active, "Default");

        let work_active: String = conn
            .query_row(
                "SELECT profile_name FROM active_profiles WHERE cli_id = 'claude' AND scope = 'tab_work'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(work_active, "Default");

        let personal_active: String = conn
            .query_row(
                "SELECT profile_name FROM active_profiles WHERE cli_id = 'claude' AND scope = 'tab_personal'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(personal_active, "Default (来自 个人项目)");

        // 5. Test idempotency
        migrate_to_v16_profile_library_decoupling(&conn).unwrap();
        let count_after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM profiles WHERE cli_id = 'claude'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count_after, 4, "幂等重跑不应更改 profiles 数据");
    }

    #[test]
    fn test_v16_to_v15_rollback_and_downgrade_compatibility() {
        let conn = Connection::open_in_memory().unwrap();
        // Setup schema with migrations up to v16
        init_schema(&conn).unwrap();

        // 1. New version saves a profile
        conn.execute(
            "INSERT INTO profiles (cli_id, scope, name, normalized_name, content_json, created_at, updated_at) \
             VALUES ('claude', 'global', 'NewGlobalProf', 'newglobalprof', '{\"model\":\"claude-3-7-sonnet\"}', '2026-08-19T00:00:00Z', '2026-08-19T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO active_profiles (cli_id, scope, profile_name, updated_at) \
             VALUES ('claude', 'global', 'NewGlobalProf', '2026-08-19T00:00:00Z')",
            [],
        )
        .unwrap();

        // 2. Simulate old v15 binary behavior:
        // Query global profile using old v15 SQL queries with `scope = ?`
        let prof_content: String = conn
            .query_row(
                "SELECT content_json FROM profiles WHERE cli_id = ?1 AND scope = ?2 AND normalized_name = ?3",
                ["claude", "global", "newglobalprof"],
                |row| row.get(0),
            )
            .unwrap();
        assert!(prof_content.contains("claude-3-7-sonnet"));

        // Query active profile using old v15 SQL
        let active_prof: String = conn
            .query_row(
                "SELECT profile_name FROM active_profiles WHERE cli_id = ?1 AND scope = ?2",
                ["claude", "global"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(active_prof, "NewGlobalProf");

        // Old v15 binary inserts a new profile
        conn.execute(
            "INSERT INTO profiles (cli_id, scope, name, normalized_name, content_json, created_at, updated_at) \
             VALUES ('claude', 'global', 'FromOldClient', 'fromoldclient', '{\"model\":\"claude-3-5-haiku\"}', '2026-08-19T00:00:00Z', '2026-08-19T00:00:00Z')",
            [],
        )
        .unwrap();

        // Old v15 binary updates and deletes with `scope = ?`
        conn.execute(
            "UPDATE profiles SET content_json = '{\"model\":\"updated\"}' WHERE cli_id = ?1 AND scope = ?2 AND normalized_name = ?3",
            ["claude", "global", "fromoldclient"],
        )
        .unwrap();

        conn.execute(
            "DELETE FROM profiles WHERE cli_id = ?1 AND scope = ?2 AND normalized_name = ?3",
            ["claude", "global", "fromoldclient"],
        )
        .unwrap();
    }
}

