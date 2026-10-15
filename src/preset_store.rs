use crate::database::{Database, DatabaseError};
use chrono::{DateTime, Utc};
use rusqlite::Row;
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
    #[allow(dead_code)]
    pub created_at: DateTime<Utc>,
    #[allow(dead_code)]
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Error, Debug)]
pub enum PresetStoreError {
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
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
}
