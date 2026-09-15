use crate::{journal_store, paths::AppPaths};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchSource {
    pub provider: String,
    pub query: String,
    pub title: String,
    pub url: String,
    pub snippet: String,
    #[serde(default)]
    pub jif_quartile: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchRequest {
    pub title: String,
    pub abstract_text: String,
    pub keywords: String,
    pub prefer_chinese: bool,
    #[serde(default)]
    pub exclude_titles: Vec<String>,
    #[serde(default)]
    pub refresh_round: u32,
    #[serde(default)]
    pub target_jif_quartiles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchReport {
    pub sources: Vec<WebSearchSource>,
    pub warning: Option<String>,
}

pub fn build_web_queries(
    title: &str,
    abstract_text: &str,
    keywords: &str,
    prefer_chinese: bool,
) -> Vec<String> {
    let mut seed = [title, keywords, abstract_text]
        .iter()
        .map(|s| s.trim())
        .find(|s| !s.is_empty())
        .unwrap_or("journal")
        .chars()
        .take(80)
        .collect::<String>();
    if seed.is_empty() {
        seed = "journal".to_string();
    }
    let mut queries = vec![
        format!("{} journal", seed),
        format!("{} submission scope journal", seed),
        format!("{} author guidelines journal", seed),
    ];
    if prefer_chinese {
        queries.push(format!("{} CSSCI CSCD 北大核心 中文期刊", seed));
        queries.push(format!("{} 中文期刊 投稿", seed));
    } else {
        queries.push(format!("{} SCIE SSCI journal", seed));
        queries.push(format!("{} SCI journal", seed));
    }
    queries.sort();
    queries.dedup();
    queries.truncate(5);
    queries
}

pub async fn search_sources(request: &WebSearchRequest) -> WebSearchReport {
    let mut queries = build_web_queries(
        &request.title,
        &request.abstract_text,
        &request.keywords,
        request.prefer_chinese,
    );
    if !queries.is_empty() {
        let shift = request.refresh_round as usize % queries.len();
        queries.rotate_left(shift);
    }
    if queries.is_empty() {
        return WebSearchReport {
            sources: Vec::new(),
            warning: Some("没有生成可用的联网搜索关键词。".to_string()),
        };
    }

    let client = match reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(10))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) ToukanRobot/0.1")
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            return WebSearchReport {
                sources: Vec::new(),
                warning: Some(format!("联网搜索客户端初始化失败：{}", error)),
            };
        }
    };

    let mut sources = Vec::new();
    let mut errors = Vec::new();
    let mut seen = HashSet::new();
    for title in &request.exclude_titles {
        let normalized = normalize_title(title);
        if !normalized.is_empty() {
            seen.insert(format!("journal:{}", normalized));
        }
    }

    let offset = (request.refresh_round as usize * 8).min(80);
    for query in queries.iter().take(3) {
        match search_crossref(&client, query, offset).await {
            Ok(items) => push_unique(&mut sources, &mut seen, items),
            Err(error) => errors.push(format!("Crossref / {}：{}", query, error)),
        }
        if sources.len() >= 5 {
            break;
        }
    }

    sources.truncate(5);
    let warning = if sources.is_empty() {
        Some(if errors.is_empty() {
            "联网搜索没有找到可用结果。".to_string()
        } else {
            format!(
                "联网搜索暂时不可用：{}",
                errors.into_iter().take(2).collect::<Vec<_>>().join("；")
            )
        })
    } else {
        Some(
            "联网补充结果仅作参考；分区、影响因子、收录情况和投稿要求必须以官网和数据库为准。"
                .to_string(),
        )
    };
    WebSearchReport { sources, warning }
}

