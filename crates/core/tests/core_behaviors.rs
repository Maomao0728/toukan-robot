use std::time::{SystemTime, UNIX_EPOCH};
use toukan_robot_core::{
    db, journal_store, rag_index, recommendation, resource_import, web_search, AppPaths,
};

#[test]
fn title_only_can_recommend_with_low_info_warning() {
    let quality = recommendation::assess_input_quality("A Study on AI", "", "", "");
    assert!(quality.can_recommend);
    assert!(quality.low_info);
    assert_eq!(quality.filled_fields, 1);
    assert!(quality.message.contains("信息较少"));
}

#[test]
fn local_recommendation_uses_rag_hits_after_rebuild() {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let root = std::env::temp_dir().join(format!("toukan_robot_rag_test_{}", millis));
    let paths = AppPaths::from_root(root);
    let conn = db::open_database(&paths).unwrap();

    journal_store::upsert_journal(
        &conn,
        &journal_store::JournalUpsert {
            name: "Journal of Graph Traffic Forecasting".to_string(),
            issn: "1234-5678".to_string(),
            eissn: String::new(),
            source_db: "SCIE EI".to_string(),
            jif_quartile: "Q2".to_string(),
            cas_quartile: "Zone 3".to_string(),
            cas_zone: "3".to_string(),
            impact_factor: Some(3.2),
            wos_articles: Some(120.0),
            subjects: "graph neural networks traffic forecasting".to_string(),
            publisher: "Local Test Press".to_string(),
            ccf: String::new(),
            ei: "EI".to_string(),
            cssci: String::new(),
            cscd: String::new(),
            pku_core: String::new(),
            ami_level: String::new(),
            top_flag: String::new(),
            oa_flag: "OA".to_string(),
            oa_detail: String::new(),
            website: String::new(),
            difficulty: "easy".to_string(),
            recommendation_preference: "backup".to_string(),
            apc: "1000 USD".to_string(),
            review_cycle: "2 months".to_string(),
            tags: "fast review".to_string(),
            summary: "Good fit for graph neural network papers.".to_string(),
            experience: "Accepted similar traffic forecasting work.".to_string(),
            grade: "Test".to_string(),
        },
    )
    .unwrap();
    let results = recommendation::local_database_recommendations(
        &paths,
        &recommendation::RecommendationRequest {
            title: "Graph neural network traffic forecasting".to_string(),
            abstract_text: String::new(),
            keywords: String::new(),
            full_text: String::new(),
            preferences: vec!["EI".to_string()],
            journal_type: "system".to_string(),
            target_zone: "system".to_string(),
            target_jif_quartiles: Vec::new(),
            strategy: "safe".to_string(),
            privacy_mode: false,
            disable_web_search: false,
            do_not_send_full_text: false,
            ai_title_abstract_only: false,
            exclude_journal_ids: Vec::new(),
            exclude_journal_names: Vec::new(),
            refresh_round: 0,
        },
    );
    let first = results.first().unwrap();
    assert_eq!(first.journal_name, "Journal of Graph Traffic Forecasting");
    assert!(first.source.contains("RAG"));
    assert!(first.semantic_score > 0.0);
    assert!(first.evidence.contains("RAG"));
}

