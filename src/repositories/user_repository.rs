use crate::database::{Database, DatabaseError};
use crate::domain::{Permission, Token, User};
use chrono::{DateTime, TimeDelta, Utc};
use rusqlite::{Connection, OptionalExtension, params};
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
        self.database.call(move |conn| Ok(find_user(conn, "WHERE users.name = ?1", &name)?)).await
    }

    pub async fn authenticate(&self, token: &Token) -> Result<Option<StoredUser>, UserRepositoryError> {
        let hash = token.hash();
        self.database
            .call(move |conn| {
                let user = find_user(
                    conn,
                    "JOIN api_tokens ON api_tokens.user_id = users.id WHERE api_tokens.token_hash = ?1",
                    &hash,
                )?;
                if user.is_some() {
                    let now = Utc::now();
                    // Only write once a minute to avoid write queues and wear in the machine running this
                    conn.execute(
                        "UPDATE api_tokens SET last_used_at = ?2
                         WHERE token_hash = ?1 AND (last_used_at IS NULL OR last_used_at < ?3)",
                        params![hash, now.to_rfc3339(), (now - TimeDelta::minutes(1)).to_rfc3339()],
                    )?;
                }
                Ok(user)
            })
            .await
    }
}

fn is_name_conflict(err: &rusqlite::Error) -> bool {
    matches!(err, rusqlite::Error::SqliteFailure(err, _) if err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE)
}

fn find_user(conn: &Connection, join_filter_query: &str, value: &str) -> rusqlite::Result<Option<StoredUser>> {
    let row = conn
        .query_row(
            &format!("SELECT users.id, users.name, users.created_at FROM users {join_filter_query}"),
            [value],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
        )
        .optional()?;
    let Some((id, name, created_at)) = row else {
        return Ok(None);
    };

    let mut statement = conn.prepare("SELECT permission FROM user_permissions WHERE user_id = ?1")?;
    let permissions = statement
        .query_map([&id], |row| {
            let value: String = row.get(0)?;
            Permission::from_str(&value)
                .map_err(|err| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(err)))
        })?
        .collect::<Result<BTreeSet<_>, _>>()?;
    let created_at = DateTime::parse_from_rfc3339(&created_at)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(err)))?;

    Ok(Some(StoredUser { id, name, permissions, created_at }))
}

#[derive(Debug)]
pub struct StoredUser {
    pub id: String,
    pub name: String,
    pub permissions: BTreeSet<Permission>,
    pub created_at: DateTime<Utc>,
}

impl From<StoredUser> for User {
    fn from(user: StoredUser) -> Self {
        User { id: user.id, name: user.name, permissions: user.permissions, created_at: user.created_at }
    }
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
    use crate::repositories::TokenRepository;
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

    async fn insert_token(database: &Database, user_id: &str, name: &str) -> Token {
        let token = Token::generate();
        TokenRepository::new(database.clone())
            .insert(&format!("{user_id}-{name}"), user_id, name, &token)
            .await
            .unwrap();
        token
    }

    async fn last_used_at(database: &Database, token: &Token) -> Option<DateTime<Utc>> {
        let hash = token.hash();
        let value: Option<String> = database
            .call(move |conn| {
                conn.query_row("SELECT last_used_at FROM api_tokens WHERE token_hash = ?1", [hash], |row| row.get(0))
                    .map_err(DatabaseError::from)
            })
            .await
            .unwrap();
        value.map(|value| DateTime::parse_from_rfc3339(&value).unwrap().with_timezone(&Utc))
    }

