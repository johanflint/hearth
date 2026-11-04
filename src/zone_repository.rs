use crate::database::{Database, DatabaseError};
use crate::domain::{Owner, Zone};
use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, Row, params};
use std::collections::BTreeSet;
use thiserror::Error;

/// Persists the zones owned by Hearth. Zones owned by an integration are discovered, not stored.
#[derive(Debug)]
pub struct ZoneRepository {
    database: Database,
}

impl ZoneRepository {
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    pub async fn list(&self) -> Result<Vec<StoredZone>, ZoneRepositoryError> {
        self.database
            .call(|conn| {
                let mut statement = conn.prepare(
                    "SELECT id, revision, name, group_ids, device_ids, created_at, updated_at FROM zones ORDER BY id",
                )?;
                let rows = statement.query_map([], row_to_stored_zone)?;
                Ok(rows.collect::<Result<Vec<_>, _>>()?)
            })
            .await
    }

    pub async fn insert(
        &self,
        id: &str,
        name: &str,
        group_ids: &BTreeSet<String>,
        device_ids: &BTreeSet<String>,
    ) -> Result<StoredZone, InsertError> {
        let id = id.to_string();
        let name = name.to_string();
        let group_ids = serde_json::json!(group_ids);
        let device_ids = serde_json::json!(device_ids);
        self.database
            .call(move |conn| {
                let now = Utc::now().to_rfc3339();
                let stored_zone = conn
                    .query_row(
                        "INSERT INTO zones (id, revision, name, group_ids, device_ids, created_at, updated_at)
                        VALUES (?1, 0, ?2, ?3, ?4, ?5, ?5)
                        RETURNING id, revision, name, group_ids, device_ids, created_at, updated_at",
                        params![id, name, group_ids, device_ids, now],
                        row_to_stored_zone,
                    )
                    .map_err(|err| if is_name_conflict(&err) { InsertError::NameAlreadyExists } else { err.into() })?;
                Ok(stored_zone)
            })
            .await
    }

    pub async fn update(
        &self,
        id: &str,
        base_revision: u64,
        name: &str,
        group_ids: &BTreeSet<String>,
        device_ids: &BTreeSet<String>,
    ) -> Result<StoredZone, UpdateError> {
        let id = id.to_string();
        let name = name.to_string();
        let group_ids = serde_json::json!(group_ids);
        let device_ids = serde_json::json!(device_ids);
        self.database
            .call(move |conn| {
                let now = Utc::now().to_rfc3339();
                let updated = conn
                    .query_row(
                        "UPDATE zones SET name = ?3, group_ids = ?4, device_ids = ?5, revision = revision + 1, updated_at = ?6
                        WHERE id = ?1 AND revision = ?2
                        RETURNING id, revision, name, group_ids, device_ids, created_at, updated_at",
                        params![id, base_revision as i64, name, group_ids, device_ids, now],
                        row_to_stored_zone,
                    )
                    .optional()
                    .map_err(|err| if is_name_conflict(&err) { UpdateError::NameAlreadyExists } else { err.into() })?;

                if let Some(updated) = updated {
                    return Ok(updated);
                }

                // No row matched; either the zone doesn't exist or someone else updated it first
                let current: Option<i64> = conn
                    .query_row("SELECT revision FROM zones WHERE id = ?1", params![id], |row| row.get(0))
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
                    .execute("DELETE FROM zones WHERE id = ?1 AND revision = ?2", params![id, base_revision as i64])?;
                if rows_deleted == 1 {
                    return Ok(());
                }

                // No row matched; either the zone doesn't exist or someone else updated it first
                let current: Option<i64> = conn
                    .query_row("SELECT revision FROM zones WHERE id = ?1", params![id], |row| row.get(0))
                    .optional()?;
                match current {
                    None => Err(DeleteError::NotFound),
                    Some(current) => {
                        Err(DeleteError::RevisionConflict { base_revision, current_revision: to_revision(current)? })
                    }
                }
            })
            .await
    }
}

