use crate::{db, paths::AppPaths, rag_index, settings};
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticSummary {
    pub app_name: String,
    pub root: String,
    pub database: String,
    pub schema_version: i64,
    pub index_status: String,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheckItem {
    pub name: String,
    pub status: String,
    pub detail: String,
    pub level: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuickHealthReport {
    pub checked_at: String,
    pub items: Vec<HealthCheckItem>,
}

pub fn collect(paths: &AppPaths) -> Result<DiagnosticSummary> {
    let conn = db::open_database(paths)?;
    let schema_version = db::schema_version(&conn).unwrap_or(0);
    let index = rag_index::current_index_health(paths)
        .unwrap_or_else(|_| rag_index::initial_index_health());
    Ok(DiagnosticSummary {
        app_name: "投刊机器人".to_string(),
        root: paths.root.display().to_string(),
        database: paths.database_file().display().to_string(),
        schema_version,
        index_status: index.status,
        notes: vec![
            "旧版功能审计已生成在 docs/legacy_audit.md。".to_string(),
            "当前为新版骨架阶段，旧版项目未被修改。".to_string(),
        ],
    })
}

pub fn quick_health_check(paths: &AppPaths) -> Result<QuickHealthReport> {
    paths.ensure_all()?;
    let mut items = Vec::new();

    items.push(HealthCheckItem {
        name: "数据目录".to_string(),
        status: if paths.data.is_dir() {
            "正常".to_string()
        } else {
            "异常".to_string()
        },
        detail: paths.data.display().to_string(),
        level: if paths.data.is_dir() {
            "Normal".to_string()
        } else {
            "Danger".to_string()
        },
    });

    match db::open_database(paths) {
        Ok(conn) => {
            let schema = db::schema_version(&conn).unwrap_or(0);
            items.push(HealthCheckItem {
                name: "数据库".to_string(),
                status: "正常".to_string(),
                detail: format!(
                    "schema_version={}; {}",
                    schema,
                    paths.database_file().display()
                ),
                level: "Normal".to_string(),
            });
        }
        Err(error) => items.push(HealthCheckItem {
            name: "数据库".to_string(),
            status: "异常".to_string(),
            detail: error.to_string(),
            level: "Danger".to_string(),
        }),
    }

    match rag_index::current_index_health(paths) {
        Ok(index) => items.push(HealthCheckItem {
            name: "RAG 索引".to_string(),
            status: index.status,
            detail: format!(
                "{}；待更新 {}；占用 {} 字节",
                index.detail, index.dirty_chunks, index.index_bytes
            ),
            level: if index.dirty_chunks > 0 {
                "Important".to_string()
            } else {
                "Normal".to_string()
            },
        }),
        Err(error) => items.push(HealthCheckItem {
            name: "RAG 索引".to_string(),
            status: "异常".to_string(),
            detail: error.to_string(),
            level: "Danger".to_string(),
        }),
    }

    let openrouter = settings::load_ai_settings(paths, "openrouter").ok();
    let xfastapi = settings::load_ai_settings(paths, "xfastapi").ok();
    let api_ready = [openrouter.as_ref(), xfastapi.as_ref()]
        .into_iter()
        .flatten()
        .any(|record| !record.api_key.trim().is_empty());
    items.push(HealthCheckItem {
        name: "API Key".to_string(),
        status: if api_ready {
            "已保存".to_string()
        } else {
            "未保存".to_string()
        },
        detail: if api_ready {
            "至少一个 AI 接口已保存 API Key。".to_string()
        } else {
            "没有保存 API Key；本地 RAG/规则推荐仍可使用。".to_string()
        },
        level: if api_ready {
            "Normal".to_string()
        } else {
            "Important".to_string()
        },
    });

    let backup_count = std::fs::read_dir(&paths.backups)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| entry.path().is_dir())
                .count()
        })
        .unwrap_or(0);
    items.push(HealthCheckItem {
        name: "备份".to_string(),
        status: if backup_count > 0 {
            "正常".to_string()
        } else {
            "建议备份".to_string()
        },
        detail: format!(
            "已有 {} 个备份目录；{}",
            backup_count,
            paths.backups.display()
        ),
        level: if backup_count > 0 {
            "Normal".to_string()
        } else {
            "Important".to_string()
        },
    });

    items.push(HealthCheckItem {
        name: "日志目录".to_string(),
        status: if paths.logs.is_dir() {
            "正常".to_string()
        } else {
            "异常".to_string()
        },
        detail: paths.logs.display().to_string(),
        level: if paths.logs.is_dir() {
            "Normal".to_string()
        } else {
            "Danger".to_string()
        },
    });

    Ok(QuickHealthReport {
        checked_at: chrono::Local::now().to_rfc3339(),
        items,
    })
}
