use crate::{db, paths::AppPaths};
use anyhow::{Context, Result};
use chrono::Local;
use regex::Regex;
use reqwest::{Client, Url};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::OnceLock;
use std::time::Duration;

const MAX_HTML_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichmentRequest {
    pub journal_ids: Vec<i64>,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichmentFieldReport {
    pub field_name: String,
    pub status: String,
    pub source_kind: String,
    pub source_url: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichmentJournalReport {
    pub journal_id: i64,
    pub journal_name: String,
    pub fields: Vec<EnrichmentFieldReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichmentReport {
    pub dry_run: bool,
    pub processed: usize,
    pub updated_fields: usize,
    pub skipped_fields: usize,
    pub warnings: Vec<String>,
    pub journals: Vec<EnrichmentJournalReport>,
}

#[derive(Debug, Clone)]
struct CrossrefMetadata {
    issn: String,
    article_topics: String,
    publisher: String,
    website_url: String,
    source_url: String,
}

#[derive(Debug, Clone)]
struct WebsiteMetadata {
    scope_text: String,
    scope_source_url: String,
    submission_guidelines: String,
    submission_source_url: String,
    oa_flag: String,
    oa_source_url: String,
    oa_detail: String,
    oa_detail_source_url: String,
    apc_text: String,
    apc_source_url: String,
    review_cycle: String,
    review_source_url: String,
    jcr_rank_detail: String,
    jcr_source_url: String,
    source_url: String,
}

pub async fn enrich_selected(
    paths: &AppPaths,
    request: &EnrichmentRequest,
) -> Result<EnrichmentReport> {
    paths.ensure_all()?;
    let conn = db::open_database(paths)?;
    let journals = load_selected_journals(&conn, &request.journal_ids)?;
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(15))
        .user_agent("ToukanRobot/0.1 controlled-enrichment")
        .build()
        .context("初始化期刊信息补全网络客户端失败")?;

    let mut report = EnrichmentReport {
        dry_run: request.dry_run,
        processed: 0,
        updated_fields: 0,
        skipped_fields: 0,
        warnings: Vec::new(),
        journals: Vec::new(),
    };

    for journal in journals {
        report.processed += 1;
        let mut fields = Vec::new();
        let crossref = fetch_crossref_metadata(&client, &journal).await;
        let website_url = choose_website_url(&journal, crossref.as_ref());
        let website = match website_url.as_deref() {
            Some(url) => match fetch_website_metadata(&client, url).await {
                Ok(value) => Some(value),
                Err(error) => {
                    fields.push(EnrichmentFieldReport {
                        field_name: "scope_text/submission_guidelines".to_string(),
                        status: "skipped".to_string(),
                        source_kind: "official_website".to_string(),
                        source_url: url.to_string(),
                        message: format!("官网页面无法可靠解析，未写入：{}", error),
                    });
                    None
                }
            },
            None => None,
        };

        if let Some(metadata) = crossref {
            apply_crossref(&conn, &journal, &metadata, request.dry_run, &mut fields)?;
        } else {
            fields.push(EnrichmentFieldReport {
                field_name: "issn/eissn/article_topics".to_string(),
                status: "skipped".to_string(),
                source_kind: "crossref".to_string(),
                source_url: String::new(),
                message: "Crossref 没有返回可核验记录，未写入猜测数据。".to_string(),
            });
        }

        if let Some(metadata) = website {
            apply_website(&conn, &journal, &metadata, request.dry_run, &mut fields)?;
        }

        report.updated_fields += fields.iter().filter(|field| field.status == "updated").count();
        report.skipped_fields += fields.iter().filter(|field| field.status == "skipped").count();
        report.journals.push(EnrichmentJournalReport {
            journal_id: journal.id,
            journal_name: journal.name,
            fields,
        });
    }

    Ok(report)
}

#[derive(Debug, Clone)]
struct SelectedJournal {
    id: i64,
    name: String,
    issn: String,
    eissn: String,
    publisher: String,
    oa_flag: String,
    oa_detail: String,
    website: String,
    scope_text: String,
    jcr_rank_detail: String,
    submission_guidelines: String,
    article_topics: String,
    apc: String,
    review_cycle: String,
}

fn load_selected_journals(conn: &Connection, ids: &[i64]) -> Result<Vec<SelectedJournal>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = std::iter::repeat("?")
        .take(ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let mut statement = conn.prepare(&format!(
        "SELECT id, name, COALESCE(issn,''), COALESCE(eissn,''), COALESCE(publisher,''), \
                COALESCE(oa_flag,''), COALESCE(oa_detail,''), COALESCE(website,''), \
                COALESCE(scope_text,''), COALESCE(jcr_rank_detail,''), \
                COALESCE(p.submission_guidelines,''), COALESCE(p.article_topics,''), \
                COALESCE(p.apc,''), COALESCE(p.review_cycle,'')
         FROM journals j
         LEFT JOIN journal_user_profiles p ON p.journal_id=j.id
         WHERE j.deleted_at IS NULL AND j.id IN ({})",
        placeholders
    ))?;
    let values: Vec<&dyn rusqlite::ToSql> =
        ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();
    let rows = statement.query_map(values.as_slice(), |row| {
        Ok(SelectedJournal {
            id: row.get(0)?,
            name: row.get(1)?,
            issn: row.get(2)?,
            eissn: row.get(3)?,
            publisher: row.get(4)?,
            oa_flag: row.get(5)?,
            oa_detail: row.get(6)?,
            website: row.get(7)?,
            scope_text: row.get(8)?,
            jcr_rank_detail: row.get(9)?,
            submission_guidelines: row.get(10)?,
            article_topics: row.get(11)?,
            apc: row.get(12)?,
            review_cycle: row.get(13)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

async fn fetch_crossref_metadata(
    client: &Client,
    journal: &SelectedJournal,
) -> Option<CrossrefMetadata> {
    let known_issn = normalize_issn(&journal.issn).or_else(|| normalize_issn(&journal.eissn));
    let (message, source_url) = if let Some(issn) = known_issn.as_deref() {
        let url = format!("https://api.crossref.org/journals/{}", issn);
        let response = client.get(&url).send().await.ok()?.error_for_status().ok()?;
        let data: Value = response.json().await.ok()?;
        (data.get("message")?.clone(), url)
    } else {
        let response = client
            .get("https://api.crossref.org/works")
            .query(&[
                ("query.container-title", journal.name.as_str()),
                ("rows", "1"),
                ("select", "container-title,ISSN"),
            ])
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?;
        let data: Value = response.json().await.ok()?;
        let item = data
            .get("message")?
            .get("items")?
            .as_array()?
            .first()?;
        (
            item.clone(),
            "https://api.crossref.org/works?query.container-title".to_string(),
        )
    };

    let (journal_name, issn) = if message.get("title").is_some() {
        (
            first_json_string(message.get("title"))
                .unwrap_or_default()
                .to_string(),
            message
                .get("ISSN")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        )
    } else {
        (
            first_json_string(message.get("container-title"))
                .or_else(|| first_json_string(message.get("title")))
                .unwrap_or_default()
                .to_string(),
            message
                .get("ISSN")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        )
    };
    if journal_name.trim().is_empty() {
        return None;
    }
    if known_issn.is_none() && !title_matches(&journal.name, &journal_name) {
        return None;
    }

    let articles_response = if !issn.trim().is_empty() {
        client
            .get(format!("https://api.crossref.org/journals/{}/works", issn))
            .query(&[("rows", "5"), ("select", "title,DOI")])
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?
    } else {
        client
            .get("https://api.crossref.org/works")
            .query(&[
                ("query.container-title", journal_name.as_str()),
                ("rows", "5"),
                ("select", "title,DOI"),
            ])
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?
    };
    let articles = articles_response
        .json::<Value>()
        .await
        .ok()?;
    let article_topics = articles
        .get("message")
        .and_then(|value| value.get("items"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    item.get("title")
                        .and_then(Value::as_array)
                        .and_then(|titles| titles.first())
                        .and_then(Value::as_str)
                })
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    let publisher = first_json_string(message.get("publisher"))
        .unwrap_or_default()
        .to_string();
    let website_url = first_json_string(message.get("URL"))
        .unwrap_or_default()
        .to_string();

    Some(CrossrefMetadata {
        issn,
        article_topics,
        publisher,
        website_url,
        source_url,
    })
}

async fn fetch_website_metadata(client: &Client, url: &str) -> Result<WebsiteMetadata> {
    let pages = crawl_site_pages(client, url).await?;
    let mut combined_text = String::new();
    let mut scope_text = String::new();
    let mut scope_source_url = String::new();
    let mut submission_guidelines = String::new();
    let mut submission_source_url = String::new();
    let mut oa_flag = String::new();
    let mut oa_detail = String::new();
    let mut oa_detail_source_url = String::new();
    let mut apc_text = String::new();
    let mut apc_source_url = String::new();
    let mut review_cycle = String::new();
    let mut review_source_url = String::new();
    let mut jcr_rank_detail = String::new();
    let mut jcr_source_url = String::new();

    for page in &pages {
        if !page.text.trim().is_empty() {
            if !combined_text.is_empty() {
                combined_text.push_str("\n\n");
            }
            combined_text.push_str(&page.text);
        }
        if scope_text.is_empty() {
            let candidate = extract_section(
                &page.text,
                &["aims and scope", "aims & scope", "scope", "收稿范围"],
            );
            if !candidate.is_empty() {
                scope_text = candidate;
                scope_source_url = page.url.clone();
            }
        }
        if submission_guidelines.is_empty() {
            let candidate = extract_section(
                &page.text,
                &[
                    "author guidelines",
                    "instructions for authors",
                    "submission guidelines",
                    "投稿指南",
                    "投稿须知",
                ],
            );
            if !candidate.is_empty() {
                submission_guidelines = candidate;
                submission_source_url = page.url.clone();
            }
        }
        if oa_detail.is_empty() {
            let candidate = extract_open_access_text(&page.text);
            if !candidate.is_empty() {
                oa_detail = candidate;
                oa_detail_source_url = page.url.clone();
            }
        }
        if apc_text.is_empty() {
            let candidate = extract_fee_text(&page.text);
            if !candidate.is_empty() {
                apc_text = candidate;
                apc_source_url = page.url.clone();
            }
        }
        if review_cycle.is_empty() {
            let candidate = extract_review_cycle_text(&page.text);
            if !candidate.is_empty() {
                review_cycle = candidate;
                review_source_url = page.url.clone();
            }
        }
        if jcr_rank_detail.is_empty() {
            let candidate = extract_jcr_rank_detail(&page.text);
            if !candidate.is_empty() {
                jcr_rank_detail = candidate;
                jcr_source_url = page.url.clone();
            }
        }
    }

    if scope_text.is_empty() {
        let candidate = extract_section(
            &combined_text,
            &["aims and scope", "aims & scope", "scope", "收稿范围"],
        );
        if !candidate.is_empty() {
            scope_text = candidate;
            scope_source_url = url.to_string();
        }
    }
    if submission_guidelines.is_empty() {
        let candidate = extract_section(
            &combined_text,
            &[
                "author guidelines",
                "instructions for authors",
                "submission guidelines",
                "投稿指南",
                "投稿须知",
            ],
        );
        if !candidate.is_empty() {
            submission_guidelines = candidate;
            submission_source_url = url.to_string();
        }
    }
    if oa_detail.is_empty() {
        let candidate = extract_open_access_text(&combined_text);
        if !candidate.is_empty() {
            oa_detail = candidate;
            oa_detail_source_url = url.to_string();
        }
    }
    if apc_text.is_empty() {
        let candidate = extract_fee_text(&combined_text);
        if !candidate.is_empty() {
            apc_text = candidate;
            apc_source_url = url.to_string();
        }
    }
    if review_cycle.is_empty() {
        let candidate = extract_review_cycle_text(&combined_text);
        if !candidate.is_empty() {
            review_cycle = candidate;
            review_source_url = url.to_string();
        }
    }
    if jcr_rank_detail.is_empty() {
        let candidate = extract_jcr_rank_detail(&combined_text);
        if !candidate.is_empty() {
            jcr_rank_detail = candidate;
            jcr_source_url = url.to_string();
        }
    }
    if oa_flag.is_empty() {
        oa_flag = infer_oa_flag(&oa_detail, &apc_text);
    }
    if scope_text.is_empty()
        && submission_guidelines.is_empty()
        && oa_detail.is_empty()
        && apc_text.is_empty()
        && review_cycle.is_empty()
        && jcr_rank_detail.is_empty()
    {
        anyhow::bail!("未识别到明确的 scope、投稿指南或补充信息");
    }

    Ok(WebsiteMetadata {
        scope_text,
        scope_source_url,
        submission_guidelines,
        submission_source_url,
        oa_flag,
        oa_source_url: if !oa_detail_source_url.is_empty() {
            oa_detail_source_url.clone()
        } else if !apc_source_url.is_empty() {
            apc_source_url.clone()
        } else {
            url.to_string()
        },
        oa_detail,
        oa_detail_source_url,
        apc_text,
        apc_source_url,
        review_cycle,
        review_source_url,
        jcr_rank_detail,
        jcr_source_url,
        source_url: url.to_string(),
    })
}

fn choose_website_url(journal: &SelectedJournal, crossref: Option<&CrossrefMetadata>) -> Option<String> {
    let value = journal.website.trim();
    if value.starts_with("https://") || value.starts_with("http://") {
        return Some(value.to_string());
    }
    if let Some(crossref) = crossref {
        let candidate = crossref.website_url.trim();
        if candidate.starts_with("https://") || candidate.starts_with("http://") {
            return Some(candidate.to_string());
        }
    }
    None
}

fn apply_crossref(
    conn: &Connection,
    journal: &SelectedJournal,
    metadata: &CrossrefMetadata,
    dry_run: bool,
    fields: &mut Vec<EnrichmentFieldReport>,
) -> Result<()> {
    let mut updated = false;
    if journal.issn.trim().is_empty() && !metadata.issn.trim().is_empty() {
        if !dry_run {
            conn.execute(
                "UPDATE journals SET issn=?1, updated_at=CURRENT_TIMESTAMP, version=version+1 WHERE id=?2",
                params![metadata.issn, journal.id],
            )?;
        }
        record_source(conn, journal.id, "issn", "crossref", &metadata.source_url, "updated", "Crossref 返回 ISSN", dry_run)?;
        fields.push(updated_field("issn", "crossref", &metadata.source_url, "已补充 ISSN"));
        updated = true;
    }
    if journal.publisher.trim().is_empty() && !metadata.publisher.trim().is_empty() {
        if !dry_run {
            conn.execute(
                "UPDATE journals SET publisher=?1, updated_at=CURRENT_TIMESTAMP, version=version+1 WHERE id=?2",
                params![metadata.publisher, journal.id],
            )?;
        }
        record_source(
            conn,
            journal.id,
            "publisher",
            "crossref",
            &metadata.source_url,
            "updated",
            "Crossref 返回出版社",
            dry_run,
        )?;
        fields.push(updated_field("publisher", "crossref", &metadata.source_url, "已补充出版社"));
        updated = true;
    }
    if journal.website.trim().is_empty() && !metadata.website_url.trim().is_empty() {
        if !dry_run {
            conn.execute(
                "UPDATE journals SET website=?1, updated_at=CURRENT_TIMESTAMP, version=version+1 WHERE id=?2",
                params![metadata.website_url, journal.id],
            )?;
        }
        record_source(
            conn,
            journal.id,
            "website",
            "crossref",
            &metadata.source_url,
            "updated",
            "Crossref 返回官网入口",
            dry_run,
        )?;
        fields.push(updated_field("website", "crossref", &metadata.source_url, "已补充官网入口"));
        updated = true;
    }
    if journal.article_topics.trim().is_empty() && !metadata.article_topics.trim().is_empty() {
        if !dry_run {
            conn.execute(
                "INSERT INTO journal_user_profiles (journal_id, article_topics, index_dirty)
                 VALUES (?1,?2,1)
                 ON CONFLICT(journal_id) DO UPDATE SET article_topics=excluded.article_topics,
                 index_dirty=1, updated_at=CURRENT_TIMESTAMP, version=version+1",
                params![journal.id, metadata.article_topics],
            )?;
            mark_index_dirty(conn, journal.id)?;
        }
        record_source(conn, journal.id, "article_topics", "crossref", &metadata.source_url, "updated", "Crossref 近期文章标题", dry_run)?;
        fields.push(updated_field("article_topics", "crossref", &metadata.source_url, "已补充近期文章标题"));
        updated = true;
    }
    if !updated {
        fields.push(EnrichmentFieldReport {
            field_name: "issn/eissn/article_topics".to_string(),
            status: "skipped".to_string(),
            source_kind: "crossref".to_string(),
            source_url: metadata.source_url.clone(),
            message: "字段已有内容，未覆盖手动或已有数据。".to_string(),
        });
    }
    Ok(())
}

fn apply_website(
    conn: &Connection,
    journal: &SelectedJournal,
    metadata: &WebsiteMetadata,
    dry_run: bool,
    fields: &mut Vec<EnrichmentFieldReport>,
) -> Result<()> {
    if journal.scope_text.trim().is_empty() && !metadata.scope_text.is_empty() {
        if !dry_run {
            conn.execute(
                "UPDATE journals SET scope_text=?1, updated_at=CURRENT_TIMESTAMP, version=version+1 WHERE id=?2",
                params![metadata.scope_text, journal.id],
            )?;
            mark_index_dirty(conn, journal.id)?;
        }
        let source_url = if metadata.scope_source_url.is_empty() {
            &metadata.source_url
        } else {
            &metadata.scope_source_url
        };
        record_source(conn, journal.id, "scope_text", "official_website", source_url, "updated", "官网明确 scope 段落", dry_run)?;
        fields.push(updated_field("scope_text", "official_website", source_url, "已补充官网 scope"));
    } else if !metadata.scope_text.is_empty() {
        fields.push(skipped_field(
            "scope_text",
            "official_website",
            if metadata.scope_source_url.is_empty() { &metadata.source_url } else { &metadata.scope_source_url },
            "已有 scope，未覆盖",
        ));
    }
    if journal.oa_flag.trim().is_empty() && !metadata.oa_flag.trim().is_empty() {
        if !dry_run {
            conn.execute(
                "UPDATE journals SET oa_flag=?1, updated_at=CURRENT_TIMESTAMP, version=version+1 WHERE id=?2",
                params![metadata.oa_flag, journal.id],
            )?;
        }
        let source_url = if metadata.oa_source_url.is_empty() {
            &metadata.source_url
        } else {
            &metadata.oa_source_url
        };
        record_source(conn, journal.id, "oa_flag", "official_website", source_url, "updated", "官网明确开放获取类型", dry_run)?;
        fields.push(updated_field("oa_flag", "official_website", source_url, "已补充开放获取类型"));
    } else if !metadata.oa_flag.is_empty() {
        fields.push(skipped_field(
            "oa_flag",
            "official_website",
            if metadata.oa_source_url.is_empty() { &metadata.source_url } else { &metadata.oa_source_url },
            "已有开放获取类型，未覆盖",
        ));
    }
    if journal.oa_detail.trim().is_empty() && !metadata.oa_detail.trim().is_empty() {
        if !dry_run {
            conn.execute(
                "UPDATE journals SET oa_detail=?1, updated_at=CURRENT_TIMESTAMP, version=version+1 WHERE id=?2",
                params![metadata.oa_detail, journal.id],
            )?;
        }
        let source_url = if metadata.oa_detail_source_url.is_empty() {
            &metadata.source_url
        } else {
            &metadata.oa_detail_source_url
        };
        record_source(conn, journal.id, "oa_detail", "official_website", source_url, "updated", "官网开放获取说明页", dry_run)?;
        fields.push(updated_field("oa_detail", "official_website", source_url, "已补充开放获取说明"));
    } else if !metadata.oa_detail.is_empty() {
        fields.push(skipped_field(
            "oa_detail",
            "official_website",
            if metadata.oa_detail_source_url.is_empty() { &metadata.source_url } else { &metadata.oa_detail_source_url },
            "已有开放获取说明，未覆盖",
        ));
    }
    if journal.jcr_rank_detail.trim().is_empty() && !metadata.jcr_rank_detail.trim().is_empty() {
        if !dry_run {
            conn.execute(
                "UPDATE journals SET jcr_rank_detail=?1, updated_at=CURRENT_TIMESTAMP, version=version+1 WHERE id=?2",
                params![metadata.jcr_rank_detail, journal.id],
            )?;
        }
        let source_url = if metadata.jcr_source_url.is_empty() {
            &metadata.source_url
        } else {
            &metadata.jcr_source_url
        };
        record_source(conn, journal.id, "jcr_rank_detail", "official_website", source_url, "updated", "官网 metrics / bibliometrics 页", dry_run)?;
        fields.push(updated_field(
            "jcr_rank_detail",
            "official_website",
            source_url,
            "已补充 JCR 排名 / 百分位",
        ));
    } else if !metadata.jcr_rank_detail.is_empty() {
        fields.push(skipped_field(
            "jcr_rank_detail",
            "official_website",
            if metadata.jcr_source_url.is_empty() { &metadata.source_url } else { &metadata.jcr_source_url },
            "已有 JCR 排名信息，未覆盖",
        ));
    }
    if journal.jcr_rank_detail.trim().is_empty() && metadata.jcr_rank_detail.trim().is_empty() {
        fields.push(skipped_field(
            "jcr_rank_detail",
            "official_website",
            &metadata.source_url,
            "页面未识别到 JCR 排名或百分位",
        ));
    }
    if journal.submission_guidelines.trim().is_empty() && !metadata.submission_guidelines.is_empty() {
        if !dry_run {
            conn.execute(
                "INSERT INTO journal_user_profiles (journal_id, submission_guidelines, index_dirty)
                 VALUES (?1,?2,1)
                 ON CONFLICT(journal_id) DO UPDATE SET submission_guidelines=excluded.submission_guidelines,
                 index_dirty=1, updated_at=CURRENT_TIMESTAMP, version=version+1",
                params![journal.id, metadata.submission_guidelines],
            )?;
            mark_index_dirty(conn, journal.id)?;
        }
        let source_url = if metadata.submission_source_url.is_empty() {
            &metadata.source_url
        } else {
            &metadata.submission_source_url
        };
        record_source(conn, journal.id, "submission_guidelines", "official_website", source_url, "updated", "官网明确投稿指南段落", dry_run)?;
        fields.push(updated_field(
            "submission_guidelines",
            "official_website",
            source_url,
            "已补充官网投稿指南",
        ));
    } else if !metadata.submission_guidelines.is_empty() {
        fields.push(skipped_field(
            "submission_guidelines",
            "official_website",
            if metadata.submission_source_url.is_empty() { &metadata.source_url } else { &metadata.submission_source_url },
            "已有投稿指南，未覆盖",
        ));
    }
    if journal.apc.trim().is_empty() && !metadata.apc_text.trim().is_empty() {
        if !dry_run {
            conn.execute(
                "INSERT INTO journal_user_profiles (journal_id, apc, index_dirty)
                 VALUES (?1,?2,1)
                 ON CONFLICT(journal_id) DO UPDATE SET apc=excluded.apc,
                 index_dirty=1, updated_at=CURRENT_TIMESTAMP, version=version+1",
                params![journal.id, metadata.apc_text],
            )?;
            mark_index_dirty(conn, journal.id)?;
        }
        let source_url = if metadata.apc_source_url.is_empty() {
            &metadata.source_url
        } else {
            &metadata.apc_source_url
        };
        record_source(conn, journal.id, "apc", "official_website", source_url, "updated", "官网费用或 APC 页面", dry_run)?;
        fields.push(updated_field("apc", "official_website", source_url, "已补充版面费 / APC"));
    } else if !metadata.apc_text.is_empty() {
        fields.push(skipped_field(
            "apc",
            "official_website",
            if metadata.apc_source_url.is_empty() { &metadata.source_url } else { &metadata.apc_source_url },
            "已有版面费信息，未覆盖",
        ));
    }
    if journal.review_cycle.trim().is_empty() && !metadata.review_cycle.trim().is_empty() {
        if !dry_run {
            conn.execute(
                "INSERT INTO journal_user_profiles (journal_id, review_cycle, index_dirty)
                 VALUES (?1,?2,1)
                 ON CONFLICT(journal_id) DO UPDATE SET review_cycle=excluded.review_cycle,
                 index_dirty=1, updated_at=CURRENT_TIMESTAMP, version=version+1",
                params![journal.id, metadata.review_cycle],
            )?;
            mark_index_dirty(conn, journal.id)?;
        }
        let source_url = if metadata.review_source_url.is_empty() {
            &metadata.source_url
        } else {
            &metadata.review_source_url
        };
        record_source(conn, journal.id, "review_cycle", "official_website", source_url, "updated", "官网审稿时长 / 首次决定时间", dry_run)?;
        fields.push(updated_field("review_cycle", "official_website", source_url, "已补充审稿周期"));
    } else if !metadata.review_cycle.is_empty() {
        fields.push(skipped_field(
            "review_cycle",
            "official_website",
            if metadata.review_source_url.is_empty() { &metadata.source_url } else { &metadata.review_source_url },
            "已有审稿周期，未覆盖",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct FetchedPage {
    url: String,
    html: String,
    text: String,
}

async fn crawl_site_pages(client: &Client, start_url: &str) -> Result<Vec<FetchedPage>> {
    let start = Url::parse(start_url).context("官网地址格式不正确")?;
    let mut queue = vec![start.clone()];
    let mut queued = HashSet::from([start.as_str().to_string()]);
    for fallback in site_fallback_urls(&start) {
        if let Ok(url) = Url::parse(&fallback) {
            if queued.insert(url.as_str().to_string()) {
                queue.push(url);
            }
        }
    }

    let mut pages = Vec::new();
    let mut seen = HashSet::new();
    let mut index = 0usize;
    while index < queue.len() && pages.len() < 6 {
        let current = queue[index].clone();
        index += 1;
        let current_key = current.as_str().to_string();
        if !seen.insert(current_key) {
            continue;
        }
        let page = match fetch_page(client, &current).await {
            Ok(page) => page,
            Err(_) => continue,
        };
        for candidate in extract_candidate_links(&page.html, &current) {
            if pages.len() + queue.len() >= 12 {
                break;
            }
            if queued.insert(candidate.as_str().to_string()) {
                queue.push(candidate);
            }
        }
        pages.push(page);
    }

    if pages.is_empty() {
        anyhow::bail!("官网页面抓取失败");
    }
    Ok(pages)
}

async fn fetch_page(client: &Client, url: &Url) -> Result<FetchedPage> {
    let response = client.get(url.clone()).send().await?.error_for_status()?;
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_lowercase();
    if !content_type.contains("text/html") && !content_type.is_empty() {
        anyhow::bail!("页面不是 HTML");
    }
    let bytes = response.bytes().await?;
    if bytes.len() > MAX_HTML_BYTES {
        anyhow::bail!("页面超过 2MB，已跳过");
    }
    let html = String::from_utf8_lossy(&bytes).to_string();
    let text = html_to_text(&html);
    Ok(FetchedPage {
        url: url.as_str().to_string(),
        html,
        text,
    })
}

fn extract_candidate_links(html: &str, base: &Url) -> Vec<Url> {
    static HREF_RE: OnceLock<Regex> = OnceLock::new();
    let href_re = HREF_RE.get_or_init(|| Regex::new(r#"(?i)href\s*=\s*["']([^"'#>]+)["']"#).unwrap());
    let mut scored = href_re
        .captures_iter(html)
        .filter_map(|capture| capture.get(1).map(|value| value.as_str().trim().to_string()))
        .filter_map(|raw| normalize_candidate_url(base, &raw))
        .map(|url| (score_candidate_url(url.as_str()), url))
        .filter(|(score, _)| *score > 0)
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| right.0.cmp(&left.0));
    scored
        .into_iter()
        .map(|(_, url)| url)
        .take(5)
        .collect()
}

fn normalize_candidate_url(base: &Url, raw: &str) -> Option<Url> {
    let lower = raw.trim().to_lowercase();
    if lower.starts_with("javascript:")
        || lower.starts_with("mailto:")
        || lower.starts_with('#')
        || lower.is_empty()
    {
        return None;
    }
    let resolved = base.join(raw).ok()?;
    let base_host = base.host_str()?;
    let candidate_host = resolved.host_str()?;
    if candidate_host != base_host {
        return None;
    }
    let path = resolved.path().to_lowercase();
    if path.ends_with(".pdf") || path.ends_with(".zip") || path.ends_with(".doc") || path.ends_with(".docx") {
        return None;
    }
    Some(resolved)
}

fn score_candidate_url(url: &str) -> i32 {
    let lower = url.to_lowercase();
    let mut score = 0;
    for (needle, weight) in [
        ("journal-information", 100),
        ("aims", 95),
        ("scope", 95),
        ("author", 90),
        ("guideline", 90),
        ("submission", 90),
        ("open-access", 85),
        ("openaccess", 85),
        ("apc", 80),
        ("fee", 80),
        ("charge", 80),
        ("price", 80),
        ("bibliometr", 75),
        ("metric", 75),
        ("ranking", 70),
        ("rank", 70),
        ("journal", 40),
        ("about", 35),
    ] {
        if lower.contains(needle) {
            score += weight;
        }
    }
    score
}

fn site_fallback_urls(start: &Url) -> Vec<String> {
    let host = start.host_str().unwrap_or_default().to_lowercase();
    let path = start.path().trim_matches('/').to_string();
    let mut urls = Vec::new();
    if host.contains("nature.com") {
        if !path.is_empty() {
            let first = path.split('/').next().unwrap_or_default();
            if !matches!(
                first,
                "journal-information" | "how-to-submit" | "open-access-fees-and-funding"
            ) {
                urls.push(format!("https://www.nature.com/{}/journal-information", first));
                urls.push(format!("https://www.nature.com/{}/how-to-submit", first));
                urls.push(format!("https://www.nature.com/{}/open-access-fees-and-funding", first));
            }
        }
    } else if host.contains("ieeeaccess.ieee.org") {
        urls.push("https://ieeeaccess.ieee.org/about/bibliometrics/".to_string());
        urls.push("https://ieeeaccess.ieee.org/about/article-processing-charges/".to_string());
        urls.push("https://ieeeaccess.ieee.org/authors/submission-guidelines/".to_string());
    } else if host.contains("journals.plos.org") {
        if path.is_empty() || path.contains("plosone") {
            urls.push("https://journals.plos.org/plosone/s/journal-information".to_string());
            urls.push("https://plos.org/publish/fees/".to_string());
        }
    } else if host.contains("frontiersin.org") {
        urls.push("https://www.frontiersin.org/about/open-access".to_string());
        urls.push("https://www.frontiersin.org/about/fee-policy".to_string());
        urls.push("https://www.frontiersin.org/for-authors/where-to-publish/what-are-publishing-fees".to_string());
    }
    urls
}

fn extract_open_access_text(text: &str) -> String {
    extract_section(
        text,
        &[
            "open access policy",
            "open access",
            "open-access",
            "开放获取",
            "开放 access",
        ],
    )
}

fn extract_fee_text(text: &str) -> String {
    extract_section(
        text,
        &[
            "article processing charges",
            "apc",
            "publication fees",
            "publishing fees",
            "publication fee",
            "page charges",
            "fee policy",
            "费用",
            "版面费",
        ],
    )
}

fn extract_review_cycle_text(text: &str) -> String {
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    for line in &lines {
        let lower = line.to_lowercase();
        let looks_like_cycle = lower.contains("first decision")
            || lower.contains("editorial decision")
            || lower.contains("peer review")
            || lower.contains("review time")
            || lower.contains("submission to publication")
            || lower.contains("time to publication")
            || lower.contains("publication time")
            || lower.contains("审稿")
            || lower.contains("决定")
            || lower.contains("出版时间")
            || lower.contains("评审");
        if !looks_like_cycle {
            continue;
        }
        if line.len() > 280 {
            return line.chars().take(280).collect();
        }
        return (*line).to_string();
    }
    String::new()
}

fn extract_jcr_rank_detail(text: &str) -> String {
    let lower = text.to_lowercase();
    for (original, lowered) in text.lines().zip(lower.lines()) {
        let clean = lowered.trim();
        if clean.is_empty() {
            continue;
        }
        let original_line = original.trim().to_string();
        let interesting = clean.contains("percentile")
            || clean.contains("top ")
            || clean.contains("rank")
            || clean.contains("ranking")
            || (clean.contains("quartile") && clean.contains('%'))
            || (clean.contains("q1") && clean.contains('%'));
        if interesting {
            if let Some(detail) = parse_rank_line(&original_line) {
                return detail;
            }
            if original_line.len() <= 220 {
                return original_line;
            }
        }
    }
    parse_rank_line(text).unwrap_or_default()
}

fn parse_rank_line(text: &str) -> Option<String> {
    static TOP_RE: OnceLock<Regex> = OnceLock::new();
    static RANK_RE: OnceLock<Regex> = OnceLock::new();
    static PERCENT_RE: OnceLock<Regex> = OnceLock::new();
    let top_re = TOP_RE.get_or_init(|| Regex::new(r"(?i)\btop\s+(\d{1,2}(?:\.\d+)?)\s*%").unwrap());
    let rank_re =
        RANK_RE.get_or_init(|| Regex::new(r"(?i)\b(?:ranked?|ranking)\D*(\d+)\D*(?:/|of)\D*(\d+)").unwrap());
    let percent_re =
        PERCENT_RE.get_or_init(|| Regex::new(r"(?i)\bpercentile\D*(\d{1,3}(?:\.\d+)?)\s*%").unwrap());

    if let Some(capture) = rank_re.captures(text) {
        let rank = capture.get(1)?.as_str().parse::<f64>().ok()?;
        let total = capture.get(2)?.as_str().parse::<f64>().ok()?;
        if total > 0.0 {
            let top_percent = rank / total * 100.0;
            return Some(format!(
                "Rank {}/{} (Top {:.1}%)",
                rank as i64,
                total as i64,
                top_percent
            ));
        }
    }
    if let Some(capture) = top_re.captures(text) {
        let value = capture.get(1)?.as_str();
        return Some(format!("Top {}%", value));
    }
    if let Some(capture) = percent_re.captures(text) {
        let value = capture.get(1)?.as_str();
        return Some(format!("Percentile {}%", value));
    }
    None
}

fn infer_oa_flag(oa_detail: &str, apc_text: &str) -> String {
    let combined = format!("{} {}", oa_detail, apc_text).to_lowercase();
    if combined.contains("diamond open access") || combined.contains("diamond oa") {
        return "Diamond OA".to_string();
    }
    if combined.contains("hybrid open access") || combined.contains("hybrid oa") {
        return "Hybrid OA".to_string();
    }
    if combined.contains("fully open access")
        || combined.contains("gold open access")
        || combined.contains("open access journal")
        || combined.contains("open access only")
    {
        return "Fully OA".to_string();
    }
    if combined.contains("open access") {
        return "OA".to_string();
    }
    if combined.contains("no article processing charge")
        || combined.contains("no apc")
        || combined.contains("no publication fee")
        || combined.contains("no page charge")
    {
        return "OA / No APC".to_string();
    }
    String::new()
}

fn updated_field(field_name: &str, source_kind: &str, source_url: &str, message: &str) -> EnrichmentFieldReport {
    EnrichmentFieldReport {
        field_name: field_name.to_string(),
        status: "updated".to_string(),
        source_kind: source_kind.to_string(),
        source_url: source_url.to_string(),
        message: message.to_string(),
    }
}

fn skipped_field(field_name: &str, source_kind: &str, source_url: &str, message: &str) -> EnrichmentFieldReport {
    EnrichmentFieldReport {
        field_name: field_name.to_string(),
        status: "skipped".to_string(),
        source_kind: source_kind.to_string(),
        source_url: source_url.to_string(),
        message: message.to_string(),
    }
}

fn record_source(
    conn: &Connection,
    journal_id: i64,
    field_name: &str,
    source_kind: &str,
    source_url: &str,
    status: &str,
    note: &str,
    dry_run: bool,
) -> Result<()> {
    if dry_run {
        return Ok(());
    }
    let hash = Sha256::digest(note.as_bytes());
    conn.execute(
        "INSERT INTO journal_enrichment_sources
         (journal_id,field_name,source_kind,source_url,fetched_at,status,note,content_hash)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            journal_id,
            field_name,
            source_kind,
            source_url,
            Local::now().to_rfc3339(),
            status,
            note,
            format!("{:x}", hash)
        ],
    )?;
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

fn normalize_issn(value: &str) -> Option<String> {
    let digits = value
        .chars()
        .filter(|character| character.is_ascii_digit() || *character == 'X' || *character == 'x')
        .collect::<String>()
        .to_uppercase();
    (digits.len() == 8).then(|| format!("{}-{}", &digits[..4], &digits[4..]))
}

fn normalize_title(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect::<String>()
        .to_uppercase()
}

fn title_matches(local: &str, remote: &str) -> bool {
    let local = normalize_title(local);
    let remote = normalize_title(remote);
    !local.is_empty()
        && !remote.is_empty()
        && (local == remote || local.contains(&remote) || remote.contains(&local))
}

fn html_to_text(html: &str) -> String {
    let normalized = Regex::new(r"(?is)(<br\s*/?>|</\s*(p|div|li|section|article|h[1-6]|tr|td|th)\s*>)")
        .unwrap()
        .replace_all(html, "\n");
    let without_script = Regex::new(r"(?is)<script[^>]*>.*?</script>")
        .unwrap()
        .replace_all(&normalized, " ");
    let without_style = Regex::new(r"(?is)<style[^>]*>.*?</style>")
        .unwrap()
        .replace_all(&without_script, " ");
    let without_noscript = Regex::new(r"(?is)<noscript[^>]*>.*?</noscript>")
        .unwrap()
        .replace_all(&without_style, " ");
    let without_tags = Regex::new(r"(?is)<[^>]+>")
        .unwrap()
        .replace_all(&without_noscript, " ");
    html_entities(&without_tags)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn html_entities(value: &str) -> String {
    value
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

fn extract_section(text: &str, headings: &[&str]) -> String {
    let lower = text.to_lowercase();
    let mut start = None;
    for heading in headings {
        if let Some(index) = lower.find(&heading.to_lowercase()) {
            start = Some(index + heading.len());
            break;
        }
    }
    let Some(start) = start else {
        return String::new();
    };
    let tail = text.get(start..).unwrap_or_default();
    let lower_tail = tail.to_lowercase();
    let boundaries = [
        "author guidelines",
        "instructions for authors",
        "submission guidelines",
        "references",
        "contact",
        "投稿指南",
        "投稿须知",
    ];
    let end = boundaries
        .iter()
        .filter_map(|boundary| lower_tail.find(&boundary.to_lowercase()))
        .min()
        .unwrap_or(1200);
    tail.get(..end.min(tail.len()))
        .unwrap_or(tail)
        .trim()
        .chars()
        .take(1200)
        .collect()
}

fn first_json_string(value: Option<&Value>) -> Option<&str> {
    match value? {
        Value::String(text) => Some(text.as_str()),
        Value::Array(items) => items.first().and_then(Value::as_str),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{extract_section, html_to_text, title_matches};

    #[test]
    fn html_parser_keeps_reliable_text_and_handles_unicode() {
        let html = r#"
            <html><script>ignore()</script><style>.x{}</style>
            <h1>Aims &amp; Scope</h1>
            <p>研究人工智能与交通系统。</p>
            <h2>Author Guidelines</h2>
        "#;
        let text = html_to_text(html);
        assert!(!text.contains("ignore"));
        assert!(!text.contains(".x"));
        assert!(extract_section(&text, &["aims & scope"]).contains("研究人工智能"));
    }

    #[test]
    fn title_matching_is_conservative() {
        assert!(title_matches("Journal of AI", "Journal of AI"));
        assert!(!title_matches("Journal of AI", "Journal of Biology"));
    }
}
