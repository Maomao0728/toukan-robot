use toukan_robot_core::{
    ai_clients, backup_restore, db, diagnostics, journal_enrichment, legacy_import, paper_parser,
    paths::AppPaths, rag_index, recommendation, resource_import, settings, template_manager,
    web_search,
};
use tauri::Manager;

#[tauri::command]
fn initialize_app(app: tauri::AppHandle) -> Result<diagnostics::DiagnosticSummary, String> {
    let paths = AppPaths::discover();
    paths.ensure_all().map_err(|e| e.to_string())?;
    let seed_path = app
        .path()
        .resource_dir()
        .ok()
        .map(|directory| directory.join("seed_data").join("toukan_robot.sqlite3"));
    db::open_database_with_seed(&paths, seed_path.as_deref()).map_err(|e| e.to_string())?;
    let seed_templates = app
        .path()
        .resource_dir()
        .ok()
        .map(|directory| directory.join("seed_templates"));
    template_manager::seed_templates_if_needed(&paths, seed_templates.as_deref())
        .map_err(|e| e.to_string())?;
    rag_index::start_background_indexing(&paths);
    diagnostics::collect(&paths).map_err(|e| e.to_string())
}

#[tauri::command]
fn quick_health_check() -> Result<diagnostics::QuickHealthReport, String> {
    let paths = AppPaths::discover();
    diagnostics::quick_health_check(&paths).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_app_paths() -> Result<AppPaths, String> {
    let paths = AppPaths::discover();
    paths.ensure_all().map_err(|e| e.to_string())?;
    Ok(paths)
}

#[tauri::command]
fn open_data_directory() -> Result<String, String> {
    let paths = AppPaths::discover();
    paths.ensure_all().map_err(|e| e.to_string())?;
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer.exe")
            .arg(&paths.data)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(paths.data.display().to_string())
}

#[tauri::command]
fn open_external_url(url: String) -> Result<(), String> {
    let url = url.trim();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("只能打开 http 或 https 链接".to_string());
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer.exe")
            .arg(url)
            .spawn()
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn assess_input_quality(
    title: String,
    abstract_text: String,
    keywords: String,
    full_text: String,
) -> recommendation::InputQuality {
    recommendation::assess_input_quality(&title, &abstract_text, &keywords, &full_text)
}

#[tauri::command]
fn local_fallback_recommend(
    request: recommendation::RecommendationRequest,
) -> Vec<recommendation::RecommendationItem> {
    let paths = AppPaths::discover();
    recommendation::local_database_recommendations(&paths, &request)
}

#[tauri::command]
async fn ai_recommend(
    request: recommendation::AiRecommendationRequest,
) -> Result<recommendation::AiRecommendationResponse, String> {
    let paths = AppPaths::discover();
    recommendation::ai_recommend(&paths, &request).await
}

#[tauri::command]
fn get_ai_services() -> Vec<ai_clients::AiServiceConfig> {
    ai_clients::default_services()
}

#[tauri::command]
fn mask_api_key(value: String) -> String {
    ai_clients::mask_api_key(&value)
}

#[tauri::command]
fn get_default_hints() -> Vec<settings::UiHint> {
    settings::default_hints()
}

#[tauri::command]
fn save_ai_settings(
    record: settings::AiSettingsRecord,
) -> Result<settings::AiSettingsRecord, String> {
    let paths = AppPaths::discover();
    settings::save_ai_settings(&paths, &record).map_err(|e| e.to_string())
}

#[tauri::command]
fn load_ai_settings(service_key: String) -> Result<settings::AiSettingsRecord, String> {
    let paths = AppPaths::discover();
    settings::load_ai_settings(&paths, &service_key).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_index_health() -> Result<rag_index::IndexHealth, String> {
    let paths = AppPaths::discover();
    rag_index::current_index_health(&paths).map_err(|e| e.to_string())
}

#[tauri::command]
fn rebuild_rag_index() -> Result<rag_index::IndexHealth, String> {
    let paths = AppPaths::discover();
    rag_index::rebuild_rag_index(&paths).map_err(|e| e.to_string())
}

#[tauri::command]
fn clear_index_cache() -> Result<rag_index::IndexHealth, String> {
    let paths = AppPaths::discover();
    rag_index::clear_index_cache(&paths).map_err(|e| e.to_string())
}

#[tauri::command]
fn create_backup(reason: String) -> Result<backup_restore::BackupReport, String> {
    let paths = AppPaths::discover();
    backup_restore::create_backup(&paths, &reason).map_err(|e| e.to_string())
}

#[tauri::command]
fn export_all_data() -> Result<backup_restore::BackupReport, String> {
    let paths = AppPaths::discover();
    backup_restore::export_all_data(&paths).map_err(|e| e.to_string())
}

#[tauri::command]
fn list_backups() -> Result<Vec<backup_restore::BackupEntry>, String> {
    let paths = AppPaths::discover();
    backup_restore::list_backups(&paths).map_err(|e| e.to_string())
}

#[tauri::command]
fn restore_from_backup(backup_path: String) -> Result<backup_restore::RestoreReport, String> {
    let paths = AppPaths::discover();
    backup_restore::restore_from_backup(&paths, std::path::Path::new(&backup_path))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn detect_resource_columns(columns: Vec<String>) -> std::collections::HashMap<String, String> {
    resource_import::detect_columns(&columns)
}

#[tauri::command]
fn save_resource_file(
    request: resource_import::ResourceSaveRequest,
) -> Result<resource_import::ResourceSaveReport, String> {
    let paths = AppPaths::discover();
    resource_import::save_resource_file(&paths, &request).map_err(|e| e.to_string())
}

#[tauri::command]
fn save_and_extract_attachment(
    source_path: String,
) -> Result<paper_parser::AttachmentParseReport, String> {
    let paths = AppPaths::discover();
    paper_parser::save_and_extract_attachment(&paths, &source_path).map_err(|e| e.to_string())
}

#[tauri::command]
fn upload_attachment(
    file_name: String,
    bytes: Vec<u8>,
) -> Result<paper_parser::AttachmentParseReport, String> {
    let paths = AppPaths::discover();
    paper_parser::save_and_extract_attachment_bytes(&paths, &file_name, &bytes)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn upload_resource(
    file_name: String,
    bytes: Vec<u8>,
    resource_type: String,
    journal_name: String,
    notes: String,
) -> Result<resource_import::ResourceSaveReport, String> {
    let paths = AppPaths::discover();
    resource_import::save_resource_bytes(
        &paths,
        &file_name,
        &bytes,
        &resource_type,
        &journal_name,
        &notes,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn list_journal_templates(
    query: String,
    offset: usize,
    limit: usize,
) -> Result<template_manager::TemplateListPage, String> {
    let paths = AppPaths::discover();
    template_manager::list_journal_templates(&paths, &query, offset, limit)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn upload_journal_template(
    journal_name: String,
    file_name: String,
    bytes: Vec<u8>,
) -> Result<template_manager::TemplateUploadReport, String> {
    let paths = AppPaths::discover();
    template_manager::save_journal_template_bytes(&paths, &journal_name, &file_name, &bytes)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn build_web_queries(
    title: String,
    abstract_text: String,
    keywords: String,
    prefer_chinese: bool,
) -> Vec<String> {
    web_search::build_web_queries(&title, &abstract_text, &keywords, prefer_chinese)
}

#[tauri::command]
async fn web_search_sources(
    request: web_search::WebSearchRequest,
) -> Result<web_search::WebSearchReport, String> {
    let paths = AppPaths::discover();
    let target_jif_quartiles = request.target_jif_quartiles.clone();
    let report = web_search::search_sources(&request).await;
    Ok(web_search::filter_sources_by_jif_quartiles(
        &paths,
        report,
        &target_jif_quartiles,
    ))
}

#[tauri::command]
async fn enrich_journals(
    request: journal_enrichment::EnrichmentRequest,
) -> Result<journal_enrichment::EnrichmentReport, String> {
    let paths = AppPaths::discover();
    journal_enrichment::enrich_selected(&paths, &request)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn legacy_import_plan(source_root: String) -> legacy_import::LegacyImportReport {
    legacy_import::planned_report(&source_root)
}

#[tauri::command]
fn list_journals(
    filter: toukan_robot_core::journal_store::JournalListFilter,
) -> Result<Vec<toukan_robot_core::journal_store::JournalRecord>, String> {
    let paths = AppPaths::discover();
    toukan_robot_core::journal_store::list_journals(&paths, &filter).map_err(|e| e.to_string())
}

#[tauri::command]
fn count_journals(
    filter: toukan_robot_core::journal_store::JournalListFilter,
) -> Result<usize, String> {
    let paths = AppPaths::discover();
    toukan_robot_core::journal_store::count_journals(&paths, &filter).map_err(|e| e.to_string())
}

#[tauri::command]
fn import_legacy_data(source_root: String) -> Result<legacy_import::LegacyImportReport, String> {
    let paths = AppPaths::discover();
    legacy_import::import_legacy(std::path::Path::new(&source_root), &paths)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn list_submissions(
    filter: toukan_robot_core::journal_store::SubmissionListFilter,
) -> Result<Vec<toukan_robot_core::journal_store::SubmissionRecord>, String> {
    let paths = AppPaths::discover();
    toukan_robot_core::journal_store::list_submissions(&paths, &filter).map_err(|e| e.to_string())
}

#[tauri::command]
fn update_journal_profile(
    update: toukan_robot_core::journal_store::JournalProfileUpdate,
) -> Result<(), String> {
    let paths = AppPaths::discover();
    toukan_robot_core::journal_store::update_journal_profile(&paths, &update)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn set_recommendation_preference(journal_id: i64, preference: String) -> Result<(), String> {
    let paths = AppPaths::discover();
    toukan_robot_core::journal_store::set_recommendation_preference(&paths, journal_id, &preference)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_journal(journal_id: i64) -> Result<(), String> {
    let paths = AppPaths::discover();
    toukan_robot_core::journal_store::soft_delete_journal(&paths, journal_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn save_submission(
    submission: toukan_robot_core::journal_store::SubmissionUpsert,
) -> Result<i64, String> {
    let paths = AppPaths::discover();
    toukan_robot_core::journal_store::save_submission(&paths, &submission)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_submission(submission_id: i64) -> Result<(), String> {
    let paths = AppPaths::discover();
    toukan_robot_core::journal_store::soft_delete_submission(&paths, submission_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn export_journals_csv(
    filter: toukan_robot_core::journal_store::JournalListFilter,
) -> Result<toukan_robot_core::journal_store::ExportReport, String> {
    let paths = AppPaths::discover();
    toukan_robot_core::journal_store::export_journals_csv(&paths, &filter)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn export_submissions_csv(
    filter: toukan_robot_core::journal_store::SubmissionListFilter,
) -> Result<toukan_robot_core::journal_store::ExportReport, String> {
    let paths = AppPaths::discover();
    toukan_robot_core::journal_store::export_submissions_csv(&paths, &filter)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn copy_generic_templates(
    journal_name: String,
) -> Result<template_manager::TemplateCopyReport, String> {
    let paths = AppPaths::discover();
    template_manager::copy_generic_templates_to_journal(&paths, &journal_name)
        .map_err(|e| e.to_string())
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            initialize_app,
            quick_health_check,
            get_app_paths,
            open_data_directory,
            open_external_url,
            assess_input_quality,
            local_fallback_recommend,
            ai_recommend,
            get_ai_services,
            mask_api_key,
            get_default_hints,
            save_ai_settings,
            load_ai_settings,
            get_index_health,
            rebuild_rag_index,
            clear_index_cache,
            create_backup,
            export_all_data,
            list_backups,
            restore_from_backup,
            detect_resource_columns,
            save_resource_file,
            save_and_extract_attachment,
            upload_attachment,
            upload_resource,
            list_journal_templates,
            upload_journal_template,
            build_web_queries,
            web_search_sources,
            enrich_journals,
            legacy_import_plan,
            list_journals,
            count_journals,
            import_legacy_data,
            list_submissions,
            update_journal_profile,
            set_recommendation_preference,
            delete_journal,
            save_submission,
            delete_submission,
            export_journals_csv,
            export_submissions_csv,
            copy_generic_templates,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run 投刊机器人");
}
