use crate::paths::AppPaths;
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const CURRENT_SCHEMA_VERSION: i64 = 8;

pub fn open_database(paths: &AppPaths) -> Result<Connection> {
    paths.ensure_all()?;
    let conn = Connection::open(paths.database_file())?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    migrate(&conn)?;
    Ok(conn)
}

pub fn open_database_with_seed(paths: &AppPaths, seed_path: Option<&Path>) -> Result<Connection> {
    paths.ensure_all()?;
    seed_database_if_needed(paths, seed_path)?;
    open_database(paths)
}

fn seed_database_if_needed(paths: &AppPaths, seed_path: Option<&Path>) -> Result<()> {
    let Some(seed_path) = seed_path.filter(|path| path.is_file()) else {
        return Ok(());
    };
    let database_file = paths.database_file();
    let should_seed = if !database_file.exists() {
        true
    } else {
        !database_has_journals(paths).unwrap_or(false)
    };
    if !should_seed {
        return Ok(());
    }
    if database_file.exists() {
        backup_existing_empty_database(paths, &database_file)?;
        remove_sqlite_companions(&database_file)?;
    }
    fs::copy(seed_path, &database_file)?;
    Ok(())
}

fn backup_existing_empty_database(paths: &AppPaths, database_file: &Path) -> Result<()> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let backup_dir = paths.backups.join(format!("empty_db_before_seed_{}", stamp));
    fs::create_dir_all(&backup_dir)?;
    for path in sqlite_family_paths(database_file) {
        if path.exists() {
            let file_name = path
                .file_name()
                .map(|value| value.to_owned())
                .unwrap_or_else(|| "database.sqlite3".into());
            fs::copy(&path, backup_dir.join(file_name))?;
        }
    }
    Ok(())
}

