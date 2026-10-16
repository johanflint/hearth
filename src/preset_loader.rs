use crate::domain::Preset;
use crate::preset_store::{PresetStore, PresetStoreError};
use tracing::{info, instrument, warn};

#[instrument(skip_all)]
pub async fn load_presets_from_store(preset_store: &PresetStore) -> Result<Vec<Preset>, PresetStoreError> {
    info!("🗄️ Loading presets...");
    let stored_presets = preset_store.list().await?;
    let mut presets = Vec::with_capacity(stored_presets.len());
    let mut failed = 0;

    for stored_preset in stored_presets {
        match serde_json::from_value(stored_preset.value) {
            Ok(value) => presets.push(Preset { id: stored_preset.id, name: stored_preset.name, value }),
            Err(err) => {
                failed += 1;
                warn!("⚠️ Failed to load preset '{}': {}", stored_preset.id, err);
            }
        }
    }

    info!("🗄️ Loading presets... OK, {} loaded, {failed} failed", presets.len());
    Ok(presets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{Database, DatabaseError};
    use crate::domain::{Color, Number, Value};
    use chrono::Utc;
    use rusqlite::params;
    use std::path::Path;

    fn open_in_memory() -> (Database, PresetStore) {
        let database = Database::open(Path::new(":memory:")).expect("failed to open in-memory database");
        (database.clone(), PresetStore::new(database))
    }

    // PresetStore can't write presets yet, so write the rows directly
    async fn insert_row(database: &Database, id: &str, value: &str) {
        let (id, value) = (id.to_string(), value.to_string());
        database
            .call(move |conn| {
                let now = Utc::now().to_rfc3339();
                conn.execute(
                    "INSERT INTO presets (id, name, value, created_at) VALUES (?1, ?1, ?2, ?3)",
                    params![id, value, now],
                )
                .map_err(DatabaseError::from)
            })
            .await
            .expect("insert row");
    }

    #[tokio::test]
    async fn load_presets_from_store_converts_the_stored_values() {
        let (database, preset_store) = open_in_memory();
        insert_row(&database, "brightness", "42").await;
        insert_row(&database, "red", r##"{"type":"color","value":"#ff0000"}"##).await;

        let presets = load_presets_from_store(&preset_store).await.expect("load succeeded");

        assert_eq!(presets.len(), 2);
        assert_eq!(presets[0].id, "brightness");
        assert_eq!(presets[0].name, "brightness");
        assert_eq!(presets[0].value, Value::Number(Number::PositiveInt(42)));
        assert_eq!(presets[1].id, "red");
        assert_eq!(presets[1].value, Value::Color(Color::Hex("#ff0000".to_string())));
    }

    #[tokio::test]
    async fn load_presets_from_store_skips_invalid_presets() {
        let (database, preset_store) = open_in_memory();
        insert_row(&database, "valid", "true").await;
        // Valid JSON, but not a valid value
        insert_row(&database, "invalid", r#"{"type":"temperature","value":20}"#).await;

        let presets = load_presets_from_store(&preset_store).await.expect("load succeeded");

        assert_eq!(presets.len(), 1);
        assert_eq!(presets[0].id, "valid");
        assert_eq!(presets[0].value, Value::Boolean(true));
    }

    #[tokio::test]
    async fn load_presets_from_store_fails_when_the_store_fails() {
        let (database, preset_store) = open_in_memory();
        insert_row(&database, "valid", "true").await;
        insert_row(&database, "corrupt", "not valid json").await;

        let result = load_presets_from_store(&preset_store).await;

        assert!(matches!(result, Err(PresetStoreError::Sqlite(_))), "got {result:?}");
    }
}
