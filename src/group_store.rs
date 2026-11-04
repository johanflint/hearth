use crate::database::{Database, DatabaseError};
use crate::domain::{Group, Owner};
use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, Row, params};
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

    pub async fn insert(
        &self,
        id: &str,
        name: &str,
        device_ids: &BTreeSet<String>,
    ) -> Result<StoredGroup, InsertError> {
        let id = id.to_string();
        let name = name.to_string();
        let device_ids = serde_json::json!(device_ids);
        self.database
            .call(move |conn| {
                let now = Utc::now().to_rfc3339();
                let stored_group = conn
                    .query_row(
                        "INSERT INTO groups (id, revision, name, device_ids, created_at, updated_at)
                        VALUES (?1, 0, ?2, ?3, ?4, ?4)
                        RETURNING id, revision, name, device_ids, created_at, updated_at",
                        params![id, name, device_ids, now],
                        row_to_stored_group,
                    )
                    .map_err(|err| if is_name_conflict(&err) { InsertError::NameAlreadyExists } else { err.into() })?;
                Ok(stored_group)
            })
            .await
    }

    pub async fn update(
        &self,
        id: &str,
        base_revision: u64,
        name: &str,
        device_ids: &BTreeSet<String>,
    ) -> Result<StoredGroup, UpdateError> {
        let id = id.to_string();
        let name = name.to_string();
        let device_ids = serde_json::json!(device_ids);
        self.database
            .call(move |conn| {
                let now = Utc::now().to_rfc3339();
                let updated = conn
                    .query_row(
                        "UPDATE groups SET name = ?3, device_ids = ?4, revision = revision + 1, updated_at = ?5
                        WHERE id = ?1 AND revision = ?2
                        RETURNING id, revision, name, device_ids, created_at, updated_at",
                        params![id, base_revision as i64, name, device_ids, now],
                        row_to_stored_group,
                    )
                    .optional()
                    .map_err(|err| if is_name_conflict(&err) { UpdateError::NameAlreadyExists } else { err.into() })?;

                if let Some(updated) = updated {
                    return Ok(updated);
                }

                // No row matched; either the group doesn't exist or someone else updated it first
                let current: Option<i64> = conn
                    .query_row("SELECT revision FROM groups WHERE id = ?1", params![id], |row| row.get(0))
                    .optional()?;
                match current {
                    None => Err(UpdateError::NotFound),
                    Some(current) => {
                        Err(UpdateError::RevisionConflict { base_revision, current_revision: to_revision(current)? })
                    }
                }
            })
            .await
    }

    pub async fn delete(&self, id: &str, base_revision: u64) -> Result<(), DeleteError> {
        let id = id.to_string();
        self.database
            .call(move |conn| {
                let rows_deleted: usize = conn
                    .execute("DELETE FROM groups WHERE id = ?1 AND revision = ?2", params![id, base_revision as i64])?;
                if rows_deleted == 1 {
                    return Ok(());
                }

                // No row matched; either the group doesn't exist or someone else updated it first
                let current: Option<i64> = conn
                    .query_row("SELECT revision FROM groups WHERE id = ?1", params![id], |row| row.get(0))
                    .optional()?;
                match current {
                    None => Err(DeleteError::NotFound),
                    Some(current) => {
                        Err(DeleteError::RevisionConflict { base_revision, current_revision: to_revision(current)? })
                    }
                }
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

fn is_name_conflict(err: &rusqlite::Error) -> bool {
    matches!(err, rusqlite::Error::SqliteFailure(err, _) if err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE)
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
            owner: Owner::Hearth,
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

#[derive(Error, Debug)]
pub enum InsertError {
    #[error("a group with this name already exists")]
    NameAlreadyExists,
    #[error(transparent)]
    Store(#[from] GroupStoreError),
}

// `?` applies a single `From`, so `DatabaseError` and `rusqlite::Error` can't reach `Store` via `GroupStoreError` on their own
impl From<DatabaseError> for InsertError {
    fn from(err: DatabaseError) -> Self {
        Self::Store(err.into())
    }
}

impl From<rusqlite::Error> for InsertError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Store(err.into())
    }
}

#[derive(Error, Debug)]
pub enum UpdateError {
    #[error("group not found")]
    NotFound,
    #[error(
        "revision conflict: update is based on revision {base_revision}, but the current revision is {current_revision}"
    )]
    RevisionConflict { base_revision: u64, current_revision: u64 },
    #[error("a group with this name already exists")]
    NameAlreadyExists,
    #[error(transparent)]
    Store(#[from] GroupStoreError),
}

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

#[derive(Error, Debug)]
pub enum DeleteError {
    #[error("group not found")]
    NotFound,
    #[error(
        "revision conflict: delete is based on revision {base_revision}, but the current revision is {current_revision}"
    )]
    RevisionConflict { base_revision: u64, current_revision: u64 },
    #[error(transparent)]
    Store(#[from] GroupStoreError),
}

impl From<DatabaseError> for DeleteError {
    fn from(err: DatabaseError) -> Self {
        Self::Store(err.into())
    }
}

impl From<rusqlite::Error> for DeleteError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Store(err.into())
    }
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
        assert_eq!(group.owner, Owner::Hearth);
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

    fn device_ids(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    #[tokio::test]
    async fn insert_returns_the_stored_group_at_revision_zero() {
        let store = open_in_memory();

        let inserted = store.insert("group", "Reading lights", &device_ids(&["lamp", "desk"])).await;

        let inserted = inserted.expect("insert succeeded");
        assert_eq!(inserted.id, "group");
        assert_eq!(inserted.revision, 0);
        assert_eq!(inserted.name, "Reading lights");
        assert_eq!(inserted.device_ids, device_ids(&["desk", "lamp"]));
        assert_eq!(inserted.updated_at, Some(inserted.created_at));
    }

    #[tokio::test]
    async fn insert_persists_the_group_and_leaves_others_untouched() {
        let store = open_in_memory();
        insert_row(&store, "other", "Other", r#"["a"]"#, None).await;

        store.insert("group", "Group", &device_ids(&["b", "c"])).await.expect("insert succeeded");

        let rows = store.list().await.expect("list succeeded");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "group");
        assert_eq!(rows[0].name, "Group");
        assert_eq!(rows[0].revision, 0);
        assert_eq!(rows[0].device_ids, device_ids(&["b", "c"]));
        assert_eq!(rows[1].id, "other");
        assert_eq!(rows[1].name, "Other");
        assert_eq!(rows[1].device_ids, device_ids(&["a"]));
    }

    #[tokio::test]
    async fn insert_accepts_a_group_without_devices() {
        let store = open_in_memory();

        let inserted = store.insert("group", "Group", &BTreeSet::new()).await.expect("insert succeeded");

        assert!(inserted.device_ids.is_empty());
        assert!(store.list().await.expect("list succeeded")[0].device_ids.is_empty());
    }

    #[tokio::test]
    async fn insert_stores_the_device_ids_as_a_json_array() {
        let store = open_in_memory();

        store.insert("group", "Group", &device_ids(&["b", "a"])).await.expect("insert succeeded");

        let stored: String = store
            .database
            .call(|conn| {
                conn.query_row("SELECT device_ids FROM groups", [], |row| row.get(0)).map_err(DatabaseError::from)
            })
            .await
            .expect("query");
        assert_eq!(stored, r#"["a","b"]"#);
    }

    #[rstest]
    #[case::same_case("Lights")]
    #[case::different_case("lIGHTS")]
    #[tokio::test]
    async fn insert_rejects_a_name_that_is_already_in_use(#[case] name: &str) {
        let store = open_in_memory();
        insert_row(&store, "group", "Lights", r#"["a"]"#, None).await;

        let result = store.insert("duplicate", name, &device_ids(&["b"])).await;

        assert!(matches!(result, Err(InsertError::NameAlreadyExists)), "got {result:?}");
        let rows = store.list().await.expect("list succeeded");
        assert_eq!(rows.len(), 1, "nothing must be stored");
        assert_eq!(rows[0].device_ids, device_ids(&["a"]));
    }

    #[tokio::test]
    async fn insert_fails_for_an_id_that_is_already_in_use() {
        let store = open_in_memory();
        insert_row(&store, "group", "Lights", "[]", None).await;

        let result = store.insert("group", "Other", &device_ids(&["a"])).await;

        assert!(matches!(result, Err(InsertError::Store(GroupStoreError::Sqlite(_)))), "got {result:?}");
        let rows = store.list().await.expect("list succeeded");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Lights");
    }

    #[rstest]
    #[case::same_case("Other")]
    #[case::different_case("oTHER")]
    #[tokio::test]
    async fn update_rejects_a_name_used_by_another_group(#[case] name: &str) {
        let store = open_in_memory();
        insert_row(&store, "group", "Lights", r#"["a"]"#, None).await;
        insert_row(&store, "other", "Other", "[]", None).await;

        let result = store.update("group", 0, name, &device_ids(&["b"])).await;

        assert!(matches!(result, Err(UpdateError::NameAlreadyExists)), "got {result:?}");
        let rows = store.list().await.expect("list succeeded");
        assert_eq!(rows[0].name, "Lights", "the group must be unchanged");
        assert_eq!(rows[0].device_ids, device_ids(&["a"]));
        assert_eq!(rows[0].revision, 0);
    }

    #[rstest]
    #[case::same_name("Lights")]
    #[case::different_case("LIGHTS")]
    #[tokio::test]
    async fn update_allows_a_group_to_keep_its_own_name(#[case] name: &str) {
        let store = open_in_memory();
        insert_row(&store, "group", "Lights", r#"["a"]"#, None).await;

        let updated = store.update("group", 0, name, &device_ids(&["b"])).await.expect("update succeeded");

        assert_eq!(updated.name, name);
        assert_eq!(updated.revision, 1);
    }

    #[tokio::test]
    async fn update_replaces_the_name_and_devices_and_returns_the_stored_group() {
        let store = open_in_memory();
        insert_row(&store, "group", "Lights", r#"["a"]"#, None).await;

        let updated = store.update("group", 0, "Reading lights", &device_ids(&["lamp", "desk"])).await;

        let updated = updated.expect("update succeeded");
        assert_eq!(updated.id, "group");
        assert_eq!(updated.name, "Reading lights");
        assert_eq!(updated.device_ids, device_ids(&["desk", "lamp"]));
        let rows = store.list().await.expect("list succeeded");
        assert_eq!(rows[0].name, "Reading lights");
        assert_eq!(rows[0].device_ids, device_ids(&["desk", "lamp"]));
    }

    #[tokio::test]
    async fn update_accepts_a_group_without_devices() {
        let store = open_in_memory();
        insert_row(&store, "group", "Lights", r#"["a"]"#, None).await;

        let updated = store.update("group", 0, "Lights", &BTreeSet::new()).await.expect("update succeeded");

        assert!(updated.device_ids.is_empty());
    }

    #[tokio::test]
    async fn update_stores_the_device_ids_as_a_sorted_json_array() {
        let store = open_in_memory();
        insert_row(&store, "group", "Lights", "[]", None).await;

        store.update("group", 0, "Lights", &device_ids(&["b", "a"])).await.expect("update succeeded");

        let stored: String = store
            .database
            .call(|conn| {
                conn.query_row("SELECT device_ids FROM groups", [], |row| row.get(0)).map_err(DatabaseError::from)
            })
            .await
            .expect("query");
        assert_eq!(stored, r#"["a","b"]"#);
    }

    #[tokio::test]
    async fn update_increments_the_revision() {
        let store = open_in_memory();
        insert_row(&store, "group", "Lights", "[]", None).await;

        let first = store.update("group", 0, "Lights", &device_ids(&["a"])).await.expect("first update succeeded");
        let second = store.update("group", 1, "Lights", &device_ids(&["b"])).await.expect("second update succeeded");

        assert_eq!(first.revision, 1);
        assert_eq!(second.revision, 2);
        assert_eq!(store.list().await.expect("list succeeded")[0].revision, 2);
    }

    #[tokio::test]
    async fn update_sets_updated_at_and_keeps_created_at() {
        let store = open_in_memory();
        insert_row(&store, "group", "Lights", "[]", None).await;
        execute(&store, "UPDATE groups SET created_at = '2000-01-01T00:00:00Z'").await;
        let before = Utc::now();

        let updated = store.update("group", 0, "Lights", &device_ids(&["a"])).await.expect("update succeeded");

        let long_ago = DateTime::parse_from_rfc3339("2000-01-01T00:00:00Z").unwrap().with_timezone(&Utc);
        assert_eq!(updated.created_at, long_ago);
        assert!(updated.updated_at.expect("updated_at is set") >= before, "got {updated:?}");
    }

    #[tokio::test]
    async fn update_leaves_other_groups_untouched() {
        let store = open_in_memory();
        insert_row(&store, "group", "Lights", "[]", None).await;
        insert_row(&store, "other", "Other", r#"["x"]"#, None).await;

        store.update("group", 0, "Renamed", &device_ids(&["a"])).await.expect("update succeeded");

        let rows = store.list().await.expect("list succeeded");
        assert_eq!(rows[1].id, "other");
        assert_eq!(rows[1].name, "Other");
        assert_eq!(rows[1].device_ids, device_ids(&["x"]));
        assert_eq!(rows[1].revision, 0);
        assert_eq!(rows[1].updated_at, None);
    }

    #[rstest]
    #[case::fresh_database(&[])]
    #[case::other_groups(&["other"])]
    #[tokio::test]
    async fn update_returns_not_found_for_an_unknown_id(#[case] existing_ids: &[&str]) {
        let store = open_in_memory();
        for id in existing_ids {
            insert_row(&store, id, id, "[]", None).await;
        }

        let result = store.update("group", 0, "Lights", &device_ids(&["a"])).await;

        assert!(matches!(result, Err(UpdateError::NotFound)), "got {result:?}");
        assert_eq!(store.list().await.expect("list succeeded").len(), existing_ids.len(), "update must not create");
    }

    #[rstest]
    #[case::behind(1, 2)]
    #[case::ahead(5, 2)]
    #[tokio::test]
    async fn update_returns_a_revision_conflict_for_a_mismatching_base_revision(
        #[case] base_revision: u64,
        #[case] current_revision: u64,
    ) {
        let store = open_in_memory();
        insert_row(&store, "group", "Lights", r#"["a"]"#, None).await;
        execute(&store, "UPDATE groups SET revision = 2").await;

        let result = store.update("group", base_revision, "Renamed", &device_ids(&["b"])).await;

        assert!(
            matches!(
                result,
                Err(UpdateError::RevisionConflict { base_revision: b, current_revision: c })
                    if b == base_revision && c == current_revision
            ),
            "got {result:?}"
        );
        let rows = store.list().await.expect("list succeeded");
        assert_eq!(rows[0].name, "Lights", "a conflicting update must not change the group");
        assert_eq!(rows[0].device_ids, device_ids(&["a"]));
        assert_eq!(rows[0].revision, 2);
    }

    #[tokio::test]
    async fn update_fails_for_a_stored_revision_out_of_range() {
        let store = open_in_memory();
        insert_row(&store, "group", "Lights", "[]", None).await;
        execute(&store, "UPDATE groups SET revision = -1").await;

        let result = store.update("group", 0, "Lights", &BTreeSet::new()).await;

        assert!(matches!(result, Err(UpdateError::Store(GroupStoreError::InvalidRevision(-1)))), "got {result:?}");
    }

    #[tokio::test]
    async fn delete_removes_the_group_when_the_base_revision_matches() {
        let store = open_in_memory();
        insert_row(&store, "group", "Lights", "[]", None).await;
        execute(&store, "UPDATE groups SET revision = 3").await;

        store.delete("group", 3).await.expect("delete succeeded");

        assert!(store.list().await.expect("list succeeded").is_empty(), "group must be deleted");
    }

    #[tokio::test]
    async fn delete_only_removes_the_given_group() {
        let store = open_in_memory();
        insert_row(&store, "group", "Lights", "[]", None).await;
        insert_row(&store, "other", "Other", r#"["a"]"#, None).await;

        store.delete("group", 0).await.expect("delete succeeded");

        let rows = store.list().await.expect("list succeeded");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "other");
        assert_eq!(rows[0].name, "Other");
        assert_eq!(rows[0].device_ids, device_ids(&["a"]));
    }

    #[tokio::test]
    async fn delete_frees_the_name_and_id_for_a_new_group() {
        let store = open_in_memory();
        store.insert("group", "Lights", &BTreeSet::new()).await.expect("insert succeeded");

        store.delete("group", 0).await.expect("delete succeeded");
        let reinserted = store.insert("group", "lights", &BTreeSet::new()).await.expect("insert succeeded");

        assert_eq!(reinserted.revision, 0);
    }

    #[rstest]
    #[case::behind(0, 2)]
    #[case::ahead(5, 2)]
    #[tokio::test]
    async fn delete_returns_a_revision_conflict_for_a_mismatching_base_revision(
        #[case] base_revision: u64,
        #[case] current_revision: u64,
    ) {
        let store = open_in_memory();
        insert_row(&store, "group", "Lights", "[]", None).await;
        execute(&store, "UPDATE groups SET revision = 2").await;

        let result = store.delete("group", base_revision).await;

        assert!(
            matches!(
                result,
                Err(DeleteError::RevisionConflict { base_revision: b, current_revision: c })
                    if b == base_revision && c == current_revision
            ),
            "got {result:?}"
        );
        let rows = store.list().await.expect("list succeeded");
        assert_eq!(rows.len(), 1, "the group must not be deleted");
        assert_eq!(rows[0].revision, 2);
    }

    #[rstest]
    #[case::fresh_database(&[])]
    #[case::other_groups(&["other"])]
    #[tokio::test]
    async fn delete_returns_not_found_for_an_unknown_id(#[case] existing_ids: &[&str]) {
        let store = open_in_memory();
        for id in existing_ids {
            insert_row(&store, id, id, "[]", None).await;
        }

        let result = store.delete("group", 0).await;

        assert!(matches!(result, Err(DeleteError::NotFound)), "got {result:?}");
        assert_eq!(store.list().await.expect("list succeeded").len(), existing_ids.len());
    }

    #[tokio::test]
    async fn delete_fails_for_a_stored_revision_out_of_range() {
        let store = open_in_memory();
        insert_row(&store, "group", "Lights", "[]", None).await;
        execute(&store, "UPDATE groups SET revision = -1").await;

        let result = store.delete("group", 0).await;

        assert!(matches!(result, Err(DeleteError::Store(GroupStoreError::InvalidRevision(-1)))), "got {result:?}");
    }

    #[tokio::test]
    async fn delete_fails_when_the_store_fails() {
        let store = open_in_memory();
        execute(&store, "DROP TABLE groups").await;

        let result = store.delete("group", 0).await;

        assert!(matches!(result, Err(DeleteError::Store(GroupStoreError::Sqlite(_)))), "got {result:?}");
    }
}