    async fn set_last_used_at(database: &Database, token: &Token, value: DateTime<Utc>) {
        let hash = token.hash();
        database
            .call(move |conn| {
                conn.execute(
                    "UPDATE api_tokens SET last_used_at = ?2 WHERE token_hash = ?1",
                    params![hash, value.to_rfc3339()],
                )
                .map_err(DatabaseError::from)
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn authenticate_returns_the_owner_of_the_token_with_its_permissions() {
        let (database, repository) = open_in_memory();
        let permissions = || BTreeSet::from([Permission::ReadDevices, Permission::WriteFlows]);
        let inserted = repository.insert("user-1", "johan", permissions()).await.unwrap();
        let token = insert_token(&database, "user-1", "phone").await;

        let user = repository.authenticate(&token).await.unwrap().expect("the token should belong to a user");

        assert_eq!(user.id, "user-1");
        assert_eq!(user.name, "johan");
        assert_eq!(user.permissions, permissions());
        assert_eq!(user.created_at, inserted.created_at);
    }

    #[tokio::test]
    async fn authenticate_returns_the_user_that_owns_the_token() {
        let (database, repository) = open_in_memory();
        repository.insert("user-1", "johan", BTreeSet::from([Permission::ReadFlows])).await.unwrap();
        repository.insert("user-2", "anna", BTreeSet::from([Permission::WriteFlows])).await.unwrap();
        insert_token(&database, "user-1", "phone").await;
        let annas_token = insert_token(&database, "user-2", "tablet").await;

        let user = repository.authenticate(&annas_token).await.unwrap().unwrap();

        assert_eq!(user.name, "anna");
        assert_eq!(user.permissions, BTreeSet::from([Permission::WriteFlows]));
    }

    #[tokio::test]
    async fn authenticate_returns_none_for_an_unknown_token() {
        let (database, repository) = open_in_memory();
        repository.insert("user-1", "johan", BTreeSet::new()).await.unwrap();
        insert_token(&database, "user-1", "phone").await;

        let user = repository.authenticate(&Token::generate()).await.unwrap();

        assert!(user.is_none());
    }

    #[tokio::test]
    async fn authenticate_returns_none_once_the_user_is_deleted() {
        let (database, repository) = open_in_memory();
        repository.insert("user-1", "johan", BTreeSet::new()).await.unwrap();
        let token = insert_token(&database, "user-1", "phone").await;
        database.call(|conn| conn.execute("DELETE FROM users", []).map_err(DatabaseError::from)).await.unwrap();

        assert!(repository.authenticate(&token).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn authenticate_records_the_first_use() {
        let (database, repository) = open_in_memory();
        repository.insert("user-1", "johan", BTreeSet::new()).await.unwrap();
        let token = insert_token(&database, "user-1", "phone").await;
        let before = Utc::now();

        repository.authenticate(&token).await.unwrap();

        let used_at = last_used_at(&database, &token).await.expect("the use should be recorded");
        assert!(used_at >= before && used_at <= Utc::now(), "got {used_at}");
    }

    #[tokio::test]
    async fn authenticate_does_not_record_a_use_for_an_unknown_token() {
        let (database, repository) = open_in_memory();
        repository.insert("user-1", "johan", BTreeSet::new()).await.unwrap();
        let token = insert_token(&database, "user-1", "phone").await;

        repository.authenticate(&Token::generate()).await.unwrap();

        assert!(last_used_at(&database, &token).await.is_none());
    }

    #[tokio::test]
    async fn authenticate_does_not_rewrite_a_use_recorded_within_the_last_minute() {
        let (database, repository) = open_in_memory();
        repository.insert("user-1", "johan", BTreeSet::new()).await.unwrap();
        let token = insert_token(&database, "user-1", "phone").await;
        let recent = Utc::now() - TimeDelta::seconds(30);
        set_last_used_at(&database, &token, recent).await;

        repository.authenticate(&token).await.unwrap();

        assert_eq!(last_used_at(&database, &token).await, Some(recent));
    }

    #[tokio::test]
    async fn authenticate_refreshes_a_use_recorded_more_than_a_minute_ago() {
        let (database, repository) = open_in_memory();
        repository.insert("user-1", "johan", BTreeSet::new()).await.unwrap();
        let token = insert_token(&database, "user-1", "phone").await;
        let stale = Utc::now() - TimeDelta::minutes(2);
        set_last_used_at(&database, &token, stale).await;

        repository.authenticate(&token).await.unwrap();

        let used_at = last_used_at(&database, &token).await.unwrap();
        assert!(used_at > stale + TimeDelta::minutes(1), "got {used_at}");
    }

    #[tokio::test]
    async fn authenticate_only_records_the_use_of_the_presented_token() {
        let (database, repository) = open_in_memory();
        repository.insert("user-1", "johan", BTreeSet::new()).await.unwrap();
        let phone = insert_token(&database, "user-1", "phone").await;
        let tablet = insert_token(&database, "user-1", "tablet").await;

        repository.authenticate(&phone).await.unwrap();

        assert!(last_used_at(&database, &phone).await.is_some());
        assert!(last_used_at(&database, &tablet).await.is_none());
    }
}
