use crate::database::{Database, DatabaseError};
use crate::domain::Permission;
use chrono::{DateTime, Utc};
use rusqlite::params;
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Debug)]
pub struct UserRepository {
    database: Database,
}

impl UserRepository {
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    pub async fn insert(
        &self,
        id: &str,
        name: &str,
        permissions: BTreeSet<Permission>,
    ) -> Result<StoredUser, UserInsertError> {
        let id = id.to_string();
        let name = name.to_string();
        self.database
            .call(move |conn| {
                let created_at = Utc::now();
                // The user and its permissions are written together or not at all
                let tx = conn.transaction()?;
                tx.execute(
                    "INSERT INTO users (id, name, created_at) VALUES (?1, ?2, ?3)",
                    params![id, name, created_at.to_rfc3339()],
                )
                .map_err(|err| if is_name_conflict(&err) { UserInsertError::NameAlreadyExists } else { err.into() })?;
                {
                    let mut statement =
                        tx.prepare("INSERT INTO user_permissions (user_id, permission) VALUES (?1, ?2)")?;
                    for permission in &permissions {
                        statement.execute(params![id, permission.to_string()])?;
                    }
                }
                tx.commit()?;

                Ok(StoredUser { id, name, permissions, created_at })
            })
            .await
    }
}

fn is_name_conflict(err: &rusqlite::Error) -> bool {
    matches!(err, rusqlite::Error::SqliteFailure(err, _) if err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE)
}

#[derive(Debug)]
pub struct StoredUser {
    pub id: String,
    pub name: String,
    pub permissions: BTreeSet<Permission>,
    pub created_at: DateTime<Utc>,
}

#[derive(Error, Debug)]
pub enum UserRepositoryError {
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

#[derive(Error, Debug)]
pub enum UserInsertError {
    #[error("a user with this name already exists")]
    NameAlreadyExists,
    #[error(transparent)]
    Repository(#[from] UserRepositoryError),
}

// `?` applies a single `From`, so `DatabaseError` and `rusqlite::Error` can't reach `Repository` via `UserRepositoryError` on their own
impl From<DatabaseError> for UserInsertError {
    fn from(err: DatabaseError) -> Self {
        Self::Repository(err.into())
    }
}

impl From<rusqlite::Error> for UserInsertError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Repository(err.into())
    }
}
