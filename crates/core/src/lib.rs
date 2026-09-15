pub mod ai_clients;
pub mod backup_restore;
pub mod db;
pub mod diagnostics;
pub mod journal_store;
pub mod journal_enrichment;
pub mod legacy_import;
pub mod paper_parser;
pub mod paths;
pub mod rag_index;
pub mod recommendation;
pub mod resource_import;
pub mod settings;
pub mod sync;
pub mod template_manager;
pub mod web_search;

pub use paths::AppPaths;
