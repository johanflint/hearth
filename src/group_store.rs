use crate::database::{Database, DatabaseError};
use crate::domain::{Group, GroupOwner};
use chrono::{DateTime, Utc};
use rusqlite::Row;
use std::collections::BTreeSet;
use thiserror::Error;

/// Persists the groups owned by Hearth. Groups owned by an integration are discovered, not stored.
#[derive(Debug)]
pub struct GroupStore {
    database: Database,
}

impl GroupStore {
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    pub async fn list(&self) -> Result<Vec<StoredGroup>, GroupStoreError> {
        self.database
            .call(|conn| {
                let mut statement = conn
                    .prepare("SELECT id, revision, name, device_ids, created_at, updated_at FROM groups ORDER BY id")?;
                let rows = statement.query_map([], row_to_stored_group)?;
                Ok(rows.collect::<Result<Vec<_>, _>>()?)
            })
            .await
    }
}

fn row_to_stored_group(row: &Row) -> rusqlite::Result<StoredGroup> {
    let id = row.get(0)?;
    let revision = to_revision(row.get(1)?)
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Integer, Box::new(err)))?;
    let name = row.get(2)?;
    let device_ids: serde_json::Value = row.get(3)?;
    let device_ids = serde_json::from_value(device_ids)
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(err)))?;

    let created_at: String = row.get(4)?;
    let created_at = DateTime::parse_from_rfc3339(&created_at)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(err)))?;

    let updated_at: Option<String> = row.get(5)?;
    let updated_at = updated_at
        .map(|updated_at| {
            DateTime::parse_from_rfc3339(&updated_at)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|err| rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(err)))
        })
        .transpose()?;

    Ok(StoredGroup { id, revision, name, device_ids, created_at, updated_at })
}

fn to_revision(revision: i64) -> Result<u64, GroupStoreError> {
    u64::try_from(revision).map_err(|_| GroupStoreError::InvalidRevision(revision))
}

#[derive(Debug)]
pub struct StoredGroup {
    pub id: String,
    pub revision: u64,
    pub name: String,
    pub device_ids: BTreeSet<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
}

// Only Hearth-owned groups are stored, so a stored group is always owned by Hearth and has no external id
impl From<StoredGroup> for Group {
    fn from(stored: StoredGroup) -> Self {
        Group {
            id: stored.id,
            name: stored.name,
            device_ids: stored.device_ids,
            revision: stored.revision,
            owner: GroupOwner::Hearth,
            external_id: None,
        }
    }
}

