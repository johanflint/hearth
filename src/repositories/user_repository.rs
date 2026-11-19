use crate::database::{Database, DatabaseError};
use crate::domain::Permission;
use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, params};
use std::collections::BTreeSet;
use std::str::FromStr;
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

    pub async fn by_name(&self, name: &str) -> Result<Option<StoredUser>, UserRepositoryError> {
        let name = name.to_string();
        self.database
            .call(move |conn| {
                let row = conn
                    .query_row("SELECT id, name, created_at FROM users WHERE name = ?1", [name], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
                    })
                    .optional()?;
                let Some((id, name, created_at)) = row else {
                    return Ok(None);
                };

                let mut statement = conn.prepare("SELECT permission FROM user_permissions WHERE user_id = ?1")?;
                let permissions = statement
                    .query_map([&id], |row| {
                        let value: String = row.get(0)?;
                        Permission::from_str(&value).map_err(|err| {
                            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(err))
                        })
                    })?
                    .collect::<Result<BTreeSet<_>, _>>()?;
                let created_at =
                    DateTime::parse_from_rfc3339(&created_at).map(|timestamp| timestamp.with_timezone(&Utc)).map_err(
                        |err| rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(err)),
                    )?;

                Ok::<_, UserRepositoryError>(Some(StoredUser { id, name, permissions, created_at }))
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

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use std::path::Path;

    fn open_in_memory() -> (Database, UserRepository) {
        let database = Database::open(Path::new(":memory:")).expect("failed to open in-memory database");
        (database.clone(), UserRepository::new(database))
    }

    async fn count_permissions(database: &Database) -> i64 {
        database
            .call(|conn| {
                conn.query_row("SELECT COUNT(*) FROM user_permissions", [], |row| row.get(0))
                    .map_err(DatabaseError::from)
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn insert_returns_the_stored_user() {
        let (_, repository) = open_in_memory();
        let permissions = || BTreeSet::from([Permission::ReadDevices, Permission::WriteFlows]);

        let user = repository.insert("user-1", "johan", permissions()).await.unwrap();

        assert_eq!(user.id, "user-1");
        assert_eq!(user.name, "johan");
        assert_eq!(user.permissions, permissions());
    }

    #[rstest]
    #[case::same_case("johan")]
    #[case::other_case("JOHAN")]
    #[tokio::test]
    async fn insert_rejects_a_duplicate_name(#[case] name: &str) {
        let (_, repository) = open_in_memory();
        repository.insert("user-1", "johan", BTreeSet::new()).await.unwrap();

        let result = repository.insert("user-2", name, BTreeSet::new()).await;

        assert!(matches!(result, Err(UserInsertError::NameAlreadyExists)), "got {result:?}");
    }

    #[tokio::test]
    async fn a_rejected_insert_leaves_the_existing_user_untouched() {
        let (database, repository) = open_in_memory();
        repository.insert("user-1", "johan", BTreeSet::from([Permission::ReadDevices])).await.unwrap();

        let _ = repository.insert("user-2", "johan", BTreeSet::from([Permission::WriteFlows])).await;

        let user = repository.by_name("johan").await.unwrap().unwrap();
        assert_eq!(user.permissions, BTreeSet::from([Permission::ReadDevices]));
        assert_eq!(count_permissions(&database).await, 1);
    }

    #[tokio::test]
    async fn by_name_returns_the_user_with_its_permissions() {
        let (_, repository) = open_in_memory();
        let permissions =
            || BTreeSet::from([Permission::ReadLayout, Permission::WriteLayout, Permission::ReadAttention]);
        let inserted = repository.insert("user-1", "johan", permissions()).await.unwrap();

        let user = repository.by_name("johan").await.unwrap().unwrap();

        assert_eq!(user.id, "user-1");
        assert_eq!(user.name, "johan");
        assert_eq!(user.permissions, permissions());
        assert_eq!(user.created_at, inserted.created_at);
    }

    #[tokio::test]
    async fn by_name_returns_a_user_without_permissions() {
        let (_, repository) = open_in_memory();
        repository.insert("user-1", "johan", BTreeSet::new()).await.unwrap();

        let user = repository.by_name("johan").await.unwrap().unwrap();

        assert!(user.permissions.is_empty());
    }

    #[tokio::test]
    async fn by_name_ignores_the_case_of_the_name() {
        let (_, repository) = open_in_memory();
        repository.insert("user-1", "Johan", BTreeSet::new()).await.unwrap();

        let user = repository.by_name("jOHAN").await.unwrap().unwrap();

        // The stored spelling is kept
        assert_eq!(user.name, "Johan");
    }

    #[rstest]
    #[case::fresh_database(&[])]
    #[case::other_users(&["anna"])]
    #[tokio::test]
    async fn by_name_returns_none_for_an_unknown_name(#[case] existing: &[&str]) {
        let (_, repository) = open_in_memory();
        for (index, name) in existing.iter().enumerate() {
            repository.insert(&format!("user-{index}"), name, BTreeSet::new()).await.unwrap();
        }

        let user = repository.by_name("johan").await.unwrap();

        assert!(user.is_none());
    }

    #[tokio::test]
    async fn by_name_fails_on_an_unknown_stored_permission() {
        let (database, repository) = open_in_memory();
        repository.insert("user-1", "johan", BTreeSet::new()).await.unwrap();
        database
            .call(|conn| {
                conn.execute(
                    "INSERT INTO user_permissions (user_id, permission) VALUES ('user-1', 'deleteEverything')",
                    [],
                )
                .map_err(DatabaseError::from)
            })
            .await
            .unwrap();

        let result = repository.by_name("johan").await;

        assert!(matches!(result, Err(UserRepositoryError::Sqlite(_))), "got {result:?}");
    }

    #[tokio::test]
    async fn by_name_fails_on_an_invalid_timestamp() {
        let (database, repository) = open_in_memory();
        repository.insert("user-1", "johan", BTreeSet::new()).await.unwrap();
        database
            .call(|conn| conn.execute("UPDATE users SET created_at = 'yesterday'", []).map_err(DatabaseError::from))
            .await
            .unwrap();

        let result = repository.by_name("johan").await;

        assert!(matches!(result, Err(UserRepositoryError::Sqlite(_))), "got {result:?}");
    }

    #[tokio::test]
    async fn deleting_a_user_deletes_its_permissions() {
        let (database, repository) = open_in_memory();
        repository.insert("user-1", "johan", BTreeSet::from([Permission::ReadDevices])).await.unwrap();

        database.call(|conn| conn.execute("DELETE FROM users", []).map_err(DatabaseError::from)).await.unwrap();

        assert_eq!(count_permissions(&database).await, 0);
    }
}
