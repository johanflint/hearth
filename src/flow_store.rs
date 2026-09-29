use crate::flow_store::FlowStoreError::UnsupportedSchemaVersion;
use rusqlite::Connection;
use std::path::Path;
use std::sync::{Arc, Mutex};
use thiserror::Error;
use tracing::{debug, info};

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

#[derive(Error, Debug)]
pub enum FlowStoreError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
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
