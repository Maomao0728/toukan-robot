use crate::{db, paths::AppPaths};
use anyhow::Result;
use chrono::Local;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalListFilter {
    pub keyword: String,
    pub source_db: Vec<String>,
    pub jif_quartile: Vec<String>,
    pub cas_zone: Vec<String>,
    pub ccf: Vec<String>,
    pub ei_only: bool,
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

impl Default for JournalListFilter {
    fn default() -> Self {
        Self {
            keyword: String::new(),
            source_db: Vec::new(),
            jif_quartile: Vec::new(),
            cas_zone: Vec::new(),
            ccf: Vec::new(),
            ei_only: false,
            limit: 100,
            offset: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalRecord {
    pub id: i64,
    pub name: String,
    pub issn: String,
    pub eissn: String,
    pub source_db: String,
    pub jif_quartile: String,
    pub cas_quartile: String,
    pub cas_zone: String,
    pub impact_factor: Option<f64>,
    pub wos_articles: Option<f64>,
    pub subjects: String,
    pub publisher: String,
    pub ccf: String,
    pub ei: String,
    pub cssci: String,
    pub cscd: String,
    pub pku_core: String,
    pub ami_level: String,
    pub top_flag: String,
    pub oa_flag: String,
    pub oa_detail: String,
    pub website: String,
    pub scope_text: String,
    pub jcr_rank_detail: String,
    pub difficulty: String,
    pub recommendation_preference: String,
    pub apc: String,
    pub review_cycle: String,
    pub tags: String,
    pub summary: String,
    pub experience: String,
    pub rejection_reason: String,
    pub suitable_topics: String,
    pub avoid_reason: String,
    pub article_topics: String,
    pub submission_guidelines: String,
    pub grade: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalUpsert {
    pub name: String,
    pub issn: String,
    pub eissn: String,
    pub source_db: String,
    pub jif_quartile: String,
    pub cas_quartile: String,
    pub cas_zone: String,
    pub impact_factor: Option<f64>,
    pub wos_articles: Option<f64>,
    pub subjects: String,
    pub publisher: String,
    pub ccf: String,
    pub ei: String,
    pub cssci: String,
    pub cscd: String,
    pub pku_core: String,
    pub ami_level: String,
    pub top_flag: String,
    pub oa_flag: String,
    pub oa_detail: String,
    pub website: String,
    pub difficulty: String,
    pub recommendation_preference: String,
    pub apc: String,
    pub review_cycle: String,
    pub tags: String,
    pub summary: String,
    pub experience: String,
    pub grade: String,
}

pub fn normalize_journal_name(name: &str) -> String {
    name.trim()
        .replace(['\n', '\r', '\t', ' ', '\u{3000}'], "")
        .to_uppercase()
}

pub fn list_journals(paths: &AppPaths, filter: &JournalListFilter) -> Result<Vec<JournalRecord>> {
    let conn = db::open_database(paths)?;
    list_journals_conn(&conn, filter)
}

pub fn count_journals(paths: &AppPaths, filter: &JournalListFilter) -> Result<usize> {
    let conn = db::open_database(paths)?;
    let mut sql = String::from("SELECT COUNT(*) FROM journals j WHERE j.deleted_at IS NULL");
    let mut values: Vec<String> = Vec::new();
    if !filter.keyword.trim().is_empty() {
        sql.push_str(" AND (j.normalized_name LIKE ? OR UPPER(COALESCE(j.subjects,'')) LIKE ? OR UPPER(COALESCE(j.publisher,'')) LIKE ?)");
        let value = format!("%{}%", normalize_journal_name(&filter.keyword));
        values.extend([value.clone(), value.clone(), value]);
    }
    append_contains_filter(&mut sql, &mut values, "j.source_db", &filter.source_db);
    append_in_filter(
        &mut sql,
        &mut values,
        "j.jif_quartile",
        &filter.jif_quartile,
    );
    append_in_filter(&mut sql, &mut values, "j.cas_zone", &filter.cas_zone);
    append_in_filter(&mut sql, &mut values, "j.ccf", &filter.ccf);
    if filter.ei_only {
        sql.push_str(" AND COALESCE(j.ei,'') <> '' AND COALESCE(j.ei,'') <> 'No'");
    }
    let mut statement = conn.prepare(&sql)?;
    let params_vec: Vec<&dyn rusqlite::ToSql> = values
        .iter()
        .map(|value| value as &dyn rusqlite::ToSql)
        .collect();
    Ok(statement.query_row(params_vec.as_slice(), |row| row.get::<_, i64>(0))? as usize)
}

pub fn list_journals_conn(
    conn: &Connection,
    filter: &JournalListFilter,
) -> Result<Vec<JournalRecord>> {
    let mut sql = String::from(
        "SELECT j.id, j.name, COALESCE(j.issn,''), COALESCE(j.eissn,''), \
         COALESCE(j.source_db,''), COALESCE(j.jif_quartile,''), COALESCE(j.cas_quartile,''), \
         COALESCE(j.cas_zone,''), j.impact_factor, j.wos_articles, COALESCE(j.subjects,''), \
         COALESCE(j.publisher,''), COALESCE(j.ccf,''), COALESCE(j.ei,''), COALESCE(j.cssci,''), \
         COALESCE(j.cscd,''), COALESCE(j.pku_core,''), COALESCE(j.ami_level,''), \
         COALESCE(j.top_flag,''), COALESCE(j.oa_flag,''), COALESCE(j.oa_detail,''), COALESCE(j.website,''), \
         COALESCE(j.scope_text,''), COALESCE(j.jcr_rank_detail,''), \
         COALESCE(p.difficulty,'未评估'), COALESCE(p.recommendation_preference,'正常推荐'), \
         COALESCE(p.apc,''), COALESCE(p.review_cycle,''), COALESCE(p.tags,''), \
         COALESCE(p.summary,''), COALESCE(p.experience,''), COALESCE(p.rejection_reason,''), \
         COALESCE(p.suitable_topics,''), COALESCE(p.avoid_reason,''), \
         COALESCE(p.article_topics,''), COALESCE(p.submission_guidelines,''), COALESCE(j.grade,'') \
         FROM journals j \
         LEFT JOIN journal_user_profiles p ON p.journal_id=j.id \
         WHERE j.deleted_at IS NULL",
    );
    let mut values: Vec<String> = Vec::new();
    if !filter.keyword.trim().is_empty() {
        sql.push_str(" AND (j.normalized_name LIKE ? OR UPPER(COALESCE(j.subjects,'')) LIKE ? OR UPPER(COALESCE(j.publisher,'')) LIKE ?)");
        let value = format!("%{}%", normalize_journal_name(&filter.keyword));
        values.extend([value.clone(), value.clone(), value]);
    }
    append_contains_filter(&mut sql, &mut values, "j.source_db", &filter.source_db);
    append_in_filter(
        &mut sql,
        &mut values,
        "j.jif_quartile",
        &filter.jif_quartile,
    );
    append_in_filter(&mut sql, &mut values, "j.cas_zone", &filter.cas_zone);
    append_in_filter(&mut sql, &mut values, "j.ccf", &filter.ccf);
    if filter.ei_only {
        sql.push_str(" AND COALESCE(j.ei,'') <> '' AND COALESCE(j.ei,'') <> '否'");
    }
    sql.push_str(" ORDER BY j.name COLLATE NOCASE LIMIT ? OFFSET ?");
    // The radar UI requests a small page, while RAG needs the complete local catalogue.
    let limit = filter.limit.clamp(1, 50_000) as i64;
    let offset = filter.offset.min(5_000_000) as i64;

    let mut statement = conn.prepare(&sql)?;
    let mut params_vec: Vec<&dyn rusqlite::ToSql> =
        values.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
    params_vec.push(&limit);
    params_vec.push(&offset);
    let rows = statement.query_map(params_vec.as_slice(), map_journal)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn append_contains_filter(
    sql: &mut String,
    values: &mut Vec<String>,
    column: &str,
    items: &[String],
) {
    let items: Vec<String> = items
        .iter()
        .filter(|x| !x.trim().is_empty())
        .cloned()
        .collect();
    if items.is_empty() {
        return;
    }
    let predicates = std::iter::repeat(format!("COALESCE({}, '') LIKE ?", column))
        .take(items.len())
        .collect::<Vec<_>>()
        .join(" OR ");
    sql.push_str(&format!(" AND ({})", predicates));
    values.extend(items.into_iter().map(|item| format!("%{}%", item)));
}

fn append_in_filter(sql: &mut String, values: &mut Vec<String>, column: &str, items: &[String]) {
    let items: Vec<String> = items
        .iter()
        .filter(|x| !x.trim().is_empty())
        .cloned()
        .collect();
    if items.is_empty() {
        return;
    }
    let placeholders = std::iter::repeat("?")
        .take(items.len())
        .collect::<Vec<_>>()
        .join(",");
    sql.push_str(&format!(" AND {} IN ({})", column, placeholders));
    values.extend(items);
}

fn map_journal(row: &rusqlite::Row<'_>) -> rusqlite::Result<JournalRecord> {
    Ok(JournalRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        issn: row.get(2)?,
        eissn: row.get(3)?,
        source_db: row.get(4)?,
        jif_quartile: row.get(5)?,
        cas_quartile: row.get(6)?,
        cas_zone: row.get(7)?,
        impact_factor: row.get(8)?,
        wos_articles: row.get(9)?,
        subjects: row.get(10)?,
        publisher: row.get(11)?,
        ccf: row.get(12)?,
        ei: row.get(13)?,
        cssci: row.get(14)?,
        cscd: row.get(15)?,
        pku_core: row.get(16)?,
        ami_level: row.get(17)?,
        top_flag: row.get(18)?,
        oa_flag: row.get(19)?,
        oa_detail: row.get(20)?,
        website: row.get(21)?,
        scope_text: row.get(22)?,
        jcr_rank_detail: row.get(23)?,
        difficulty: row.get(24)?,
        recommendation_preference: row.get(25)?,
        apc: row.get(26)?,
        review_cycle: row.get(27)?,
        tags: row.get(28)?,
        summary: row.get(29)?,
        experience: row.get(30)?,
        rejection_reason: row.get(31)?,
        suitable_topics: row.get(32)?,
        avoid_reason: row.get(33)?,
        article_topics: row.get(34)?,
        submission_guidelines: row.get(35)?,
        grade: row.get(36)?,
    })
}

pub fn upsert_journal(conn: &Connection, journal: &JournalUpsert) -> Result<(i64, bool)> {
    let name = journal.name.trim();
    if name.is_empty() {
        anyhow::bail!("期刊名称不能为空");
    }
    let normalized = normalize_journal_name(name);
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM journals WHERE normalized_name=?1 AND deleted_at IS NULL",
            params![normalized],
            |row| row.get(0),
        )
        .optional()?;

    let id = if let Some(id) = existing {
        conn.execute(
            "UPDATE journals SET name=?1, issn=?2, eissn=?3, source_db=?4, jif_quartile=?5,
             cas_quartile=?6, cas_zone=?7, impact_factor=?8, wos_articles=?9, subjects=?10,
             publisher=?11, ccf=?12, ei=?13, cssci=?14, cscd=?15, pku_core=?16,
             ami_level=?17, top_flag=?18, oa_flag=?19, website=?20, grade=?21,
             updated_at=CURRENT_TIMESTAMP, version=version+1 WHERE id=?22",
            params![
                name,
                journal.issn,
                journal.eissn,
                journal.source_db,
                journal.jif_quartile,
                journal.cas_quartile,
                journal.cas_zone,
                journal.impact_factor,
                journal.wos_articles,
                journal.subjects,
                journal.publisher,
                journal.ccf,
                journal.ei,
                journal.cssci,
                journal.cscd,
                journal.pku_core,
                journal.ami_level,
                journal.top_flag,
                journal.oa_flag,
                journal.website,
                journal.grade,
                id
            ],
        )?;
        id
    } else {
        conn.execute(
            "INSERT INTO journals
             (uuid,name,normalized_name,issn,eissn,source_db,jif_quartile,cas_quartile,cas_zone,
              impact_factor,wos_articles,subjects,publisher,ccf,ei,cssci,cscd,pku_core,ami_level,
              top_flag,oa_flag,website,grade)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23)",
            params![
                Uuid::new_v4().to_string(), name, normalized, journal.issn, journal.eissn,
                journal.source_db, journal.jif_quartile, journal.cas_quartile, journal.cas_zone,
                journal.impact_factor, journal.wos_articles, journal.subjects, journal.publisher,
                journal.ccf, journal.ei, journal.cssci, journal.cscd, journal.pku_core,
                journal.ami_level, journal.top_flag, journal.oa_flag, journal.website, journal.grade
            ],
        )?;
        conn.last_insert_rowid()
    };

    conn.execute(
        "INSERT INTO journal_user_profiles
         (journal_id,difficulty,recommendation_preference,apc,review_cycle,tags,summary,experience)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
         ON CONFLICT(journal_id) DO UPDATE SET difficulty=excluded.difficulty,
         recommendation_preference=excluded.recommendation_preference, apc=excluded.apc,
         review_cycle=excluded.review_cycle, tags=excluded.tags, summary=excluded.summary,
         experience=excluded.experience, updated_at=CURRENT_TIMESTAMP, index_dirty=1",
        params![
            id,
            if journal.difficulty.is_empty() {
                "未评估"
            } else {
                &journal.difficulty
            },
            if journal.recommendation_preference.is_empty() {
                "正常推荐"
            } else {
                &journal.recommendation_preference
            },
            journal.apc,
            journal.review_cycle,
            journal.tags,
            journal.summary,
            journal.experience
        ],
    )?;
    mark_index_dirty(conn, id)?;
    Ok((id, existing.is_some()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalProfileUpdate {
    pub journal_id: i64,
    pub difficulty: String,
    pub recommendation_preference: String,
    pub apc: String,
    pub review_cycle: String,
    pub tags: String,
    pub website: String,
    #[serde(default)]
    pub scope_text: String,
    pub summary: String,
    pub experience: String,
    #[serde(default)]
    pub rejection_reason: String,
    #[serde(default)]
    pub suitable_topics: String,
    #[serde(default)]
    pub avoid_reason: String,
    #[serde(default)]
    pub article_topics: String,
    #[serde(default)]
    pub submission_guidelines: String,
}

pub fn update_journal_profile(paths: &AppPaths, update: &JournalProfileUpdate) -> Result<()> {
    let conn = db::open_database(paths)?;
    conn.execute(
        "UPDATE journals SET website=?1, scope_text=?2, updated_at=CURRENT_TIMESTAMP, version=version+1 WHERE id=?3 AND deleted_at IS NULL",
        params![update.website, update.scope_text, update.journal_id],
    )?;
    conn.execute(
        "INSERT INTO journal_user_profiles
         (journal_id,difficulty,recommendation_preference,apc,review_cycle,tags,summary,experience,rejection_reason,suitable_topics,avoid_reason,article_topics,submission_guidelines,index_dirty)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,1)
         ON CONFLICT(journal_id) DO UPDATE SET difficulty=excluded.difficulty,
         recommendation_preference=excluded.recommendation_preference, apc=excluded.apc,
         review_cycle=excluded.review_cycle, tags=excluded.tags, summary=excluded.summary,
         experience=excluded.experience, rejection_reason=excluded.rejection_reason,
         suitable_topics=excluded.suitable_topics, avoid_reason=excluded.avoid_reason,
         article_topics=excluded.article_topics, submission_guidelines=excluded.submission_guidelines,
         updated_at=CURRENT_TIMESTAMP, index_dirty=1,
         version=version+1",
        params![
            update.journal_id,
            if update.difficulty.is_empty() { "未评估" } else { &update.difficulty },
            if update.recommendation_preference.is_empty() { "正常推荐" } else { &update.recommendation_preference },
            update.apc,
            update.review_cycle,
            update.tags,
            update.summary,
            update.experience,
            update.rejection_reason,
            update.suitable_topics,
            update.avoid_reason,
            update.article_topics,
            update.submission_guidelines
        ],
    )?;
    mark_index_dirty(&conn, update.journal_id)?;
    Ok(())
}

pub fn set_recommendation_preference(
    paths: &AppPaths,
    journal_id: i64,
    recommendation_preference: &str,
) -> Result<()> {
    let conn = db::open_database(paths)?;
    conn.execute(
        "INSERT INTO journal_user_profiles (journal_id,recommendation_preference,index_dirty)
         VALUES (?1,?2,1)
         ON CONFLICT(journal_id) DO UPDATE SET
         recommendation_preference=excluded.recommendation_preference,
         updated_at=CURRENT_TIMESTAMP,
         index_dirty=1,
         version=version+1",
        params![
            journal_id,
            if recommendation_preference.trim().is_empty() {
                "正常推荐"
            } else {
                recommendation_preference.trim()
            }
        ],
    )?;
    mark_index_dirty(&conn, journal_id)?;
    Ok(())
}

pub fn soft_delete_journal(paths: &AppPaths, journal_id: i64) -> Result<()> {
    let conn = db::open_database(paths)?;
    conn.execute(
        "UPDATE journals SET deleted_at=CURRENT_TIMESTAMP, updated_at=CURRENT_TIMESTAMP, version=version+1 WHERE id=?1",
        params![journal_id],
    )?;
    conn.execute(
        "UPDATE journal_user_profiles SET deleted_at=CURRENT_TIMESTAMP, updated_at=CURRENT_TIMESTAMP, index_dirty=1, version=version+1 WHERE journal_id=?1",
        params![journal_id],
    )?;
    mark_index_dirty(&conn, journal_id)?;
    Ok(())
}

fn mark_index_dirty(conn: &Connection, journal_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE journal_user_profiles SET index_dirty=1, updated_at=CURRENT_TIMESTAMP WHERE journal_id=?1",
        params![journal_id],
    )?;
    conn.execute(
        "UPDATE rag_chunks SET embedding_status='dirty', updated_at=CURRENT_TIMESTAMP WHERE journal_id=?1",
        params![journal_id],
    )?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmissionListFilter {
    pub keyword: String,
    pub status: Vec<String>,
    pub limit: usize,
}

impl Default for SubmissionListFilter {
    fn default() -> Self {
        Self {
            keyword: String::new(),
            status: Vec::new(),
            limit: 100,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmissionRecord {
    pub id: i64,
    pub title: String,
    pub authors: String,
    pub corresponding: String,
    pub journal_name: String,
    pub submit_date: String,
    pub status: String,
    pub result_date: String,
    pub apc_paid: String,
    pub website: String,
    pub notes: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmissionUpsert {
    pub id: Option<i64>,
    pub title: String,
    pub authors: String,
    pub corresponding: String,
    pub journal_name: String,
    pub submit_date: String,
    pub status: String,
    pub result_date: String,
    pub apc_paid: String,
    pub website: String,
    pub notes: String,
}

pub fn list_submissions(
    paths: &AppPaths,
    filter: &SubmissionListFilter,
) -> Result<Vec<SubmissionRecord>> {
    let conn = db::open_database(paths)?;
    let mut sql = String::from(
        "SELECT id, title, COALESCE(authors,''), COALESCE(corresponding,''), \
         COALESCE(journal_name,''), COALESCE(submit_date,''), COALESCE(status,''), \
         COALESCE(result_date,''), COALESCE(apc_paid,''), COALESCE(website,''), \
         COALESCE(notes,''), COALESCE(created_at,'') FROM submissions \
         WHERE deleted_at IS NULL",
    );
    let mut values: Vec<String> = Vec::new();
    if !filter.keyword.trim().is_empty() {
        sql.push_str(
            " AND (title LIKE ? OR COALESCE(journal_name,'') LIKE ? OR COALESCE(authors,'') LIKE ? OR COALESCE(notes,'') LIKE ?)",
        );
        let value = format!("%{}%", filter.keyword.trim());
        values.extend([value.clone(), value.clone(), value.clone(), value]);
    }
    append_in_filter(&mut sql, &mut values, "status", &filter.status);
    sql.push_str(" ORDER BY id DESC LIMIT ?");
    let limit = filter.limit.clamp(1, 1000) as i64;

    let mut statement = conn.prepare(&sql)?;
    let mut params_vec: Vec<&dyn rusqlite::ToSql> =
        values.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
    params_vec.push(&limit);
    let rows = statement.query_map(params_vec.as_slice(), |row| {
        Ok(SubmissionRecord {
            id: row.get(0)?,
            title: row.get(1)?,
            authors: row.get(2)?,
            corresponding: row.get(3)?,
            journal_name: row.get(4)?,
            submit_date: row.get(5)?,
            status: row.get(6)?,
            result_date: row.get(7)?,
            apc_paid: row.get(8)?,
            website: row.get(9)?,
            notes: row.get(10)?,
            created_at: row.get(11)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn save_submission(paths: &AppPaths, submission: &SubmissionUpsert) -> Result<i64> {
    let title = submission.title.trim();
    if title.is_empty() {
        anyhow::bail!("论文题目不能为空");
    }
    let conn = db::open_database(paths)?;
    let journal_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM journals WHERE normalized_name=?1 AND deleted_at IS NULL",
            params![normalize_journal_name(&submission.journal_name)],
            |row| row.get(0),
        )
        .optional()?;

    if let Some(id) = submission.id {
        conn.execute(
            "UPDATE submissions SET title=?1, authors=?2, corresponding=?3, journal_id=?4,
             journal_name=?5, submit_date=?6, status=?7, result_date=?8, apc_paid=?9,
             website=?10, notes=?11, updated_at=CURRENT_TIMESTAMP, version=version+1
             WHERE id=?12 AND deleted_at IS NULL",
            params![
                title,
                submission.authors,
                submission.corresponding,
                journal_id,
                submission.journal_name,
                submission.submit_date,
                if submission.status.is_empty() {
                    "准备中"
                } else {
                    &submission.status
                },
                submission.result_date,
                submission.apc_paid,
                submission.website,
                submission.notes,
                id
            ],
        )?;
        Ok(id)
    } else {
        conn.execute(
            "INSERT INTO submissions
             (uuid,title,authors,corresponding,journal_id,journal_name,submit_date,status,
              result_date,apc_paid,website,notes)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                Uuid::new_v4().to_string(),
                title,
                submission.authors,
                submission.corresponding,
                journal_id,
                submission.journal_name,
                submission.submit_date,
                if submission.status.is_empty() {
                    "准备中"
                } else {
                    &submission.status
                },
                submission.result_date,
                submission.apc_paid,
                submission.website,
                submission.notes
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }
}

pub fn soft_delete_submission(paths: &AppPaths, submission_id: i64) -> Result<()> {
    let conn = db::open_database(paths)?;
    conn.execute(
        "UPDATE submissions SET deleted_at=CURRENT_TIMESTAMP, updated_at=CURRENT_TIMESTAMP, version=version+1 WHERE id=?1",
        params![submission_id],
    )?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportReport {
    pub file_path: PathBuf,
    pub rows_exported: usize,
    pub message: String,
}

pub fn export_journals_csv(paths: &AppPaths, filter: &JournalListFilter) -> Result<ExportReport> {
    paths.ensure_all()?;
    let rows = list_journals(paths, filter)?;
    let stamp = Local::now().format("%Y%m%d_%H%M%S").to_string();
    let file_path = paths.exports.join(format!("我的收藏_{}.csv", stamp));
    let mut csv = String::from("\u{feff}期刊,来源,等级,JIF分区,JCR排名,中科院分区,影响因子,学科,开放获取,开放获取说明,期刊研究范围,近期文章主题,投稿指南摘要,难度,推荐偏好,版面费,审稿周期,标签,官网,备注,经验\n");
    for row in &rows {
        csv.push_str(&csv_line(&[
            &row.name,
            &row.source_db,
            &row.grade,
            &row.jif_quartile,
            &row.jcr_rank_detail,
            &row.cas_quartile,
            &row.impact_factor.map(|v| v.to_string()).unwrap_or_default(),
            &row.subjects,
            &row.oa_flag,
            &row.oa_detail,
            &row.scope_text,
            &row.article_topics,
            &row.submission_guidelines,
            &row.difficulty,
            &row.recommendation_preference,
            &row.apc,
            &row.review_cycle,
            &row.tags,
            &row.website,
            &row.summary,
            &row.experience,
        ]));
    }
    fs::write(&file_path, csv)?;
    Ok(ExportReport {
        file_path,
        rows_exported: rows.len(),
        message: "收藏期刊 CSV 已导出。".to_string(),
    })
}

pub fn export_submissions_csv(
    paths: &AppPaths,
    filter: &SubmissionListFilter,
) -> Result<ExportReport> {
    paths.ensure_all()?;
    let rows = list_submissions(paths, filter)?;
    let stamp = Local::now().format("%Y%m%d_%H%M%S").to_string();
    let file_path = paths.exports.join(format!("投稿记录_{}.csv", stamp));
    let mut csv = String::from(
        "\u{feff}论文题目,作者,通信作者,期刊,投稿日期,状态,结果日期,版面费,网址,备注,创建日期\n",
    );
    for row in &rows {
        csv.push_str(&csv_line(&[
            &row.title,
            &row.authors,
            &row.corresponding,
            &row.journal_name,
            &row.submit_date,
            &row.status,
            &row.result_date,
            &row.apc_paid,
            &row.website,
            &row.notes,
            &row.created_at,
        ]));
    }
    fs::write(&file_path, csv)?;
    Ok(ExportReport {
        file_path,
        rows_exported: rows.len(),
        message: "投稿记录 CSV 已导出。".to_string(),
    })
}

fn csv_line(values: &[&str]) -> String {
    let row = values
        .iter()
        .map(|value| format!("\"{}\"", value.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(",");
    format!("{}\n", row)
}