#[test]
fn computer_traffic_title_does_not_recommend_history_or_literature_journals() {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let root = std::env::temp_dir().join(format!("toukan_robot_subject_guard_test_{}", millis));
    let paths = AppPaths::from_root(root);
    let conn = db::open_database(&paths).unwrap();

    for (name, subjects, source_db) in [
        (
            "Journal of Intelligent Transportation and Graph Learning",
            "computer science artificial intelligence graph neural networks traffic forecasting intelligent transportation",
            "SCIE EI",
        ),
        (
            "Modern History Review",
            "history humanities transportation history literature",
            "A&HCI",
        ),
        (
            "Comparative Literature Quarterly",
            "literature poetry language humanities",
            "A&HCI",
        ),
    ] {
        journal_store::upsert_journal(
            &conn,
            &journal_store::JournalUpsert {
                name: name.to_string(),
                issn: String::new(),
                eissn: String::new(),
                source_db: source_db.to_string(),
                jif_quartile: "Q2".to_string(),
                cas_quartile: String::new(),
                cas_zone: String::new(),
                impact_factor: Some(2.0),
                wos_articles: Some(50.0),
                subjects: subjects.to_string(),
                publisher: String::new(),
                ccf: String::new(),
                ei: if source_db.contains("EI") { "EI" } else { "" }.to_string(),
                cssci: String::new(),
                cscd: String::new(),
                pku_core: String::new(),
                ami_level: String::new(),
                top_flag: String::new(),
                oa_flag: String::new(),
                oa_detail: String::new(),
                website: String::new(),
                difficulty: String::new(),
                recommendation_preference: String::new(),
                apc: String::new(),
                review_cycle: String::new(),
                tags: String::new(),
                summary: String::new(),
                experience: String::new(),
                grade: String::new(),
            },
        )
        .unwrap();
    }
    rag_index::rebuild_rag_index(&paths).unwrap();

    let results = recommendation::local_database_recommendations(
        &paths,
        &recommendation::RecommendationRequest {
            title: "基于图神经网络与多源交通数据融合的城市道路短时流量预测研究".to_string(),
            abstract_text: String::new(),
            keywords: String::new(),
            full_text: String::new(),
            preferences: vec!["投稿方向:计算机科学|智能交通/交通工程".to_string()],
            journal_type: "system".to_string(),
            target_zone: "system".to_string(),
            target_jif_quartiles: Vec::new(),
            strategy: "safe".to_string(),
            privacy_mode: false,
            disable_web_search: false,
            do_not_send_full_text: false,
            ai_title_abstract_only: false,
            exclude_journal_ids: Vec::new(),
            exclude_journal_names: Vec::new(),
            refresh_round: 0,
        },
    );

    assert!(!results.is_empty());
    assert!(results
        .iter()
        .any(|item| item.journal_name.contains("Intelligent Transportation")));
    assert!(results
        .iter()
        .all(|item| !item.journal_name.contains("History") && !item.journal_name.contains("Literature")));
    assert!(results[0].evidence.contains("计算机科学"));
}

#[test]
fn local_recommendation_can_refresh_past_previous_results() {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let root = std::env::temp_dir().join(format!("toukan_robot_refresh_test_{}", millis));
    let paths = AppPaths::from_root(root);
    let conn = db::open_database(&paths).unwrap();

    for name in [
        "Journal of Graph Traffic Forecasting A",
        "Journal of Graph Traffic Forecasting B",
    ] {
        journal_store::upsert_journal(
            &conn,
            &journal_store::JournalUpsert {
                name: name.to_string(),
                issn: String::new(),
                eissn: String::new(),
                source_db: "SCIE EI".to_string(),
                jif_quartile: "Q2".to_string(),
                cas_quartile: "Zone 3".to_string(),
                cas_zone: "3".to_string(),
                impact_factor: Some(2.5),
                wos_articles: Some(100.0),
                subjects: "graph neural networks traffic forecasting intelligent transportation"
                    .to_string(),
                publisher: "Local Test Press".to_string(),
                ccf: String::new(),
                ei: "EI".to_string(),
                cssci: String::new(),
                cscd: String::new(),
                pku_core: String::new(),
                ami_level: String::new(),
                top_flag: String::new(),
                oa_flag: String::new(),
                oa_detail: String::new(),
                website: String::new(),
                difficulty: "未评估".to_string(),
                recommendation_preference: "正常推荐".to_string(),
                apc: String::new(),
                review_cycle: String::new(),
                tags: String::new(),
                summary: String::new(),
                experience: String::new(),
                grade: "Test".to_string(),
            },
        )
        .unwrap();
    }
    rag_index::rebuild_rag_index(&paths).unwrap();

    let first_batch = recommendation::local_database_recommendations(
        &paths,
        &recommendation::RecommendationRequest {
            title: "Graph neural network traffic forecasting".to_string(),
            abstract_text: String::new(),
            keywords: String::new(),
            full_text: String::new(),
            preferences: Vec::new(),
            journal_type: "system".to_string(),
            target_zone: "system".to_string(),
            target_jif_quartiles: Vec::new(),
            strategy: "balanced".to_string(),
            privacy_mode: false,
            disable_web_search: false,
            do_not_send_full_text: false,
            ai_title_abstract_only: false,
            exclude_journal_ids: Vec::new(),
            exclude_journal_names: Vec::new(),
            refresh_round: 0,
        },
    );
    let first = first_batch.first().unwrap();
    let second_batch = recommendation::local_database_recommendations(
        &paths,
        &recommendation::RecommendationRequest {
            title: "Graph neural network traffic forecasting".to_string(),
            abstract_text: String::new(),
            keywords: String::new(),
            full_text: String::new(),
            preferences: Vec::new(),
            journal_type: "system".to_string(),
            target_zone: "system".to_string(),
            target_jif_quartiles: Vec::new(),
            strategy: "balanced".to_string(),
            privacy_mode: false,
            disable_web_search: false,
            do_not_send_full_text: false,
            ai_title_abstract_only: false,
            exclude_journal_ids: vec![first.journal_id.unwrap()],
            exclude_journal_names: vec![first.journal_name.clone()],
            refresh_round: 1,
        },
    );
    assert!(!second_batch.is_empty());
    assert_ne!(second_batch.first().unwrap().journal_id, first.journal_id);
}

