use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{env, fs, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppPaths {
    pub root: PathBuf,
    pub app: PathBuf,
    pub frontend: PathBuf,
    pub crates: PathBuf,
    pub installer: PathBuf,
    pub docs: PathBuf,
    pub data: PathBuf,
    pub db: PathBuf,
    pub raw_resources: PathBuf,
    pub indexes: PathBuf,
    pub embeddings: PathBuf,
    pub attachments: PathBuf,
    pub templates: PathBuf,
    pub exports: PathBuf,
    pub backups: PathBuf,
    pub logs: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Self {
        let root = env::var_os("TOUKAN_ROBOT_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let preferred = PathBuf::from(r"D:\投刊机器人");
                if preferred.exists() {
                    preferred
                } else if let Some(local_app_data) = env::var_os("LOCALAPPDATA") {
                    PathBuf::from(local_app_data).join("投刊机器人")
                } else {
                    PathBuf::from("投刊机器人")
                }
            });
        Self::from_root(root)
    }

    pub fn from_root(root: PathBuf) -> Self {
        let data = root.join("data");
        Self {
            app: root.join("app"),
            frontend: root.join("frontend"),
            crates: root.join("crates"),
            installer: root.join("installer"),
            docs: root.join("docs"),
            db: data.join("db"),
            raw_resources: data.join("raw_resources"),
            indexes: data.join("indexes"),
            embeddings: data.join("embeddings"),
            attachments: data.join("attachments"),
            templates: data.join("templates"),
            exports: data.join("exports"),
            backups: data.join("backups"),
            logs: data.join("logs"),
            data,
            root,
        }
    }

    pub fn ensure_all(&self) -> Result<()> {
        for dir in [
            &self.root,
            &self.app,
            &self.frontend,
            &self.crates,
            &self.installer,
            &self.docs,
            &self.data,
            &self.db,
            &self.raw_resources,
            &self.indexes,
            &self.embeddings,
            &self.attachments,
            &self.templates,
            &self.exports,
            &self.backups,
            &self.logs,
        ] {
            fs::create_dir_all(dir)?;
        }
        Ok(())
    }

    pub fn database_file(&self) -> PathBuf {
        self.db.join("toukan_robot.sqlite3")
    }
}
