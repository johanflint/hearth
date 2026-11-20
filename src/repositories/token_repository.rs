use crate::database::{Database, DatabaseError};
use crate::domain::Token;
use chrono::{DateTime, Utc};
use rusqlite::{Row, params};
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

    pub async fn list_by_user(&self, user_id: &str) -> Result<Vec<StoredToken>, TokenRepositoryError> {
        let user_id = user_id.to_string();
        self.database
            .call(move |conn| {
                let mut statement = conn.prepare(
                    "SELECT id, user_id, name, created_at, last_used_at FROM api_tokens WHERE user_id = ?1 ORDER BY name",
                )?;
                let rows = statement.query_map([user_id], row_to_stored_token)?;
                Ok(rows.collect::<Result<Vec<_>, _>>()?)
            })
            .await
    }

    pub async fn delete(&self, user_id: &str, name: &str) -> Result<bool, TokenRepositoryError> {
        let user_id = user_id.to_string();
        let name = name.to_string();
        self.database
            .call(move |conn| {
                let deleted =
                    conn.execute("DELETE FROM api_tokens WHERE user_id = ?1 AND name = ?2", params![user_id, name])?;
                Ok::<_, TokenRepositoryError>(deleted > 0)
            })
            .await
    }
}

fn is_name_conflict(err: &rusqlite::Error) -> bool {
    matches!(err, rusqlite::Error::SqliteFailure(err, _) if err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE)
}

fn row_to_stored_token(row: &Row) -> rusqlite::Result<StoredToken> {
    Ok(StoredToken {
        id: row.get(0)?,
        user_id: row.get(1)?,
        name: row.get(2)?,
        created_at: parse_timestamp(3, &row.get::<_, String>(3)?)?,
        last_used_at: row.get::<_, Option<String>>(4)?.map(|value| parse_timestamp(4, &value)).transpose()?,
    })
}

