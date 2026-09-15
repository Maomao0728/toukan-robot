use crate::{
    backup_restore, db,
    journal_store::{self, JournalUpsert},
    paths::AppPaths,
};
use anyhow::Result;
use chrono::Local;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacyImportReport {
    pub source_root: String,
    pub journals_imported: usize,
    pub journals_updated: usize,
    pub submissions_imported: usize,
    pub templates_copied: usize,
    pub attachments_copied: usize,
    pub backup_dir: Option<PathBuf>,
    pub report_file: Option<PathBuf>,
    pub warnings: Vec<String>,
}

pub fn planned_report(source_root: &str) -> LegacyImportReport {
    LegacyImportReport {
        source_root: source_root.to_string(),
        journals_imported: 0,
        journals_updated: 0,
        submissions_imported: 0,
        templates_copied: 0,
        attachments_copied: 0,
        backup_dir: None,
        report_file: None,
        warnings: vec!["这是迁移预览，正式迁移前会自动备份新版 data 目录。".to_string()],
    }
}

pub fn import_legacy(source_root: &Path, paths: &AppPaths) -> Result<LegacyImportReport> {
    if !source_root.exists() {
        anyhow::bail!("旧版目录不存在：{}", source_root.display());
    }
    let backup = backup_restore::create_backup(paths, "legacy_import")?;
    let conn = db::open_database(paths)?;
    let mut report = LegacyImportReport {
        source_root: source_root.display().to_string(),
        journals_imported: 0,
        journals_updated: 0,
        submissions_imported: 0,
        templates_copied: 0,
        attachments_copied: 0,
        backup_dir: Some(backup.backup_dir),
        report_file: None,
        warnings: Vec::new(),
    };

    if let Some(database) = find_legacy_database(source_root) {
        import_journals(&conn, &database, &mut report)?;
        import_submissions(&conn, &database, &mut report)?;
    } else {
        report
            .warnings
            .push("未找到旧版 journal_library.db。".to_string());
    }

    report.templates_copied += copy_candidate_directories(
        source_root,
        &["模板库", "templates", "投稿模板", "通用模板"],
        &paths.templates,
        &mut report.warnings,
    )?;
    report.attachments_copied += copy_candidate_directories(
        source_root,
        &["论文附件", "attachments", "附件", "papers"],
        &paths.attachments,
        &mut report.warnings,
    )?;

    let report_path = paths.exports.join(format!(
        "legacy_import_{}.json",
        Local::now().format("%Y%m%d_%H%M%S")
    ));
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    report.report_file = Some(report_path);
    Ok(report)
}

fn find_legacy_database(root: &Path) -> Option<PathBuf> {
    let candidates = [
        root.join("journal_library.db"),
        root.join("data").join("journal_library.db"),
    ];
    candidates.into_iter().find(|path| path.is_file())
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
        params![table],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

fn import_journals(
    destination: &Connection,
    database: &Path,
    report: &mut LegacyImportReport,
) -> Result<()> {
    let source = Connection::open_with_flags(
        database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    if !table_exists(&source, "journals")? {
        report
            .warnings
            .push("旧版数据库没有 journals 表，已跳过期刊迁移。".to_string());
        return Ok(());
    }
    let mut statement = source.prepare(
        "SELECT name, issn, eissn, source_db, jif_quartile, cas_quartile, cas_zone,
                impact_factor, subjects, publisher, wos_articles, top_flag, oa_flag,
                website, difficulty, apc, review_cycle, tags, status, summary, grade
         FROM journals",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(JournalUpsert {
            name: row.get::<_, Option<String>>(0)?.unwrap_or_default(),
            issn: text(row.get(1)?),
            eissn: text(row.get(2)?),
            source_db: text(row.get(3)?),
            jif_quartile: text(row.get(4)?),
            cas_quartile: text(row.get(5)?),
            cas_zone: text(row.get(6)?),
            impact_factor: row.get(7)?,
            wos_articles: row.get(10)?,
            subjects: text(row.get(8)?),
            publisher: text(row.get(9)?),
            ccf: String::new(),
            ei: String::new(),
            cssci: String::new(),
            cscd: String::new(),
            pku_core: String::new(),
            ami_level: String::new(),
            top_flag: text(row.get(11)?),
            oa_flag: text(row.get(12)?),
            oa_detail: String::new(),
            website: text(row.get(13)?),
            difficulty: text(row.get(14)?),
            recommendation_preference: "正常推荐".to_string(),
            apc: text(row.get(15)?),
            review_cycle: text(row.get(16)?),
            tags: text(row.get(17)?),
            summary: text(row.get(19)?),
            experience: format!(
                "旧版状态：{}；旧版个人备注：{}",
                text(row.get::<_, Option<String>>(18)?),
                text(row.get::<_, Option<String>>(19)?)
            ),
            grade: text(row.get(20)?),
        })
    })?;

    for row in rows {
        let journal = row?;
        let (id, updated) = journal_store::upsert_journal(destination, &journal)?;
        if updated {
            report.journals_updated += 1;
        } else {
            report.journals_imported += 1;
        }
        destination.execute(
            "UPDATE journal_user_profiles SET index_dirty=1, updated_at=CURRENT_TIMESTAMP WHERE journal_id=?1",
            params![id],
        )?;
    }
    Ok(())
}

