use crate::flow_store::FlowStoreError::UnsupportedSchemaVersion;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params};
use std::path::Path;
use std::sync::{Arc, Mutex};
use thiserror::Error;
use tokio::task;
use tracing::{debug, info};

#[derive(Debug)]
pub struct FlowStore {
    conn: Arc<Mutex<Connection>>,
}

impl FlowStore {
    pub fn open(path: &Path) -> Result<Self, FlowStoreError> {
        let mut conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", &"WAL")?;
        conn.pragma_update(None, "foreign_keys", &"ON")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        run_migrations(&mut conn)?;

        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

    pub async fn list(&self) -> Result<Vec<StoredFlow>, FlowStoreError> {
        let conn = Arc::clone(&self.conn);
        task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|_| FlowStoreError::PoisonedConnection)?;
            let mut statement = conn.prepare("SELECT id, revision, document, created_at, updated_at FROM flows ORDER BY id")?;
            let rows = statement.query_map([], row_to_stored_flow)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(FlowStoreError::from)
        }).await?
    }

    pub async fn insert(&self, id: &str, document: serde_json::Value) -> Result<u64, InsertError> {
        let conn = Arc::clone(&self.conn);
        let id = id.to_string();
        task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|_| FlowStoreError::PoisonedConnection)?;
            let now = Utc::now().to_rfc3339();
            let result = conn.execute(
                "INSERT INTO flows (id, revision, document, created_at, updated_at) VALUES (?1, 0, ?2, ?3, ?3)",
                params![id, document, now],
            );

            match result {
                Ok(_) => Ok(0),
                Err(rusqlite::Error::SqliteFailure(err, _)) if err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY => Err(InsertError::AlreadyExists),
                Err(err) => Err(err.into()),
            }
        }).await.map_err(FlowStoreError::from)? // Handle the join JoinError
    }

    pub async fn update(&self, id: &str, base_revision: u64, document: serde_json::Value) -> Result<u64, UpdateError> {
        let conn = Arc::clone(&self.conn);
        let id = id.to_string();
        task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|_| FlowStoreError::PoisonedConnection)?;
            let now = Utc::now().to_rfc3339();
            let updated: Option<i64> = conn.query_row(
                "UPDATE flows SET document = ?3, revision = revision + 1, updated_at = ?4
                WHERE id = ?1 AND revision = ?2
                RETURNING revision",
                params![id, base_revision as i64, document, now],
                |row| row.get(0),
            ).optional()?;

            if let Some(revision) = updated {
                return Ok(to_revision(revision)?);
            }

            // No row matched; either the flow doesn't exist or someone else updated it first
            let current: Option<i64> = conn.query_row("SELECT revision FROM flows WHERE id = ?1", params![id], |row| row.get(0)).optional()?;
            match current {
                None => Err(UpdateError::NotFound),
                Some(current) => Err(UpdateError::RevisionConflict { base_revision, current_revision: to_revision(current)? }),
            }
        }).await.map_err(FlowStoreError::from)? // Handle the join JoinError
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct StoredFlow {
    pub id: String,
    pub revision: u64,
    pub document: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
}

fn run_migrations(conn: &mut Connection) -> Result<(), FlowStoreError> {
    let current_version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let current_version = current_version as usize;

    if current_version > MIGRATIONS.len() {
        return Err(UnsupportedSchemaVersion { found: current_version, supported: MIGRATIONS.len() });
    }

    debug!("Running migrations... current version {}, target version {}", current_version, MIGRATIONS.len());
    for (index, migration) in MIGRATIONS.iter().enumerate().skip(current_version) {
        let tx = conn.transaction()?;
        tx.execute_batch(migration)?;
        tx.pragma_update(None, "user_version", (index + 1) as i64)?;
        tx.commit()?;
    }
    info!("Running migrations... OK, {} executed", MIGRATIONS.len() - current_version);

    Ok(())
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
    let updated_at = updated_at.map(|updated_at| {
        DateTime::parse_from_rfc3339(&updated_at)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|err| rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(err)))
    }).transpose()?;

    Ok(StoredFlow { id, revision, document, created_at, updated_at })
}

fn to_revision(revision: i64) -> Result<u64, FlowStoreError> {
    u64::try_from(revision).map_err(|_| FlowStoreError::InvalidRevision(revision))
}

#[derive(Error, Debug)]
pub enum FlowStoreError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("flow store worker task failed: {0}")]
    Join(#[from] task::JoinError),
    #[error("flow store connection was poisoned")]
    PoisonedConnection,
    #[error("stored revision {0} is out of range for u64")]
    InvalidRevision(i64),
    #[error("database schema {found} is newer than the {supported} version(s) supported")]
    UnsupportedSchemaVersion { found: usize, supported: usize },

}

#[derive(Error, Debug)]
pub enum InsertError {
    #[error("flow already exists")]
    AlreadyExists,
    #[error(transparent)]
    Store(#[from] FlowStoreError),
}

// `?` applies a single `From`, so `rusqlite::Error` can't reach `Store` via `FlowStoreError` on its own
impl From<rusqlite::Error> for InsertError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Store(err.into())
    }
}