#[derive(Error, Debug)]
pub enum GroupStoreError {
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("stored revision {0} is out of range for u64")]
    InvalidRevision(i64),
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use rusqlite::params;
    use std::path::Path;

    fn open_in_memory() -> GroupStore {
        let database = Database::open(Path::new(":memory:")).expect("failed to open in-memory database");
        GroupStore::new(database)
    }

    // Bypasses the store to write rows directly
    async fn insert_row(store: &GroupStore, id: &str, name: &str, device_ids: &str, updated_at: Option<String>) {
        let (id, name, device_ids) = (id.to_string(), name.to_string(), device_ids.to_string());
        store
            .database
            .call(move |conn| {
                let now = Utc::now().to_rfc3339();
                conn.execute(
                    "INSERT INTO groups (id, revision, name, device_ids, created_at, updated_at) VALUES (?1, 0, ?2, ?3, ?4, ?5)",
                    params![id, name, device_ids, now, updated_at],
                )
                .map_err(DatabaseError::from)
            })
            .await
            .expect("insert row");
    }

    // Bypasses the store to change rows in ways the store never would
    async fn execute(store: &GroupStore, sql: &'static str) {
        store.database.call(move |conn| conn.execute(sql, []).map_err(DatabaseError::from)).await.expect("execute");
    }

    #[test]
    fn a_stored_group_converts_to_a_group_owned_by_hearth() {
        let stored = StoredGroup {
            id: "group".to_string(),
            revision: 3,
            name: "Group".to_string(),
            device_ids: BTreeSet::from(["a".to_string(), "b".to_string()]),
            created_at: Utc::now(),
            updated_at: None,
        };

        let group = Group::from(stored);

        assert_eq!(group.id, "group");
        assert_eq!(group.name, "Group");
        assert_eq!(group.revision, 3);
        assert_eq!(group.device_ids, BTreeSet::from(["a".to_string(), "b".to_string()]));
        assert_eq!(group.owner, GroupOwner::Hearth);
        assert_eq!(group.external_id, None);
    }

    #[tokio::test]
    async fn list_returns_empty_for_a_fresh_database() {
        let store = open_in_memory();

        let rows = store.list().await.expect("list succeeded");

        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn list_loads_a_valid_row() {
        let store = open_in_memory();
        insert_row(&store, "group", "Reading lights", r#"["lamp","desk"]"#, Some("2000-01-01T00:00:00Z".to_string()))
            .await;

        let rows = store.list().await.expect("list succeeded");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "group");
        assert_eq!(rows[0].name, "Reading lights");
        assert_eq!(rows[0].device_ids, BTreeSet::from(["desk".to_string(), "lamp".to_string()]));
        let long_ago = DateTime::parse_from_rfc3339("2000-01-01T00:00:00Z").unwrap().with_timezone(&Utc);
        assert_eq!(rows[0].updated_at, Some(long_ago));
    }

    #[tokio::test]
    async fn list_loads_the_revision() {
        let store = open_in_memory();
        insert_row(&store, "group", "Group", "[]", None).await;
        execute(&store, "UPDATE groups SET revision = 7").await;

        let rows = store.list().await.expect("list succeeded");

        assert_eq!(rows[0].revision, 7);
    }

    #[tokio::test]
    async fn list_loads_a_group_without_devices() {
        let store = open_in_memory();
        insert_row(&store, "group", "Group", "[]", None).await;

        let rows = store.list().await.expect("list succeeded");

        assert!(rows[0].device_ids.is_empty());
    }

    #[tokio::test]
    async fn list_loads_a_device_id_listed_twice_once() {
        let store = open_in_memory();
        insert_row(&store, "group", "Group", r#"["lamp","lamp"]"#, None).await;

        let rows = store.list().await.expect("list succeeded");

        assert_eq!(rows[0].device_ids.len(), 1);
    }

    #[tokio::test]
    async fn list_handles_null_updated_at() {
        let store = open_in_memory();
        insert_row(&store, "group", "Group", "[]", None).await;

        let rows = store.list().await.expect("list succeeded");

        assert_eq!(rows[0].updated_at, None);
    }

    #[tokio::test]
    async fn list_orders_the_groups_by_id() {
        let store = open_in_memory();
        for id in ["b", "c", "a"] {
            insert_row(&store, id, id, "[]", None).await;
        }

        let rows = store.list().await.expect("list succeeded");

        let ids: Vec<_> = rows.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
    }

    #[rstest]
    #[case::not_json("not valid json")]
    #[case::not_an_array(r#"{"lamp":true}"#)]
    #[case::not_strings("[1, 2]")]
    #[tokio::test]
    async fn list_fails_on_device_ids_that_are_not_a_json_array_of_strings(#[case] device_ids: &str) {
        let store = open_in_memory();
        insert_row(&store, "group", "Group", device_ids, None).await;

        let result = store.list().await;

        // Invalid JSON means the database is corrupt
        assert!(
            matches!(result, Err(GroupStoreError::Sqlite(rusqlite::Error::FromSqlConversionFailure(3, _, _)))),
            "got {result:?}"
        );
    }

    #[rstest]
    #[case::created_at("UPDATE groups SET created_at = 'yesterday'", 4)]
    #[case::updated_at("UPDATE groups SET updated_at = 'yesterday'", 5)]
    #[tokio::test]
    async fn list_fails_on_an_invalid_timestamp(#[case] sql: &'static str, #[case] column: usize) {
        let store = open_in_memory();
        insert_row(&store, "group", "Group", "[]", Some(Utc::now().to_rfc3339())).await;
        execute(&store, sql).await;

        let result = store.list().await;

        assert!(
            matches!(result, Err(GroupStoreError::Sqlite(rusqlite::Error::FromSqlConversionFailure(index, _, _))) if index == column),
            "got {result:?}"
        );
    }

    #[tokio::test]
    async fn list_fails_when_the_stored_revision_is_negative() {
        let store = open_in_memory();
        insert_row(&store, "group", "Group", "[]", None).await;
        execute(&store, "UPDATE groups SET revision = -1").await;

        let result = store.list().await;

        assert!(
            matches!(result, Err(GroupStoreError::Sqlite(rusqlite::Error::FromSqlConversionFailure(1, _, _)))),
            "got {result:?}"
        );
    }

    #[rstest]
    #[case::same_case("Lights")]
    #[case::other_case("LIGHTS")]
    #[tokio::test]
    async fn the_schema_rejects_a_second_group_with_the_same_name(#[case] name: &'static str) {
        let store = open_in_memory();
        insert_row(&store, "first", "Lights", "[]", None).await;
        let (id, name) = ("second".to_string(), name.to_string());

        let result = store
            .database
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO groups (id, revision, name, device_ids, created_at) VALUES (?1, 0, ?2, '[]', 'now')",
                    params![id, name],
                )
                .map_err(DatabaseError::from)
            })
            .await;

        assert!(
            matches!(result, Err(DatabaseError::Sqlite(rusqlite::Error::SqliteFailure(err, _))) if err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE),
            "got {result:?}"
        );
    }
}
