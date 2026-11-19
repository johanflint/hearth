use crate::database::{Database, DatabaseError};
use crate::domain::Token;
use chrono::{DateTime, Utc};
use rusqlite::params;
use thiserror::Error;

#[derive(Debug)]
pub struct TokenRepository {
    database: Database,
}

impl TokenRepository {
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    pub async fn insert(
        &self,
        id: &str,
        user_id: &str,
        name: &str,
        token: &Token,
    ) -> Result<StoredToken, TokenInsertError> {
        let id = id.to_string();
        let user_id = user_id.to_string();
        let name = name.to_string();
        let hash = token.hash();
        self.database
            .call(move |conn| {
                let created_at = Utc::now();
                conn.execute(
                    "INSERT INTO api_tokens (id, user_id, name, token_hash, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![id, user_id, name, hash, created_at.to_rfc3339()],
                )
                .map_err(|err| if is_name_conflict(&err) { TokenInsertError::NameAlreadyExists } else { err.into() })?;

                Ok(StoredToken { id, user_id, name, created_at, last_used_at: None })
            })
            .await
    }
}

fn is_name_conflict(err: &rusqlite::Error) -> bool {
    matches!(err, rusqlite::Error::SqliteFailure(err, _) if err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE)
}

/// Never contains the token or its hash
#[derive(Debug)]
pub struct StoredToken {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(Error, Debug)]
pub enum TokenRepositoryError {
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

#[derive(Error, Debug)]
pub enum TokenInsertError {
    #[error("a token with this name already exists for this user")]
    NameAlreadyExists,
    #[error(transparent)]
    Repository(#[from] TokenRepositoryError),
}

impl From<DatabaseError> for TokenInsertError {
    fn from(err: DatabaseError) -> Self {
        Self::Repository(err.into())
    }
}

impl From<rusqlite::Error> for TokenInsertError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Repository(err.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repositories::UserRepository;
    use rstest::rstest;
    use std::collections::BTreeSet;
    use std::path::Path;

    async fn open_with_users(names: &[&str]) -> (Database, TokenRepository) {
        let database = Database::open(Path::new(":memory:")).expect("failed to open in-memory database");
        let users = UserRepository::new(database.clone());
        for name in names {
            users.insert(&format!("{name}-id"), name, BTreeSet::new()).await.unwrap();
        }
        (database.clone(), TokenRepository::new(database))
    }

    async fn stored_hashes(database: &Database) -> Vec<String> {
        database
            .call(|conn| {
                let mut statement = conn.prepare("SELECT token_hash FROM api_tokens")?;
                let rows = statement.query_map([], |row| row.get(0))?;
                rows.collect::<Result<Vec<String>, _>>().map_err(DatabaseError::from)
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn insert_returns_the_stored_token() {
        let (_, repository) = open_with_users(&["johan"]).await;

        let stored = repository.insert("token-1", "johan-id", "Kitchen tablet", &Token::generate()).await.unwrap();

        assert_eq!(stored.id, "token-1");
        assert_eq!(stored.user_id, "johan-id");
        assert_eq!(stored.name, "Kitchen tablet");
        assert!(stored.last_used_at.is_none());
    }

    #[tokio::test]
    async fn insert_stores_the_hash_of_the_token() {
        let (database, repository) = open_with_users(&["johan"]).await;
        let token = Token::generate();

        repository.insert("token-1", "johan-id", "Kitchen tablet", &token).await.unwrap();

        assert_eq!(stored_hashes(&database).await, vec![token.hash()]);
    }

    #[rstest]
    #[case::same_case("Kitchen tablet")]
    #[case::other_case("KITCHEN TABLET")]
    #[tokio::test]
    async fn insert_rejects_a_duplicate_name_for_the_same_user(#[case] name: &str) {
        let (database, repository) = open_with_users(&["johan"]).await;
        repository.insert("token-1", "johan-id", "Kitchen tablet", &Token::generate()).await.unwrap();

        let result = repository.insert("token-2", "johan-id", name, &Token::generate()).await;

        assert!(matches!(result, Err(TokenInsertError::NameAlreadyExists)), "got {result:?}");
        assert_eq!(stored_hashes(&database).await.len(), 1);
    }

    #[tokio::test]
    async fn insert_allows_the_same_name_for_different_users() {
        let (database, repository) = open_with_users(&["johan", "anna"]).await;
        repository.insert("token-1", "johan-id", "Phone", &Token::generate()).await.unwrap();

        let result = repository.insert("token-2", "anna-id", "Phone", &Token::generate()).await;

        assert!(result.is_ok(), "got {result:?}");
        assert_eq!(stored_hashes(&database).await.len(), 2);
    }

    #[tokio::test]
    async fn insert_fails_for_an_unknown_user() {
        let (database, repository) = open_with_users(&[]).await;

        let result = repository.insert("token-1", "nobody-id", "Phone", &Token::generate()).await;

        assert!(matches!(result, Err(TokenInsertError::Repository(_))), "got {result:?}");
        assert!(stored_hashes(&database).await.is_empty());
    }

    #[tokio::test]
    async fn deleting_a_user_deletes_its_tokens() {
        let (database, repository) = open_with_users(&["johan", "anna"]).await;
        repository.insert("token-1", "johan-id", "Phone", &Token::generate()).await.unwrap();
        let other = Token::generate();
        repository.insert("token-2", "anna-id", "Phone", &other).await.unwrap();

        database
            .call(|conn| conn.execute("DELETE FROM users WHERE id = 'johan-id'", []).map_err(DatabaseError::from))
            .await
            .unwrap();

        assert_eq!(stored_hashes(&database).await, vec![other.hash()]);
    }
}
