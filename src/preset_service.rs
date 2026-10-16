use crate::preset_registry::PresetRegistry;
use crate::preset_store::{PresetStore, PresetStoreError, StoredPreset};
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug)]
pub struct PresetService {
    preset_store: Arc<PresetStore>,
    preset_registry: Arc<PresetRegistry>,
}

impl PresetService {
    pub fn new(preset_store: Arc<PresetStore>, preset_registry: Arc<PresetRegistry>) -> Self {
        PresetService { preset_store, preset_registry }
    }

    pub async fn list(&self) -> Result<Vec<StoredPreset>, ListPresetError> {
        Ok(self.preset_store.list().await?)
    }

    pub async fn retrieve_preset(&self, id: &str) -> Result<StoredPreset, RetrievePresetError> {
        self.preset_store.by_id(id).await?.ok_or(RetrievePresetError::NotFound)
    }
}

#[derive(Debug, Error)]
pub enum ListPresetError {
    #[error(transparent)]
    Internal(#[from] PresetStoreError),
}

#[derive(Debug, Error)]
pub enum RetrievePresetError {
    #[error("preset not found")]
    NotFound,
    #[error(transparent)]
    Internal(#[from] PresetStoreError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{Database, DatabaseError};
    use chrono::Utc;
    use rusqlite::params;
    use std::path::Path;

    fn create_service() -> (PresetService, Database) {
        let database = Database::open(Path::new(":memory:")).expect("failed to open in-memory database");
        let preset_store = Arc::new(PresetStore::new(database.clone()));
        let preset_registry = Arc::new(PresetRegistry::new(vec![]));
        (PresetService::new(preset_store, preset_registry), database)
    }

    // PresetStore can't write presets yet, so write the rows directly
    async fn insert_row(database: &Database, id: &str, value: &str) {
        let (id, value) = (id.to_string(), value.to_string());
        database
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO presets (id, name, value, created_at) VALUES (?1, ?1, ?2, ?3)",
                    params![id, value, Utc::now().to_rfc3339()],
                )
                .map_err(DatabaseError::from)
            })
            .await
            .expect("insert row");
    }

    #[tokio::test]
    async fn list_returns_an_empty_list_for_an_empty_store() {
        let (service, _) = create_service();

        let presets = service.list().await.expect("list succeeded");

        assert!(presets.is_empty(), "got {presets:?}");
    }

    #[tokio::test]
    async fn list_returns_the_stored_presets() {
        let (service, database) = create_service();
        insert_row(&database, "b", "true").await;
        insert_row(&database, "a", "10").await;

        let presets = service.list().await.expect("list succeeded");

        let ids: Vec<_> = presets.iter().map(|preset| preset.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
    }

    #[tokio::test]
    async fn list_fails_when_the_store_fails() {
        let (service, database) = create_service();
        insert_row(&database, "corrupt", "not valid json").await;

        let result = service.list().await;

        assert!(matches!(result, Err(ListPresetError::Internal(PresetStoreError::Sqlite(_)))), "got {result:?}");
    }

    #[tokio::test]
    async fn retrieve_preset_returns_the_stored_preset() {
        let (service, database) = create_service();
        insert_row(&database, "other", "true").await;
        insert_row(&database, "preset", "10").await;

        let preset = service.retrieve_preset("preset").await.expect("retrieve succeeded");

        assert_eq!(preset.id, "preset");
        assert_eq!(preset.value, serde_json::json!(10));
    }

    #[tokio::test]
    async fn retrieve_preset_returns_not_found_for_an_unknown_id() {
        let (service, database) = create_service();
        insert_row(&database, "other", "true").await;

        let result = service.retrieve_preset("preset").await;

        assert!(matches!(result, Err(RetrievePresetError::NotFound)), "got {result:?}");
    }

    #[tokio::test]
    async fn retrieve_preset_fails_when_the_store_fails() {
        let (service, database) = create_service();
        insert_row(&database, "preset", "not valid json").await;

        let result = service.retrieve_preset("preset").await;

        assert!(matches!(result, Err(RetrievePresetError::Internal(PresetStoreError::Sqlite(_)))), "got {result:?}");
    }
}
