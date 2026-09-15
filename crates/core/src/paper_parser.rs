use crate::paths::AppPaths;
use anyhow::Result;
use chrono::Local;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

pub fn safe_filename(name: &str) -> String {
    let re = Regex::new(r#"[<>:"/\\|?*]"#).unwrap();
    let cleaned = re.replace_all(name, "_").trim().to_string();
    if cleaned.is_empty() {
        "未命名文件".to_string()
    } else {
        cleaned
    }
}

pub fn supported_suffixes() -> [&'static str; 4] {
    ["docx", "pdf", "txt", "md"]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentParseReport {
    pub stored_path: PathBuf,
    pub extracted_text: String,
    pub warning: Option<String>,
}

pub fn save_and_extract_attachment(
    paths: &AppPaths,
    source_path: &str,
) -> Result<AttachmentParseReport> {
    paths.ensure_all()?;
    let source = Path::new(source_path);
    if !source.is_file() {
        anyhow::bail!("论文附件不存在：{}", source.display());
    }
    let suffix = source
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !supported_suffixes().contains(&suffix.as_str()) {
        anyhow::bail!("暂不支持该附件格式：{}", suffix);
    }
    let stamp = Local::now().format("%Y%m%d_%H%M%S").to_string();
    let file_name = source
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("paper");
    let stored_path = paths
        .attachments
        .join(format!("{}_{}", stamp, safe_filename(file_name)));
    fs::copy(source, &stored_path)?;

    let (extracted_text, warning) = match suffix.as_str() {
        "txt" | "md" => {
            let bytes = fs::read(&stored_path)?;
            let text = String::from_utf8_lossy(&bytes).to_string();
            if text.trim().is_empty() {
                (
                    String::new(),
                    Some("附件已保存，但没有提取到文本内容。".to_string()),
                )
            } else {
                (text, None)
            }
        }
        "pdf" => extract_pdf_text(&stored_path),
        "docx" => extract_docx_text(&stored_path),
        _ => (
            String::new(),
            Some("附件已保存，但未提取文本。".to_string()),
        ),
    };

    Ok(AttachmentParseReport {
        stored_path,
        extracted_text,
        warning,
    })
}

fn extract_pdf_text(path: &Path) -> (String, Option<String>) {
    match pdf_extract::extract_text(path) {
        Ok(text) => {
            let text = compact_text(&text);
            if text.is_empty() {
                (
                    String::new(),
                    Some("PDF \u{5df2}\u{4fdd}\u{5b58}\u{ff0c}\u{4f46}\u{6ca1}\u{6709}\u{63d0}\u{53d6}\u{5230}\u{6b63}\u{6587}\u{3002}\u{5b83}\u{53ef}\u{80fd}\u{662f}\u{626b}\u{63cf}\u{7248} PDF\u{ff0c}\u{8bf7}\u{4eba}\u{5de5}\u{6838}\u{9a8c}\u{540e}\u{518d}\u{7528}\u{4e8e} AI \u{63a8}\u{8350}\u{3002}".to_string()),
                )
            } else {
                (text, None)
            }
        }
        Err(err) => (
            String::new(),
            Some(format!(
                "PDF \u{5df2}\u{4fdd}\u{5b58}\u{ff0c}\u{4f46}\u{6b63}\u{6587}\u{89e3}\u{6790}\u{5931}\u{8d25}\u{ff1a}{}\u{3002}\u{626b}\u{63cf}\u{7248}\u{6216}\u{52a0}\u{5bc6} PDF \u{9700}\u{8981}\u{4eba}\u{5de5}\u{6838}\u{9a8c}\u{3002}",
                err
            )),
        ),
    }
}

fn extract_docx_text(path: &Path) -> (String, Option<String>) {
    match read_docx_document_xml(path) {
        Ok(xml) => {
            let text = compact_text(&docx_xml_to_text(&xml));
            if text.is_empty() {
                (
                    String::new(),
                    Some("DOCX \u{5df2}\u{4fdd}\u{5b58}\u{ff0c}\u{4f46}\u{6ca1}\u{6709}\u{63d0}\u{53d6}\u{5230}\u{6b63}\u{6587}\u{ff0c}\u{8bf7}\u{786e}\u{8ba4}\u{6587}\u{6863}\u{4e0d}\u{662f}\u{7a7a}\u{6587}\u{4ef6}\u{6216}\u{53d7}\u{4fdd}\u{62a4}\u{6587}\u{4ef6}\u{3002}".to_string()),
                )
            } else {
                (text, None)
            }
        }
        Err(err) => (
            String::new(),
            Some(format!("DOCX \u{5df2}\u{4fdd}\u{5b58}\u{ff0c}\u{4f46}\u{6b63}\u{6587}\u{89e3}\u{6790}\u{5931}\u{8d25}\u{ff1a}{}\u{3002}\u{8bf7}\u{4eba}\u{5de5}\u{6838}\u{9a8c}\u{9644}\u{4ef6}\u{5185}\u{5bb9}\u{3002}", err)),
        ),
    }
}

fn read_docx_document_xml(path: &Path) -> Result<String> {
    let file = fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    let mut document = archive.by_name("word/document.xml")?;
    let mut xml = String::new();
    document.read_to_string(&mut xml)?;
    Ok(xml)
}

fn docx_xml_to_text(xml: &str) -> String {
    let paragraph_re = Regex::new(r"</w:p>").unwrap();
    let tag_re = Regex::new(r"<[^>]+>").unwrap();
    let with_breaks = paragraph_re.replace_all(xml, "\n");
    let without_tags = tag_re.replace_all(&with_breaks, " ");
    decode_xml_entities(&without_tags)
}

fn decode_xml_entities(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

fn compact_text(value: &str) -> String {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn save_and_extract_attachment_bytes(
    paths: &AppPaths,
    file_name: &str,
    bytes: &[u8],
) -> Result<AttachmentParseReport> {
    paths.ensure_all()?;
    if bytes.len() > 40 * 1024 * 1024 {
        anyhow::bail!("附件超过 40MB，请缩小文件后再导入。");
    }
    let suffix = Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !supported_suffixes().contains(&suffix.as_str()) {
        anyhow::bail!("暂不支持该附件格式：{}", suffix);
    }
    let stamp = Local::now().format("%Y%m%d_%H%M%S").to_string();
    let stored_path = paths
        .attachments
        .join(format!("{}_{}", stamp, safe_filename(file_name)));
    fs::write(&stored_path, bytes)?;
    let (extracted_text, warning) = match suffix.as_str() {
        "txt" | "md" => {
            let text = String::from_utf8_lossy(bytes).to_string();
            if text.trim().is_empty() {
                (
                    String::new(),
                    Some("附件已保存，但没有提取到文本内容。".to_string()),
                )
            } else {
                (text, None)
            }
        }
        "pdf" => extract_pdf_text(&stored_path),
        "docx" => extract_docx_text(&stored_path),
        _ => (
            String::new(),
            Some("附件已保存，但未提取文本。".to_string()),
        ),
    };
    Ok(AttachmentParseReport {
        stored_path,
        extracted_text,
        warning,
    })
}