fn row_to_stored_zone(row: &Row) -> rusqlite::Result<StoredZone> {
    let id = row.get(0)?;
    let revision = to_revision(row.get(1)?)
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Integer, Box::new(err)))?;
    let name = row.get(2)?;
    let group_ids = id_set(row, 3)?;
    let device_ids = id_set(row, 4)?;

    let created_at: String = row.get(5)?;
    let created_at = DateTime::parse_from_rfc3339(&created_at)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(err)))?;

    let updated_at: Option<String> = row.get(6)?;
    let updated_at = updated_at
        .map(|updated_at| {
            DateTime::parse_from_rfc3339(&updated_at)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|err| rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(err)))
        })
        .transpose()?;

    Ok(StoredZone { id, revision, name, group_ids, device_ids, created_at, updated_at })
}

// Reads a column that holds a JSON array of ids
fn id_set(row: &Row, index: usize) -> rusqlite::Result<BTreeSet<String>> {
    let ids: serde_json::Value = row.get(index)?;
    serde_json::from_value(ids)
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(err)))
}

fn is_name_conflict(err: &rusqlite::Error) -> bool {
    matches!(err, rusqlite::Error::SqliteFailure(err, _) if err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE)
}

fn to_revision(revision: i64) -> Result<u64, ZoneRepositoryError> {
    u64::try_from(revision).map_err(|_| ZoneRepositoryError::InvalidRevision(revision))
}

#[derive(Debug)]
pub struct StoredZone {
    pub id: String,
    pub revision: u64,
    pub name: String,
    pub group_ids: BTreeSet<String>,
    pub device_ids: BTreeSet<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
}

// Only Hearth-owned zones are stored, so a stored zone is always owned by Hearth and has no external id
impl From<StoredZone> for Zone {
    fn from(stored: StoredZone) -> Self {
        Zone {
            id: stored.id,
            name: stored.name,
            group_ids: stored.group_ids,
            device_ids: stored.device_ids,
            revision: stored.revision,
            owner: Owner::Hearth,
            external_id: None,
        }
    }
}

#[derive(Error, Debug)]
pub enum ZoneRepositoryError {
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("stored revision {0} is out of range for u64")]
    InvalidRevision(i64),
}

#[derive(Error, Debug)]
pub enum InsertError {
    #[error("a zone with this name already exists")]
    NameAlreadyExists,
    #[error(transparent)]
    Repository(#[from] ZoneRepositoryError),
}

// `?` applies a single `From`, so `DatabaseError` and `rusqlite::Error` can't reach `Repository` via `ZoneRepositoryError` on their own
impl From<DatabaseError> for InsertError {
    fn from(err: DatabaseError) -> Self {
        Self::Repository(err.into())
    }
}

impl From<rusqlite::Error> for InsertError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Repository(err.into())
    }
}

#[derive(Error, Debug)]
pub enum UpdateError {
    #[error("zone not found")]
    NotFound,
    #[error(
        "revision conflict: update is based on revision {base_revision}, but the current revision is {current_revision}"
    )]
    RevisionConflict { base_revision: u64, current_revision: u64 },
    #[error("a zone with this name already exists")]
    NameAlreadyExists,
    #[error(transparent)]
    Repository(#[from] ZoneRepositoryError),
}

impl From<DatabaseError> for UpdateError {
    fn from(err: DatabaseError) -> Self {
        Self::Repository(err.into())
    }
}

impl From<rusqlite::Error> for UpdateError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Repository(err.into())
    }
}

#[derive(Error, Debug)]
pub enum DeleteError {
    #[error("zone not found")]
    NotFound,
    #[error(
        "revision conflict: delete is based on revision {base_revision}, but the current revision is {current_revision}"
    )]
    RevisionConflict { base_revision: u64, current_revision: u64 },
    #[error(transparent)]
    Repository(#[from] ZoneRepositoryError),
}

impl From<DatabaseError> for DeleteError {
    fn from(err: DatabaseError) -> Self {
        Self::Repository(err.into())
    }
}

