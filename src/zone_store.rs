use crate::database::{Database, DatabaseError};
use crate::domain::{Owner, Zone};
use chrono::{DateTime, Utc};
use rusqlite::{Row, params};
use std::collections::BTreeSet;
use thiserror::Error;

/// Persists the zones owned by Hearth. Zones owned by an integration are discovered, not stored.
#[derive(Debug)]
pub struct ZoneStore {
    database: Database,
}

impl ZoneStore {
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    pub async fn list(&self) -> Result<Vec<StoredZone>, ZoneStoreError> {
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

fn to_revision(revision: i64) -> Result<u64, ZoneStoreError> {
    u64::try_from(revision).map_err(|_| ZoneStoreError::InvalidRevision(revision))
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
pub enum ZoneStoreError {
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
    Store(#[from] ZoneStoreError),
}

// `?` applies a single `From`, so `DatabaseError` and `rusqlite::Error` can't reach `Store` via `ZoneStoreError` on their own
impl From<DatabaseError> for InsertError {
    fn from(err: DatabaseError) -> Self {
        Self::Store(err.into())
    }
}

impl From<rusqlite::Error> for InsertError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Store(err.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use rusqlite::params;
    use std::path::Path;

    fn open_in_memory() -> ZoneStore {
        let database = Database::open(Path::new(":memory:")).expect("failed to open in-memory database");
        ZoneStore::new(database)
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

    // Bypasses the store to write rows directly
    async fn insert_row(
        store: &ZoneStore,
        id: &str,
        name: &str,
        group_ids: &str,
        device_ids: &str,
        updated_at: Option<String>,
    ) {
        let (id, name, group_ids, device_ids) =
            (id.to_string(), name.to_string(), group_ids.to_string(), device_ids.to_string());
        store
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

    // Bypasses the store to change rows in ways the store never would
    async fn execute(store: &ZoneStore, sql: &'static str) {
        store.database.call(move |conn| conn.execute(sql, []).map_err(DatabaseError::from)).await.expect("execute");
    }

    #[tokio::test]
    async fn list_returns_empty_for_a_fresh_database() {
        let store = open_in_memory();

        let rows = store.list().await.expect("list succeeded");

        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn list_loads_a_valid_row() {
        let store = open_in_memory();
        insert_row(
            &store,
            "zone",
            "Downstairs",
            r#"["room","hall"]"#,
            r#"["lamp","desk"]"#,
            Some("2000-01-01T00:00:00Z".to_string()),
        )
        .await;

        let rows = store.list().await.expect("list succeeded");

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
        let store = open_in_memory();
        insert_row(&store, "zone", "Zone", "[]", "[]", None).await;
        execute(&store, "UPDATE zones SET revision = 7").await;

        let rows = store.list().await.expect("list succeeded");

        assert_eq!(rows[0].revision, 7);
    }

    #[tokio::test]
    async fn list_loads_a_zone_without_groups_and_devices() {
        let store = open_in_memory();
        insert_row(&store, "zone", "Zone", "[]", "[]", None).await;

        let rows = store.list().await.expect("list succeeded");

        assert!(rows[0].group_ids.is_empty());
        assert!(rows[0].device_ids.is_empty());
    }

    #[tokio::test]
    async fn list_loads_an_id_listed_twice_once() {
        let store = open_in_memory();
        insert_row(&store, "zone", "Zone", r#"["room","room"]"#, r#"["lamp","lamp"]"#, None).await;

        let rows = store.list().await.expect("list succeeded");

        assert_eq!(rows[0].group_ids.len(), 1);
        assert_eq!(rows[0].device_ids.len(), 1);
    }

    #[tokio::test]
    async fn list_handles_null_updated_at() {
        let store = open_in_memory();
        insert_row(&store, "zone", "Zone", "[]", "[]", None).await;

        let rows = store.list().await.expect("list succeeded");

        assert_eq!(rows[0].updated_at, None);
    }

    #[tokio::test]
    async fn list_orders_the_zones_by_id() {
        let store = open_in_memory();
        for id in ["b", "c", "a"] {
            insert_row(&store, id, id, "[]", "[]", None).await;
        }

        let rows = store.list().await.expect("list succeeded");

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
        let store = open_in_memory();
        insert_row(&store, "zone", "Zone", group_ids, device_ids, None).await;

        let result = store.list().await;

        // Invalid JSON means the database is corrupt
        assert!(
            matches!(result, Err(ZoneStoreError::Sqlite(rusqlite::Error::FromSqlConversionFailure(index, _, _))) if index == column),
            "got {result:?}"
        );
    }

    #[rstest]
    #[case::created_at("UPDATE zones SET created_at = 'yesterday'", 5)]
    #[case::updated_at("UPDATE zones SET updated_at = 'yesterday'", 6)]
    #[tokio::test]
    async fn list_fails_on_an_invalid_timestamp(#[case] sql: &'static str, #[case] column: usize) {
        let store = open_in_memory();
        insert_row(&store, "zone", "Zone", "[]", "[]", Some(Utc::now().to_rfc3339())).await;
        execute(&store, sql).await;

        let result = store.list().await;

        assert!(
            matches!(result, Err(ZoneStoreError::Sqlite(rusqlite::Error::FromSqlConversionFailure(index, _, _))) if index == column),
            "got {result:?}"
        );
    }

    #[tokio::test]
    async fn list_fails_when_the_stored_revision_is_negative() {
        let store = open_in_memory();
        insert_row(&store, "zone", "Zone", "[]", "[]", None).await;
        execute(&store, "UPDATE zones SET revision = -1").await;

        let result = store.list().await;

        assert!(
            matches!(result, Err(ZoneStoreError::Sqlite(rusqlite::Error::FromSqlConversionFailure(1, _, _)))),
            "got {result:?}"
        );
    }

    #[rstest]
    #[case::same_case("Downstairs")]
    #[case::other_case("DOWNSTAIRS")]
    #[tokio::test]
    async fn the_schema_rejects_a_second_zone_with_the_same_name(#[case] name: &'static str) {
        let store = open_in_memory();
        insert_row(&store, "first", "Downstairs", "[]", "[]", None).await;
        let (id, name) = ("second".to_string(), name.to_string());

        let result = store
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
        let store = open_in_memory();

        let inserted = store.insert("zone", "Downstairs", &ids(&["room", "hall"]), &ids(&["lamp", "desk"])).await;

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
        let store = open_in_memory();
        insert_row(&store, "other", "Other", r#"["g"]"#, r#"["a"]"#, None).await;

        store.insert("zone", "Zone", &ids(&["room"]), &ids(&["b", "c"])).await.expect("insert succeeded");

        let rows = store.list().await.expect("list succeeded");
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
        let store = open_in_memory();

        let inserted = store.insert("zone", "Zone", &ids(group_ids), &ids(device_ids)).await.expect("insert succeeded");

        assert_eq!(inserted.group_ids, ids(group_ids));
        assert_eq!(inserted.device_ids, ids(device_ids));
        let rows = store.list().await.expect("list succeeded");
        assert_eq!(rows[0].group_ids, ids(group_ids));
        assert_eq!(rows[0].device_ids, ids(device_ids));
    }

    #[tokio::test]
    async fn insert_stores_the_ids_as_json_arrays() {
        let store = open_in_memory();

        store.insert("zone", "Zone", &ids(&["y", "x"]), &ids(&["b", "a"])).await.expect("insert succeeded");

        let (group_ids, device_ids): (String, String) = store
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
        let store = open_in_memory();
        insert_row(&store, "zone", "Downstairs", "[]", r#"["a"]"#, None).await;

        let result = store.insert("duplicate", name, &ids(&["room"]), &ids(&["b"])).await;

        assert!(matches!(result, Err(InsertError::NameAlreadyExists)), "got {result:?}");
        let rows = store.list().await.expect("list succeeded");
        assert_eq!(rows.len(), 1, "nothing must be stored");
        assert_eq!(rows[0].device_ids, ids(&["a"]));
    }

    #[tokio::test]
    async fn insert_fails_for_an_id_that_is_already_in_use() {
        let store = open_in_memory();
        insert_row(&store, "zone", "Downstairs", "[]", "[]", None).await;

        let result = store.insert("zone", "Other", &ids(&["room"]), &ids(&["a"])).await;

        assert!(matches!(result, Err(InsertError::Store(ZoneStoreError::Sqlite(_)))), "got {result:?}");
        let rows = store.list().await.expect("list succeeded");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Downstairs");
    }
}