pub fn filter_sources_by_jif_quartiles(
    paths: &AppPaths,
    mut report: WebSearchReport,
    target_jif_quartiles: &[String],
) -> WebSearchReport {
    let targets = target_jif_quartiles
        .iter()
        .filter(|item| !item.trim().is_empty())
        .map(|item| item.trim().to_uppercase())
        .collect::<HashSet<_>>();
    if targets.is_empty() || report.sources.is_empty() {
        return report;
    }
    let local_journals = journal_store::list_journals(
        paths,
        &journal_store::JournalListFilter {
            limit: 50_000,
            ..journal_store::JournalListFilter::default()
        },
    )
    .unwrap_or_default();
    let mut filtered = Vec::new();
    for mut source in report.sources {
        if let Some(journal) = match_local_journal(&local_journals, &source) {
            let jif = journal.jif_quartile.trim().to_uppercase();
            if targets.iter().any(|target| jif.contains(target)) {
                source.jif_quartile = journal.jif_quartile.clone();
                source.snippet = format!(
                    "{}；本地库核验 JIF 分区：{}",
                    source.snippet, journal.jif_quartile
                );
                filtered.push(source);
            }
        }
    }
    let target_text = targets.into_iter().collect::<Vec<_>>().join(" / ");
    report.sources = filtered;
    report.warning = if report.sources.is_empty() {
        Some(format!(
            "联网补充找到了 Crossref 记录，但本地库无法核验为 {} 分区，所以没有展示。可切换分区、取消分区筛选，或先导入/补全该期刊分区数据。",
            target_text
        ))
    } else {
        Some(format!(
            "联网补充结果仅作参考；Crossref 不提供 JIF 分区，本次已按本地 SQLite 核验为 {} 后再展示，影响因子、收录情况和投稿要求仍需人工核验。",
            target_text
        ))
    };
    report
}

fn match_local_journal<'a>(
    journals: &'a [journal_store::JournalRecord],
    source: &WebSearchSource,
) -> Option<&'a journal_store::JournalRecord> {
    let source_issns = extract_issns(&source.snippet);
    if !source_issns.is_empty() {
        if let Some(journal) = journals.iter().find(|journal| {
            source_issns.iter().any(|issn| {
                normalize_issn(&journal.issn) == *issn || normalize_issn(&journal.eissn) == *issn
            })
        }) {
            return Some(journal);
        }
    }
    let source_title = normalize_title(&source.title);
    if source_title.is_empty() {
        return None;
    }
    journals.iter().find(|journal| {
        let local_title = normalize_title(&journal.name);
        !local_title.is_empty()
            && (local_title == source_title
                || local_title.contains(&source_title)
                || source_title.contains(&local_title))
    })
}

