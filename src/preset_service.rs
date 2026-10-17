use crate::domain::{Preset, Value, VersionedPreset};
use crate::preset_registry::{PresetRegistry, RegisterResult};
use crate::preset_store::{PresetStore, PresetStoreError, StoredPreset, UpdateError};
use std::sync::Arc;
use thiserror::Error;
use tracing::debug;

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

    pub async fn update_preset(
        &self,
        id: &str,
        base_revision: u64,
        name: &str,
        value: Value,
    ) -> Result<StoredPreset, UpdatePresetError> {
        let json = serde_json::to_value(&value).map_err(UpdatePresetError::UnsupportedValue)?;
        let stored = self.preset_store.update(id, base_revision, name, json).await?;

        // Registry last, so a failed store write leaves it untouched
        let preset = Preset { id: stored.id.clone(), name: stored.name.clone(), value };
        match self.preset_registry.register(VersionedPreset { preset: Arc::new(preset), revision: stored.revision }) {
            RegisterResult::Added | RegisterResult::Replaced => {}
            // A concurrent write with a newer revision already landed
            RegisterResult::Stale { current_revision } => {
                debug!("Preset '{id}' revision {} is superseded by revision {current_revision}", stored.revision);
            }
        }
        Ok(stored)
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

#[derive(Debug, Error)]
pub enum UpdatePresetError {
    #[error("Unsupported value: {0}")]
    UnsupportedValue(serde_json::Error),
    #[error("preset not found")]
    NotFound,
    #[error(
        "revision conflict: update is based on revision {base_revision}, but the current revision is {current_revision}"
    )]
    RevisionConflict { base_revision: u64, current_revision: u64 },
    #[error(transparent)]
    Internal(#[from] PresetStoreError),
}

