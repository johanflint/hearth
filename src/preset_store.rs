use crate::database::{Database, DatabaseError};
use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, Row, params};
use thiserror::Error;

#[derive(Debug)]
pub struct PresetStore {
    database: Database,
}

impl PresetStore {
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    pub async fn list(&self) -> Result<Vec<StoredPreset>, PresetStoreError> {
        self.database
            .call(|conn| {
                let mut statement =
                    conn.prepare("SELECT id, name, value, created_at, updated_at FROM presets ORDER BY id")?;
                let rows = statement.query_map([], row_to_stored_preset)?;
                Ok(rows.collect::<Result<Vec<_>, _>>()?)
            })
            .await
    }

    pub async fn by_id(&self, id: &str) -> Result<Option<StoredPreset>, PresetStoreError> {
        let id = id.to_string();
        self.database
            .call(|conn| {
                let result = conn
                    .query_row(
                        "SELECT id, name, value, created_at, updated_at FROM presets WHERE id = ?1",
                        [id],
                        row_to_stored_preset,
                    )
                    .optional();
                Ok(result?)
            })
            .await
    }

    pub async fn update(&self, id: &str, name: &str, value: serde_json::Value) -> Result<StoredPreset, UpdateError> {
        let id = id.to_string();
        let name = name.to_string();
        self.database
            .call(move |conn| {
                let now = Utc::now().to_rfc3339();
                conn.query_row(
                    "UPDATE presets SET name = ?2, value = ?3, updated_at = ?4
                    WHERE id = ?1
                    RETURNING id, name, value, created_at, updated_at",
                    params![id, name, value, now],
                    row_to_stored_preset,
                )
                .optional()?
                .ok_or(UpdateError::NotFound)
            })
            .await
    }
}

fn row_to_stored_preset(row: &Row) -> rusqlite::Result<StoredPreset> {
    let id = row.get(0)?;
    let name = row.get(1)?;
    let value: serde_json::Value = row.get(2)?;

    let created_at: String = row.get(3)?;
    let created_at = DateTime::parse_from_rfc3339(&created_at)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(err)))?;

    let updated_at: Option<String> = row.get(4)?;
    let updated_at = updated_at
        .map(|updated_at| {
            DateTime::parse_from_rfc3339(&updated_at)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|err| rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(err)))
        })
        .transpose()?;

    Ok(StoredPreset { id, name, value, created_at, updated_at })
}

#[derive(Debug)]
pub struct StoredPreset {
    pub id: String,
    pub name: String,
    pub value: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Error, Debug)]
pub enum PresetStoreError {
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

#[derive(Error, Debug)]
pub enum UpdateError {
    #[error("preset not found")]
    NotFound,
    #[error(transparent)]
    Store(#[from] PresetStoreError),
}

// `?` applies a single `From`, so `DatabaseError` and `rusqlite::Error` can't reach `Store` via `PresetStoreError` on their own
impl From<DatabaseError> for UpdateError {
    fn from(err: DatabaseError) -> Self {
        Self::Store(err.into())
    }
}

impl From<rusqlite::Error> for UpdateError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Store(err.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use rusqlite::params;
    use serde_json::json;
    use std::path::Path;

    fn open_in_memory() -> PresetStore {
        let database = Database::open(Path::new(":memory:")).expect("failed to open in-memory database");
        PresetStore::new(database)
    }

    // Bypasses the store to write rows directly
    async fn insert_row(store: &PresetStore, id: &str, name: &str, value: &str, updated_at: Option<String>) {
        let (id, name, value) = (id.to_string(), name.to_string(), value.to_string());
        store
            .database
            .call(move |conn| {
                let now = Utc::now().to_rfc3339();
                conn.execute(
                    "INSERT INTO presets (id, name, value, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![id, name, value, now, updated_at],
                )
                .map_err(DatabaseError::from)
            })
            .await
            .expect("insert row");
    }

    // Bypasses the store to change rows in ways the store never would
    async fn execute(store: &PresetStore, sql: &'static str) {
        store.database.call(move |conn| conn.execute(sql, []).map_err(DatabaseError::from)).await.expect("execute");
    }

    #[tokio::test]
    async fn list_returns_empty_for_fresh_database() {
        let store = open_in_memory();
        let rows = store.list().await.expect("list succeeded");
        assert_eq!(rows.len(), 0);
    }

    #[tokio::test]
    async fn list_loads_a_valid_row() {
        let store = open_in_memory();
        let value = r##"{"type":"color","value":"#ff0000"}"##;
        insert_row(&store, "preset", "Red", value, Some("2000-01-01T00:00:00Z".to_string())).await;

        let rows = store.list().await.expect("list succeeded");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "preset");
        assert_eq!(rows[0].name, "Red");
        assert_eq!(rows[0].value, json!({ "type": "color", "value": "#ff0000" }));
        let long_ago = DateTime::parse_from_rfc3339("2000-01-01T00:00:00Z").unwrap().with_timezone(&Utc);
        assert_eq!(rows[0].updated_at, Some(long_ago));
    }