fn remove_sqlite_companions(database_file: &Path) -> Result<()> {
    for path in sqlite_family_paths(database_file) {
        if path.exists() {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn sqlite_family_paths(database_file: &Path) -> Vec<PathBuf> {
    vec![
        database_file.to_path_buf(),
        PathBuf::from(format!("{}-wal", database_file.display())),
        PathBuf::from(format!("{}-shm", database_file.display())),
    ]
}

pub fn database_has_journals(paths: &AppPaths) -> Result<bool> {
    if !paths.database_file().is_file() {
        return Ok(false);
    }
    let conn = Connection::open(paths.database_file())?;
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM journals WHERE deleted_at IS NULL",
            [],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(0);
    Ok(count > 0)
}

pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS schema_meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS journals (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            uuid TEXT NOT NULL UNIQUE,
            name TEXT NOT NULL,
            normalized_name TEXT NOT NULL UNIQUE,
            issn TEXT,
            eissn TEXT,
            source_db TEXT,
            jif_quartile TEXT,
            cas_quartile TEXT,
            cas_zone TEXT,
            impact_factor REAL,
            wos_articles REAL,
            subjects TEXT,
            publisher TEXT,
            ccf TEXT,
            ei TEXT,
            cssci TEXT,
            cscd TEXT,
            pku_core TEXT,
            ami_level TEXT,
            top_flag TEXT,
            oa_flag TEXT,
            oa_detail TEXT,
            website TEXT,
            scope_text TEXT,
            jcr_rank_detail TEXT,
            grade TEXT,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            deleted_at TEXT,
            sync_status TEXT NOT NULL DEFAULT 'local',
            version INTEGER NOT NULL DEFAULT 1
        );

        CREATE TABLE IF NOT EXISTS journal_user_profiles (
            journal_id INTEGER PRIMARY KEY,
            difficulty TEXT NOT NULL DEFAULT '未评估',
            recommendation_preference TEXT NOT NULL DEFAULT '正常推荐',
            apc TEXT,
            review_cycle TEXT,
            tags TEXT,
            summary TEXT,
            experience TEXT,
            rejection_reason TEXT,
            suitable_topics TEXT,
            avoid_reason TEXT,
            index_dirty INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            deleted_at TEXT,
            sync_status TEXT NOT NULL DEFAULT 'local',
            version INTEGER NOT NULL DEFAULT 1,
            FOREIGN KEY(journal_id) REFERENCES journals(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS submissions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            uuid TEXT NOT NULL UNIQUE,
            title TEXT NOT NULL,
            authors TEXT,
            corresponding TEXT,
            journal_id INTEGER,
            journal_name TEXT,
            submit_date TEXT,
            status TEXT NOT NULL DEFAULT '准备中',
            result_date TEXT,
            apc_paid TEXT,
            website TEXT,
            notes TEXT,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            deleted_at TEXT,
            sync_status TEXT NOT NULL DEFAULT 'local',
            version INTEGER NOT NULL DEFAULT 1,
            FOREIGN KEY(journal_id) REFERENCES journals(id) ON DELETE SET NULL
        );

        CREATE TABLE IF NOT EXISTS rag_chunks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            journal_id INTEGER NOT NULL,
            chunk_type TEXT NOT NULL,
            content TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            embedding_json TEXT,
            embedding_status TEXT NOT NULL DEFAULT 'dirty',
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY(journal_id) REFERENCES journals(id) ON DELETE CASCADE,
            UNIQUE(journal_id, chunk_type, content_hash)
        );

        CREATE TABLE IF NOT EXISTS resource_imports (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            uuid TEXT NOT NULL UNIQUE,
            file_name TEXT NOT NULL,
            stored_path TEXT NOT NULL,
            resource_type TEXT,
            column_map_json TEXT,
            notes TEXT,
            imported_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS app_settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            is_secret INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS index_health (
            key TEXT PRIMARY KEY,
            status TEXT NOT NULL,
            detail TEXT,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS journal_enrichment_sources (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            journal_id INTEGER NOT NULL,
            field_name TEXT NOT NULL,
            source_kind TEXT NOT NULL,
            source_url TEXT,
            fetched_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            status TEXT NOT NULL,
            note TEXT,
            content_hash TEXT,
            FOREIGN KEY(journal_id) REFERENCES journals(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_journals_name ON journals(normalized_name);
        CREATE INDEX IF NOT EXISTS idx_journals_issn ON journals(issn);
        CREATE INDEX IF NOT EXISTS idx_journals_eissn ON journals(eissn);
        CREATE INDEX IF NOT EXISTS idx_journals_source ON journals(source_db);
        CREATE INDEX IF NOT EXISTS idx_journals_subjects ON journals(subjects);
        CREATE INDEX IF NOT EXISTS idx_journals_jif ON journals(jif_quartile);
        CREATE INDEX IF NOT EXISTS idx_journals_cas ON journals(cas_quartile, cas_zone);
        CREATE INDEX IF NOT EXISTS idx_journals_ccf ON journals(ccf);
        CREATE INDEX IF NOT EXISTS idx_journals_ei ON journals(ei);
        CREATE INDEX IF NOT EXISTS idx_user_profiles_pref ON journal_user_profiles(recommendation_preference, difficulty);
        CREATE INDEX IF NOT EXISTS idx_rag_chunks_journal ON rag_chunks(journal_id, chunk_type, embedding_status);
        CREATE INDEX IF NOT EXISTS idx_enrichment_journal ON journal_enrichment_sources(journal_id, field_name, fetched_at);
        "#,
    )?;

    ensure_column(conn, "journals", "grade", "TEXT")?;
    ensure_column(conn, "journals", "oa_detail", "TEXT")?;
    ensure_column(conn, "journals", "jcr_rank_detail", "TEXT")?;
    ensure_column(conn, "rag_chunks", "embedding_json", "TEXT")?;
    ensure_column(conn, "journal_user_profiles", "rejection_reason", "TEXT")?;
    ensure_column(conn, "journal_user_profiles", "suitable_topics", "TEXT")?;
    ensure_column(conn, "journal_user_profiles", "avoid_reason", "TEXT")?;
    ensure_column(conn, "journal_user_profiles", "article_topics", "TEXT")?;
    ensure_column(
        conn,
        "journal_user_profiles",
        "submission_guidelines",
        "TEXT",
    )?;
    try_create_fts(conn);
    conn.execute(
        "INSERT OR REPLACE INTO schema_meta (key, value) VALUES ('schema_version', ?1)",
        params![CURRENT_SCHEMA_VERSION.to_string()],
    )?;
    Ok(())
}

fn ensure_column(conn: &Connection, table: &str, column: &str, definition: &str) -> Result<()> {
    let mut statement = conn.prepare(&format!("PRAGMA table_info({})", table))?;
    let columns = statement.query_map([], |row| row.get::<_, String>(1))?;
    let exists = columns
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .any(|name| name == column);
    if !exists {
        conn.execute(
            &format!("ALTER TABLE {} ADD COLUMN {} {}", table, column, definition),
            [],
        )?;
    }
    Ok(())
}

fn try_create_fts(conn: &Connection) {
    let _ = conn.execute_batch(
        r#"
        CREATE VIRTUAL TABLE IF NOT EXISTS journal_fts USING fts5(
            journal_name,
            subjects,
            scope_text,
            user_summary,
            user_experience,
            content=''
        );
        "#,
    );
}

pub fn schema_version(conn: &Connection) -> Result<i64> {
    let value: String = conn.query_row(
        "SELECT value FROM schema_meta WHERE key='schema_version'",
        [],
        |row| row.get(0),
    )?;
    Ok(value.parse().unwrap_or(0))
}
