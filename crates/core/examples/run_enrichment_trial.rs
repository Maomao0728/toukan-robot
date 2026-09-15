use std::env;
use toukan_robot_core::{
    backup_restore, db,
    journal_enrichment::{enrich_selected, EnrichmentRequest},
    journal_store::{list_journals, JournalListFilter},
    paths::AppPaths,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let paths = AppPaths::discover();
    let conn = db::open_database(&paths)?;
    let journals = list_journals(
        &paths,
        &JournalListFilter {
            limit: 3,
            ..JournalListFilter::default()
        },
    )?;
    if journals.is_empty() {
        anyhow::bail!("本地期刊库为空，暂时无法进行小规模验证。");
    }

    let ids = env::var("TOUKAN_TRIAL_IDS")
        .ok()
        .map(|value| {
            value
                .split(',')
                .filter_map(|item| item.trim().parse::<i64>().ok())
                .collect::<Vec<_>>()
        })
        .filter(|items| !items.is_empty())
        .unwrap_or_else(|| journals.iter().map(|journal| journal.id).collect());
    drop(conn);

    let apply = env::var("TOUKAN_TRIAL_APPLY")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if apply {
        let backup = backup_restore::create_backup(&paths, "enrichment_trial_before_apply")?;
        eprintln!("backup: {}", backup.backup_dir.display());
    }
    let report = enrich_selected(
        &paths,
        &EnrichmentRequest {
            journal_ids: ids.clone(),
            dry_run: !apply,
        },
    )
    .await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    if apply {
        let conn = db::open_database(&paths)?;
        let mut statement = conn.prepare(
            "SELECT COALESCE(p.article_topics,''), COALESCE(p.index_dirty,0),
                    (SELECT COUNT(*) FROM journal_enrichment_sources s WHERE s.journal_id=j.id)
             FROM journals j
             LEFT JOIN journal_user_profiles p ON p.journal_id=j.id
             WHERE j.id=?1",
        )?;
        for id in ids {
            let row = statement.query_row([id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?;
            println!(
                "verification journal_id={}: article_topics_chars={}, index_dirty={}, source_records={}",
                id,
                row.0.chars().count(),
                row.1,
                row.2
            );
        }
    }
    Ok(())
}