#[test]
fn weak_topic_match_is_marked_reference_only() {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let root = std::env::temp_dir().join(format!("toukan_robot_weak_match_test_{}", millis));
    let paths = AppPaths::from_root(root);
    let conn = db::open_database(&paths).unwrap();

    journal_store::upsert_journal(
        &conn,
        &journal_store::JournalUpsert {
            name: "Journal of Flow Visualization and Image Processing".to_string(),
            issn: "9988-7766".to_string(),
            eissn: String::new(),
            source_db: "SCIE".to_string(),
            jif_quartile: "Q4".to_string(),
            cas_quartile: "Zone 4".to_string(),
            cas_zone: "4".to_string(),
            impact_factor: Some(0.8),
            wos_articles: Some(40.0),
            subjects: "traffic visualization image processing".to_string(),
            publisher: "Local Test Press".to_string(),
            ccf: String::new(),
            ei: String::new(),
            cssci: String::new(),
            cscd: String::new(),
            pku_core: String::new(),
            ami_level: String::new(),
            top_flag: String::new(),
            oa_flag: String::new(),
            oa_detail: String::new(),
            website: String::new(),
            difficulty: "未评估".to_string(),
            recommendation_preference: "正常推荐".to_string(),
            apc: String::new(),
            review_cycle: String::new(),
            tags: String::new(),
            summary: String::new(),
            experience: String::new(),
            grade: "Test".to_string(),
        },
    )
    .unwrap();
    rag_index::rebuild_rag_index(&paths).unwrap();

    let results = recommendation::local_database_recommendations(
        &paths,
        &recommendation::RecommendationRequest {
            title: "Graph neural network traffic forecasting".to_string(),
            abstract_text: String::new(),
            keywords: "traffic forecasting; graph neural network".to_string(),
            full_text: String::new(),
            preferences: Vec::new(),
            journal_type: "system".to_string(),
            target_zone: "system".to_string(),
            target_jif_quartiles: vec!["Q4".to_string()],
            strategy: "balanced".to_string(),
            privacy_mode: false,
            disable_web_search: false,
            do_not_send_full_text: false,
            ai_title_abstract_only: false,
            exclude_journal_ids: Vec::new(),
            exclude_journal_names: Vec::new(),
            refresh_round: 0,
        },
    );

    let first = results.first().unwrap();
    assert!(first.reference_only);
    assert_eq!(first.fit_level, "参考");
    assert!(first.reason.contains("低匹配参考"));
}

