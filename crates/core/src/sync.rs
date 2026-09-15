use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncMetadata {
    pub uuid: String,
    pub user_id: Option<String>,
    pub sync_status: String,
    pub version: i64,
    pub last_synced_at: Option<String>,
}

pub fn sync_is_available() -> bool {
    false
}
