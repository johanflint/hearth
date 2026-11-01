use crate::database::DatabaseError::UnsupportedSchemaVersion;
use rusqlite::Connection;
use std::path::Path;
use std::sync::{Arc, Mutex};
use thiserror::Error;
use tokio::task;
use tracing::{debug, info};

#[derive(Clone, Debug)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self, DatabaseError> {
        let mut conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", &"WAL")?;
        conn.pragma_update(None, "foreign_keys", &"ON")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        run_migrations(&mut conn)?;

        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

    pub async fn ping(&self) -> Result<(), DatabaseError> {
        self.call(|conn| conn.query_row("SELECT 1", [], |_| Ok(())).map_err(DatabaseError::from)).await
    }

    pub async fn call<T, E>(&self, f: impl FnOnce(&mut Connection) -> Result<T, E> + Send + 'static) -> Result<T, E>
    where
        T: Send + 'static,
        E: From<DatabaseError> + Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        task::spawn_blocking(move || {
            let mut conn = conn.lock().map_err(|_| DatabaseError::PoisonedConnection)?;
            f(&mut conn)
        })
        .await
        .map_err(DatabaseError::from)?
    }
}

fn run_migrations(conn: &mut Connection) -> Result<(), DatabaseError> {
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

const MIGRATIONS: &[&str] = &[
    // v1: initial schema
    "CREATE TABLE flows (
        id TEXT PRIMARY KEY,
        revision INTEGER NOT NULL,
        document TEXT NOT NULL, -- SerializedFlow as JSON
        created_at TEXT NOT NULL,
        updated_at TEXT
    )",
    // v2: presets
    "CREATE TABLE presets (
        id TEXT PRIMARY KEY,
        revision INTEGER NOT NULL,
        name TEXT NOT NULL UNIQUE COLLATE NOCASE,
        value TEXT NOT NULL, -- domain::Value as JSON, same format as flow literals
        created_at TEXT NOT NULL,
        updated_at TEXT
    )",
];

#[derive(Error, Debug)]
pub enum DatabaseError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("database worker task failed: {0}")]
    Join(#[from] task::JoinError),
    #[error("database connection was poisoned")]
    PoisonedConnection,
    #[error("database schema {found} is newer than the {supported} version(s) supported")]
    UnsupportedSchemaVersion { found: usize, supported: usize },
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[derive(Debug)]
    enum TestError {
        Closure,
        Database(DatabaseError),
    }

    impl From<DatabaseError> for TestError {
        fn from(err: DatabaseError) -> Self {
            Self::Database(err)
        }
    }

    fn open_in_memory() -> Database {
        Database::open(Path::new(":memory:")).expect("failed to open in-memory database")
    }

    // A named in-memory database that is shared between connections and lives as long as one of them is open
    fn shared_in_memory_path(name: &str) -> String {
        format!("file:{name}?mode=memory&cache=shared")
    }

    fn user_version(conn: &Connection) -> i64 {
        conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn open_creates_and_migrates_a_fresh_database() {
        let database = open_in_memory();

        let conn = database.conn.lock().unwrap();

        assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);
    }

    #[rstest]
    #[case::foreign_keys("foreign_keys", 1)]
    #[case::busy_timeout("busy_timeout", 5000)]
    fn open_configures_the_connection(#[case] pragma: &str, #[case] expected: i64) {
        let database = open_in_memory();

        let conn = database.conn.lock().unwrap();
        let value: i64 = conn.query_row(&format!("PRAGMA {pragma}"), [], |r| r.get(0)).unwrap();

        assert_eq!(value, expected);
    }

    #[test]
    fn open_skips_migrations_that_already_ran() {
        let path = shared_in_memory_path("open_skips_migrations_that_already_ran");
        let first = Database::open(Path::new(&path)).expect("first open");
        let first_conn = first.conn.lock().unwrap();
        first_conn
            .execute("INSERT INTO flows (id, revision, document, created_at) VALUES ('flow', 0, '{}', 'now')", [])
            .unwrap();

        // Rerunning v1 would fail, because the flows table already exists
        let second = Database::open(Path::new(&path)).expect("second open");

        let conn = second.conn.lock().unwrap();
        assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM flows", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 1, "existing data must be kept");
    }

    #[test]
    fn open_rejects_a_schema_that_is_newer_than_supported() {
        let path = shared_in_memory_path("open_rejects_a_schema_that_is_newer_than_supported");
        let newer_version = MIGRATIONS.len() + 1;
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", newer_version as i64).unwrap();

        let result = Database::open(Path::new(&path));

        assert!(
            matches!(result, Err(UnsupportedSchemaVersion { found, supported }) if found == newer_version && supported == MIGRATIONS.len()),
            "got {result:?}"
        );
        assert_eq!(user_version(&conn), newer_version as i64, "user_version must be unchanged");
    }

    #[tokio::test]
    async fn call_returns_the_value_of_the_closure() {
        let database = open_in_memory();

        let result = database.call(|_| Ok::<_, TestError>(42)).await;

        assert!(matches!(result, Ok(42)), "got {result:?}");
    }

    #[tokio::test]
    async fn call_returns_the_error_of_the_closure() {
        let database = open_in_memory();

        let result = database.call(|_| Err::<(), _>(TestError::Closure)).await;

        assert!(matches!(result, Err(TestError::Closure)), "got {result:?}");
    }

    #[tokio::test]
    async fn call_returns_a_join_error_when_the_closure_panics() {
        let database = open_in_memory();

        let result = database.call(|_| -> Result<(), TestError> { panic!("closure panicked") }).await;

        assert!(matches!(result, Err(TestError::Database(DatabaseError::Join(_)))), "got {result:?}");
    }

    #[tokio::test]
    async fn ping_succeeds_on_an_open_database() {
        let database = open_in_memory();

        let result = database.ping().await;

        assert!(result.is_ok(), "got {result:?}");
    }

    #[tokio::test]
    async fn ping_returns_poisoned_connection_after_a_closure_panicked() {
        let database = open_in_memory();
        let _ = database.call(|_| -> Result<(), TestError> { panic!("closure panicked") }).await;

        let result = database.ping().await;

        assert!(matches!(result, Err(DatabaseError::PoisonedConnection)), "got {result:?}");
    }

    #[tokio::test]
    async fn call_returns_poisoned_connection_after_a_closure_panicked() {
        let database = open_in_memory();
        let _ = database.call(|_| -> Result<(), TestError> { panic!("closure panicked") }).await;

        let result = database.call(|_| Ok::<(), TestError>(())).await;

        assert!(matches!(result, Err(TestError::Database(DatabaseError::PoisonedConnection))), "got {result:?}");
    }
}