fn import_submissions(
    destination: &Connection,
    database: &Path,
    report: &mut LegacyImportReport,
) -> Result<()> {
    let source = Connection::open_with_flags(
        database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    if !table_exists(&source, "submissions")? {
        report
            .warnings
            .push("旧版数据库没有 submissions 表，已跳过投稿记录迁移。".to_string());
        return Ok(());
    }
    let mut statement = source.prepare(
        "SELECT title, authors, corresponding, journal_name, submit_date, status,
                result_date, apc_paid, website, notes, created_at
         FROM submissions",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            text(row.get::<_, Option<String>>(0)?),
            text(row.get::<_, Option<String>>(1)?),
            text(row.get::<_, Option<String>>(2)?),
            text(row.get::<_, Option<String>>(3)?),
            text(row.get::<_, Option<String>>(4)?),
            text(row.get::<_, Option<String>>(5)?),
            text(row.get::<_, Option<String>>(6)?),
            text(row.get::<_, Option<String>>(7)?),
            text(row.get::<_, Option<String>>(8)?),
            text(row.get::<_, Option<String>>(9)?),
            text(row.get::<_, Option<String>>(10)?),
        ))
    })?;

    for row in rows {
        let (
            title,
            authors,
            corresponding,
            journal_name,
            submit_date,
            status,
            result_date,
            apc_paid,
            website,
            notes,
            created_at,
        ) = row?;
        if title.trim().is_empty() {
            continue;
        }
        let journal_id: Option<i64> = destination
            .query_row(
                "SELECT id FROM journals WHERE normalized_name=?1",
                params![journal_store::normalize_journal_name(&journal_name)],
                |r| r.get(0),
            )
            .optional()?;
        let exists: Option<i64> = destination
            .query_row(
                "SELECT id FROM submissions WHERE title=?1 AND COALESCE(journal_name,'')=?2 AND COALESCE(submit_date,'')=?3",
                params![title, journal_name, submit_date],
                |r| r.get(0),
            )
            .optional()?;
        if exists.is_some() {
            continue;
        }
        destination.execute(
            "INSERT INTO submissions
             (uuid,title,authors,corresponding,journal_id,journal_name,submit_date,status,
              result_date,apc_paid,website,notes,created_at,updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?13)",
            params![
                Uuid::new_v4().to_string(),
                title,
                authors,
                corresponding,
                journal_id,
                journal_name,
                submit_date,
                if status.is_empty() {
                    "准备中"
                } else {
                    &status
                },
                result_date,
                apc_paid,
                website,
                notes,
                if created_at.is_empty() {
                    Local::now().to_rfc3339()
                } else {
                    created_at
                }
            ],
        )?;
        report.submissions_imported += 1;
    }
    Ok(())
}

fn copy_named_directory(
    source: &Path,
    destination: &Path,
    warnings: &mut Vec<String>,
) -> Result<usize> {
    if !source.is_dir() {
        return Ok(0);
    }
    copy_tree_skip_existing(source, destination, warnings)
}

fn copy_candidate_directories(
    source_root: &Path,
    candidates: &[&str],
    destination: &Path,
    warnings: &mut Vec<String>,
) -> Result<usize> {
    let mut count = 0;
    for name in candidates.iter().copied() {
        let source = source_root.join(name);
        count += copy_named_directory(&source, destination, warnings)?;
    }
    Ok(count)
}

fn copy_tree_skip_existing(
    source: &Path,
    destination: &Path,
    warnings: &mut Vec<String>,
) -> Result<usize> {
    fs::create_dir_all(destination)?;
    let mut count = 0;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let from = entry.path();
        let to = destination.join(entry.file_name());
        if from.is_dir() {
            count += copy_tree_skip_existing(&from, &to, warnings)?;
        } else if to.exists() {
            warnings.push(format!("已跳过已存在文件：{}", to.display()));
        } else {
            fs::copy(from, to)?;
            count += 1;
        }
    }
    Ok(count)
}

fn text(value: Option<String>) -> String {
    value.unwrap_or_default()
}