    #[tokio::test]
    async fn list_fails_on_a_value_that_is_not_json() {
        let store = open_in_memory();
        insert_row(&store, "badPreset", "Bad", "not valid json", Some(Utc::now().to_rfc3339())).await;

        let result = store.list().await;

        // Invalid JSON means the database is corrupt
        assert!(
            matches!(result, Err(PresetStoreError::Sqlite(rusqlite::Error::FromSqlConversionFailure(2, _, _)))),
            "got {result:?}"
        );
    }

    #[rstest]
    #[case::created_at("UPDATE presets SET created_at = 'yesterday'", 3)]
    #[case::updated_at("UPDATE presets SET updated_at = 'yesterday'", 4)]
    #[tokio::test]
    async fn list_fails_on_an_invalid_timestamp(#[case] sql: &'static str, #[case] column: usize) {
        let store = open_in_memory();
        insert_row(&store, "preset", "Red", "true", Some(Utc::now().to_rfc3339())).await;
        execute(&store, sql).await;

        let result = store.list().await;

        assert!(
            matches!(result, Err(PresetStoreError::Sqlite(rusqlite::Error::FromSqlConversionFailure(index, _, _))) if index == column),
            "got {result:?}"
        );
    }

    #[tokio::test]
    async fn list_orders_the_presets_by_id() {
        let store = open_in_memory();
        for id in ["b", "c", "a"] {
            insert_row(&store, id, "Preset", "true", None).await;
        }

        let rows = store.list().await.expect("list succeeded");

        let ids: Vec<_> = rows.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
    }

