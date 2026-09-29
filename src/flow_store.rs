use crate::flow_engine::flow::Flow;
use crate::flow_loader::{FlowFactoryError, from_json_string};
use crate::flow_store::FlowStoreError::UnsupportedSchemaVersion;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, Row, params};
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

    pub async fn upsert(&self, id: &str, serialized_flow_json: &str) -> Result<StoredFlow, FlowStoreError> {
        let conn = Arc::clone(&self.conn);
        let id = id.to_string();
        let serialized_flow_json = serialized_flow_json.to_string();
        task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|_| FlowStoreError::PoisonedConnection)?;
            let now = Utc::now().to_rfc3339();
            conn.query_row(
                "INSERT INTO flows (id, revision, document, created_at, updated_at)
                VALUES (?1, 0, ?2, ?3, ?3)
                ON CONFLICT(id) DO UPDATE SET
                    revision = revision + 1,
                    document = excluded.document,
                    updated_at = excluded.updated_at
                RETURNING id, revision, document, created_at, updated_at",
                params![id, serialized_flow_json, now],
                row_to_stored_flow,
            ).map_err(FlowStoreError::from)
        }).await?
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct StoredFlow {
    pub id: String,
    pub revision: u64,
    pub flow: Result<Flow, FlowFactoryError>,
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
    let revision: i64 = row.get(1)?;
    let revision = u64::try_from(revision)
        .map_err(|_| rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Integer, Box::new(FlowStoreError::InvalidRevision(revision))))?;

    let document_json: String = row.get(2)?;
    let flow = from_json_string(&document_json);

    let created_at: String = row.get(3)?;
    let created_at = DateTime::parse_from_rfc3339(&created_at)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(err)))?;

    let updated_at: Option<String> = row.get(4)?;
    let updated_at = updated_at.map(|updated_at| {
        DateTime::parse_from_rfc3339(&updated_at)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|err| rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(err)))
    }).transpose()?;

    Ok(StoredFlow { id, revision, flow, created_at, updated_at })
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
    async fn list_skips_rows_with_invalid_document_and_continues() {
        let store = open_in_memory();
        let conn = store.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO flows (id, revision, document, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!("badFlow", "1", "not valid json", &now, &now),
        ).expect("insert bad");
        conn.execute(
            "INSERT INTO flows (id, revision, document, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!("good-flow", "0", r#"{"id":"good-flow","name":"Good","nodes":[{"id":"startNode","type":"startNode","outgoingNode":"endNode"},{"id":"endNode","type":"endNode"}]}"#, &now, &now),
        ).expect("insert good");
        drop(conn);

        let rows = store.list().await.expect("list succeeded");
        assert_eq!(rows.len(), 2);
        // Both rows are returned; caller checks flow.is_ok()
        assert!(rows[0].flow.is_err());
        assert!(rows[1].flow.is_ok());
    }

    #[tokio::test]
    async fn list_handles_null_updated_at() {
        let store = open_in_memory();
        let conn = store.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO flows (id, revision, document, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!("flow", "0", r#"{"id":"flow","name":"Test","nodes":[]}"#, &now, None::<String>),
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
    async fn upsert_inserts_a_new_flow() {
        let store = open_in_memory();
        let json = r#"{"id":"flow","name":"Test","nodes":[{"id":"startNode","type":"startNode","outgoingNode":"endNode"},{"id":"endNode","type":"endNode"}]}"#;

        let result = store.upsert("flow", json).await.expect("upsert succeeded");
        assert_eq!(result.id, "flow");
        assert_eq!(result.revision, 0);
    }

    #[tokio::test]
    async fn upsert_updates_existing_flow_and_bumps_revision() {
        let store = open_in_memory();
        let json = r#"{"id":"flow","name":"Test","nodes":[{"id":"startNode","type":"startNode","outgoingNode":"endNode"},{"id":"endNode","type":"endNode"}]}"#;

        store.upsert("flow", json).await.expect("first upsert");
        let result = store.upsert("flow", json).await.expect("second upsert");
        assert_eq!(result.revision, 1);
    }
}
