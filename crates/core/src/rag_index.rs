use crate::{
    db,
    journal_store::{JournalListFilter, JournalRecord},
    paths::AppPaths,
};
use anyhow::Result;
use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex, OnceLock,
};
use std::thread;
use std::time::Duration;

const EMBEDDING_INDEX_VERSION: &str = "multilingual-e5-small-v1";
static LOCAL_EMBEDDING_MODEL: OnceLock<Result<Mutex<TextEmbedding>, String>> = OnceLock::new();
static BACKGROUND_INDEXING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalProfileForChunks {
    pub journal_id: i64,
    pub name: String,
    pub subjects: String,
    pub source_db: String,
    pub metrics: String,
    pub scope_text: String,
    pub user_experience: String,
    pub article_topics: String,
    pub guidelines: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagChunkDraft {
    pub journal_id: i64,
    pub chunk_type: String,
    pub content: String,
    pub content_hash: String,
}

pub fn generate_chunks(profile: &JournalProfileForChunks) -> Vec<RagChunkDraft> {
    let mut chunks = Vec::new();
    push_chunk(
        &mut chunks,
        profile.journal_id,
        "profile",
        format!(
            "期刊：{}\n来源：{}\n学科：{}\n指标：{}",
            profile.name, profile.source_db, profile.subjects, profile.metrics
        ),
    );
    push_chunk(
        &mut chunks,
        profile.journal_id,
        "scope",
        profile.scope_text.clone(),
    );
    push_chunk(
        &mut chunks,
        profile.journal_id,
        "experience",
        profile.user_experience.clone(),
    );
    push_chunk(
        &mut chunks,
        profile.journal_id,
        "articles",
        profile.article_topics.clone(),
    );
    push_chunk(
        &mut chunks,
        profile.journal_id,
        "guidelines",
        profile.guidelines.clone(),
    );
    chunks
}

fn push_chunk(chunks: &mut Vec<RagChunkDraft>, journal_id: i64, chunk_type: &str, content: String) {
    let cleaned = content.trim();
    if cleaned.is_empty() {
        return;
    }
    let mut hasher = Sha256::new();
    hasher.update(cleaned.as_bytes());
    let content_hash = format!("{:x}", hasher.finalize());
    chunks.push(RagChunkDraft {
        journal_id,
        chunk_type: chunk_type.to_string(),
        content: cleaned.to_string(),
        content_hash,
    });
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexHealth {
    pub status: String,
    pub detail: String,
    pub dirty_chunks: usize,
    pub index_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagSearchHit {
    pub journal_id: i64,
    pub chunk_type: String,
    pub snippet: String,
    pub score: f32,
}

pub fn initial_index_health() -> IndexHealth {
    IndexHealth {
        status: "需要更新".to_string(),
        detail: "新版骨架已创建，等待导入期刊库并构建 RAG/FTS/向量索引。".to_string(),
        dirty_chunks: 0,
        index_bytes: 0,
    }
}

pub fn start_background_indexing(paths: &AppPaths) {
    if BACKGROUND_INDEXING.swap(true, Ordering::SeqCst) {
        return;
    }
    let root = paths.root.clone();
    thread::spawn(move || {
        let paths = AppPaths::from_root(root);
        loop {
            let health = match refresh_dirty_rag_index(&paths) {
                Ok(health) => health,
                Err(_) => break,
            };
            if health.dirty_chunks == 0 {
                break;
            }
            thread::sleep(Duration::from_millis(150));
        }
        BACKGROUND_INDEXING.store(false, Ordering::SeqCst);
    });
}

pub fn rebuild_rag_index(paths: &AppPaths) -> Result<IndexHealth> {
    paths.ensure_all()?;
    let conn = db::open_database(paths)?;
    ensure_embedding_index_version(&conn)?;
    conn.execute("DELETE FROM rag_chunks", [])?;
    conn.execute_batch(
        "DROP TABLE IF EXISTS journal_fts;
         CREATE VIRTUAL TABLE journal_fts USING fts5(
           journal_name, subjects, scope_text, user_summary, user_experience, content=''
         );",
    )?;
    conn.execute(
        "UPDATE journal_user_profiles SET index_dirty=1, updated_at=CURRENT_TIMESTAMP WHERE deleted_at IS NULL",
        [],
    )?;
    rebuild_dirty_batch(
        paths,
        &conn,
        128,
        "已开始使用本地多语言 Embedding 分批构建索引",
    )
}

fn rebuild_rag_index_full_batch(paths: &AppPaths) -> Result<IndexHealth> {
    paths.ensure_all()?;
    let conn = db::open_database(paths)?;
    ensure_embedding_index_version(&conn)?;
    let journals = journal_store_like_rows(&conn)?;
    let mut prepared: Vec<(JournalRecord, JournalProfileForChunks, Vec<RagChunkDraft>)> =
        Vec::new();
    let mut texts = Vec::new();
    for journal in journals {
        let profile = JournalProfileForChunks {
            journal_id: journal.id,
            name: journal.name.clone(),
            subjects: journal.subjects.clone(),
            source_db: journal.source_db.clone(),
            metrics: format!(
                "JIF={}; CAS={}; IF={}; OA={}; JCR={}; grade={}",
                journal.jif_quartile,
                journal.cas_quartile,
                journal
                    .impact_factor
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                journal.oa_flag,
                journal.jcr_rank_detail,
                journal.grade
            ),
            scope_text: build_scope_text(&journal),
            user_experience: build_experience_text(&conn, journal.id)?,
            article_topics: build_article_topics(&conn, journal.id)?,
            guidelines: build_guidelines_text(&conn, &journal)?,
        };
        let chunks = generate_chunks(&profile);
        texts.extend(chunks.iter().map(|chunk| chunk.content.clone()));
        prepared.push((journal, profile, chunks));
    }
    let embeddings = build_document_embeddings(paths, &texts)?;
    if embeddings.len() != texts.len() {
        anyhow::bail!("本地 Embedding 模型返回的向量数量不完整");
    }

    conn.execute_batch("BEGIN IMMEDIATE")?;
    conn.execute("DELETE FROM rag_chunks", [])?;
    let mut embedding_index = 0usize;
    let mut total = 0usize;
    for (journal, profile, chunks) in prepared {
        for chunk in chunks {
            let embedding_json = embedding_to_json(&embeddings[embedding_index]);
            embedding_index += 1;
            conn.execute(
                "INSERT OR REPLACE INTO rag_chunks
                 (journal_id, chunk_type, content, content_hash, embedding_json, embedding_status, updated_at)
                 VALUES (?1,?2,?3,?4,?5,'clean',CURRENT_TIMESTAMP)",
                params![chunk.journal_id, chunk.chunk_type, chunk.content, chunk.content_hash, embedding_json],
            )?;
            total += 1;
        }
        conn.execute(
            "UPDATE journal_user_profiles SET index_dirty=0, updated_at=CURRENT_TIMESTAMP WHERE journal_id=?1",
            params![journal.id],
        )?;
        update_fts(&conn, &journal, &profile)?;
    }
    conn.execute_batch("COMMIT")?;
    Ok(IndexHealth {
        status: "正常".to_string(),
        detail: format!("已用本地多语言 Embedding 重建 {} 个 RAG chunk。", total),
        dirty_chunks: 0,
        index_bytes: compute_index_bytes(paths)?,
    })
}

fn rebuild_rag_index_sequential(paths: &AppPaths) -> Result<IndexHealth> {
    paths.ensure_all()?;
    let conn = db::open_database(paths)?;
    ensure_embedding_index_version(&conn)?;
    // A full catalogue rebuild may write tens of thousands of chunks. Keep it in
    // one SQLite transaction so it is fast and never exposes a half-built index.
    conn.execute_batch("BEGIN IMMEDIATE")?;
    conn.execute("DELETE FROM rag_chunks", [])?;
    let journals = journal_store_like_rows(&conn)?;
    let mut total = 0usize;
    for journal in journals {
        let profile = JournalProfileForChunks {
            journal_id: journal.id,
            name: journal.name.clone(),
            subjects: journal.subjects.clone(),
            source_db: journal.source_db.clone(),
            metrics: format!(
                "JIF={}；中科院={}；影响因子={}；等级={}",
                journal.jif_quartile,
                journal.cas_quartile,
                journal
                    .impact_factor
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "未记录".to_string()),
                journal.grade
            ),
            scope_text: build_scope_text(&journal),
            user_experience: build_experience_text(&conn, journal.id)?,
            article_topics: build_article_topics(&conn, journal.id)?,
            guidelines: build_guidelines_text(&conn, &journal)?,
        };
        let chunks = generate_chunks(&profile);
        for chunk in chunks {
            let embedding_json = embedding_to_json(&build_document_embedding(&chunk.content));
            conn.execute(
                "INSERT OR REPLACE INTO rag_chunks
                 (journal_id, chunk_type, content, content_hash, embedding_json, embedding_status, updated_at)
                 VALUES (?1,?2,?3,?4,?5,'clean',CURRENT_TIMESTAMP)",
                params![
                    chunk.journal_id,
                    chunk.chunk_type,
                    chunk.content,
                    chunk.content_hash,
                    embedding_json
                ],
            )?;
            total += 1;
        }
        conn.execute(
            "UPDATE journal_user_profiles SET index_dirty=0, updated_at=CURRENT_TIMESTAMP WHERE journal_id=?1",
            params![journal.id],
        )?;
        update_fts(&conn, &journal, &profile)?;
    }
    conn.execute_batch("COMMIT")?;
    let index_bytes = compute_index_bytes(paths)?;
    Ok(IndexHealth {
        status: if total > 0 {
            "正常".to_string()
        } else {
            "需要更新".to_string()
        },
        detail: format!("RAG 索引已重建，生成 {} 个 chunk。", total),
        dirty_chunks: 0,
        index_bytes,
    })
}

pub fn refresh_dirty_rag_index(paths: &AppPaths) -> Result<IndexHealth> {
    paths.ensure_all()?;
    let conn = db::open_database(paths)?;
    ensure_embedding_index_version(&conn)?;
    let total_chunks: i64 = conn
        .query_row("SELECT COUNT(*) FROM rag_chunks", [], |row| row.get(0))
        .unwrap_or(0);
    if total_chunks == 0 {
        return rebuild_rag_index(paths);
    }

    let mut dirty_statement = conn.prepare(
        "SELECT journal_id FROM journal_user_profiles WHERE index_dirty=1 AND deleted_at IS NULL",
    )?;
    let dirty_ids = dirty_statement
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<HashSet<_>>>()?;
    if dirty_ids.is_empty() {
        return current_index_health(paths);
    }

    rebuild_dirty_batch(paths, &conn, 128, "已增量更新本地多语言 Embedding 索引")
}

fn rebuild_dirty_batch(
    paths: &AppPaths,
    conn: &Connection,
    batch_size: usize,
    action: &str,
) -> Result<IndexHealth> {
    let mut statement = conn.prepare(
        "SELECT journal_id FROM journal_user_profiles
         WHERE index_dirty=1 AND deleted_at IS NULL
         ORDER BY journal_id LIMIT ?1",
    )?;
    let ids = statement
        .query_map(params![batch_size as i64], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<HashSet<_>>>()?;
    if ids.is_empty() {
        return current_index_health(paths);
    }
    let mut prepared: Vec<(JournalRecord, JournalProfileForChunks, Vec<RagChunkDraft>)> =
        Vec::new();
    let mut texts = Vec::new();
    for journal in journal_store_like_rows(conn)?
        .into_iter()
        .filter(|journal| ids.contains(&journal.id))
    {
        let profile = journal_profile_for_index(conn, &journal)?;
        let chunks = generate_chunks(&profile);
        texts.extend(chunks.iter().map(|chunk| chunk.content.clone()));
        prepared.push((journal, profile, chunks));
    }
    let embeddings = build_document_embeddings(paths, &texts)?;
    if embeddings.len() != texts.len() {
        anyhow::bail!("本地 Embedding 模型返回的向量数量不完整");
    }
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let mut embedding_index = 0usize;
    let mut refreshed = 0usize;
    for (journal, profile, chunks) in prepared {
        conn.execute(
            "DELETE FROM rag_chunks WHERE journal_id=?1",
            params![journal.id],
        )?;
        for chunk in chunks {
            let embedding_json = embedding_to_json(&embeddings[embedding_index]);
            embedding_index += 1;
            conn.execute(
                "INSERT OR REPLACE INTO rag_chunks
                 (journal_id, chunk_type, content, content_hash, embedding_json, embedding_status, updated_at)
                 VALUES (?1,?2,?3,?4,?5,'clean',CURRENT_TIMESTAMP)",
                params![chunk.journal_id, chunk.chunk_type, chunk.content, chunk.content_hash, embedding_json],
            )?;
            refreshed += 1;
        }
        conn.execute(
            "UPDATE journal_user_profiles SET index_dirty=0, updated_at=CURRENT_TIMESTAMP WHERE journal_id=?1",
            params![journal.id],
        )?;
        update_fts(conn, &journal, &profile)?;
    }
    conn.execute_batch("COMMIT")?;
    let remaining: i64 = conn.query_row(
        "SELECT COUNT(*) FROM journal_user_profiles WHERE index_dirty=1 AND deleted_at IS NULL",
        [],
        |row| row.get(0),
    )?;
    Ok(IndexHealth {
        status: if remaining == 0 {
            "正常"
        } else {
            "构建中"
        }
        .to_string(),
        detail: format!(
            "{}：本批更新 {} 个 chunk，剩余 {} 本期刊待处理。",
            action, refreshed, remaining
        ),
        dirty_chunks: remaining as usize,
        index_bytes: compute_index_bytes(paths)?,
    })
}

pub fn clear_index_cache(paths: &AppPaths) -> Result<IndexHealth> {
    paths.ensure_all()?;
    let conn = db::open_database(paths)?;
    conn.execute("DELETE FROM rag_chunks", [])?;
    conn.execute("DROP TABLE IF EXISTS journal_fts", [])?;
    conn.execute_batch(
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
    )?;
    conn.execute(
        "UPDATE journal_user_profiles SET index_dirty=1, updated_at=CURRENT_TIMESTAMP WHERE deleted_at IS NULL",
        [],
    )?;
    Ok(IndexHealth {
        status: "\u{9700}\u{8981}\u{66f4}\u{65b0}".to_string(),
        detail: "\u{7d22}\u{5f15}\u{7f13}\u{5b58}\u{5df2}\u{6e05}\u{7406}\u{ff0c}\u{4e0b}\u{6b21}\u{63a8}\u{8350}\u{524d}\u{4f1a}\u{81ea}\u{52a8}\u{589e}\u{91cf}\u{5237}\u{65b0}\u{ff0c}\u{4e5f}\u{53ef}\u{4ee5}\u{624b}\u{52a8}\u{91cd}\u{5efa}\u{7d22}\u{5f15}\u{3002}".to_string(),
        dirty_chunks: 0,
        index_bytes: compute_index_bytes(paths)?,
    })
}

fn journal_profile_for_index(
    conn: &Connection,
    journal: &JournalRecord,
) -> Result<JournalProfileForChunks> {
    Ok(JournalProfileForChunks {
        journal_id: journal.id,
        name: journal.name.clone(),
        subjects: journal.subjects.clone(),
        source_db: journal.source_db.clone(),
        metrics: format!(
            "JIF={}; CAS={}; IF={}; grade={}",
            journal.jif_quartile,
            journal.cas_quartile,
            journal
                .impact_factor
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            journal.grade
        ),
        scope_text: build_scope_text(journal),
        user_experience: build_experience_text(conn, journal.id)?,
        article_topics: build_article_topics(conn, journal.id)?,
        guidelines: build_guidelines_text(conn, journal)?,
    })
}

fn rebuild_one_journal_index(conn: &Connection, journal: &JournalRecord) -> Result<usize> {
    conn.execute(
        "DELETE FROM rag_chunks WHERE journal_id=?1",
        params![journal.id],
    )?;
    let profile = JournalProfileForChunks {
        journal_id: journal.id,
        name: journal.name.clone(),
        subjects: journal.subjects.clone(),
        source_db: journal.source_db.clone(),
        metrics: format!(
            "JIF={}; CAS={}; IF={}; grade={}",
            journal.jif_quartile,
            journal.cas_quartile,
            journal
                .impact_factor
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            journal.grade
        ),
        scope_text: build_scope_text(journal),
        user_experience: build_experience_text(conn, journal.id)?,
        article_topics: build_article_topics(conn, journal.id)?,
        guidelines: build_guidelines_text(conn, journal)?,
    };
    let chunks = generate_chunks(&profile);
    let count = chunks.len();
    for chunk in chunks {
        let embedding_json = embedding_to_json(&build_document_embedding(&chunk.content));
        conn.execute(
            "INSERT OR REPLACE INTO rag_chunks
             (journal_id, chunk_type, content, content_hash, embedding_json, embedding_status, updated_at)
             VALUES (?1,?2,?3,?4,?5,'clean',CURRENT_TIMESTAMP)",
            params![
                chunk.journal_id,
                chunk.chunk_type,
                chunk.content,
                chunk.content_hash,
                embedding_json
            ],
        )?;
    }
    conn.execute(
        "UPDATE journal_user_profiles SET index_dirty=0, updated_at=CURRENT_TIMESTAMP WHERE journal_id=?1",
        params![journal.id],
    )?;
    update_fts(conn, journal, &profile)?;
    Ok(count)
}

pub fn current_index_health(paths: &AppPaths) -> Result<IndexHealth> {
    paths.ensure_all()?;
    let conn = db::open_database(paths)?;
    let dirty_chunks: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM journal_user_profiles WHERE index_dirty = 1 AND deleted_at IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let total_chunks: i64 = conn
        .query_row("SELECT COUNT(*) FROM rag_chunks", [], |row| row.get(0))
        .unwrap_or(0);
    let has_fts = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='journal_fts'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;
    let index_bytes = compute_index_bytes(paths)?;
    let status = if total_chunks == 0 {
        "需要更新"
    } else if dirty_chunks > 0 {
        "构建中"
    } else if has_fts {
        "正常"
    } else {
        "失败"
    };
    let content_chunks: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM rag_chunks
             WHERE chunk_type IN ('scope','articles','experience','guidelines')
               AND embedding_status='clean'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let profile_chunks: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM rag_chunks
             WHERE chunk_type='profile' AND embedding_status='clean'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    Ok(IndexHealth {
        status: status.to_string(),
        detail: format!(
            "chunk 总数 {}，内容证据 chunk {}，基础档案 chunk {}，待更新 {}，FTS {}。",
            total_chunks,
            content_chunks,
            profile_chunks,
            dirty_chunks,
            if has_fts { "已启用" } else { "未启用" }
        ),
        dirty_chunks: dirty_chunks as usize,
        index_bytes,
    })
}

pub fn search_rag(paths: &AppPaths, query: &str, limit: usize) -> Result<Vec<RagSearchHit>> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    paths.ensure_all()?;
    let _ = refresh_dirty_rag_index(paths);
    let conn = db::open_database(paths)?;
    let query_embedding = build_query_embedding(trimmed);
    let limit = limit.clamp(1, 100) as i64;
    let mut merged: HashMap<i64, RagSearchHit> = HashMap::new();

    if let Some(fts_query) = build_fts_query(trimmed) {
        if let Ok(hits) = search_fts(&conn, &fts_query, &query_embedding, limit) {
            for hit in hits {
                merged.insert(hit.journal_id, hit);
            }
        }
    }

    for hit in search_chunks_semantic(&conn, &query_embedding, limit)? {
        merged
            .entry(hit.journal_id)
            .and_modify(|existing| {
                if hit.score > existing.score {
                    *existing = hit.clone();
                }
            })
            .or_insert(hit);
    }

    let mut hits: Vec<RagSearchHit> = merged.into_values().collect();
    hits.sort_by(|left, right| right.score.total_cmp(&left.score));
    Ok(hits.into_iter().take(limit as usize).collect())
}

fn journal_store_like_rows(conn: &Connection) -> Result<Vec<JournalRecord>> {
    let filter = JournalListFilter {
        keyword: String::new(),
        source_db: Vec::new(),
        jif_quartile: Vec::new(),
        cas_zone: Vec::new(),
        ccf: Vec::new(),
        ei_only: false,
        // Keep the RAG corpus complete for the expected 10k-30k journal range.
        limit: 50_000,
        offset: 0,
    };
    crate::journal_store::list_journals_conn(conn, &filter)
}

fn search_fts(
    conn: &Connection,
    fts_query: &str,
    query_embedding: &[f32],
    limit: i64,
) -> Result<Vec<RagSearchHit>> {
    let mut statement = conn.prepare(
        "SELECT rowid, bm25(journal_fts) AS rank
         FROM journal_fts
         WHERE journal_fts MATCH ?1
         ORDER BY rank
         LIMIT ?2",
    )?;
    let rows = statement.query_map(params![fts_query, limit], |row| {
        let journal_id: i64 = row.get(0)?;
        let rank: f64 = row.get(1)?;
        let (chunk_type, snippet, embedding) = best_chunk_for_journal(conn, journal_id)
            .unwrap_or_else(|_| ("profile".to_string(), String::new(), None));
        let semantic_score = if let Some(embedding) = embedding.as_ref() {
            cosine_similarity(query_embedding, embedding)
        } else {
            0.0
        };
        let weighted_score = weighted_rag_score(
            &chunk_type,
            (0.45 * bm25_to_score(rank) + 0.55 * semantic_score).clamp(0.0, 1.0),
        );
        Ok(RagSearchHit {
            journal_id,
            chunk_type,
            snippet,
            score: weighted_score,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn search_chunks_semantic(
    conn: &Connection,
    query_embedding: &[f32],
    limit: i64,
) -> Result<Vec<RagSearchHit>> {
    let mut statement = conn.prepare(
        "SELECT journal_id, chunk_type, content, embedding_json
         FROM rag_chunks
         WHERE embedding_status='clean'",
    )?;
    let rows = statement.query_map([], |row| {
        let journal_id: i64 = row.get(0)?;
        let chunk_type: String = row.get(1)?;
        let content: String = row.get(2)?;
        let embedding = embedding_from_row(row.get::<_, Option<String>>(3)?.as_deref(), &content)
            .unwrap_or_default();
        let raw_score = cosine_similarity(query_embedding, &embedding);
        Ok(RagSearchHit {
            journal_id,
            chunk_type: chunk_type.clone(),
            snippet: snippet(&content, 220),
            score: weighted_rag_score(&chunk_type, raw_score),
        })
    })?;
    let mut best_by_journal: HashMap<i64, RagSearchHit> = HashMap::new();
    for row in rows {
        let hit = row?;
        if hit.score <= 0.0 {
            continue;
        }
        best_by_journal
            .entry(hit.journal_id)
            .and_modify(|existing| {
                if hit.score > existing.score {
                    *existing = hit.clone();
                }
            })
            .or_insert(hit);
    }
    let mut hits: Vec<RagSearchHit> = best_by_journal.into_values().collect();
    hits.sort_by(|left, right| right.score.total_cmp(&left.score));
    Ok(hits.into_iter().take(limit as usize).collect())
}

fn best_chunk_for_journal(
    conn: &Connection,
    journal_id: i64,
) -> Result<(String, String, Option<Vec<f32>>)> {
    let (chunk_type, content, embedding_json): (String, String, Option<String>) = conn.query_row(
        "SELECT chunk_type, content, embedding_json
         FROM rag_chunks
         WHERE journal_id=?1 AND embedding_status='clean'
         ORDER BY
           CASE chunk_type
             WHEN 'experience' THEN 0
             WHEN 'scope' THEN 1
             WHEN 'articles' THEN 2
             WHEN 'profile' THEN 3
             ELSE 4
           END,
           updated_at DESC
         LIMIT 1",
        params![journal_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    Ok((
        chunk_type,
        snippet(&content, 220),
        embedding_from_row(embedding_json.as_deref(), &content)
            .map(Some)
            .unwrap_or(None),
    ))
}

fn build_fts_query(query: &str) -> Option<String> {
    let terms: Vec<String> = split_search_terms(query)
        .into_iter()
        .take(8)
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" OR "))
    }
}

fn split_search_terms(query: &str) -> Vec<String> {
    query
        .split(|c: char| !c.is_alphanumeric() && !('\u{4e00}'..='\u{9fff}').contains(&c))
        .map(str::trim)
        .filter(|term| term.chars().count() >= 2)
        .map(str::to_lowercase)
        .collect()
}

fn snippet(value: &str, max_chars: usize) -> String {
    let mut out: String = value.chars().take(max_chars).collect();
    if value.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

fn bm25_to_score(rank: f64) -> f32 {
    (1.0 / (1.0 + rank.abs() as f32)).clamp(0.0, 1.0)
}

fn weighted_rag_score(chunk_type: &str, score: f32) -> f32 {
    let weight = match chunk_type {
        "scope" => 1.0,
        "articles" => 0.95,
        "experience" => 0.9,
        "guidelines" => 0.75,
        "profile" => 0.25,
        _ => 0.5,
    };
    (score * weight).clamp(0.0, 1.0)
}

fn build_document_embedding(text: &str) -> Vec<f32> {
    embed_locally("passage: ", text)
}

fn build_document_embeddings(paths: &AppPaths, texts: &[String]) -> Result<Vec<Vec<f32>>> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    let model = LOCAL_EMBEDDING_MODEL.get_or_init(|| {
        let cache_dir = paths.embeddings.join("multilingual-e5-small");
        fs::create_dir_all(&cache_dir).map_err(|error| error.to_string())?;
        TextEmbedding::try_new(
            TextInitOptions::new(EmbeddingModel::MultilingualE5Small)
                .with_cache_dir(cache_dir)
                .with_show_download_progress(false)
                .with_intra_threads(2),
        )
        .map(Mutex::new)
        .map_err(|error| error.to_string())
    });
    let model = model
        .as_ref()
        .map_err(|error| anyhow::anyhow!(error.clone()))?;
    let mut model = model
        .lock()
        .map_err(|_| anyhow::anyhow!("本地 Embedding 模型已被其他任务占用"))?;
    let inputs = texts
        .iter()
        .map(|text| format!("passage: {}", text))
        .collect::<Vec<_>>();
    model.embed(inputs, None).map_err(Into::into)
}

fn ensure_embedding_index_version(conn: &Connection) -> Result<()> {
    let current: Option<String> = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key='embedding_index_version'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if current.as_deref() == Some(EMBEDDING_INDEX_VERSION) {
        return Ok(());
    }
    conn.execute(
        "UPDATE rag_chunks SET embedding_status='dirty', updated_at=CURRENT_TIMESTAMP",
        [],
    )?;
    conn.execute(
        "UPDATE journal_user_profiles SET index_dirty=1, updated_at=CURRENT_TIMESTAMP WHERE deleted_at IS NULL",
        [],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO schema_meta (key,value) VALUES ('embedding_index_version',?1)",
        params![EMBEDDING_INDEX_VERSION],
    )?;
    Ok(())
}

fn build_query_embedding(text: &str) -> Vec<f32> {
    embed_locally("query: ", text)
}

fn embed_locally(prefix: &str, text: &str) -> Vec<f32> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    let paths = AppPaths::discover();
    if paths.ensure_all().is_err() {
        return Vec::new();
    }
    let model = LOCAL_EMBEDDING_MODEL.get_or_init(|| {
        let cache_dir = paths.embeddings.join("multilingual-e5-small");
        fs::create_dir_all(&cache_dir).map_err(|error| error.to_string())?;
        TextEmbedding::try_new(
            TextInitOptions::new(EmbeddingModel::MultilingualE5Small)
                .with_cache_dir(cache_dir)
                .with_show_download_progress(false)
                .with_intra_threads(2),
        )
        .map(Mutex::new)
        .map_err(|error| error.to_string())
    });
    let Ok(model) = model else {
        return Vec::new();
    };
    let Ok(mut model) = model.lock() else {
        return Vec::new();
    };
    model
        .embed(vec![format!("{}{}", prefix, text)], None)
        .ok()
        .and_then(|mut embeddings| embeddings.pop())
        .unwrap_or_default()
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let len = left.len().min(right.len());
    let mut dot = 0.0f32;
    let mut left_norm = 0.0f32;
    let mut right_norm = 0.0f32;
    for index in 0..len {
        dot += left[index] * right[index];
        left_norm += left[index] * left[index];
        right_norm += right[index] * right[index];
    }
    if left_norm == 0.0 || right_norm == 0.0 {
        0.0
    } else {
        (dot / (left_norm.sqrt() * right_norm.sqrt())).clamp(0.0, 1.0)
    }
}

fn embedding_to_json(embedding: &[f32]) -> String {
    serde_json::to_string(embedding).unwrap_or_else(|_| "[]".to_string())
}

fn embedding_from_row(raw: Option<&str>, content: &str) -> Option<Vec<f32>> {
    if let Some(raw) = raw {
        if let Ok(values) = serde_json::from_str::<Vec<f32>>(raw) {
            if !values.is_empty() {
                return Some(values);
            }
        }
    }
    let embedding = build_document_embedding(content);
    if embedding.iter().any(|value| *value > 0.0) {
        Some(embedding)
    } else {
        None
    }
}

fn build_scope_text(journal: &JournalRecord) -> String {
    let mut parts = Vec::new();
    if !journal.scope_text.trim().is_empty() {
        parts.push(journal.scope_text.trim().to_string());
    }
    if !journal.oa_flag.trim().is_empty() {
        parts.push(format!("开放获取：{}", journal.oa_flag.trim()));
    }
    if !journal.oa_detail.trim().is_empty() {
        parts.push(format!("开放获取说明：{}", journal.oa_detail.trim()));
    }
    if !journal.jcr_rank_detail.trim().is_empty() {
        parts.push(format!("JCR排名：{}", journal.jcr_rank_detail.trim()));
    }
    parts.join("；")
}

fn build_experience_text(conn: &Connection, journal_id: i64) -> Result<String> {
    let row = conn.query_row(
        "SELECT difficulty, recommendation_preference, apc, review_cycle, tags, summary, experience,
                rejection_reason, suitable_topics, avoid_reason
         FROM journal_user_profiles WHERE journal_id=?1",
        params![journal_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
            ))
        },
    );
    if let Ok((
        difficulty,
        preference,
        apc,
        review_cycle,
        tags,
        summary,
        experience,
        rejection_reason,
        suitable_topics,
        avoid_reason,
    )) = row
    {
        let mut parts = Vec::new();
        if !difficulty.trim().is_empty() && difficulty != "未评估" {
            parts.push(format!("难度：{}", difficulty));
        }
        if !preference.trim().is_empty() && preference != "正常推荐" {
            parts.push(format!("推荐偏好：{}", preference));
        }
        for (label, value) in [
            ("版面费", apc),
            ("审稿周期", review_cycle),
            ("标签", tags),
            ("总结", summary),
            ("经验", experience),
        ] {
            if !value.trim().is_empty() {
                parts.push(format!("{}：{}", label, value));
            }
        }
        for (label, value) in [
            ("rejection reason", rejection_reason),
            ("suitable topics", suitable_topics),
            ("avoid recommendation reason", avoid_reason),
        ] {
            if !value.trim().is_empty() {
                parts.push(format!("{}: {}", label, value));
            }
        }
        return Ok(parts.join("；"));
    }
    Ok(String::new())
}

fn build_article_topics(conn: &Connection, journal_id: i64) -> Result<String> {
    Ok(conn
        .query_row(
            "SELECT COALESCE(article_topics,'') FROM journal_user_profiles WHERE journal_id=?1",
            params![journal_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .unwrap_or_default())
}

fn build_guidelines_text(conn: &Connection, journal: &JournalRecord) -> Result<String> {
    let mut parts = Vec::new();
    if let Some(guidelines) = conn
        .query_row(
            "SELECT COALESCE(submission_guidelines,'') FROM journal_user_profiles WHERE journal_id=?1",
            params![journal.id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        if !guidelines.trim().is_empty() {
            parts.push(format!("投稿指南摘要：{}", guidelines));
        }
    }
    if !journal.website.trim().is_empty() {
        parts.push(format!("官网：{}", journal.website));
    }
    if !journal.oa_detail.trim().is_empty() {
        parts.push(format!("开放获取说明：{}", journal.oa_detail));
    }
    if !journal.ei.trim().is_empty() {
        parts.push(format!("EI：{}", journal.ei));
    }
    if !journal.ccf.trim().is_empty() {
        parts.push(format!("CCF：{}", journal.ccf));
    }
    Ok(parts.join("；"))
}

fn update_fts(
    conn: &Connection,
    journal: &JournalRecord,
    profile: &JournalProfileForChunks,
) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO journal_fts
         (rowid, journal_name, subjects, scope_text, user_summary, user_experience)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            journal.id,
            journal.name,
            journal.subjects,
            profile.scope_text,
            journal.summary,
            profile.user_experience
        ],
    )?;
    Ok(())
}

fn compute_index_bytes(paths: &AppPaths) -> Result<u64> {
    let mut total = 0u64;
    for dir in [&paths.indexes, &paths.embeddings] {
        if !dir.exists() {
            continue;
        }
        total += dir_size(dir)?;
    }
    Ok(total)
}

fn dir_size(path: &std::path::Path) -> Result<u64> {
    let mut total = 0u64;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let p = entry.path();
        if p.is_dir() {
            total += dir_size(&p)?;
        } else {
            total += entry.metadata()?.len();
        }
    }
    Ok(total)
}