    #[tokio::test]
    async fn list_handles_null_updated_at() {
        let store = open_in_memory();
        insert_row(&store, "preset", "Red", "true", None).await;

        let rows = store.list().await.expect("list succeeded");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].updated_at, None);
    }

    #[tokio::test]
    async fn by_id_returns_the_requested_preset() {
        let store = open_in_memory();
        let value = r##"{"type":"color","value":"#ff0000"}"##;
        insert_row(&store, "other", "Other", "true", None).await;
        insert_row(&store, "preset", "Red", value, Some("2000-01-01T00:00:00Z".to_string())).await;

        let row = store.by_id("preset").await.expect("by_id succeeded").expect("preset exists");

        assert_eq!(row.id, "preset");
        assert_eq!(row.name, "Red");
        assert_eq!(row.value, json!({ "type": "color", "value": "#ff0000" }));
        let long_ago = DateTime::parse_from_rfc3339("2000-01-01T00:00:00Z").unwrap().with_timezone(&Utc);
        assert_eq!(row.updated_at, Some(long_ago));
    }

    #[tokio::test]
    async fn by_id_handles_null_updated_at() {
        let store = open_in_memory();
        insert_row(&store, "preset", "Red", "true", None).await;

        let row = store.by_id("preset").await.expect("by_id succeeded").expect("preset exists");

        assert_eq!(row.updated_at, None);
    }

    #[rstest]
    #[case::fresh_database(&[])]
    #[case::other_presets(&["other"])]
    #[tokio::test]
    async fn by_id_returns_none_for_an_unknown_id(#[case] existing_ids: &[&str]) {
        let store = open_in_memory();
        for id in existing_ids {
            insert_row(&store, id, "Preset", "true", None).await;
        }

        let row = store.by_id("preset").await.expect("by_id succeeded");

        assert!(row.is_none(), "got {row:?}");
    }

    #[tokio::test]
    async fn by_id_fails_on_a_value_that_is_not_json() {
        let store = open_in_memory();
        insert_row(&store, "preset", "Bad", "not valid json", None).await;

        let result = store.by_id("preset").await;

        assert!(
            matches!(result, Err(PresetStoreError::Sqlite(rusqlite::Error::FromSqlConversionFailure(2, _, _)))),
            "got {result:?}"
        );
    }

    #[rstest]
    #[case::created_at("UPDATE presets SET created_at = 'yesterday'", 3)]
    #[case::updated_at("UPDATE presets SET updated_at = 'yesterday'", 4)]
    #[tokio::test]
    async fn by_id_fails_on_an_invalid_timestamp(#[case] sql: &'static str, #[case] column: usize) {
        let store = open_in_memory();
        insert_row(&store, "preset", "Red", "true", Some(Utc::now().to_rfc3339())).await;
        execute(&store, sql).await;

        let result = store.by_id("preset").await;

        assert!(
            matches!(result, Err(PresetStoreError::Sqlite(rusqlite::Error::FromSqlConversionFailure(index, _, _))) if index == column),
            "got {result:?}"
        );
    }

    #[tokio::test]
    async fn update_replaces_the_name_and_value_and_returns_the_stored_preset() {
        let store = open_in_memory();
        insert_row(&store, "preset", "Red", "true", None).await;

        let updated = store.update("preset", "Blue", json!({ "type": "color", "value": "#0000ff" })).await;

        let updated = updated.expect("update succeeded");
        assert_eq!(updated.id, "preset");
        assert_eq!(updated.name, "Blue");
        assert_eq!(updated.value, json!({ "type": "color", "value": "#0000ff" }));
        let stored = store.by_id("preset").await.expect("by_id succeeded").expect("preset exists");
        assert_eq!(stored.name, "Blue");
        assert_eq!(stored.value, json!({ "type": "color", "value": "#0000ff" }));
    }

    #[tokio::test]
    async fn update_sets_updated_at_and_keeps_created_at() {
        let store = open_in_memory();
        insert_row(&store, "preset", "Red", "true", None).await;
        execute(&store, "UPDATE presets SET created_at = '2000-01-01T00:00:00Z'").await;
        let before = Utc::now();

        let updated = store.update("preset", "Red", json!(false)).await.expect("update succeeded");

        let long_ago = DateTime::parse_from_rfc3339("2000-01-01T00:00:00Z").unwrap().with_timezone(&Utc);
        assert_eq!(updated.created_at, long_ago);
        assert!(updated.updated_at.expect("updated_at is set") >= before, "got {updated:?}");
    }

    #[tokio::test]
    async fn update_leaves_other_presets_untouched() {
        let store = open_in_memory();
        insert_row(&store, "preset", "Red", "true", None).await;
        insert_row(&store, "other", "Other", "10", None).await;

        store.update("preset", "Blue", json!(false)).await.expect("update succeeded");

        let other = store.by_id("other").await.expect("by_id succeeded").expect("preset exists");
        assert_eq!(other.name, "Other");
        assert_eq!(other.value, json!(10));
        assert_eq!(other.updated_at, None);
    }

    #[rstest]
    #[case::fresh_database(&[])]
    #[case::other_presets(&["other"])]
    #[tokio::test]
    async fn update_returns_not_found_for_an_unknown_id(#[case] existing_ids: &[&str]) {
        let store = open_in_memory();
        for id in existing_ids {
            insert_row(&store, id, "Preset", "true", None).await;
        }

        let result = store.update("preset", "Blue", json!(false)).await;

        assert!(matches!(result, Err(UpdateError::NotFound)), "got {result:?}");
        assert!(store.by_id("preset").await.expect("by_id succeeded").is_none(), "update must not create the preset");
    }

    #[tokio::test]
    async fn update_fails_when_the_stored_row_is_corrupt() {
        let store = open_in_memory();
        insert_row(&store, "preset", "Red", "true", None).await;
        execute(&store, "UPDATE presets SET created_at = 'yesterday'").await;

        let result = store.update("preset", "Blue", json!(false)).await;

        assert!(
            matches!(
                result,
                Err(UpdateError::Store(PresetStoreError::Sqlite(rusqlite::Error::FromSqlConversionFailure(3, _, _))))
            ),
            "got {result:?}"
        );
    }
}