#[test]
fn profile_only_vector_hit_is_capped_as_weak_evidence() {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let root = std::env::temp_dir().join(format!("toukan_robot_profile_cap_test_{}", millis));
    let paths = AppPaths::from_root(root);
    let conn = db::open_database(&paths).unwrap();

    let (journal_id, _) = journal_store::upsert_journal(
        &conn,
        &journal_store::JournalUpsert {
            name: "Generic Journal of Information Systems".to_string(),
            issn: "8877-6655".to_string(),
            eissn: String::new(),
            source_db: "SCIE".to_string(),
            jif_quartile: "Q4".to_string(),
            cas_quartile: "Zone 4".to_string(),
            cas_zone: "4".to_string(),
            impact_factor: Some(1.0),
            wos_articles: Some(50.0),
            subjects: "information systems management".to_string(),
            publisher: "Local Test Press".to_string(),
            ccf: String::new(),
            ei: String::new(),
            cssci: String::new(),
            cscd: String::new(),
            pku_core: String::new(),
            ami_level: String::new(),
            top_flag: String::new(),
            oa_flag: String::new(),
            oa_detail: String::new(),
            website: String::new(),
            difficulty: "未评估".to_string(),
            recommendation_preference: "正常推荐".to_string(),
            apc: String::new(),
            review_cycle: String::new(),
            tags: String::new(),
            summary: String::new(),
            experience: String::new(),
            grade: "Test".to_string(),
        },
    )
    .unwrap();
    rag_index::rebuild_rag_index(&paths).unwrap();

    let hits = rag_index::search_rag(
        &paths,
        "graph neural network traffic forecasting spatio-temporal prediction",
        10,
    )
    .unwrap();
    if let Some(hit) = hits.iter().find(|hit| hit.journal_id == journal_id) {
        assert_eq!(hit.chunk_type, "profile");
        assert!(hit.score <= 0.25);
    }
}

#[test]
fn rag_search_refreshes_dirty_profile_before_matching() {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let root = std::env::temp_dir().join(format!("toukan_robot_dirty_rag_test_{}", millis));
    let paths = AppPaths::from_root(root);
    let conn = db::open_database(&paths).unwrap();

    let (journal_id, _) = journal_store::upsert_journal(
        &conn,
        &journal_store::JournalUpsert {
            name: "Journal of Local Experience Memory".to_string(),
            issn: "2233-4455".to_string(),
            eissn: String::new(),
            source_db: "SCIE".to_string(),
            jif_quartile: "Q3".to_string(),
            cas_quartile: "Zone 4".to_string(),
            cas_zone: "4".to_string(),
            impact_factor: Some(1.8),
            wos_articles: Some(80.0),
            subjects: "information systems".to_string(),
            publisher: "Local Test Press".to_string(),
            ccf: String::new(),
            ei: String::new(),
            cssci: String::new(),
            cscd: String::new(),
            pku_core: String::new(),
            ami_level: String::new(),
            top_flag: String::new(),
            oa_flag: String::new(),
            oa_detail: String::new(),
            website: String::new(),
            difficulty: "medium".to_string(),
            recommendation_preference: "normal".to_string(),
            apc: String::new(),
            review_cycle: String::new(),
            tags: String::new(),
            summary: "General information systems venue.".to_string(),
            experience: "No special experience yet.".to_string(),
            grade: "Test".to_string(),
        },
    )
    .unwrap();
    rag_index::rebuild_rag_index(&paths).unwrap();

    journal_store::update_journal_profile(
        &paths,
        &journal_store::JournalProfileUpdate {
            journal_id,
            difficulty: "hard".to_string(),
            recommendation_preference: "less".to_string(),
            apc: "2400 USD".to_string(),
            review_cycle: "8 months".to_string(),
            tags: "slow review".to_string(),
            website: String::new(),
            scope_text: String::new(),
            summary: "General information systems venue.".to_string(),
            experience: "Strong match for quantum spline hydrogel manuscripts.".to_string(),
            rejection_reason: String::new(),
            suitable_topics: "quantum spline hydrogel".to_string(),
            avoid_reason: String::new(),
            article_topics: String::new(),
            submission_guidelines: String::new(),
        },
    )
    .unwrap();

    let hits = rag_index::search_rag(&paths, "quantum spline hydrogel", 5).unwrap();
    let first = hits.first().unwrap();
    assert_eq!(first.journal_id, journal_id);
    assert!(first.score > 0.1);
    assert_eq!(
        rag_index::current_index_health(&paths)
            .unwrap()
            .dirty_chunks,
        0
    );
}