fn parse_timestamp(index: usize, value: &str) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(err)))
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

    #[tokio::test]
    async fn list_by_user_returns_the_tokens_of_that_user_ordered_by_name() {
        let (_, repository) = open_with_users(&["johan", "anna"]).await;
        repository.insert("token-1", "johan-id", "phone", &Token::generate()).await.unwrap();
        repository.insert("token-2", "johan-id", "Kitchen tablet", &Token::generate()).await.unwrap();
        repository.insert("token-3", "anna-id", "Laptop", &Token::generate()).await.unwrap();

        let tokens = repository.list_by_user("johan-id").await.unwrap();

        let names: Vec<_> = tokens.iter().map(|token| token.name.as_str()).collect();
        assert_eq!(names, vec!["Kitchen tablet", "phone"]);
        assert!(tokens.iter().all(|token| token.user_id == "johan-id"));
    }

    #[tokio::test]
    async fn list_by_user_returns_the_stored_timestamps() {
        let (database, repository) = open_with_users(&["johan"]).await;
        let inserted = repository.insert("token-1", "johan-id", "Phone", &Token::generate()).await.unwrap();
        database
            .call(|conn| {
                conn.execute("UPDATE api_tokens SET last_used_at = '2026-11-19T20:00:00+00:00'", [])
                    .map_err(DatabaseError::from)
            })
            .await
            .unwrap();

        let tokens = repository.list_by_user("johan-id").await.unwrap();

        assert_eq!(tokens[0].created_at, inserted.created_at);
        assert_eq!(tokens[0].last_used_at, Some("2026-11-19T20:00:00Z".parse().unwrap()));
    }

    #[tokio::test]
    async fn list_by_user_returns_no_last_used_for_a_token_that_was_never_used() {
        let (_, repository) = open_with_users(&["johan"]).await;
        repository.insert("token-1", "johan-id", "Phone", &Token::generate()).await.unwrap();

        let tokens = repository.list_by_user("johan-id").await.unwrap();

        assert!(tokens[0].last_used_at.is_none());
    }

    #[rstest]
    #[case::user_without_tokens("johan-id")]
    #[case::unknown_user("nobody-id")]
    #[tokio::test]
    async fn list_by_user_returns_an_empty_list_without_tokens(#[case] user_id: &str) {
        let (_, repository) = open_with_users(&["johan"]).await;

        let tokens = repository.list_by_user(user_id).await.unwrap();

        assert!(tokens.is_empty());
    }

    #[rstest]
    #[case::created_at("UPDATE api_tokens SET created_at = 'yesterday'")]
    #[case::last_used_at("UPDATE api_tokens SET last_used_at = 'yesterday'")]
    #[tokio::test]
    async fn list_by_user_fails_on_an_invalid_timestamp(#[case] sql: &'static str) {
        let (database, repository) = open_with_users(&["johan"]).await;
        repository.insert("token-1", "johan-id", "Phone", &Token::generate()).await.unwrap();
        database.call(move |conn| conn.execute(sql, []).map_err(DatabaseError::from)).await.unwrap();

        let result = repository.list_by_user("johan-id").await;

        assert!(matches!(result, Err(TokenRepositoryError::Sqlite(_))), "got {result:?}");
    }

    #[tokio::test]
    async fn delete_removes_the_token_and_returns_true() {
        let (database, repository) = open_with_users(&["johan"]).await;
        repository.insert("token-1", "johan-id", "Phone", &Token::generate()).await.unwrap();

        let deleted = repository.delete("johan-id", "Phone").await.unwrap();

        assert!(deleted);
        assert!(stored_hashes(&database).await.is_empty());
    }

    #[tokio::test]
    async fn delete_matches_the_name_case_insensitively() {
        let (database, repository) = open_with_users(&["johan"]).await;
        repository.insert("token-1", "johan-id", "Phone", &Token::generate()).await.unwrap();

        assert!(repository.delete("johan-id", "PHONE").await.unwrap());
        assert!(stored_hashes(&database).await.is_empty());
    }

    #[tokio::test]
    async fn delete_returns_false_for_an_unknown_name() {
        let (database, repository) = open_with_users(&["johan"]).await;
        repository.insert("token-1", "johan-id", "Phone", &Token::generate()).await.unwrap();

        let deleted = repository.delete("johan-id", "Tablet").await.unwrap();

        assert!(!deleted);
        assert_eq!(stored_hashes(&database).await.len(), 1);
    }

    #[tokio::test]
    async fn delete_only_removes_the_token_of_that_user() {
        let (database, repository) = open_with_users(&["johan", "anna"]).await;
        let annas = Token::generate();
        repository.insert("token-1", "johan-id", "Phone", &Token::generate()).await.unwrap();
        repository.insert("token-2", "anna-id", "Phone", &annas).await.unwrap();

        repository.delete("johan-id", "Phone").await.unwrap();

        assert_eq!(stored_hashes(&database).await, vec![annas.hash()]);
    }

    #[tokio::test]
    async fn delete_keeps_the_other_tokens_of_the_user() {
        let (_, repository) = open_with_users(&["johan"]).await;
        repository.insert("token-1", "johan-id", "Phone", &Token::generate()).await.unwrap();
        repository.insert("token-2", "johan-id", "Tablet", &Token::generate()).await.unwrap();

        repository.delete("johan-id", "Phone").await.unwrap();

        let names: Vec<_> = repository.list_by_user("johan-id").await.unwrap().into_iter().map(|t| t.name).collect();
        assert_eq!(names, vec!["Tablet"]);
    }

    #[tokio::test]
    async fn a_deleted_token_no_longer_authenticates() {
        let (database, repository) = open_with_users(&["johan"]).await;
        let token = Token::generate();
        repository.insert("token-1", "johan-id", "Phone", &token).await.unwrap();
        let users = UserRepository::new(database);
        assert!(users.authenticate(&token).await.unwrap().is_some());

        repository.delete("johan-id", "Phone").await.unwrap();

        assert!(users.authenticate(&token).await.unwrap().is_none());
    }
}
