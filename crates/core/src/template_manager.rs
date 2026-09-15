use crate::paths::AppPaths;
use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{fs, path::{Path, PathBuf}};

pub fn seed_templates_if_needed(paths: &AppPaths, seed_root: Option<&Path>) -> Result<usize> {
    let Some(seed_root) = seed_root.filter(|path| path.is_dir()) else {
        return Ok(0);
    };

    paths.ensure_all()?;
    let mut copied = 0;
    for entry in walk_files(seed_root)? {
        let relative = entry
            .strip_prefix(seed_root)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let target = paths.templates.join(relative);
        if target.exists() {
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(entry, target)?;
        copied += 1;
    }
    Ok(copied)
}

fn walk_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if !root.is_dir() {
        return Ok(files);
    }
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        if path.is_dir() {
            files.extend(walk_files(&path)?);
        } else if path.is_file() {
            files.push(path);
        }
    }
    Ok(files)
}

pub fn safe_folder_name(name: &str) -> String {
    let re = Regex::new(r#"[<>:"/\\|?*]"#).unwrap();
    let cleaned = re.replace_all(name, "_").trim().to_string();
    if cleaned.is_empty() {
        "未命名期刊".to_string()
    } else {
        cleaned
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateCopyReport {
    pub target_dir: PathBuf,
    pub copied: usize,
    pub skipped: usize,
    pub message: String,
}

pub fn copy_generic_templates_to_journal(
    paths: &AppPaths,
    journal_name: &str,
) -> Result<TemplateCopyReport> {
    paths.ensure_all()?;
    let generic_dir = paths.templates.join("通用模板");
    let target_dir = paths
        .templates
        .join("按期刊")
        .join(safe_folder_name(journal_name));
    fs::create_dir_all(&generic_dir)?;
    fs::create_dir_all(&target_dir)?;

    let mut copied = 0;
    let mut skipped = 0;
    for entry in fs::read_dir(&generic_dir)? {
        let entry = entry?;
        let from = entry.path();
        if !from.is_file() {
            continue;
        }
        let to = target_dir.join(entry.file_name());
        if to.exists() {
            skipped += 1;
            continue;
        }
        fs::copy(from, to)?;
        copied += 1;
    }
    Ok(TemplateCopyReport {
        target_dir,
        copied,
        skipped,
        message: "模板复制完成；已存在文件已跳过，不会覆盖。".to_string(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateEntry {
    pub journal_name: String,
    pub file_name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateListPage {
    pub items: Vec<TemplateEntry>,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateUploadReport {
    pub stored_path: PathBuf,
    pub skipped: bool,
    pub message: String,
}

pub fn list_journal_templates(
    paths: &AppPaths,
    query: &str,
    offset: usize,
    limit: usize,
) -> Result<TemplateListPage> {
    paths.ensure_all()?;
    let root = paths.templates.join("按期刊");
    let needle = query.trim().to_lowercase();
    let mut items = Vec::new();
    if root.is_dir() {
        for journal in fs::read_dir(&root)? {
            let journal = journal?;
            if !journal.path().is_dir() {
                continue;
            }
            let journal_name = journal.file_name().to_string_lossy().to_string();
            for file in fs::read_dir(journal.path())? {
                let file = file?;
                if !file.path().is_file() {
                    continue;
                }
                let file_name = file.file_name().to_string_lossy().to_string();
                let searchable = format!("{} {}", journal_name, file_name).to_lowercase();
                if needle.is_empty() || searchable.contains(&needle) {
                    items.push(TemplateEntry {
                        journal_name: journal_name.clone(),
                        file_name,
                        path: file.path(),
                    });
                }
            }
        }
    }
    items.sort_by(|a, b| {
        a.journal_name
            .cmp(&b.journal_name)
            .then(a.file_name.cmp(&b.file_name))
    });
    let total = items.len();
    Ok(TemplateListPage {
        items: items
            .into_iter()
            .skip(offset)
            .take(limit.clamp(1, 100))
            .collect(),
        total,
    })
}

pub fn save_journal_template_bytes(
    paths: &AppPaths,
    journal_name: &str,
    file_name: &str,
    bytes: &[u8],
) -> Result<TemplateUploadReport> {
    paths.ensure_all()?;
    if journal_name.trim().is_empty() {
        anyhow::bail!("请先填写关联期刊名称。");
    }
    if bytes.is_empty() {
        anyhow::bail!("模板文件为空。");
    }
    let folder = paths
        .templates
        .join("按期刊")
        .join(safe_folder_name(journal_name));
    fs::create_dir_all(&folder)?;
    let stored_path = folder.join(safe_folder_name(file_name));
    if stored_path.exists() {
        return Ok(TemplateUploadReport {
            stored_path,
            skipped: true,
            message: "同名模板已存在，已跳过，未覆盖原文件。".to_string(),
        });
    }
    fs::write(&stored_path, bytes)?;
    Ok(TemplateUploadReport {
        stored_path,
        skipped: false,
        message: "期刊模板已保存。".to_string(),
    })
}