fn extract_issns(value: &str) -> HashSet<String> {
    Regex::new(r"(?i)\b\d{4}-\d{3}[\dX]\b")
        .ok()
        .map(|pattern| {
            pattern
                .find_iter(value)
                .map(|matched| normalize_issn(matched.as_str()))
                .filter(|issn| !issn.is_empty())
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default()
}

fn normalize_issn(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_uppercase)
        .collect()
}

async fn search_crossref(
    client: &reqwest::Client,
    query: &str,
    offset: usize,
) -> Result<Vec<WebSearchSource>, String> {
    let offset_text = offset.to_string();
    let data: Value = client
        .get("https://api.crossref.org/works")
        .query(&[
            ("query", query),
            ("rows", "15"),
            ("offset", offset_text.as_str()),
            ("select", "title,container-title,ISSN,URL,DOI,type"),
        ])
        .send()
        .await
        .map_err(|error| error.to_string())?
        .json()
        .await
        .map_err(|error| error.to_string())?;
    let mut out = Vec::new();
    if let Some(items) = data.pointer("/message/items").and_then(Value::as_array) {
        for item in items {
            if item.get("type").and_then(Value::as_str) != Some("journal-article") {
                continue;
            }
            let journal = first_array_text(item.get("container-title"));
            if journal.trim().is_empty() {
                continue;
            }
            let article = first_array_text(item.get("title"));
            let issn = item
                .get("ISSN")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            out.push(WebSearchSource {
                provider: "Crossref".to_string(),
                query: query.to_string(),
                title: journal,
                url: item
                    .get("URL")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                snippet: format!("相关论文：{}；ISSN：{}", article, issn),
                jif_quartile: String::new(),
            });
            if out.len() >= 8 {
                break;
            }
        }
    }
    Ok(out)
}

async fn search_duckduckgo(
    client: &reqwest::Client,
    query: &str,
) -> Result<Vec<WebSearchSource>, String> {
    let html = client
        .get("https://html.duckduckgo.com/html/")
        .query(&[("q", query)])
        .send()
        .await
        .map_err(|error| error.to_string())?
        .text()
        .await
        .map_err(|error| error.to_string())?;
    let block_re = Regex::new(r#"(?is)<div class="result__body".*?</div>\s*</div>"#).unwrap();
    let title_re = Regex::new(
        r#"(?is)<a\b(?=[^>]*class="[^"]*result__a[^"]*")(?=[^>]*href="([^"]+)")[^>]*>(.*?)</a>"#,
    )
    .unwrap();
    let snippet_re = Regex::new(
        r#"(?is)<(?:a|div)[^>]+class="[^"]*result__snippet[^"]*"[^>]*>(.*?)</(?:a|div)>"#,
    )
    .unwrap();
    let mut out = Vec::new();
    for block in block_re.find_iter(&html) {
        let block = block.as_str();
        let Some(caps) = title_re.captures(block) else {
            continue;
        };
        let url = unwrap_duckduckgo_url(caps.get(1).map(|m| m.as_str()).unwrap_or(""));
        let title = strip_html(caps.get(2).map(|m| m.as_str()).unwrap_or(""));
        if title.is_empty() || url.is_empty() {
            continue;
        }
        let snippet = snippet_re
            .captures(block)
            .and_then(|caps| caps.get(1))
            .map(|m| strip_html(m.as_str()))
            .unwrap_or_default();
        out.push(WebSearchSource {
            provider: "DuckDuckGo".to_string(),
            query: query.to_string(),
            title,
            url,
            snippet,
            jif_quartile: String::new(),
        });
        if out.len() >= 8 {
            break;
        }
    }
    Ok(out)
}

async fn search_bing(
    client: &reqwest::Client,
    query: &str,
) -> Result<Vec<WebSearchSource>, String> {
    let html = client
        .get("https://www.bing.com/search")
        .query(&[("q", query)])
        .send()
        .await
        .map_err(|error| error.to_string())?
        .text()
        .await
        .map_err(|error| error.to_string())?;
    let re = Regex::new(r#"(?is)<li\b[^>]*class="[^"]*\bb_algo\b[^"]*"[^>]*>.*?<h2[^>]*>\s*<a\b[^>]*href="([^"]+)"[^>]*>(.*?)</a>(?:.*?<p[^>]*>(.*?)</p>)?.*?</li>"#).unwrap();
    let mut out = Vec::new();
    for caps in re.captures_iter(&html) {
        let url = caps
            .get(1)
            .map(|m| html_unescape(m.as_str()))
            .unwrap_or_default();
        let title = caps
            .get(2)
            .map(|m| strip_html(m.as_str()))
            .unwrap_or_default();
        if url.is_empty() || title.is_empty() || url.contains("bing.com/search") {
            continue;
        }
        out.push(WebSearchSource {
            provider: "Bing".to_string(),
            query: query.to_string(),
            title,
            url,
            snippet: caps
                .get(3)
                .map(|m| strip_html(m.as_str()))
                .unwrap_or_default(),
            jif_quartile: String::new(),
        });
        if out.len() >= 8 {
            break;
        }
    }
    Ok(out)
}

fn push_unique(
    out: &mut Vec<WebSearchSource>,
    seen: &mut HashSet<String>,
    items: Vec<WebSearchSource>,
) {
    for item in items {
        let key = journal_identity_key(&item);
        if seen.insert(key) {
            out.push(item);
        }
        if out.len() >= 8 {
            break;
        }
    }
}

fn journal_identity_key(item: &WebSearchSource) -> String {
    let issn = Regex::new(r"(?i)\b\d{4}-\d{3}[\dX]\b")
        .ok()
        .and_then(|pattern| {
            pattern
                .find(&item.snippet)
                .map(|matched| matched.as_str().to_uppercase())
        });
    if let Some(issn) = issn {
        return format!("issn:{}", issn);
    }
    let normalized = normalize_title(&item.title);
    if normalized.is_empty() {
        item.url.to_lowercase()
    } else {
        format!("journal:{}", normalized)
    }
}

fn normalize_title(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn first_array_text(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn strip_html(value: &str) -> String {
    let tag_re = Regex::new(r#"(?is)<[^>]+>"#).unwrap();
    html_unescape(&tag_re.replace_all(value, " "))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn unwrap_duckduckgo_url(value: &str) -> String {
    let href = html_unescape(value);
    if let Some(index) = href.find("uddg=") {
        let encoded = href[index + 5..].split('&').next().unwrap_or("");
        return percent_decode(encoded);
    }
    if href.starts_with("//") {
        format!("https:{}", href)
    } else {
        href
    }
}

fn percent_decode(value: &str) -> String {
    let mut out = String::new();
    let mut chars = value.as_bytes().iter().copied().peekable();
    while let Some(byte) = chars.next() {
        if byte == b'%' {
            let hi = chars.next();
            let lo = chars.next();
            if let (Some(hi), Some(lo)) = (hi, lo) {
                if let Ok(hex) = u8::from_str_radix(&format!("{}{}", hi as char, lo as char), 16) {
                    out.push(hex as char);
                    continue;
                }
            }
        }
        out.push(byte as char);
    }
    out
}

fn html_unescape(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}
