use rusqlite::Connection;
use std::path::PathBuf;
use toukan_robot_core::paths::AppPaths;

fn main() -> anyhow::Result<()> {
    let seed = AppPaths::discover()
        .db
        .join("seed_data")
        .join("toukan_robot.sqlite3");
    if !seed.is_file() {
        anyhow::bail!("种子数据库不存在：{}", seed.display());
    }
    assert_seed_path(&seed)?;
    let conn = Connection::open(&seed)?;
    conn.execute_batch(
        r#"
        PRAGMA foreign_keys=OFF;
        DELETE FROM app_settings;
        DELETE FROM submissions;
        DELETE FROM resource_imports;
        DELETE FROM rag_chunks;
        DELETE FROM index_health;
        UPDATE journal_user_profiles
           SET difficulty='未评估',
               recommendation_preference='正常推荐',
               apc='',
               review_cycle='',
               tags='',
               summary='',
               experience='',
               rejection_reason='',
               suitable_topics='',
               avoid_reason='',
               index_dirty=1,
               updated_at=CURRENT_TIMESTAMP;
        INSERT OR REPLACE INTO schema_meta (key, value) VALUES ('seed_sanitized', CURRENT_TIMESTAMP);
        PRAGMA wal_checkpoint(TRUNCATE);
        VACUUM;
        PRAGMA foreign_keys=ON;
        "#,
    )?;
    let journal_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM journals WHERE deleted_at IS NULL", [], |row| {
            row.get(0)
        })?;
    let setting_count: i64 = conn.query_row("SELECT COUNT(*) FROM app_settings", [], |row| {
        row.get(0)
    })?;
    let submission_count: i64 = conn.query_row("SELECT COUNT(*) FROM submissions", [], |row| {
        row.get(0)
    })?;
    println!(
        "seed={}, journals={}, app_settings={}, submissions={}",
        seed.display(),
        journal_count,
        setting_count,
        submission_count
    );
    Ok(())
}

fn assert_seed_path(seed: &PathBuf) -> anyhow::Result<()> {
    let normalized = seed.to_string_lossy().replace('/', "\\");
    if !normalized.contains("\\data\\db\\seed_data\\toukan_robot.sqlite3") {
        anyhow::bail!("拒绝清理非种子数据库：{}", seed.display());
    }
    Ok(())
}