impl From<UpdateError> for UpdatePresetError {
    fn from(err: UpdateError) -> Self {
        match err {
            UpdateError::NotFound => Self::NotFound,
            UpdateError::RevisionConflict { base_revision, current_revision } => {
                Self::RevisionConflict { base_revision, current_revision }
            }
            UpdateError::Store(err) => Self::Internal(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{Database, DatabaseError};
    use crate::domain::Number;
    use chrono::Utc;
    use rstest::rstest;
    use rusqlite::params;
    use std::path::Path;

    fn create_service() -> (PresetService, Database) {
        let (service, database, _) = create_service_with_registry(vec![]);
        (service, database)
    }

    fn create_service_with_registry(presets: Vec<VersionedPreset>) -> (PresetService, Database, Arc<PresetRegistry>) {
        let database = Database::open(Path::new(":memory:")).expect("failed to open in-memory database");
        let preset_store = Arc::new(PresetStore::new(database.clone()));
        let preset_registry = Arc::new(PresetRegistry::new(presets));
        (PresetService::new(preset_store, Arc::clone(&preset_registry)), database, preset_registry)
    }

    fn preset(id: &str, name: &str, value: Value, revision: u64) -> VersionedPreset {
        VersionedPreset { preset: Arc::new(Preset { id: id.to_string(), name: name.to_string(), value }), revision }
    }

    // PresetStore can't write presets yet, so write the rows directly
    async fn insert_row(database: &Database, id: &str, value: &str) {
        let (id, value) = (id.to_string(), value.to_string());
        database
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO presets (id, revision, name, value, created_at) VALUES (?1, 0, ?1, ?2, ?3)",
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

    async fn set_revision(database: &Database, revision: i64) {
        database
            .call(move |conn| {
                conn.execute("UPDATE presets SET revision = ?1", params![revision]).map_err(DatabaseError::from)
            })
            .await
            .expect("set revision");
    }

    #[tokio::test]
    async fn update_preset_stores_the_canonical_value_and_returns_the_stored_preset() {
        let (service, database, _) = create_service_with_registry(vec![]);
        insert_row(&database, "preset", "true").await;

        let updated = service.update_preset("preset", 0, "Dim", Value::Number(Number::PositiveInt(10))).await;

        let updated = updated.expect("update succeeded");
        assert_eq!(updated.id, "preset");
        assert_eq!(updated.name, "Dim");
        assert_eq!(updated.value, serde_json::json!(10));
        assert_eq!(updated.revision, 1);
        assert!(updated.updated_at.is_some(), "got {updated:?}");
        let stored = service.retrieve_preset("preset").await.expect("retrieve succeeded");
        assert_eq!(stored.name, "Dim");
        assert_eq!(stored.value, serde_json::json!(10));
        assert_eq!(stored.revision, 1);
    }

    #[tokio::test]
    async fn update_preset_replaces_the_preset_in_the_registry_with_the_stored_revision() {
        let (service, database, registry) =
            create_service_with_registry(vec![preset("preset", "Old", Value::Boolean(true), 0)]);
        insert_row(&database, "preset", "true").await;

        service
            .update_preset("preset", 0, "Dim", Value::Number(Number::PositiveInt(10)))
            .await
            .expect("update succeeded");

        let registered = registry.by_id("preset").expect("preset is registered");
        assert_eq!(registered.name, "Dim");
        assert_eq!(registered.value, Value::Number(Number::PositiveInt(10)));
        assert_eq!(registered.revision, 1);
    }

    #[tokio::test]
    async fn update_preset_registers_a_stored_preset_that_is_missing_from_the_registry() {
        let (service, database, registry) = create_service_with_registry(vec![]);
        insert_row(&database, "preset", "true").await;

        service.update_preset("preset", 0, "Dim", Value::Boolean(false)).await.expect("update succeeded");

        let registered = registry.by_id("preset").expect("preset is registered");
        assert_eq!(registered.value, Value::Boolean(false));
        assert_eq!(registered.revision, 1);
    }

    #[tokio::test]
    async fn update_preset_keeps_a_newer_registry_entry_when_a_concurrent_update_already_landed() {
        let (service, database, registry) =
            create_service_with_registry(vec![preset("preset", "Newer", Value::Boolean(true), 5)]);
        insert_row(&database, "preset", "true").await;

        let result = service.update_preset("preset", 0, "Older", Value::Boolean(false)).await;

        assert_eq!(result.expect("the store accepted the update").revision, 1);
        let registered = registry.by_id("preset").expect("preset is registered");
        assert_eq!(registered.name, "Newer");
        assert_eq!(registered.revision, 5);
    }

    #[tokio::test]
    async fn update_preset_returns_not_found_for_an_unknown_id_and_leaves_the_registry_unchanged() {
        let (service, _, registry) = create_service_with_registry(vec![]);

        let result = service.update_preset("preset", 0, "Dim", Value::Boolean(false)).await;

        assert!(matches!(result, Err(UpdatePresetError::NotFound)), "got {result:?}");
        assert!(registry.by_id("preset").is_none());
    }

    #[rstest]
    #[case::behind(1, 2)]
    #[case::ahead(5, 2)]
    #[tokio::test]
    async fn update_preset_returns_a_revision_conflict_for_a_mismatching_base_revision(
        #[case] base_revision: u64,
        #[case] current_revision: u64,
    ) {
        let (service, database, registry) =
            create_service_with_registry(vec![preset("preset", "Old", Value::Boolean(true), 2)]);
        insert_row(&database, "preset", "true").await;
        set_revision(&database, 2).await;

        let result = service.update_preset("preset", base_revision, "Dim", Value::Boolean(false)).await;

        assert!(
            matches!(
                result,
                Err(UpdatePresetError::RevisionConflict { base_revision: b, current_revision: c })
                    if b == base_revision && c == current_revision
            ),
            "got {result:?}"
        );
        let stored = service.retrieve_preset("preset").await.expect("retrieve succeeded");
        assert_eq!(stored.value, serde_json::json!(true), "the store must be unchanged");
        assert_eq!(stored.revision, 2);
        let registered = registry.by_id("preset").expect("preset is registered");
        assert_eq!(registered.name, "Old", "the registry must be unchanged");
        assert_eq!(registered.revision, 2);
    }

    #[rstest]
    #[case::date_time(Value::DateTime(Utc::now()))]
    #[case::none(Value::None)]
    #[tokio::test]
    async fn update_preset_rejects_a_value_that_cannot_be_stored(#[case] value: Value) {
        let (service, database, registry) =
            create_service_with_registry(vec![preset("preset", "Old", Value::Boolean(true), 0)]);
        insert_row(&database, "preset", "true").await;

        let result = service.update_preset("preset", 0, "Dim", value).await;

        assert!(matches!(result, Err(UpdatePresetError::UnsupportedValue(_))), "got {result:?}");
        let stored = service.retrieve_preset("preset").await.expect("retrieve succeeded");
        assert_eq!(stored.name, "preset", "the store must be unchanged");
        assert_eq!(stored.value, serde_json::json!(true));
        assert_eq!(stored.revision, 0);
        let registered = registry.by_id("preset").expect("preset is registered");
        assert_eq!(registered.name, "Old", "the registry must be unchanged");
    }

    #[tokio::test]
    async fn update_preset_fails_and_leaves_the_registry_unchanged_when_the_store_fails() {
        let (service, database, registry) =
            create_service_with_registry(vec![preset("preset", "Old", Value::Boolean(true), 0)]);
        insert_row(&database, "preset", "true").await;
        database
            .call(|conn| conn.execute("UPDATE presets SET created_at = 'yesterday'", []).map_err(DatabaseError::from))
            .await
            .expect("corrupt the row");

        let result = service.update_preset("preset", 0, "Dim", Value::Boolean(false)).await;

        assert!(matches!(result, Err(UpdatePresetError::Internal(PresetStoreError::Sqlite(_)))), "got {result:?}");
        let registered = registry.by_id("preset").expect("preset is registered");
        assert_eq!(registered.name, "Old");
        assert_eq!(registered.revision, 0);
    }
}
