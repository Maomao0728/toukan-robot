use crate::paths::AppPaths;
use anyhow::Result;
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupReport {
    pub backup_dir: PathBuf,
    pub files_copied: usize,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupEntry {
    pub name: String,
    pub path: PathBuf,
    pub modified_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreReport {
    pub restored_from: PathBuf,
    pub pre_restore_backup_dir: PathBuf,
    pub files_restored: usize,
    pub message: String,
}

pub fn create_backup(paths: &AppPaths, reason: &str) -> Result<BackupReport> {
    paths.ensure_all()?;
    let stamp = Local::now().format("%Y%m%d_%H%M%S").to_string();
    let suffix = sanitize(reason);
    let target = paths.backups.join(format!(
        "backup_{}_{}",
        stamp,
        if suffix.is_empty() { "manual" } else { &suffix }
    ));
    fs::create_dir_all(&target)?;

    let mut files_copied = 0;
    for source in [
        &paths.db,
        &paths.raw_resources,
        &paths.indexes,
        &paths.embeddings,
        &paths.attachments,
        &paths.templates,
        &paths.exports,
        &paths.logs,
    ] {
        if source.exists() {
            let destination = target.join(source.file_name().unwrap_or_default());
            files_copied += copy_tree(source, &destination)?;
        }
    }
    Ok(BackupReport {
        backup_dir: target,
        files_copied,
        message: "完整数据备份已创建，包含数据库、资源、附件、模板、索引和日志。".to_string(),
    })
}

pub fn export_all_data(paths: &AppPaths) -> Result<BackupReport> {
    paths.ensure_all()?;
    let stamp = Local::now().format("%Y%m%d_%H%M%S").to_string();
    let target = paths.exports.join(format!("full_data_export_{}", stamp));
    fs::create_dir_all(&target)?;
    let mut files_copied = 0;
    for source in [
        &paths.db,
        &paths.raw_resources,
        &paths.indexes,
        &paths.embeddings,
        &paths.attachments,
        &paths.templates,
        &paths.logs,
    ] {
        if source.exists() {
            let destination = target.join(source.file_name().unwrap_or_default());
            files_copied += copy_tree(source, &destination)?;
        }
    }
    Ok(BackupReport {
        backup_dir: target,
        files_copied,
        message: "全部数据已导出到 exports，可用于手动归档或迁移。".to_string(),
    })
}

pub fn list_backups(paths: &AppPaths) -> Result<Vec<BackupEntry>> {
    paths.ensure_all()?;
    let mut entries = Vec::new();
    for entry in fs::read_dir(&paths.backups)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let modified_at = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs().to_string())
            .unwrap_or_default();
        entries.push(BackupEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            path,
            modified_at,
        });
    }
    entries.sort_by(|left, right| right.name.cmp(&left.name));
    Ok(entries)
}

pub fn restore_from_backup(paths: &AppPaths, backup_path: &Path) -> Result<RestoreReport> {
    paths.ensure_all()?;
    let backups_root = paths.backups.canonicalize()?;
    let source = backup_path.canonicalize()?;
    if !source.is_dir() {
        anyhow::bail!("备份目录不存在：{}", backup_path.display());
    }
    if !source.starts_with(&backups_root) {
        anyhow::bail!("只能从 D:\\投刊机器人\\data\\backups 内的备份恢复。");
    }

    let pre_restore = create_backup(paths, "pre_restore")?;
    let mut files_restored = 0;
    for (name, destination) in [
        ("db", &paths.db),
        ("raw_resources", &paths.raw_resources),
        ("indexes", &paths.indexes),
        ("embeddings", &paths.embeddings),
        ("attachments", &paths.attachments),
        ("templates", &paths.templates),
        ("exports", &paths.exports),
        ("logs", &paths.logs),
    ] {
        let source_dir = source.join(name);
        if source_dir.is_dir() {
            files_restored += copy_tree_overwrite(&source_dir, destination)?;
        }
    }
    Ok(RestoreReport {
        restored_from: source,
        pre_restore_backup_dir: pre_restore.backup_dir,
        files_restored,
        message: "备份恢复完成；恢复前已自动创建 pre_restore 备份。".to_string(),
    })
}

fn copy_tree(source: &Path, destination: &Path) -> Result<usize> {
    fs::create_dir_all(destination)?;
    let mut count = 0;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let from = entry.path();
        let to = destination.join(entry.file_name());
        if from.is_dir() {
            count += copy_tree(&from, &to)?;
        } else {
            fs::copy(from, to)?;
            count += 1;
        }
    }
    Ok(count)
}

fn copy_tree_overwrite(source: &Path, destination: &Path) -> Result<usize> {
    fs::create_dir_all(destination)?;
    let mut count = 0;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let from = entry.path();
        let to = destination.join(entry.file_name());
        if from.is_dir() {
            count += copy_tree_overwrite(&from, &to)?;
        } else {
            fs::copy(from, to)?;
            count += 1;
        }
    }
    Ok(count)
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}
