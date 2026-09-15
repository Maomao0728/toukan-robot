use crate::{
    ai_clients::{self, AiChatRequest, AiMessage},
    journal_store::{self, JournalListFilter, JournalRecord},
    paths::AppPaths,
    rag_index,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputQuality {
    pub can_recommend: bool,
    pub low_info: bool,
    pub filled_fields: usize,
    pub message: String,
}

pub fn assess_input_quality(
    title: &str,
    abstract_text: &str,
    keywords: &str,
    full_text: &str,
) -> InputQuality {
    let filled_fields = [title, abstract_text, keywords, full_text]
        .iter()
        .filter(|value| !value.trim().is_empty())
        .count();

    match filled_fields {
        0 => InputQuality {
            can_recommend: false,
            low_info: false,
            filled_fields,
            message: "请至少填写题目、摘要、关键词，或上传全文附件后再推荐。".to_string(),
        },
        1 => InputQuality {
            can_recommend: true,
            low_info: true,
            filled_fields,
            message: "当前信息较少，推荐可能不够准确。建议补充摘要、关键词或上传全文后再推荐。"
                .to_string(),
        },
        2 => InputQuality {
            can_recommend: true,
            low_info: false,
            filled_fields,
            message: "当前信息可以推荐；补充更多内容会更准。".to_string(),
        },
        _ => InputQuality {
            can_recommend: true,
            low_info: false,
            filled_fields,
            message: "当前信息较完整，可以开始推荐。".to_string(),
        },
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecommendationRequest {
    pub title: String,
    pub abstract_text: String,
    pub keywords: String,
    pub full_text: String,
    pub preferences: Vec<String>,
    pub journal_type: String,
    pub target_zone: String,
    #[serde(default)]
    pub target_jif_quartiles: Vec<String>,
    pub strategy: String,
    pub privacy_mode: bool,
    pub disable_web_search: bool,
    pub do_not_send_full_text: bool,
    pub ai_title_abstract_only: bool,
    #[serde(default)]
    pub exclude_journal_ids: Vec<i64>,
    #[serde(default)]
    pub exclude_journal_names: Vec<String>,
    #[serde(default)]
    pub refresh_round: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecommendationItem {
    pub journal_id: Option<i64>,
    pub journal_name: String,
    pub website: Option<String>,
    pub fit_level: String,
    pub source: String,
    pub semantic_score: f32,
    pub keyword_score: f32,
    pub rule_score: f32,
    pub total_score: f32,
    #[serde(default)]
    pub reference_only: bool,
    pub personal_experience_effect: String,
    pub reason: String,
    pub evidence: String,
    pub risk: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiRecommendationRequest {
    pub ai: AiChatRequest,
    pub recommendation: RecommendationRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiRecommendationResponse {
    pub items: Vec<RecommendationItem>,
    pub warning: Option<String>,
    pub raw_ai_text: Option<String>,
}

pub fn local_fallback_recommendations(request: &RecommendationRequest) -> Vec<RecommendationItem> {
    let topic = if request.title.trim().is_empty() {
        "当前论文主题".to_string()
    } else {
        request.title.trim().to_string()
    };
    ["冲刺", "稳妥", "稳妥", "保底", "保底"]
        .iter()
        .enumerate()
        .map(|(idx, level)| RecommendationItem {
            journal_id: None,
            website: None,
            journal_name: format!("待从本地库匹配的候选期刊 {}", idx + 1),
            fit_level: (*level).to_string(),
            source: "本地规则兜底".to_string(),
            semantic_score: 0.0,
            keyword_score: 0.0,
            rule_score: 0.0,
            total_score: 0.0,
            reference_only: true,
            personal_experience_effect: "暂无个人经验修正。".to_string(),
            reason: format!(
                "AI/RAG 尚未完成或不可用时，先按旧版规则为《{}》保留本地兜底推荐链路。",
                topic
            ),
            evidence: "后续将接入 SQLite 硬筛选、FTS5/BM25 和向量召回。".to_string(),
            risk: "这是骨架阶段占位结果，正式推荐必须来自本地候选库。".to_string(),
        })
        .collect()
}

pub fn local_database_recommendations(
    paths: &AppPaths,
    request: &RecommendationRequest,
) -> Vec<RecommendationItem> {
    let query = [
        request.title.as_str(),
        request.abstract_text.as_str(),
        request.keywords.as_str(),
        request.full_text.as_str(),
    ]
    .join(" ")
    .to_lowercase();
    let subject_guard = SubjectGuard::from_request(request, &query);
    let mut journals = match journal_store::list_journals(
        paths,
        &JournalListFilter {
            limit: 50_000,
            ..JournalListFilter::default()
        },
    ) {
        Ok(items) => items,
        Err(_) => return Vec::new(),
    };
    if !request.target_jif_quartiles.is_empty() {
        journals.retain(|journal| {
            request
                .target_jif_quartiles
                .iter()
                .any(|target| journal.jif_quartile.contains(target))
        });
    }
    let excluded_ids: HashSet<i64> = request.exclude_journal_ids.iter().copied().collect();
    let excluded_names: HashSet<String> = request
        .exclude_journal_names
        .iter()
        .map(|name| normalize_for_exclusion(name))
        .filter(|name| !name.is_empty())
        .collect();
    if !excluded_ids.is_empty() || !excluded_names.is_empty() {
        journals.retain(|journal| {
            !excluded_ids.contains(&journal.id)
                && !excluded_names.contains(&normalize_for_exclusion(&journal.name))
        });
    }
    if journals.is_empty() {
        return Vec::new();
    }
    journals.retain(|journal| !subject_guard.is_hard_excluded(journal));
    if journals.is_empty() {
        return Vec::new();
    }
    let tokens = tokenize_query(&query);
    let minimum_topic_hits = if tokens.len() >= 4 { 2 } else { 1 };
    let mut rag_hits_by_journal: HashMap<i64, rag_index::RagSearchHit> = HashMap::new();
    for hit in rag_index::search_rag(paths, &query, 80).unwrap_or_default() {
        rag_hits_by_journal
            .entry(hit.journal_id)
            .and_modify(|existing| {
                if hit.score > existing.score {
                    *existing = hit.clone();
                }
            })
            .or_insert(hit);
    }

    let mut strong_candidates = Vec::new();
    let mut reference_candidates = Vec::new();
    for journal in journals {
        let topical_hits = topical_match_count(&tokens, &journal);
        let semantic_score = semantic_candidate_score(rag_hits_by_journal.get(&journal.id));
        let domain_score = subject_guard.alignment_score(&journal);
        let domain_ok = subject_guard.is_empty() || domain_score > 0.0;
        if domain_ok && (topical_hits >= minimum_topic_hits || semantic_score >= 0.55) {
            strong_candidates.push(journal);
        } else if domain_ok && (topical_hits >= 1 || semantic_score >= 0.18) {
            reference_candidates.push(journal);
        }
    }
    let reference_only = strong_candidates.is_empty();
    journals = if reference_only {
        reference_candidates
    } else {
        strong_candidates
    };
    if journals.is_empty() {
        return Vec::new();
    }

    let mut ranked: Vec<(
        f32,
        f32,
        f32,
        Option<rag_index::RagSearchHit>,
        JournalRecord,
    )> = journals
        .into_iter()
        .map(|journal| {
            let searchable = journal_searchable_text(&journal);
            let keyword_hits = tokens
                .iter()
                .filter(|token| searchable.contains(token.as_str()))
                .count() as f32;
            let mut rule_score: f32 = 0.0;
            let preference_text = request.preferences.join(" ");
            if preference_text.contains("中文")
                && matches_any(&journal.source_db, &["CSSCI", "CSCD", "北大", "AMI"])
            {
                rule_score += 3.0;
            }
            if preference_text.contains("英文")
                && matches_any(&journal.source_db, &["SCIE", "SSCI"])
            {
                rule_score += 3.0;
            }
            if preference_text.contains("EI") && !journal.ei.trim().is_empty() {
                rule_score += 2.0;
            }
            if preference_text.contains("CCF") && !journal.ccf.trim().is_empty() {
                rule_score += 2.0;
            }
            if request.journal_type.contains("中文")
                && matches_any(&journal.source_db, &["CSSCI", "CSCD", "北大", "AMI"])
            {
                rule_score += 2.0;
            }
            if request.target_zone != "系统推荐"
                && (journal.cas_zone.contains(&request.target_zone)
                    || journal.cas_quartile.contains(&request.target_zone)
                    || journal.jif_quartile.contains(&request.target_zone))
            {
                rule_score += 2.0;
            }
            let domain_score = subject_guard.alignment_score(&journal);
            if domain_score > 0.0 {
                rule_score += domain_score;
            } else if !subject_guard.is_empty() {
                rule_score -= 4.0;
            }
            match journal.difficulty.as_str() {
                "极难" | "难"
                    if request.strategy.contains("稳妥") || request.strategy.contains("毕业") =>
                {
                    rule_score -= 2.0;
                }
                "易" if request.strategy.contains("稳妥") || request.strategy.contains("毕业") =>
                {
                    rule_score += 2.0;
                }
                _ => {}
            }
            let rag_hit = rag_hits_by_journal.get(&journal.id).cloned();
            let semantic_score = semantic_candidate_score(rag_hit.as_ref());
            let preference_adjustment =
                personal_preference_adjustment(&journal.recommendation_preference);
            let keyword_points = if tokens.is_empty() {
                0.0
            } else {
                (keyword_hits / tokens.len() as f32 * 20.0).min(20.0)
            };
            let semantic_points = semantic_score * 35.0;
            let catalogue_points = (if journal.subjects.trim().is_empty() {
                0.0
            } else {
                10.0
            }) + (if journal.source_db.trim().is_empty() {
                0.0
            } else {
                8.0
            }) + (if journal.impact_factor.is_some() {
                4.0
            } else {
                0.0
            });
            let strategy_points: f32 = (rule_score * 2.0).clamp(-12.0_f32, 15.0_f32);
            let personal_points = (preference_adjustment * 2.0).clamp(-20.0, 10.0);
            let total_score = (8.0
                + keyword_points
                + semantic_points
                + catalogue_points
                + strategy_points
                + personal_points)
                .clamp(0.0, 100.0);
            (total_score, keyword_points, rule_score, rag_hit, journal)
        })
        .collect();
    ranked.sort_by(|left, right| right.0.total_cmp(&left.0));

    let levels = if reference_only {
        ["参考", "参考", "参考", "参考", "参考"]
    } else {
        ["冲刺", "稳妥", "稳妥", "保底", "保底"]
    };
    ranked
        .into_iter()
        .take(5)
        .enumerate()
        .map(|(index, (score, keyword_score, rule_score, rag_hit, journal))| RecommendationItem {
            journal_id: Some(journal.id),
            journal_name: journal.name.clone(),
            website: if journal.website.trim().is_empty() { None } else { Some(journal.website.clone()) },
            fit_level: levels[index].to_string(),
            source: if rag_hit.is_some() {
                "本地 SQLite + RAG/FTS + 规则".to_string()
            } else {
                "本地 SQLite + 规则".to_string()
            },
            semantic_score: semantic_candidate_score(rag_hit.as_ref()),
            keyword_score: keyword_score.max(0.0),
            rule_score: rule_score.max(0.0),
            total_score: score,
            reference_only,
            personal_experience_effect: personal_experience_effect(&journal),
            reason: format!(
                "{}根据本地期刊字段、RAG/FTS 知识库命中、分区收录条件和个人经验综合排序；综合分 {:.2}。",
                if reference_only {
                    "本地库没有高匹配期刊，以下是低匹配参考候选；"
                } else {
                    ""
                },
                score.max(0.0)
            ),
            evidence: format!(
                "投稿方向：{}；来源：{}；学科：{}；JIF：{}；中科院：{}；影响因子：{}{}",
                subject_guard.label(),
                journal.source_db,
                journal.subjects,
                journal.jif_quartile,
                journal.cas_quartile,
                journal
                    .impact_factor
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "未记录".to_string()),
                rag_hit
                    .as_ref()
                    .map(explain_rag_hit)
                    .unwrap_or_default()
            ),
            risk: "推荐依据来自本地库和本地索引；分区、影响因子、收录情况和官网投稿要求仍需人工核验。".to_string(),
        })
        .collect()
}

fn tokenize_query(query: &str) -> HashSet<String> {
    let mut tokens = HashSet::new();
    let ignored = [
        "研究", "方法", "基于", "数据", "模型", "系统", "分析", "融合", "预测", "智能", "论文",
        "算法", "应用", "设计", "问题", "机制", "技术", "实现", "网络", "神经", "多源", "短时",
        "城市", "道路", "相关", "提出", "一种",
    ];
    for token in query
        .split(|character: char| {
            !character.is_alphanumeric() && !('\u{4e00}'..='\u{9fff}').contains(&character)
        })
        .map(str::trim)
        .filter(|token| token.len() >= 2)
    {
        let characters: Vec<char> = token.chars().collect();
        if characters
            .iter()
            .all(|character| ('\u{4e00}'..='\u{9fff}').contains(character))
        {
            for window in characters.windows(2) {
                let term: String = window.iter().collect();
                if !ignored.contains(&term.as_str()) {
                    tokens.insert(term);
                }
            }
            if characters.len() >= 3 {
                for window in characters.windows(3) {
                    let term: String = window.iter().collect();
                    if !ignored.contains(&term.as_str()) {
                        tokens.insert(term);
                    }
                }
            }
        } else {
            let term = token.to_lowercase();
            if term.len() >= 3 && !matches!(term.as_str(), "the" | "and" | "for" | "with" | "from")
            {
                tokens.insert(term);
            }
        }
    }
    expand_cross_language_terms(query, &mut tokens);
    tokens
}

#[derive(Debug, Clone)]
struct SubjectProfile {
    key: &'static str,
    label: &'static str,
    triggers: &'static [&'static str],
    positive: &'static [&'static str],
}

#[derive(Debug, Clone)]
struct SubjectGuard {
    profiles: Vec<&'static SubjectProfile>,
}

static SUBJECT_PROFILES: &[SubjectProfile] = &[
    SubjectProfile {
        key: "computer",
        label: "计算机科学",
        triggers: &[
            "计算机",
            "人工智能",
            "图神经",
            "神经网络",
            "深度学习",
            "机器学习",
            "算法",
            "数据挖掘",
            "graph neural",
            "gnn",
            "deep learning",
            "machine learning",
            "computer",
            "artificial intelligence",
        ],
        positive: &[
            "计算机",
            "人工智能",
            "软件",
            "算法",
            "数据挖掘",
            "模式识别",
            "图神经",
            "机器学习",
            "深度学习",
            "computer",
            "computing",
            "informatics",
            "information",
            "artificial intelligence",
            "machine learning",
            "deep learning",
            "neural network",
            "graph neural",
            "data mining",
            "pattern recognition",
        ],
    },
    SubjectProfile {
        key: "transportation",
        label: "智能交通/交通工程",
        triggers: &[
            "交通",
            "道路",
            "车流",
            "流量预测",
            "智能交通",
            "智慧交通",
            "traffic",
            "transport",
            "transportation",
            "mobility",
        ],
        positive: &[
            "交通",
            "道路",
            "运输",
            "智能交通",
            "智慧交通",
            "交通工程",
            "traffic",
            "transport",
            "transportation",
            "mobility",
            "vehicle",
            "road",
            "urban transport",
            "intelligent transportation",
        ],
    },
    SubjectProfile {
        key: "management",
        label: "管理科学",
        triggers: &["管理", "决策", "供应链", "management", "decision", "supply chain"],
        positive: &["管理", "决策", "供应链", "management", "decision", "operations research", "supply chain"],
    },
    SubjectProfile {
        key: "marxism",
        label: "马克思主义",
        triggers: &["马克思", "思想政治", "党建", "marxism"],
        positive: &["马克思", "思想政治", "党建", "marxism", "political education"],
    },
    SubjectProfile {
        key: "education",
        label: "教育学",
        triggers: &["教育", "教学", "课程", "education", "teaching"],
        positive: &["教育", "教学", "课程", "education", "teaching", "pedagogy"],
    },
    SubjectProfile {
        key: "medicine",
        label: "医学",
        triggers: &["医学", "临床", "患者", "medical", "clinical", "patient"],
        positive: &["医学", "临床", "患者", "medical", "clinical", "medicine", "health"],
    },
    SubjectProfile {
        key: "history",
        label: "历史学",
        triggers: &["历史", "史学", "history"],
        positive: &["历史", "史学", "history", "historical"],
    },
    SubjectProfile {
        key: "literature",
        label: "文学",
        triggers: &["文学", "诗歌", "小说", "literature", "poetry", "novel"],
        positive: &["文学", "诗歌", "小说", "literature", "poetry", "novel", "philology"],
    },
];

impl SubjectGuard {
    fn from_request(request: &RecommendationRequest, query: &str) -> Self {
        let selected = request
            .preferences
            .iter()
            .filter_map(|preference| preference.strip_prefix("投稿方向:"))
            .flat_map(|value| value.split('|'))
            .map(str::trim)
            .filter(|value| !value.is_empty() && *value != "自动识别")
            .collect::<Vec<_>>();
        let mut profiles = Vec::new();
        if selected.is_empty() {
            for profile in SUBJECT_PROFILES {
                if profile
                    .triggers
                    .iter()
                    .any(|trigger| contains_case_insensitive(query, trigger))
                {
                    profiles.push(profile);
                }
            }
        } else {
            for item in selected {
                if let Some(profile) = SUBJECT_PROFILES
                    .iter()
                    .find(|profile| profile.label == item || profile.key == item)
                {
                    profiles.push(profile);
                }
            }
        }
        profiles.sort_by_key(|profile| profile.key);
        profiles.dedup_by_key(|profile| profile.key);
        Self { profiles }
    }

    fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }

    fn label(&self) -> String {
        if self.profiles.is_empty() {
            "自动识别未限定".to_string()
        } else {
            self.profiles
                .iter()
                .map(|profile| profile.label)
                .collect::<Vec<_>>()
                .join(" / ")
        }
    }

    fn alignment_score(&self, journal: &JournalRecord) -> f32 {
        if self.profiles.is_empty() {
            return 0.0;
        }
        let text = journal_searchable_text(journal);
        let hits = self
            .profiles
            .iter()
            .flat_map(|profile| profile.positive.iter())
            .filter(|term| contains_case_insensitive(&text, term))
            .count() as f32;
        (hits * 1.5).min(6.0)
    }

    fn is_hard_excluded(&self, journal: &JournalRecord) -> bool {
        if self.profiles.is_empty() {
            return false;
        }
        let text = journal_searchable_text(journal);
        let humanities_hit = has_unrelated_humanities_terms(&text);
        if humanities_hit
            && self.has_profile("computer")
            && !self.has_profile_alignment(journal, "computer")
        {
            return true;
        }
        let has_alignment = self.alignment_score(journal) > 0.0;
        if has_alignment {
            return false;
        }
        humanities_hit
    }

    fn has_profile(&self, key: &str) -> bool {
        self.profiles.iter().any(|profile| profile.key == key)
    }

    fn has_profile_alignment(&self, journal: &JournalRecord, key: &str) -> bool {
        let Some(profile) = self.profiles.iter().find(|profile| profile.key == key) else {
            return false;
        };
        let text = journal_searchable_text(journal);
        profile
            .positive
            .iter()
            .any(|term| contains_case_insensitive(&text, term))
    }
}

fn contains_case_insensitive(value: &str, term: &str) -> bool {
    value.to_lowercase().contains(&term.to_lowercase())
}

fn has_unrelated_humanities_terms(text: &str) -> bool {
    let unrelated = [
        "历史",
        "史学",
        "文学",
        "诗歌",
        "小说",
        "语言学",
        "艺术",
        "哲学",
        "history",
        "historical",
        "literature",
        "poetry",
        "novel",
        "language",
        "linguistics",
        "arts",
        "philosophy",
        "humanities",
    ];
    unrelated
        .iter()
        .any(|term| contains_case_insensitive(text, term))
}

fn expand_cross_language_terms(query: &str, tokens: &mut HashSet<String>) {
    let lower = query.to_lowercase();
    let rules: &[(&[&str], &[&str])] = &[
        (
            &["交通", "道路", "车流", "流量", "traffic", "transport"],
            &[
                "traffic",
                "transport",
                "transportation",
                "intelligent transportation",
                "mobility",
                "vehicle",
                "road",
                "urban transport",
            ],
        ),
        (
            &["图神经", "gnn", "graph neural"],
            &[
                "graph neural",
                "gnn",
                "graph learning",
                "spatio-temporal",
                "spatiotemporal",
            ],
        ),
        (
            &["时空", "短时", "预测", "forecast"],
            &[
                "forecast",
                "forecasting",
                "prediction",
                "spatio-temporal",
                "time series",
            ],
        ),
        (
            &["深度学习", "机器学习", "deep learning", "machine learning"],
            &[
                "deep learning",
                "machine learning",
                "neural network",
                "artificial intelligence",
            ],
        ),
        (
            &["智慧交通", "智能交通", "its"],
            &[
                "intelligent transportation",
                "intelligent transport",
                "smart mobility",
            ],
        ),
    ];
    for (triggers, expansions) in rules {
        if triggers
            .iter()
            .any(|trigger| query.contains(trigger) || lower.contains(trigger))
        {
            for expansion in *expansions {
                tokens.insert((*expansion).to_string());
            }
        }
    }
}

fn journal_searchable_text(journal: &JournalRecord) -> String {
    format!(
        "{} {} {} {} {} {} {} {} {} {}",
        journal.name,
        journal.subjects,
        journal.scope_text,
        journal.article_topics,
        journal.submission_guidelines,
        journal.source_db,
        journal.tags,
        journal.summary,
        journal.experience,
        journal.suitable_topics,
    )
    .to_lowercase()
}

fn topical_match_count(tokens: &HashSet<String>, journal: &JournalRecord) -> usize {
    let searchable = journal_searchable_text(journal);
    tokens
        .iter()
        .filter(|term| searchable.contains(term.as_str()))
        .count()
}

fn semantic_candidate_score(hit: Option<&rag_index::RagSearchHit>) -> f32 {
    let Some(hit) = hit else {
        return 0.0;
    };
    if is_content_rag_hit(hit) {
        hit.score.clamp(0.0, 1.0)
    } else {
        (hit.score * 0.25).clamp(0.0, 0.25)
    }
}

fn is_content_rag_hit(hit: &rag_index::RagSearchHit) -> bool {
    matches!(
        hit.chunk_type.as_str(),
        "scope" | "articles" | "experience" | "guidelines"
    )
}

fn matches_any(value: &str, candidates: &[&str]) -> bool {
    candidates.iter().any(|candidate| value.contains(candidate))
}

fn normalize_for_exclusion(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn personal_preference_adjustment(preference: &str) -> f32 {
    if preference.contains("不推荐") {
        -8.0
    } else if preference.contains("少推荐") {
        -3.0
    } else if preference.contains("保底") {
        2.0
    } else if preference.contains("冲刺") {
        1.0
    } else {
        0.0
    }
}

fn personal_experience_effect(journal: &JournalRecord) -> String {
    let mut parts = Vec::new();
    if !journal.difficulty.trim().is_empty() {
        parts.push(format!("难度：{}", journal.difficulty));
    }
    if !journal.recommendation_preference.trim().is_empty() {
        parts.push(format!("推荐偏好：{}", journal.recommendation_preference));
    }
    if !journal.apc.trim().is_empty() {
        parts.push(format!("版面费：{}", journal.apc));
    }
    if !journal.review_cycle.trim().is_empty() {
        parts.push(format!("审稿周期：{}", journal.review_cycle));
    }
    if parts.is_empty() {
        "个人经验：暂未记录".to_string()
    } else {
        format!("个人经验：{}", parts.join("；"))
    }
}

fn explain_rag_hit(hit: &rag_index::RagSearchHit) -> String {
    let note = if is_content_rag_hit(hit) {
        ""
    } else {
        "（基础档案命中，仅作弱参考）"
    };
    if hit.snippet.trim().is_empty() {
        format!("；RAG命中：{} chunk{}", hit.chunk_type, note)
    } else {
        format!(
            "；RAG命中：{} chunk{}，{}",
            hit.chunk_type, note, hit.snippet
        )
    }
}

pub async fn ai_recommend(
    paths: &AppPaths,
    request: &AiRecommendationRequest,
) -> Result<AiRecommendationResponse, String> {
    let quality = assess_input_quality(
        &request.recommendation.title,
        &request.recommendation.abstract_text,
        &request.recommendation.keywords,
        &request.recommendation.full_text,
    );
    if !quality.can_recommend {
        return Err(quality.message);
    }

    let local_items = local_database_recommendations(paths, &request.recommendation);
    let mut warning_parts = recommendation_warnings(paths, &local_items, quality.low_info);
    if local_items.is_empty() {
        return Ok(AiRecommendationResponse {
            items: Vec::new(),
            warning: Some(
                "本地期刊库中没有找到与当前论文主题匹配的候选期刊，不是软件故障。可补充英文关键词、放宽分区筛选，或使用联网补充继续查找。".to_string(),
            ),
            raw_ai_text: None,
        });
    }
    if request.recommendation.privacy_mode {
        warning_parts
            .push("隐私模式已开启：本次仅使用本地 RAG/规则推荐，没有调用 AI 接口。".to_string());
        return Ok(AiRecommendationResponse {
            items: local_items,
            warning: Some(warning_parts.join("；")),
            raw_ai_text: None,
        });
    }
    let mut prompt_request = request.recommendation.clone();
    if prompt_request.ai_title_abstract_only {
        prompt_request.keywords.clear();
        prompt_request.full_text.clear();
    } else if prompt_request.do_not_send_full_text {
        prompt_request.full_text.clear();
    }
    let mut ai_request = request.ai.clone();
    ai_request.messages = vec![
        AiMessage {
            role: "system".to_string(),
            content: "你是严谨的科研投稿顾问。只能从用户提供的候选期刊中选择和解释，不允许编造候选外期刊。优先返回简短中文说明。".to_string(),
        },
        AiMessage {
            role: "user".to_string(),
            content: build_ai_prompt(&prompt_request, &local_items),
        },
    ];
    ai_request.temperature = if ai_request.temperature <= 0.0 {
        0.15
    } else {
        ai_request.temperature
    };
    ai_request.max_tokens = if ai_request.max_tokens == 0 {
        1800
    } else {
        ai_request.max_tokens
    };

    match ai_clients::chat_completion(&ai_request).await {
        Ok(text) => Ok(AiRecommendationResponse {
            items: local_items,
            warning: warning_parts_to_option(warning_parts),
            raw_ai_text: Some(text),
        }),
        Err(error) => Ok(AiRecommendationResponse {
            items: local_items,
            warning: Some(
                [
                    warning_parts.join("；"),
                    format!("AI 推荐暂不可用，已使用本地规则兜底：{}", error),
                ]
                .into_iter()
                .filter(|item| !item.trim().is_empty())
                .collect::<Vec<_>>()
                .join("；"),
            ),
            raw_ai_text: None,
        }),
    }
}

fn recommendation_warnings(
    paths: &AppPaths,
    local_items: &[RecommendationItem],
    low_info: bool,
) -> Vec<String> {
    let mut warnings = Vec::new();
    if low_info {
        warnings.push("当前信息较少，推荐可能不够准确；AI 解释仅供初筛。".to_string());
    }
    if local_items.iter().any(|item| item.reference_only) {
        warnings
            .push("本地库没有高匹配期刊，以下是低匹配参考候选；请人工核验学科范围。".to_string());
    }
    if let Ok(health) = rag_index::current_index_health(paths) {
        if health.status == "构建中" || health.status == "需要更新" {
            warnings.push(format!(
                "RAG/Embedding 索引{}：{}；索引完成后语义相似度会更稳定。",
                health.status, health.detail
            ));
        }
    }
    warnings
}

fn warning_parts_to_option(warnings: Vec<String>) -> Option<String> {
    if warnings.is_empty() {
        None
    } else {
        Some(warnings.join("；"))
    }
}

fn build_ai_prompt(request: &RecommendationRequest, candidates: &[RecommendationItem]) -> String {
    let candidates_json =
        serde_json::to_string_pretty(candidates).unwrap_or_else(|_| "[]".to_string());
    format!(
        "论文题目：{}\n关键词：{}\n摘要：{}\n正文节选：{}\n投稿偏好：{}\n目标期刊类型：{}\n目标分区：{}\n推荐策略：{}\n候选期刊 JSON：{}\n\n请从候选中推荐 5 个，并说明每个期刊的定位、匹配依据、个人经验影响和风险提醒。联网信息、影响因子、分区和收录情况必须提示人工核验。",
        request.title,
        request.keywords,
        request.abstract_text,
        request.full_text.chars().take(6000).collect::<String>(),
        request.preferences.join("、"),
        request.journal_type,
        request.target_zone,
        request.strategy,
        candidates_json
    )
}