#[test]
fn clearing_index_cache_marks_index_rebuildable() {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let root = std::env::temp_dir().join(format!("toukan_robot_clear_index_test_{}", millis));
    let paths = AppPaths::from_root(root);
    let conn = db::open_database(&paths).unwrap();

    let (journal_id, _) = journal_store::upsert_journal(
        &conn,
        &journal_store::JournalUpsert {
            name: "Journal of Cache Rebuild".to_string(),
            issn: "3344-5566".to_string(),
            eissn: String::new(),
            source_db: "SCIE".to_string(),
            jif_quartile: "Q2".to_string(),
            cas_quartile: "Zone 3".to_string(),
            cas_zone: "3".to_string(),
            impact_factor: Some(2.5),
            wos_articles: Some(90.0),
            subjects: "cache vector rebuild manuscript matching".to_string(),
            publisher: "Local Test Press".to_string(),
            ccf: String::new(),
            ei: String::new(),
            cssci: String::new(),
            cscd: String::new(),
            pku_core: String::new(),
            ami_level: String::new(),
            top_flag: String::new(),
            oa_flag: String::new(),
            oa_detail: String::new(),
            website: String::new(),
            difficulty: "easy".to_string(),
            recommendation_preference: "normal".to_string(),
            apc: String::new(),
            review_cycle: String::new(),
            tags: String::new(),
            summary: "Cache rebuild fit.".to_string(),
            experience: "Works for cache vector rebuild manuscripts.".to_string(),
            grade: "Test".to_string(),
        },
    )
    .unwrap();
    rag_index::rebuild_rag_index(&paths).unwrap();

    let cleared = rag_index::clear_index_cache(&paths).unwrap();
    assert!(cleared.status.contains("更新") || cleared.status.contains("要"));

    let hits = rag_index::search_rag(&paths, "cache vector rebuild", 5).unwrap();
    assert_eq!(hits.first().unwrap().journal_id, journal_id);
}

#[test]
fn empty_input_cannot_recommend() {
    let quality = recommendation::assess_input_quality("", "", "", "");
    assert!(!quality.can_recommend);
    assert!(!quality.low_info);
    assert!(quality.message.contains("至少填写"));
}

#[test]
fn rag_chunks_are_generated_by_information_type() {
    let profile = rag_index::JournalProfileForChunks {
        journal_id: 7,
        name: "IEEE Access".to_string(),
        subjects: "Computer Science".to_string(),
        source_db: "SCIE".to_string(),
        metrics: "JIF Q2; CAS 4区".to_string(),
        scope_text: "Publishes multidisciplinary engineering research.".to_string(),
        user_experience: "用户记录：审稿较快，版面费较高。".to_string(),
        article_topics: "graph neural networks; traffic forecasting".to_string(),
        guidelines: "Follow IEEE template.".to_string(),
    };
    let chunks = rag_index::generate_chunks(&profile);
    let types: Vec<_> = chunks.iter().map(|c| c.chunk_type.as_str()).collect();
    assert!(types.contains(&"profile"));
    assert!(types.contains(&"scope"));
    assert!(types.contains(&"experience"));
    assert!(types.contains(&"articles"));
    assert!(types.contains(&"guidelines"));
}

