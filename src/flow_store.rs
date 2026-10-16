use crate::database::{Database, DatabaseError};
use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, Row, params};
use thiserror::Error;

#[derive(Debug)]
pub struct FlowStore {
    database: Database,
}

impl FlowStore {
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    pub async fn list(&self) -> Result<Vec<StoredFlow>, FlowStoreError> {
        self.database
            .call(|conn| {
                let mut statement =
                    conn.prepare("SELECT id, revision, document, created_at, updated_at FROM flows ORDER BY id")?;
                let rows = statement.query_map([], row_to_stored_flow)?;
                Ok(rows.collect::<Result<Vec<_>, _>>()?)
            })
            .await
    }

    pub async fn by_id(&self, id: &str) -> Result<Option<StoredFlow>, FlowStoreError> {
        let id = id.to_string();
        self.database
            .call(|conn| {
                let result = conn
                    .query_row(
                        "SELECT id, revision, document, created_at, updated_at FROM flows WHERE id = ?1",
                        [id],
                        row_to_stored_flow,
                    )
                    .optional();
                Ok(result?)
            })
            .await
    }

    pub async fn insert(&self, id: &str, document: serde_json::Value) -> Result<u64, InsertError> {
        let id = id.to_string();
        self.database
            .call(move |conn| {
                let now = Utc::now().to_rfc3339();
                let result = conn.execute(
                    "INSERT INTO flows (id, revision, document, created_at, updated_at) VALUES (?1, 0, ?2, ?3, ?3)",
                    params![id, document, now],
                );

                match result {
                    Ok(_) => Ok(0),
                    #[rustfmt::skip]
                    Err(rusqlite::Error::SqliteFailure(err, _)) if err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY =>
                    {
                        Err(InsertError::AlreadyExists)
                    }
                    Err(err) => Err(err.into()),
                }
            })
            .await
    }

    pub async fn update(&self, id: &str, base_revision: u64, document: serde_json::Value) -> Result<u64, UpdateError> {
        let id = id.to_string();
        self.database
            .call(move |conn| {
                let now = Utc::now().to_rfc3339();
                let updated: Option<i64> = conn
                    .query_row(
                        "UPDATE flows SET document = ?3, revision = revision + 1, updated_at = ?4
                        WHERE id = ?1 AND revision = ?2
                        RETURNING revision",
                        params![id, base_revision as i64, document, now],
                        |row| row.get(0),
                    )
                    .optional()?;

                if let Some(revision) = updated {
                    return Ok(to_revision(revision)?);
                }

                // No row matched; either the flow doesn't exist or someone else updated it first
                let current: Option<i64> = conn
                    .query_row("SELECT revision FROM flows WHERE id = ?1", params![id], |row| row.get(0))
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
                    .execute("DELETE FROM flows WHERE id = ?1 AND revision = ?2", params![id, base_revision as i64])?;
                if rows_deleted == 1 {
                    return Ok(());
                }

                // No row matched: either the flow doesn't exist or someone else updated it first
                let current: Option<i64> = conn
                    .query_row("SELECT revision FROM flows WHERE id = ?1", params![id], |row| row.get(0))
                    .optional()?;
                match current {
                    Some(current) => {
                        Err(DeleteError::RevisionConflict { base_revision, current_revision: to_revision(current)? })
                    }
                    None => Err(DeleteError::NotFound),
                }
            })
            .await
    }
}

#[derive(Debug)]
pub struct StoredFlow {
    pub id: String,
    pub revision: u64,
    pub document: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
}

fn row_to_stored_flow(row: &Row) -> rusqlite::Result<StoredFlow> {
    let id = row.get(0)?;
    let revision = to_revision(row.get(1)?)
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Integer, Box::new(err)))?;

    let document: serde_json::Value = row.get(2)?;

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

    Ok(StoredFlow { id, revision, document, created_at, updated_at })
}

fn to_revision(revision: i64) -> Result<u64, FlowStoreError> {
    u64::try_from(revision).map_err(|_| FlowStoreError::InvalidRevision(revision))
}

#[derive(Error, Debug)]
pub enum FlowStoreError {
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("stored revision {0} is out of range for u64")]
    InvalidRevision(i64),
}