impl From<rusqlite::Error> for DeleteError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Repository(err.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use rusqlite::params;
    use std::path::Path;

    fn open_in_memory() -> ZoneRepository {
        let database = Database::open(Path::new(":memory:")).expect("failed to open in-memory database");
        ZoneRepository::new(database)
    }

    fn ids(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    #[test]
    fn a_stored_zone_converts_to_a_zone_owned_by_hearth() {
        let stored = StoredZone {
            id: "zone".to_string(),
            revision: 3,
            name: "Zone".to_string(),
            group_ids: ids(&["room"]),
            device_ids: ids(&["a", "b"]),
            created_at: Utc::now(),
            updated_at: None,
        };

        let zone = Zone::from(stored);

        assert_eq!(zone.id, "zone");
        assert_eq!(zone.name, "Zone");
        assert_eq!(zone.revision, 3);
        assert_eq!(zone.group_ids, ids(&["room"]));
        assert_eq!(zone.device_ids, ids(&["a", "b"]));
        assert_eq!(zone.owner, Owner::Hearth);
        assert_eq!(zone.external_id, None);
    }

    // Bypasses the repository to write rows directly
    async fn insert_row(
        repository: &ZoneRepository,
        id: &str,
        name: &str,
        group_ids: &str,
        device_ids: &str,
        updated_at: Option<String>,
    ) {
        let (id, name, group_ids, device_ids) =
            (id.to_string(), name.to_string(), group_ids.to_string(), device_ids.to_string());
        repository
            .database
            .call(move |conn| {
                let now = Utc::now().to_rfc3339();
                conn.execute(
                    "INSERT INTO zones (id, revision, name, group_ids, device_ids, created_at, updated_at)
                     VALUES (?1, 0, ?2, ?3, ?4, ?5, ?6)",
                    params![id, name, group_ids, device_ids, now, updated_at],
                )
                .map_err(DatabaseError::from)
            })
            .await
            .expect("insert row");
    }

    // Bypasses the repository to change rows in ways the repository never would
    async fn execute(repository: &ZoneRepository, sql: &'static str) {
        repository
            .database
            .call(move |conn| conn.execute(sql, []).map_err(DatabaseError::from))
            .await
            .expect("execute");
    }

    #[tokio::test]
    async fn list_returns_empty_for_a_fresh_database() {
        let repository = open_in_memory();

        let rows = repository.list().await.expect("list succeeded");

        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn list_loads_a_valid_row() {
        let repository = open_in_memory();
        insert_row(
            &repository,
            "zone",
            "Downstairs",
            r#"["room","hall"]"#,
            r#"["lamp","desk"]"#,
            Some("2000-01-01T00:00:00Z".to_string()),
        )
        .await;

        let rows = repository.list().await.expect("list succeeded");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "zone");
        assert_eq!(rows[0].name, "Downstairs");
        assert_eq!(rows[0].group_ids, ids(&["hall", "room"]));
        assert_eq!(rows[0].device_ids, ids(&["desk", "lamp"]));
        let long_ago = DateTime::parse_from_rfc3339("2000-01-01T00:00:00Z").unwrap().with_timezone(&Utc);
        assert_eq!(rows[0].updated_at, Some(long_ago));
        assert!(rows[0].created_at <= Utc::now());
    }

    #[tokio::test]
    async fn list_loads_the_revision() {
        let repository = open_in_memory();
        insert_row(&repository, "zone", "Zone", "[]", "[]", None).await;
        execute(&repository, "UPDATE zones SET revision = 7").await;

        let rows = repository.list().await.expect("list succeeded");

        assert_eq!(rows[0].revision, 7);
    }

    #[tokio::test]
    async fn list_loads_a_zone_without_groups_and_devices() {
        let repository = open_in_memory();
        insert_row(&repository, "zone", "Zone", "[]", "[]", None).await;

        let rows = repository.list().await.expect("list succeeded");

        assert!(rows[0].group_ids.is_empty());
        assert!(rows[0].device_ids.is_empty());
    }

    #[tokio::test]
    async fn list_loads_an_id_listed_twice_once() {
        let repository = open_in_memory();
        insert_row(&repository, "zone", "Zone", r#"["room","room"]"#, r#"["lamp","lamp"]"#, None).await;

        let rows = repository.list().await.expect("list succeeded");

        assert_eq!(rows[0].group_ids.len(), 1);
        assert_eq!(rows[0].device_ids.len(), 1);
    }

    #[tokio::test]
    async fn list_handles_null_updated_at() {
        let repository = open_in_memory();
        insert_row(&repository, "zone", "Zone", "[]", "[]", None).await;

        let rows = repository.list().await.expect("list succeeded");

        assert_eq!(rows[0].updated_at, None);
    }

    #[tokio::test]
    async fn list_orders_the_zones_by_id() {
        let repository = open_in_memory();
        for id in ["b", "c", "a"] {
            insert_row(&repository, id, id, "[]", "[]", None).await;
        }

        let rows = repository.list().await.expect("list succeeded");

        let ids: Vec<_> = rows.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
    }

    #[rstest]
    #[case::group_ids("not valid json", "[]", 3)]
    #[case::device_ids("[]", "not valid json", 4)]
    #[case::not_an_array(r#"{"room":true}"#, "[]", 3)]
    #[case::not_strings("[]", "[1, 2]", 4)]
    #[tokio::test]
    async fn list_fails_on_ids_that_are_not_a_json_array_of_strings(
        #[case] group_ids: &str,
        #[case] device_ids: &str,
        #[case] column: usize,
    ) {
        let repository = open_in_memory();
        insert_row(&repository, "zone", "Zone", group_ids, device_ids, None).await;

        let result = repository.list().await;

        // Invalid JSON means the database is corrupt
        assert!(
            matches!(result, Err(ZoneRepositoryError::Sqlite(rusqlite::Error::FromSqlConversionFailure(index, _, _))) if index == column),
            "got {result:?}"
        );
    }

    #[rstest]
    #[case::created_at("UPDATE zones SET created_at = 'yesterday'", 5)]
    #[case::updated_at("UPDATE zones SET updated_at = 'yesterday'", 6)]
    #[tokio::test]
    async fn list_fails_on_an_invalid_timestamp(#[case] sql: &'static str, #[case] column: usize) {
        let repository = open_in_memory();
        insert_row(&repository, "zone", "Zone", "[]", "[]", Some(Utc::now().to_rfc3339())).await;
        execute(&repository, sql).await;

        let result = repository.list().await;

        assert!(
            matches!(result, Err(ZoneRepositoryError::Sqlite(rusqlite::Error::FromSqlConversionFailure(index, _, _))) if index == column),
            "got {result:?}"
        );
    }

    #[tokio::test]
    async fn list_fails_when_the_stored_revision_is_negative() {
        let repository = open_in_memory();
        insert_row(&repository, "zone", "Zone", "[]", "[]", None).await;
        execute(&repository, "UPDATE zones SET revision = -1").await;

        let result = repository.list().await;

        assert!(
            matches!(result, Err(ZoneRepositoryError::Sqlite(rusqlite::Error::FromSqlConversionFailure(1, _, _)))),
            "got {result:?}"
        );
    }

    #[rstest]
    #[case::same_case("Downstairs")]
    #[case::other_case("DOWNSTAIRS")]
    #[tokio::test]
    async fn the_schema_rejects_a_second_zone_with_the_same_name(#[case] name: &'static str) {
        let repository = open_in_memory();
        insert_row(&repository, "first", "Downstairs", "[]", "[]", None).await;
        let (id, name) = ("second".to_string(), name.to_string());

        let result = repository
            .database
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO zones (id, revision, name, group_ids, device_ids, created_at)
                     VALUES (?1, 0, ?2, '[]', '[]', 'now')",
                    params![id, name],
                )
                .map_err(DatabaseError::from)
            })
            .await;

        assert!(
            matches!(
                result,
                Err(DatabaseError::Sqlite(rusqlite::Error::SqliteFailure(err, _)))
                    if err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
            ),
            "got {result:?}"
        );
    }

    #[tokio::test]
    async fn insert_returns_the_stored_zone_at_revision_zero() {
        let repository = open_in_memory();

        let inserted = repository.insert("zone", "Downstairs", &ids(&["room", "hall"]), &ids(&["lamp", "desk"])).await;

        let inserted = inserted.expect("insert succeeded");
        assert_eq!(inserted.id, "zone");
        assert_eq!(inserted.revision, 0);
        assert_eq!(inserted.name, "Downstairs");
        assert_eq!(inserted.group_ids, ids(&["hall", "room"]));
        assert_eq!(inserted.device_ids, ids(&["desk", "lamp"]));
        assert_eq!(inserted.updated_at, Some(inserted.created_at));
    }

    #[tokio::test]
    async fn insert_persists_the_zone_and_leaves_others_untouched() {
        let repository = open_in_memory();
        insert_row(&repository, "other", "Other", r#"["g"]"#, r#"["a"]"#, None).await;

        repository.insert("zone", "Zone", &ids(&["room"]), &ids(&["b", "c"])).await.expect("insert succeeded");

        let rows = repository.list().await.expect("list succeeded");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "other");
        assert_eq!(rows[0].name, "Other");
        assert_eq!(rows[0].group_ids, ids(&["g"]));
        assert_eq!(rows[0].device_ids, ids(&["a"]));
        assert_eq!(rows[1].id, "zone");
        assert_eq!(rows[1].name, "Zone");
        assert_eq!(rows[1].revision, 0);
        assert_eq!(rows[1].group_ids, ids(&["room"]));
        assert_eq!(rows[1].device_ids, ids(&["b", "c"]));
    }

    #[rstest]
    #[case::groups_only(&["room"], &[])]
    #[case::devices_only(&[], &["lamp"])]
    #[case::neither(&[], &[])]
    #[tokio::test]
    async fn insert_accepts_a_zone_without_groups_or_without_devices(
        #[case] group_ids: &[&str],
        #[case] device_ids: &[&str],
    ) {
        let repository = open_in_memory();

        let inserted =
            repository.insert("zone", "Zone", &ids(group_ids), &ids(device_ids)).await.expect("insert succeeded");

        assert_eq!(inserted.group_ids, ids(group_ids));
        assert_eq!(inserted.device_ids, ids(device_ids));
        let rows = repository.list().await.expect("list succeeded");
        assert_eq!(rows[0].group_ids, ids(group_ids));
        assert_eq!(rows[0].device_ids, ids(device_ids));
    }

    #[tokio::test]
    async fn insert_stores_the_ids_as_json_arrays() {
        let repository = open_in_memory();

        repository.insert("zone", "Zone", &ids(&["y", "x"]), &ids(&["b", "a"])).await.expect("insert succeeded");

        let (group_ids, device_ids): (String, String) = repository
            .database
            .call(|conn| {
                conn.query_row("SELECT group_ids, device_ids FROM zones", [], |row| Ok((row.get(0)?, row.get(1)?)))
                    .map_err(DatabaseError::from)
            })
            .await
            .expect("query");
        assert_eq!(group_ids, r#"["x","y"]"#);
        assert_eq!(device_ids, r#"["a","b"]"#);
    }

    #[rstest]
    #[case::same_case("Downstairs")]
    #[case::different_case("dOWNSTAIRS")]
    #[tokio::test]
    async fn insert_rejects_a_name_that_is_already_in_use(#[case] name: &str) {
        let repository = open_in_memory();
        insert_row(&repository, "zone", "Downstairs", "[]", r#"["a"]"#, None).await;

        let result = repository.insert("duplicate", name, &ids(&["room"]), &ids(&["b"])).await;

        assert!(matches!(result, Err(InsertError::NameAlreadyExists)), "got {result:?}");
        let rows = repository.list().await.expect("list succeeded");
        assert_eq!(rows.len(), 1, "nothing must be stored");
        assert_eq!(rows[0].device_ids, ids(&["a"]));
    }

    #[tokio::test]
    async fn insert_fails_for_an_id_that_is_already_in_use() {
        let repository = open_in_memory();
        insert_row(&repository, "zone", "Downstairs", "[]", "[]", None).await;

        let result = repository.insert("zone", "Other", &ids(&["room"]), &ids(&["a"])).await;

        assert!(matches!(result, Err(InsertError::Repository(ZoneRepositoryError::Sqlite(_)))), "got {result:?}");
        let rows = repository.list().await.expect("list succeeded");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Downstairs");
    }

    #[rstest]
    #[case::same_case("Other")]
    #[case::different_case("oTHER")]
    #[tokio::test]
    async fn update_rejects_a_name_used_by_another_zone(#[case] name: &str) {
        let repository = open_in_memory();
        insert_row(&repository, "zone", "Upstairs", r#"["g"]"#, r#"["a"]"#, None).await;
        insert_row(&repository, "other", "Other", "[]", "[]", None).await;

        let result = repository.update("zone", 0, name, &ids(&["h"]), &ids(&["b"])).await;

        assert!(matches!(result, Err(UpdateError::NameAlreadyExists)), "got {result:?}");
        let rows = repository.list().await.expect("list succeeded");
        assert_eq!(rows[1].name, "Upstairs", "the zone must be unchanged");
        assert_eq!(rows[1].group_ids, ids(&["g"]));
        assert_eq!(rows[1].device_ids, ids(&["a"]));
        assert_eq!(rows[1].revision, 0);
    }

    #[rstest]
    #[case::same_name("Upstairs")]
    #[case::different_case("UPSTAIRS")]
    #[tokio::test]
    async fn update_allows_a_zone_to_keep_its_own_name(#[case] name: &str) {
        let repository = open_in_memory();
        insert_row(&repository, "zone", "Upstairs", "[]", r#"["a"]"#, None).await;

        let updated = repository.update("zone", 0, name, &ids(&[]), &ids(&["b"])).await.expect("update succeeded");

        assert_eq!(updated.name, name);
        assert_eq!(updated.revision, 1);
    }

    #[tokio::test]
    async fn update_replaces_the_name_groups_and_devices_and_returns_the_stored_zone() {
        let repository = open_in_memory();
        insert_row(&repository, "zone", "Upstairs", r#"["g"]"#, r#"["a"]"#, None).await;

        let updated = repository.update("zone", 0, "Attic", &ids(&["room", "hall"]), &ids(&["lamp", "desk"])).await;

        let updated = updated.expect("update succeeded");
        assert_eq!(updated.id, "zone");
        assert_eq!(updated.name, "Attic");
        assert_eq!(updated.group_ids, ids(&["hall", "room"]));
        assert_eq!(updated.device_ids, ids(&["desk", "lamp"]));
        let rows = repository.list().await.expect("list succeeded");
        assert_eq!(rows[0].name, "Attic");
        assert_eq!(rows[0].group_ids, ids(&["hall", "room"]));
        assert_eq!(rows[0].device_ids, ids(&["desk", "lamp"]));
    }

    #[rstest]
    #[case::groups_only(&["g"], &[])]
    #[case::devices_only(&[], &["a"])]
    #[case::neither(&[], &[])]
    #[tokio::test]
    async fn update_accepts_a_zone_without_groups_or_without_devices(
        #[case] group_ids: &[&str],
        #[case] device_ids: &[&str],
    ) {
        let repository = open_in_memory();
        insert_row(&repository, "zone", "Upstairs", r#"["x"]"#, r#"["y"]"#, None).await;

        let updated = repository.update("zone", 0, "Upstairs", &ids(group_ids), &ids(device_ids)).await;

        let updated = updated.expect("update succeeded");
        assert_eq!(updated.group_ids, ids(group_ids));
        assert_eq!(updated.device_ids, ids(device_ids));
    }

    #[tokio::test]
    async fn update_stores_the_ids_as_sorted_json_arrays() {
        let repository = open_in_memory();
        insert_row(&repository, "zone", "Upstairs", "[]", "[]", None).await;

        repository.update("zone", 0, "Upstairs", &ids(&["y", "x"]), &ids(&["b", "a"])).await.expect("update succeeded");

        let (group_ids, device_ids): (String, String) = repository
            .database
            .call(|conn| {
                conn.query_row("SELECT group_ids, device_ids FROM zones", [], |row| Ok((row.get(0)?, row.get(1)?)))
                    .map_err(DatabaseError::from)
            })
            .await
            .expect("query");
        assert_eq!(group_ids, r#"["x","y"]"#);
        assert_eq!(device_ids, r#"["a","b"]"#);
    }

    #[tokio::test]
    async fn update_increments_the_revision() {
        let repository = open_in_memory();
        insert_row(&repository, "zone", "Upstairs", "[]", "[]", None).await;

        let first =
            repository.update("zone", 0, "Upstairs", &ids(&[]), &ids(&["a"])).await.expect("first update succeeded");
        let second =
            repository.update("zone", 1, "Upstairs", &ids(&[]), &ids(&["b"])).await.expect("second update succeeded");

        assert_eq!(first.revision, 1);
        assert_eq!(second.revision, 2);
        assert_eq!(repository.list().await.expect("list succeeded")[0].revision, 2);
    }

    #[tokio::test]
    async fn update_sets_updated_at_and_keeps_created_at() {
        let repository = open_in_memory();
        insert_row(&repository, "zone", "Upstairs", "[]", "[]", None).await;
        execute(&repository, "UPDATE zones SET created_at = '2000-01-01T00:00:00Z'").await;
        let before = Utc::now();

        let updated =
            repository.update("zone", 0, "Upstairs", &ids(&[]), &ids(&["a"])).await.expect("update succeeded");

        let long_ago = DateTime::parse_from_rfc3339("2000-01-01T00:00:00Z").unwrap().with_timezone(&Utc);
        assert_eq!(updated.created_at, long_ago);
        assert!(updated.updated_at.expect("updated_at is set") >= before, "got {updated:?}");
    }

    #[tokio::test]
    async fn update_leaves_other_zones_untouched() {
        let repository = open_in_memory();
        insert_row(&repository, "zone", "Upstairs", "[]", "[]", None).await;
        insert_row(&repository, "other", "Other", r#"["g"]"#, r#"["x"]"#, None).await;

        repository.update("zone", 0, "Renamed", &ids(&["h"]), &ids(&["a"])).await.expect("update succeeded");

        let rows = repository.list().await.expect("list succeeded");
        assert_eq!(rows[0].id, "other");
        assert_eq!(rows[0].name, "Other");
        assert_eq!(rows[0].group_ids, ids(&["g"]));
        assert_eq!(rows[0].device_ids, ids(&["x"]));
        assert_eq!(rows[0].revision, 0);
        assert_eq!(rows[0].updated_at, None);
    }

    #[rstest]
    #[case::fresh_database(&[])]
    #[case::other_zones(&["other"])]
    #[tokio::test]
    async fn update_returns_not_found_for_an_unknown_id(#[case] existing_ids: &[&str]) {
        let repository = open_in_memory();
        for id in existing_ids {
            insert_row(&repository, id, id, "[]", "[]", None).await;
        }

        let result = repository.update("zone", 0, "Upstairs", &ids(&[]), &ids(&["a"])).await;

        assert!(matches!(result, Err(UpdateError::NotFound)), "got {result:?}");
        assert_eq!(
            repository.list().await.expect("list succeeded").len(),
            existing_ids.len(),
            "update must not create"
        );
    }

    #[rstest]
    #[case::behind(1, 2)]
    #[case::ahead(5, 2)]
    #[tokio::test]
    async fn update_returns_a_revision_conflict_for_a_mismatching_base_revision(
        #[case] base_revision: u64,
        #[case] current_revision: u64,
    ) {
        let repository = open_in_memory();
        insert_row(&repository, "zone", "Upstairs", r#"["g"]"#, r#"["a"]"#, None).await;
        execute(&repository, "UPDATE zones SET revision = 2").await;

        let result = repository.update("zone", base_revision, "Renamed", &ids(&["h"]), &ids(&["b"])).await;

        assert!(
            matches!(
                result,
                Err(UpdateError::RevisionConflict { base_revision: b, current_revision: c })
                    if b == base_revision && c == current_revision
            ),
            "got {result:?}"
        );
        let rows = repository.list().await.expect("list succeeded");
        assert_eq!(rows[0].name, "Upstairs", "a conflicting update must not change the zone");
        assert_eq!(rows[0].group_ids, ids(&["g"]));
        assert_eq!(rows[0].device_ids, ids(&["a"]));
        assert_eq!(rows[0].revision, 2);
    }

    #[tokio::test]
    async fn update_fails_for_a_stored_revision_out_of_range() {
        let repository = open_in_memory();
        insert_row(&repository, "zone", "Upstairs", "[]", "[]", None).await;
        execute(&repository, "UPDATE zones SET revision = -1").await;

        let result = repository.update("zone", 0, "Upstairs", &ids(&[]), &ids(&[])).await;

        assert!(
            matches!(result, Err(UpdateError::Repository(ZoneRepositoryError::InvalidRevision(-1)))),
            "got {result:?}"
        );
    }

    #[tokio::test]
    async fn delete_removes_the_zone_when_the_base_revision_matches() {
        let repository = open_in_memory();
        insert_row(&repository, "zone", "Upstairs", "[]", r#"["a"]"#, None).await;
        execute(&repository, "UPDATE zones SET revision = 3").await;

        repository.delete("zone", 3).await.expect("delete succeeded");

        assert!(repository.list().await.expect("list succeeded").is_empty(), "zone must be deleted");
    }

    #[tokio::test]
    async fn delete_only_removes_the_given_zone() {
        let repository = open_in_memory();
        insert_row(&repository, "zone", "Upstairs", r#"["g"]"#, "[]", None).await;
        insert_row(&repository, "other", "Other", r#"["h"]"#, r#"["a"]"#, None).await;

        repository.delete("zone", 0).await.expect("delete succeeded");

        let rows = repository.list().await.expect("list succeeded");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "other");
        assert_eq!(rows[0].name, "Other");
        assert_eq!((&rows[0].group_ids, &rows[0].device_ids), (&ids(&["h"]), &ids(&["a"])));
    }

    #[tokio::test]
    async fn delete_frees_the_name_and_id_for_a_new_zone() {
        let repository = open_in_memory();
        repository.insert("zone", "Upstairs", &ids(&["g"]), &ids(&[])).await.expect("insert succeeded");

        repository.delete("zone", 0).await.expect("delete succeeded");
        let reinserted =
            repository.insert("zone", "upstairs", &ids(&["g"]), &ids(&[])).await.expect("insert succeeded");

        assert_eq!(reinserted.revision, 0);
    }

    #[rstest]
    #[case::behind(0, 2)]
    #[case::ahead(5, 2)]
    #[tokio::test]
    async fn delete_returns_a_revision_conflict_for_a_mismatching_base_revision(
        #[case] base_revision: u64,
        #[case] current_revision: u64,
    ) {
        let repository = open_in_memory();
        insert_row(&repository, "zone", "Upstairs", "[]", "[]", None).await;
        execute(&repository, "UPDATE zones SET revision = 2").await;

        let result = repository.delete("zone", base_revision).await;

        assert!(
            matches!(
                result,
                Err(DeleteError::RevisionConflict { base_revision: b, current_revision: c })
                    if b == base_revision && c == current_revision
            ),
            "got {result:?}"
        );
        let rows = repository.list().await.expect("list succeeded");
        assert_eq!(rows.len(), 1, "the zone must not be deleted");
        assert_eq!(rows[0].revision, 2);
    }

    #[rstest]
    #[case::fresh_database(&[])]
    #[case::other_zones(&["other"])]
    #[tokio::test]
    async fn delete_returns_not_found_for_an_unknown_id(#[case] existing_ids: &[&str]) {
        let repository = open_in_memory();
        for id in existing_ids {
            insert_row(&repository, id, id, "[]", "[]", None).await;
        }

        let result = repository.delete("zone", 0).await;

        assert!(matches!(result, Err(DeleteError::NotFound)), "got {result:?}");
        assert_eq!(repository.list().await.expect("list succeeded").len(), existing_ids.len());
    }

    #[tokio::test]
    async fn delete_fails_for_a_stored_revision_out_of_range() {
        let repository = open_in_memory();
        insert_row(&repository, "zone", "Upstairs", "[]", "[]", None).await;
        execute(&repository, "UPDATE zones SET revision = -1").await;

        let result = repository.delete("zone", 0).await;

        assert!(
            matches!(result, Err(DeleteError::Repository(ZoneRepositoryError::InvalidRevision(-1)))),
            "got {result:?}"
        );
    }

    #[tokio::test]
    async fn delete_fails_when_the_repository_fails() {
        let repository = open_in_memory();
        execute(&repository, "DROP TABLE zones").await;

        let result = repository.delete("zone", 0).await;

        assert!(matches!(result, Err(DeleteError::Repository(ZoneRepositoryError::Sqlite(_)))), "got {result:?}");
    }
}