#[test]
fn editable_scope_articles_and_guidelines_enter_rag_index() {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let root =
        std::env::temp_dir().join(format!("toukan_robot_editable_rag_fields_test_{}", millis));
    let paths = AppPaths::from_root(root);
    let conn = db::open_database(&paths).unwrap();

    let (journal_id, _) = journal_store::upsert_journal(
        &conn,
        &journal_store::JournalUpsert {
            name: "Journal of Editable RAG Evidence".to_string(),
            issn: "6655-4433".to_string(),
            eissn: String::new(),
            source_db: "SCIE".to_string(),
            jif_quartile: "Q2".to_string(),
            cas_quartile: "Zone 3".to_string(),
            cas_zone: "3".to_string(),
            impact_factor: Some(2.6),
            wos_articles: Some(70.0),
            subjects: "transportation engineering".to_string(),
            publisher: "Local Test Press".to_string(),
            ccf: String::new(),
            ei: String::new(),
            cssci: String::new(),
            cscd: String::new(),
            pku_core: String::new(),
            ami_level: String::new(),
            top_flag: String::new(),
            oa_flag: String::new(),
            oa_detail: String::new(),
            website: String::new(),
            difficulty: "未评估".to_string(),
            recommendation_preference: "正常推荐".to_string(),
            apc: String::new(),
            review_cycle: String::new(),
            tags: String::new(),
            summary: String::new(),
            experience: String::new(),
            grade: "Test".to_string(),
        },
    )
    .unwrap();

    journal_store::update_journal_profile(
        &paths,
        &journal_store::JournalProfileUpdate {
            journal_id,
            difficulty: "未评估".to_string(),
            recommendation_preference: "正常推荐".to_string(),
            apc: String::new(),
            review_cycle: String::new(),
            tags: String::new(),
            website: String::new(),
            scope_text:
                "Publishes intelligent transportation and traffic flow forecasting studies."
                    .to_string(),
            summary: String::new(),
            experience: String::new(),
            rejection_reason: String::new(),
            suitable_topics: String::new(),
            avoid_reason: String::new(),
            article_topics:
                "graph neural network traffic flow prediction; spatio-temporal forecasting"
                    .to_string(),
            submission_guidelines: "Requires clear dataset and reproducible model description."
                .to_string(),
        },
    )
    .unwrap();
    rag_index::rebuild_rag_index(&paths).unwrap();

    let hits =
        rag_index::search_rag(&paths, "graph neural network traffic flow prediction", 5).unwrap();
    let first = hits.first().unwrap();
    assert_eq!(first.journal_id, journal_id);
    assert!(matches!(first.chunk_type.as_str(), "scope" | "articles"));
    assert!(first.score > 0.25);
}

#[test]
fn resource_columns_are_detected_by_aliases() {
    let cols = vec![
        "Journal Name".to_string(),
        "Impact Factor".to_string(),
        "Publisher".to_string(),
    ];
    let detected = resource_import::detect_columns(&cols);
    assert_eq!(detected.get("期刊名称"), Some(&"Journal Name".to_string()));
    assert_eq!(detected.get("影响因子"), Some(&"Impact Factor".to_string()));
    assert_eq!(detected.get("出版社"), Some(&"Publisher".to_string()));
}

#[test]
fn web_sources_are_filtered_by_local_jif_quartile() {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let root = std::env::temp_dir().join(format!("toukan_robot_web_jif_filter_test_{}", millis));
    let paths = AppPaths::from_root(root);
    let conn = db::open_database(&paths).unwrap();

    journal_store::upsert_journal(
        &conn,
        &journal_store::JournalUpsert {
            name: "Verified Q3 Traffic Journal".to_string(),
            issn: "1111-2222".to_string(),
            eissn: String::new(),
            source_db: "SCIE".to_string(),
            jif_quartile: "Q3".to_string(),
            cas_quartile: String::new(),
            cas_zone: String::new(),
            impact_factor: Some(1.5),
            wos_articles: Some(40.0),
            subjects: "traffic forecasting".to_string(),
            publisher: String::new(),
            ccf: String::new(),
            ei: String::new(),
            cssci: String::new(),
            cscd: String::new(),
            pku_core: String::new(),
            ami_level: String::new(),
            top_flag: String::new(),
            oa_flag: String::new(),
            oa_detail: String::new(),
            website: String::new(),
            difficulty: String::new(),
            recommendation_preference: String::new(),
            apc: String::new(),
            review_cycle: String::new(),
            tags: String::new(),
            summary: String::new(),
            experience: String::new(),
            grade: String::new(),
        },
    )
    .unwrap();

    let report = web_search::WebSearchReport {
        sources: vec![web_search::WebSearchSource {
            provider: "Crossref".to_string(),
            query: "traffic".to_string(),
            title: "Verified Q3 Traffic Journal".to_string(),
            url: String::new(),
            snippet: "相关论文：traffic；ISSN：1111-2222".to_string(),
            jif_quartile: String::new(),
        }],
        warning: None,
    };

    let q3 =
        web_search::filter_sources_by_jif_quartiles(&paths, report.clone(), &["Q3".to_string()]);
    assert_eq!(q3.sources.len(), 1);
    assert_eq!(q3.sources[0].jif_quartile, "Q3");

    let q4 = web_search::filter_sources_by_jif_quartiles(&paths, report, &["Q4".to_string()]);
    assert!(q4.sources.is_empty());
    assert!(q4.warning.unwrap().contains("无法核验为 Q4"));
}