#[derive(Error, Debug)]
pub enum InsertError {
    #[error("flow already exists")]
    AlreadyExists,
    #[error(transparent)]
    Store(#[from] FlowStoreError),
}

// `?` applies a single `From`, so `DatabaseError` and `rusqlite::Error` can't reach `Store` via `FlowStoreError` on their own
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
    #[error("flow not found")]
    NotFound,
    #[error(
        "revision conflict: update is based on revision {base_revision}, but the current revision is {current_revision}"
    )]
    RevisionConflict { base_revision: u64, current_revision: u64 },
    #[error(transparent)]
    Store(#[from] FlowStoreError),
}

// `?` applies a single `From`, so `DatabaseError` and `rusqlite::Error` can't reach `Store` via `FlowStoreError` on their own
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
    #[error("flow not found")]
    NotFound,
    #[error(
        "revision conflict: delete is based on revision {base_revision}, but the current revision is {current_revision}"
    )]
    RevisionConflict { base_revision: u64, current_revision: u64 },
    #[error(transparent)]
    Store(#[from] FlowStoreError),
}

// `?` applies a single `From`, so `DatabaseError` and `rusqlite::Error` can't reach `Store` via `FlowStoreError` on their own
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
    use serde_json::json;
    use std::path::Path;

    fn flow_document() -> serde_json::Value {
        json!({"id":"flow","name":"Test","nodes":[{"id":"startNode","type":"startNode","outgoingNode":"endNode"},{"id":"endNode","type":"endNode"}]})
    }

    fn open_in_memory() -> FlowStore {
        let database = Database::open(Path::new(":memory:")).expect("failed to open in-memory database");
        FlowStore::new(database)
    }

    // Bypasses the store to write rows that `insert` would never produce
    async fn insert_row(store: &FlowStore, id: &str, revision: i64, document: &str, updated_at: Option<String>) {
        let (id, document) = (id.to_string(), document.to_string());
        store
            .database
            .call(move |conn| {
                let now = Utc::now().to_rfc3339();
                conn.execute(
                    "INSERT INTO flows (id, revision, document, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![id, revision, document, now, updated_at],
                )
                .map_err(DatabaseError::from)
            })
            .await
            .expect("insert row");
    }

    // Bypasses the store to change rows in ways the store never would
    async fn execute(store: &FlowStore, sql: &'static str) {
        store.database.call(move |conn| conn.execute(sql, []).map_err(DatabaseError::from)).await.expect("execute");
    }

    #[tokio::test]
    async fn list_returns_empty_for_fresh_database() {
        let store = open_in_memory();
        let rows = store.list().await.expect("list succeeded");
        assert_eq!(rows.len(), 0);
    }

    #[tokio::test]
    async fn list_loads_a_valid_row_and_preserves_revision() {
        let store = open_in_memory();
        let document = r#"{"id":"1d3a9341-e462-4663-bb3d-5d977053cc0f","name":"Test","nodes":[],"triggers":[]}"#;
        insert_row(&store, "1d3a9341-e462-4663-bb3d-5d977053cc0f", 42, document, Some(Utc::now().to_rfc3339())).await;

        let rows = store.list().await.expect("list succeeded");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "1d3a9341-e462-4663-bb3d-5d977053cc0f");
        assert_eq!(rows[0].revision, 42);
        // flow parse might fail if document is incomplete, but revision is preserved
    }

    #[tokio::test]
    async fn list_fails_on_a_document_that_is_not_json() {
        let store = open_in_memory();
        insert_row(&store, "badFlow", 1, "not valid json", Some(Utc::now().to_rfc3339())).await;

        let result = store.list().await;

        // Invalid JSON means the database is corrupt
        assert!(
            matches!(result, Err(FlowStoreError::Sqlite(rusqlite::Error::FromSqlConversionFailure(2, _, _)))),
            "got {result:?}"
        );
    }

    #[rstest]
    #[case::created_at("UPDATE flows SET created_at = 'yesterday'", 3)]
    #[case::updated_at("UPDATE flows SET updated_at = 'yesterday'", 4)]
    #[tokio::test]
    async fn list_fails_on_an_invalid_timestamp(#[case] sql: &'static str, #[case] column: usize) {
        let store = open_in_memory();
        store.insert("flow", flow_document()).await.expect("insert");
        execute(&store, sql).await;

        let result = store.list().await;

        assert!(
            matches!(result, Err(FlowStoreError::Sqlite(rusqlite::Error::FromSqlConversionFailure(index, _, _))) if index == column),
            "got {result:?}"
        );
    }

    #[tokio::test]
    async fn list_orders_the_flows_by_id() {
        let store = open_in_memory();
        for id in ["b", "c", "a"] {
            store.insert(id, flow_document()).await.expect("insert");
        }

        let rows = store.list().await.expect("list succeeded");

        let ids: Vec<_> = rows.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
    }

    #[tokio::test]
    async fn list_handles_null_updated_at() {
        let store = open_in_memory();
        insert_row(&store, "flow", 0, r#"{"id":"flow","name":"Test","nodes":[]}"#, None).await;

        let rows = store.list().await.expect("list succeeded");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].updated_at, None);
    }

    #[tokio::test]
    async fn list_rejects_invalid_revision_i64() {
        let store = open_in_memory();
        let document = r#"{"id":"flow","name":"Test","nodes":[]}"#;
        // A negative revision, which u64::try_from will reject
        insert_row(&store, "flow", -1, document, Some(Utc::now().to_rfc3339())).await;

        let result = store.list().await;

        let Err(FlowStoreError::Sqlite(rusqlite::Error::FromSqlConversionFailure(1, _, err))) = &result else {
            panic!("got {result:?}");
        };
        assert!(matches!(err.downcast_ref(), Some(FlowStoreError::InvalidRevision(-1))), "got {err:?}");
    }

    #[tokio::test]
    async fn by_id_returns_the_stored_flow() {
        let store = open_in_memory();
        store.insert("flow", flow_document()).await.expect("insert");

        let stored_flow = store.by_id("flow").await.expect("by_id succeeded").expect("flow exists");

        assert_eq!(stored_flow.id, "flow");
        assert_eq!(stored_flow.revision, 0);
        assert_eq!(stored_flow.document, flow_document());
    }

    #[tokio::test]
    async fn by_id_fails_on_a_document_that_is_not_json() {
        let store = open_in_memory();
        insert_row(&store, "badFlow", 1, "not valid json", Some(Utc::now().to_rfc3339())).await;

        let result = store.by_id("badFlow").await;

        assert!(
            matches!(result, Err(FlowStoreError::Sqlite(rusqlite::Error::FromSqlConversionFailure(2, _, _)))),
            "got {result:?}"
        );
    }

    #[tokio::test]
    async fn by_id_returns_none_for_an_unknown_flow() {
        let store = open_in_memory();

        let result = store.by_id("flow").await.expect("by_id succeeded");

        assert!(result.is_none(), "got {result:?}");
    }

    #[tokio::test]
    async fn insert_stores_a_new_flow_at_revision_0() {
        let store = open_in_memory();
        let updated_document = json!({"id": "flow", "name": "Updated", "nodes": []});

        let revision = store.insert("flow", updated_document.clone()).await.expect("insert succeeded");

        assert_eq!(revision, 0);
        let rows = store.list().await.expect("list succeeded");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "flow");
        assert_eq!(rows[0].revision, 0);
        assert_eq!(rows[0].document, updated_document);
    }

    #[tokio::test]
    async fn insert_returns_already_exists_for_a_duplicate_id() {
        let store = open_in_memory();
        store.insert("flow", flow_document()).await.expect("first insert");

        let result = store.insert("flow", flow_document()).await;

        assert!(matches!(result, Err(InsertError::AlreadyExists)), "got {result:?}");
    }

    #[tokio::test]
    async fn update_bumps_the_revision_when_the_base_revision_matches() {
        let store = open_in_memory();
        store.insert("flow", flow_document()).await.expect("insert");

        let revision = store.update("flow", 0, flow_document()).await.expect("update succeeded");

        assert_eq!(revision, 1);
        let rows = store.list().await.expect("list succeeded");
        assert_eq!(rows[0].revision, 1);
    }

    #[tokio::test]
    async fn update_keeps_created_at_and_changes_updated_at() {
        let store = open_in_memory();
        store.insert("flow", flow_document()).await.expect("insert");
        execute(&store, "UPDATE flows SET created_at = '2000-01-01T00:00:00Z', updated_at = '2000-01-01T00:00:00Z'")
            .await;
        let long_ago = DateTime::parse_from_rfc3339("2000-01-01T00:00:00Z").unwrap().with_timezone(&Utc);

        store.update("flow", 0, flow_document()).await.expect("update succeeded");

        let stored_flow = store.by_id("flow").await.unwrap().expect("flow exists");
        assert_eq!(stored_flow.created_at, long_ago);
        assert!(stored_flow.updated_at.is_some_and(|updated_at| updated_at > long_ago), "got {stored_flow:?}");
    }

    #[tokio::test]
    async fn update_returns_revision_conflict_for_a_stale_base_revision() {
        let store = open_in_memory();
        store.insert("flow", flow_document()).await.expect("insert");
        store.update("flow", 0, flow_document()).await.expect("first update");

        let result = store.update("flow", 0, flow_document()).await;

        assert!(
            matches!(result, Err(UpdateError::RevisionConflict { base_revision: 0, current_revision: 1 })),
            "got {result:?}"
        );
        assert_eq!(store.list().await.unwrap()[0].revision, 1, "revision must be unchanged");
    }

    #[tokio::test]
    async fn update_returns_revision_conflict_for_a_future_base_revision() {
        let store = open_in_memory();
        store.insert("flow", flow_document()).await.expect("insert");

        let result = store.update("flow", 7, flow_document()).await;

        assert!(
            matches!(result, Err(UpdateError::RevisionConflict { base_revision: 7, current_revision: 0 })),
            "got {result:?}"
        );
    }

    #[tokio::test]
    async fn update_returns_not_found_for_an_unknown_flow() {
        let store = open_in_memory();

        let result = store.update("flow", 0, flow_document()).await;

        assert!(matches!(result, Err(UpdateError::NotFound)), "got {result:?}");
        assert!(store.list().await.unwrap().is_empty(), "update must not insert");
    }

    #[tokio::test]
    async fn delete_removes_the_flow_when_the_base_revision_matches() {
        let store = open_in_memory();
        store.insert("flow", flow_document()).await.expect("insert");
        store.update("flow", 0, flow_document()).await.expect("update");

        store.delete("flow", 1).await.expect("delete succeeded");

        assert!(store.by_id("flow").await.unwrap().is_none(), "flow must be deleted");
    }

    #[tokio::test]
    async fn delete_returns_revision_conflict_for_a_stale_base_revision() {
        let store = open_in_memory();
        store.insert("flow", flow_document()).await.expect("insert");
        store.update("flow", 0, flow_document()).await.expect("update");

        let result = store.delete("flow", 0).await;

        assert!(
            matches!(result, Err(DeleteError::RevisionConflict { base_revision: 0, current_revision: 1 })),
            "got {result:?}"
        );
        assert!(store.by_id("flow").await.unwrap().is_some(), "flow must not be deleted");
    }

    #[tokio::test]
    async fn delete_returns_revision_conflict_for_a_future_base_revision() {
        let store = open_in_memory();
        store.insert("flow", flow_document()).await.expect("insert");

        let result = store.delete("flow", 7).await;

        assert!(
            matches!(result, Err(DeleteError::RevisionConflict { base_revision: 7, current_revision: 0 })),
            "got {result:?}"
        );
        assert!(store.by_id("flow").await.unwrap().is_some(), "flow must not be deleted");
    }

    #[tokio::test]
    async fn delete_returns_not_found_for_an_unknown_flow() {
        let store = open_in_memory();

        let result = store.delete("flow", 0).await;

        assert!(matches!(result, Err(DeleteError::NotFound)), "got {result:?}");
    }

    #[tokio::test]
    async fn delete_only_removes_the_given_flow() {
        let store = open_in_memory();
        store.insert("flow", flow_document()).await.expect("insert");
        store.insert("other", flow_document()).await.expect("insert");

        store.delete("flow", 0).await.expect("delete succeeded");

        let rows = store.list().await.expect("list succeeded");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "other");
    }
}
