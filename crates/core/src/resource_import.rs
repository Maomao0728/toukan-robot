use crate::{backup_restore, db, paths::AppPaths};
use anyhow::Result;
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub fn detect_columns(columns: &[String]) -> HashMap<String, String> {
    let aliases: [(&str, &[&str]); 8] = [
        (
            "期刊名称",
            &[
                "期刊名称",
                "刊名",
                "期刊名",
                "journal",
                "journal name",
                "title",
                "刊物名称",
            ],
        ),
        ("ISSN", &["issn", "print issn", "p-issn"]),
        ("eISSN", &["eissn", "e-issn", "online issn"]),
        ("影响因子", &["影响因子", "if", "jif", "impact factor"]),
        (
            "JIF分区",
            &[
                "jif分区",
                "jcr分区",
                "quartile",
                "jif quartile",
                "jcr quartile",
            ],
        ),
        (
            "中科院分区",
            &["中科院分区", "分区", "cas", "大类分区", "小类分区"],
        ),
        (
            "学科",
            &["学科", "所属学科", "subject", "category", "学科分类"],
        ),
        ("出版社", &["出版社", "publisher"]),
    ];
    let lower: Vec<(String, String)> = columns
        .iter()
        .map(|c| (c.trim().to_lowercase(), c.clone()))
        .collect();
    let mut result = HashMap::new();
    for (target, names) in aliases {
        for alias in names {
            let alias_l = alias.to_lowercase();
            if let Some((_, original)) = lower
                .iter()
                .find(|(low, _)| low == &alias_l || low.contains(&alias_l))
            {
                result.insert(target.to_string(), original.clone());
                break;
            }
        }
    }
    result
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceSaveRequest {
    pub source_path: String,
    pub resource_type: String,
    pub column_map: HashMap<String, String>,
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceSaveReport {
    pub stored_path: PathBuf,
    pub backup_dir: Option<PathBuf>,
    pub message: String,
}

pub fn save_resource_file(
    paths: &AppPaths,
    request: &ResourceSaveRequest,
) -> Result<ResourceSaveReport> {
    paths.ensure_all()?;
    let source = Path::new(&request.source_path);
    if !source.is_file() {
        anyhow::bail!("资源文件不存在：{}", source.display());
    }
    let backup = backup_restore::create_backup(paths, "resource_import")?;
    let stamp = Local::now().format("%Y%m%d_%H%M%S").to_string();
    let file_name = source
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("resource");
    let stored_path = paths
        .raw_resources
        .join(format!("{}_{}", stamp, safe_filename(file_name)));
    fs::copy(source, &stored_path)?;

    let conn = db::open_database(paths)?;
    conn.execute(
        "INSERT INTO resource_imports
         (uuid,file_name,stored_path,resource_type,column_map_json,notes)
         VALUES (?1,?2,?3,?4,?5,?6)",
        rusqlite::params![
            Uuid::new_v4().to_string(),
            file_name,
            stored_path.display().to_string(),
            request.resource_type,
            serde_json::to_string(&request.column_map)?,
            request.notes
        ],
    )?;

    Ok(ResourceSaveReport {
        stored_path,
        backup_dir: Some(backup.backup_dir),
        message: "资源文件已保存，数据库已自动备份，导入记录已写入。".to_string(),
    })
}

fn safe_filename(name: &str) -> String {
    let re = regex::Regex::new(r#"[<>:"/\\|?*]"#).unwrap();
    let cleaned = re.replace_all(name, "_").trim().to_string();
    if cleaned.is_empty() {
        "未命名资源".to_string()
    } else {
        cleaned
    }
}

pub fn save_resource_bytes(
    paths: &AppPaths,
    file_name: &str,
    bytes: &[u8],
    resource_type: &str,
    journal_name: &str,
    notes: &str,
) -> Result<ResourceSaveReport> {
    paths.ensure_all()?;
    if bytes.len() > 50 * 1024 * 1024 {
        anyhow::bail!("资源文件超过 50MB，请缩小文件后再导入。");
    }
    let backup = backup_restore::create_backup(paths, "resource_upload")?;
    let stamp = Local::now().format("%Y%m%d_%H%M%S").to_string();
    let stored_path = paths
        .raw_resources
        .join(format!("{}_{}", stamp, safe_filename(file_name)));
    fs::write(&stored_path, bytes)?;
    let linked_note = if journal_name.trim().is_empty() {
        notes.trim().to_string()
    } else {
        format!("关联期刊：{}；{}", journal_name.trim(), notes.trim())
    };
    let conn = db::open_database(paths)?;
    conn.execute(
        "INSERT INTO resource_imports (uuid,file_name,stored_path,resource_type,column_map_json,notes) VALUES (?1,?2,?3,?4,?5,?6)",
        rusqlite::params![Uuid::new_v4().to_string(), file_name, stored_path.display().to_string(), resource_type, "{}", linked_note],
    )?;
    if !journal_name.trim().is_empty() {
        let normalized = crate::journal_store::normalize_journal_name(journal_name);
        conn.execute(
            "UPDATE journal_user_profiles SET summary=CASE WHEN COALESCE(summary,'')='' THEN ?1 ELSE summary || char(10) || ?1 END, index_dirty=1, updated_at=CURRENT_TIMESTAMP WHERE journal_id IN (SELECT id FROM journals WHERE normalized_name=?2 AND deleted_at IS NULL)",
            rusqlite::params![format!("已关联资料：{}", file_name), normalized],
        )?;
    }
    Ok(ResourceSaveReport {
        stored_path,
        backup_dir: Some(backup.backup_dir),
        message: "资源已保存并登记；关联期刊会在下次推荐前增量更新索引。".to_string(),
    })
}
