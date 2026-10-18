use crate::preset_store::StoredPreset;
use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PresetResponse {
    pub id: String,
    pub revision: u64,
    pub name: String,
    pub value: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl From<StoredPreset> for PresetResponse {
    fn from(preset: StoredPreset) -> Self {
        Self {
            id: preset.id,
            revision: preset.revision,
            name: preset.name,
            value: preset.value,
            created_at: preset.created_at,
            updated_at: preset.updated_at,
        }
    }
}