#[derive(Error, Debug)]
pub enum UpdateError {
    #[error("flow not found")]
    NotFound,
    #[error("revision conflict: update is based on revision {base_revision}, but the current revision is {current_revision}")]
    RevisionConflict { base_revision: u64, current_revision: u64 },
    #[error(transparent)]
    Store(#[from] FlowStoreError),
}

// `?` applies a single `From`, so `rusqlite::Error` can't reach `Store` via `FlowStoreError` on its own
impl From<rusqlite::Error> for UpdateError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Store(err.into())
    }
}

const MIGRATIONS: &[&str] = &[
    // v1: initial schema
    "CREATE TABLE flows (
        id TEXT PRIMARY KEY,
        revision INTEGER NOT NULL,
        document TEXT NOT NULL, -- SerializedFlow as JSON
        created_at TEXT NOT NULL,
        updated_at TEXT
    )",
];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn flow_document() -> serde_json::Value {
        json!({"id":"flow","name":"Test","nodes":[{"id":"startNode","type":"startNode","outgoingNode":"endNode"},{"id":"endNode","type":"endNode"}]})
    }

    fn open_in_memory() -> FlowStore {
        FlowStore::open(Path::new(":memory:")).expect("failed to open in-memory flow store")
    }

    #[test]
    fn open_creates_and_migrates_a_fresh_database() {
        let store = open_in_memory();
        // If migration failed, open would have returned Err
        assert!(store.conn.lock().is_ok());

        let conn = store.conn.lock().unwrap();
        let current_version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();

        assert_eq!(current_version, MIGRATIONS.len() as i64);
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
        let conn = store.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO flows (id, revision, document, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            [
                "1d3a9341-e462-4663-bb3d-5d977053cc0f",
                "42",
                r#"{"id":"1d3a9341-e462-4663-bb3d-5d977053cc0f","name":"Test","nodes":[],"triggers":[]}"#,
                &now,
                &now,
            ],
        ).expect("insert");
        drop(conn);

        let rows = store.list().await.expect("list succeeded");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "1d3a9341-e462-4663-bb3d-5d977053cc0f");
        assert_eq!(rows[0].revision, 42);
        // flow parse might fail if document is incomplete, but revision is preserved
    }

    #[tokio::test]
    async fn list_fails_on_a_document_that_is_not_json() {
        let store = open_in_memory();
        let conn = store.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO flows (id, revision, document, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params!("badFlow", "1", "not valid json", &now, &now),
        ).expect("insert bad");
        drop(conn);

        let result = store.list().await;

        // Invalid JSON means the database is corrupt
        assert!(matches!(result, Err(FlowStoreError::Sqlite(rusqlite::Error::FromSqlConversionFailure(2, _, _)))), "got {result:?}");
    }

    #[tokio::test]
    async fn list_handles_null_updated_at() {
        let store = open_in_memory();
        let conn = store.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO flows (id, revision, document, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params!("flow", "0", r#"{"id":"flow","name":"Test","nodes":[]}"#, &now, None::<String>),
        ).expect("insert");
        drop(conn);

        let rows = store.list().await.expect("list succeeded");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].updated_at, None);
    }

    #[tokio::test]
    async fn list_rejects_invalid_revision_i64() {
        let store = open_in_memory();
        let conn = store.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();
        // Insert with a negative revision (which u64::try_from will reject)
        conn.execute(
            "INSERT INTO flows (id, revision, document, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            ["flow", "-1", r#"{"id":"flow","name":"Test","nodes":[]}"#, &now, &now],
        ).expect("insert");
        drop(conn);

        let result = store.list().await;
        // Should fail with InvalidRevision error
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), FlowStoreError::Sqlite(_)));
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
        assert!(rows[0].updated_at.is_some());
    }

    #[tokio::test]
    async fn update_returns_revision_conflict_for_a_stale_base_revision() {
        let store = open_in_memory();
        store.insert("flow", flow_document()).await.expect("insert");
        store.update("flow", 0, flow_document()).await.expect("first update");

        let result = store.update("flow", 0, flow_document()).await;

        assert!(matches!(result, Err(UpdateError::RevisionConflict { base_revision: 0, current_revision: 1 })), "got {result:?}");
        assert_eq!(store.list().await.unwrap()[0].revision, 1, "revision must be unchanged");
    }

    #[tokio::test]
    async fn update_returns_revision_conflict_for_a_future_base_revision() {
        let store = open_in_memory();
        store.insert("flow", flow_document()).await.expect("insert");

        let result = store.update("flow", 7, flow_document()).await;

        assert!(matches!(result, Err(UpdateError::RevisionConflict { base_revision: 7, current_revision: 0 })), "got {result:?}");
    }

    #[tokio::test]
    async fn update_returns_not_found_for_an_unknown_flow() {
        let store = open_in_memory();

        let result = store.update("flow", 0, flow_document()).await;

        assert!(matches!(result, Err(UpdateError::NotFound)), "got {result:?}");
        assert!(store.list().await.unwrap().is_empty(), "update must not insert");
    }
}
