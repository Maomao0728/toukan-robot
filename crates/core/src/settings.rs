use crate::{ai_clients, db, paths::AppPaths};
use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivacySettings {
    pub local_only_recommendation: bool,
    pub disable_web_search: bool,
    pub do_not_send_full_text: bool,
    pub ai_title_abstract_only: bool,
}

impl Default for PrivacySettings {
    fn default() -> Self {
        Self {
            local_only_recommendation: false,
            disable_web_search: false,
            do_not_send_full_text: false,
            ai_title_abstract_only: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HintLevel {
    Normal,
    Important,
    Danger,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiHint {
    pub level: HintLevel,
    pub text: String,
}

pub fn default_hints() -> Vec<UiHint> {
    vec![
        UiHint {
            level: HintLevel::Important,
            text: "建议写清难度、审稿周期、版面费和拒稿原因；这些内容会影响以后推荐。".to_string(),
        },
        UiHint {
            level: HintLevel::Important,
            text: "AI 推荐可能会把题目、摘要或正文节选发送给所选接口。".to_string(),
        },
        UiHint {
            level: HintLevel::Danger,
            text: "删除、恢复、导入和同步操作前请确认已备份。".to_string(),
        },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiSettingsRecord {
    pub service_key: String,
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    pub api_key_hint: String,
}

pub fn save_ai_settings(paths: &AppPaths, record: &AiSettingsRecord) -> Result<AiSettingsRecord> {
    let conn = db::open_database(paths)?;
    let service = normalize_service_key(&record.service_key);
    set_setting(
        &conn,
        &format!("ai.{}.base_url", service),
        record.base_url.trim(),
        false,
    )?;
    set_setting(
        &conn,
        &format!("ai.{}.model", service),
        record.model.trim(),
        false,
    )?;
    if !record.api_key.trim().is_empty() {
        let encrypted = protect_secret(&ai_clients::clean_api_key(&record.api_key))?;
        set_setting(
            &conn,
            &format!("ai.{}.api_key.dpapi", service),
            &encrypted,
            true,
        )?;
    }
    load_ai_settings(paths, &service)
}

pub fn load_ai_settings(paths: &AppPaths, service_key: &str) -> Result<AiSettingsRecord> {
    let conn = db::open_database(paths)?;
    let service = normalize_service_key(service_key);
    let defaults = ai_clients::default_services()
        .into_iter()
        .find(|item| item.key == service)
        .or_else(|| ai_clients::default_services().into_iter().next());
    let default_base = defaults
        .as_ref()
        .map(|item| item.base_url.as_str())
        .unwrap_or("");
    let default_model = defaults
        .as_ref()
        .map(|item| item.model.as_str())
        .unwrap_or("");
    let base_url = get_setting(&conn, &format!("ai.{}.base_url", service))?
        .unwrap_or_else(|| default_base.to_string());
    let model = get_setting(&conn, &format!("ai.{}.model", service))?
        .unwrap_or_else(|| default_model.to_string());
    let api_key = get_setting(&conn, &format!("ai.{}.api_key.dpapi", service))?
        .and_then(|value| unprotect_secret(&value).ok())
        .unwrap_or_default();
    let api_key_hint = ai_clients::mask_api_key(&api_key);
    Ok(AiSettingsRecord {
        service_key: service,
        base_url,
        model,
        api_key,
        api_key_hint,
    })
}

fn normalize_service_key(value: &str) -> String {
    let key = value.trim().to_lowercase();
    if key.is_empty() {
        "openrouter".to_string()
    } else {
        key
    }
}

fn set_setting(conn: &rusqlite::Connection, key: &str, value: &str, is_secret: bool) -> Result<()> {
    conn.execute(
        "INSERT INTO app_settings (key,value,is_secret,updated_at)
         VALUES (?1,?2,?3,CURRENT_TIMESTAMP)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value,
         is_secret=excluded.is_secret, updated_at=CURRENT_TIMESTAMP",
        params![key, value, if is_secret { 1 } else { 0 }],
    )?;
    Ok(())
}

fn get_setting(conn: &rusqlite::Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            params![key],
            |row| row.get(0),
        )
        .optional()?)
}

#[cfg(target_os = "windows")]
fn protect_secret(secret: &str) -> Result<String> {
    run_powershell_secret(
        "$plain=[Console]::In.ReadToEnd(); $secure=ConvertTo-SecureString -String $plain -AsPlainText -Force; ConvertFrom-SecureString -SecureString $secure",
        secret,
    )
}

#[cfg(target_os = "windows")]
fn unprotect_secret(secret: &str) -> Result<String> {
    run_powershell_secret(
        "$blob=[Console]::In.ReadToEnd(); $secure=ConvertTo-SecureString $blob; $bstr=[Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure); try { [Runtime.InteropServices.Marshal]::PtrToStringUni($bstr) } finally { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr) }",
        secret,
    )
}

#[cfg(target_os = "windows")]
fn run_powershell_secret(script: &str, input: &str) -> Result<String> {
    let mut child = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(input.as_bytes())?;
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        anyhow::bail!("API Key 加密/解密失败");
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(not(target_os = "windows"))]
fn protect_secret(secret: &str) -> Result<String> {
    Ok(secret.to_string())
}

#[cfg(not(target_os = "windows"))]
fn unprotect_secret(secret: &str) -> Result<String> {
    Ok(secret.to_string())
}
