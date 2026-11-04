use crate::domain::{Event, Owner, Zone};
use crate::store::StoreSnapshot;
use crate::zone_store::{InsertError, StoredZone, ZoneStore, ZoneStoreError};
use std::collections::BTreeSet;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::mpsc::Sender;
use tokio::sync::watch::Receiver;

#[derive(Debug)]
pub struct ZoneService {
    snapshot_rx: Receiver<StoreSnapshot>,
    zone_store: Arc<ZoneStore>,
    store_tx: Sender<Event>,
}

impl ZoneService {
    pub fn new(snapshot_rx: Receiver<StoreSnapshot>, zone_store: Arc<ZoneStore>, store_tx: Sender<Event>) -> Self {
        ZoneService { snapshot_rx, zone_store, store_tx }
    }

    pub fn list(&self) -> Vec<Arc<Zone>> {
        let store_snapshot = self.snapshot_rx.borrow().clone(); // Cheap: clones the Arc<ZoneMap>
        let mut zones: Vec<_> = store_snapshot.zones.values().cloned().collect();
        // First sort by name, then by id
        zones.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
        zones
    }

    pub fn retrieve_zone(&self, id: &str) -> Result<Arc<Zone>, RetrieveZoneError> {
        self.snapshot_rx.borrow().zones.get(id).cloned().ok_or(RetrieveZoneError::NotFound)
    }

    pub async fn create_zone(
        &self,
        id: &str,
        name: &str,
        group_ids: Vec<String>,
        device_ids: Vec<String>,
    ) -> Result<StoredZone, CreateZoneError> {
        let store_snapshot = self.snapshot_rx.borrow().clone(); // Cheap
        if store_snapshot.zones.contains_key(id) {
            return Err(CreateZoneError::IdAlreadyExists);
        }
        if store_snapshot.zones.values().any(|zone| zone.name.eq_ignore_ascii_case(name)) {
            return Err(CreateZoneError::NameAlreadyExists);
        }

        if group_ids.is_empty() && device_ids.is_empty() {
            return Err(CreateZoneError::ZoneEmpty);
        }

        let zone = Zone {
            id: id.to_string(),
            name: name.to_string(),
            group_ids: group_ids.into_iter().collect(),   // BTreeSet dedupes
            device_ids: device_ids.into_iter().collect(), // BTreeSet dedupes
            revision: 0,
            owner: Owner::Hearth,
            external_id: None,
        };

        let unknown_group_ids: BTreeSet<String> = zone.unknown_group_ids(&store_snapshot.groups).cloned().collect();
        if !unknown_group_ids.is_empty() {
            return Err(CreateZoneError::UnknownGroups { group_ids: unknown_group_ids });
        }

        let unknown_device_ids: BTreeSet<String> = zone.unknown_device_ids(&store_snapshot.devices).cloned().collect();
        if !unknown_device_ids.is_empty() {
            return Err(CreateZoneError::UnknownDevices { device_ids: unknown_device_ids });
        }

        let permit = self.store_tx.reserve().await.map_err(|_| CreateZoneError::StoreUnavailable)?;

        let stored_zone =
            self.zone_store.insert(id, name, &zone.group_ids, &zone.device_ids).await.map_err(|err| match err {
                InsertError::NameAlreadyExists => CreateZoneError::NameAlreadyExists,
                InsertError::Store(err) => CreateZoneError::Internal(err),
            })?;

        permit.send(Event::ZoneCreated(Zone { revision: stored_zone.revision, ..zone }));
        Ok(stored_zone)
    }
}

#[derive(Debug, Error)]
pub enum RetrieveZoneError {
    #[error("zone not found")]
    NotFound,
}

#[derive(Debug, Error)]
pub enum CreateZoneError {
    #[error("a zone with this id already exists")]
    IdAlreadyExists,
    #[error("a zone with this name already exists")]
    NameAlreadyExists,
    #[error("a zone must contain at least one group or device")]
    ZoneEmpty,
    #[error("unknown group ids: '{}'", group_ids.iter().map(String::as_str).collect::<Vec<_>>().join(", "))]
    UnknownGroups { group_ids: BTreeSet<String> },
    #[error("unknown device ids: '{}'", device_ids.iter().map(String::as_str).collect::<Vec<_>>().join(", "))]
    UnknownDevices { device_ids: BTreeSet<String> },
    #[error("store is unavailable")]
    StoreUnavailable,
    #[error(transparent)]
    Internal(#[from] ZoneStoreError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use crate::domain::{Group, Owner};
    use crate::test_support::DeviceBuilder;
    use rstest::rstest;
    use std::collections::{BTreeSet, HashMap};
    use std::path::Path;
    use tokio::sync::{mpsc, watch};

    fn zone(id: &str, name: &str) -> Arc<Zone> {
        Arc::new(Zone {
            id: id.to_string(),
            name: name.to_string(),
            group_ids: BTreeSet::new(),
            device_ids: BTreeSet::new(),
            revision: 0,
            owner: Owner::Integration,
            external_id: None,
        })
    }

    fn snapshot(zones: Vec<Arc<Zone>>) -> StoreSnapshot {
        let zones: HashMap<_, _> = zones.into_iter().map(|zone| (zone.id.clone(), zone)).collect();
        StoreSnapshot { zones: Arc::new(zones), ..Default::default() }
    }

    fn service(rx: Receiver<StoreSnapshot>) -> ZoneService {
        let database = Database::open(Path::new(":memory:")).expect("failed to open in-memory database");
        let (store_tx, _store_rx) = mpsc::channel(1);
        ZoneService::new(rx, Arc::new(ZoneStore::new(database)), store_tx)
    }

    /// Bind `store_rx` when the service reserves a permit: dropping it, e.g. by leaving it out with `..`,
    /// makes the store unavailable.
    struct Fixture {
        service: ZoneService,
        zone_store: Arc<ZoneStore>,
        store_rx: mpsc::Receiver<Event>,
    }

    fn create_service(snapshot: StoreSnapshot) -> Fixture {
        let database = Database::open(Path::new(":memory:")).expect("failed to open in-memory database");
        let zone_store = Arc::new(ZoneStore::new(database));
        let (store_tx, store_rx) = mpsc::channel(8);
        let (_tx, rx) = watch::channel(snapshot);
        let service = ZoneService::new(rx, Arc::clone(&zone_store), store_tx);
        Fixture { service, zone_store, store_rx }
    }

    /// A service whose `zones` table is gone, so every database call fails
    async fn create_service_with_broken_database(snapshot: StoreSnapshot) -> (ZoneService, mpsc::Receiver<Event>) {
        let database = Database::open(Path::new(":memory:")).expect("failed to open in-memory database");
        let (store_tx, store_rx) = mpsc::channel(8);
        let (_tx, rx) = watch::channel(snapshot);
        let service = ZoneService::new(rx, Arc::new(ZoneStore::new(database.clone())), store_tx);
        database
            .call(|conn| conn.execute("DROP TABLE zones", []).map_err(crate::database::DatabaseError::from))
            .await
            .unwrap();
        (service, store_rx)
    }

    /// A snapshot with the given groups and devices, and optionally zones
    fn snapshot_with(group_ids: &[&str], device_ids: &[&str]) -> StoreSnapshot {
        let groups: HashMap<_, _> = group_ids
            .iter()
            .map(|id| {
                let group = Group {
                    id: id.to_string(),
                    name: id.to_string(),
                    device_ids: BTreeSet::new(),
                    revision: 0,
                    owner: Owner::Hearth,
                    external_id: None,
                };
                (id.to_string(), Arc::new(group))
            })
            .collect();
        let devices: HashMap<_, _> =
            device_ids.iter().map(|id| (id.to_string(), Arc::new(DeviceBuilder::new(id).build()))).collect();
        StoreSnapshot { groups: Arc::new(groups), devices: Arc::new(devices), ..Default::default() }
    }

    fn ids(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    fn set(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    fn names(zones: &[Arc<Zone>]) -> Vec<&str> {
        zones.iter().map(|zone| zone.name.as_str()).collect()
    }

    #[test]
    fn list_returns_nothing_for_an_empty_snapshot() {
        let (_tx, rx) = watch::channel(StoreSnapshot::default());

        assert!(service(rx).list().is_empty());
    }

    #[test]
    fn list_returns_all_zones_in_the_snapshot() {
        let (_tx, rx) = watch::channel(snapshot(vec![zone("1", "Garden"), zone("2", "Upstairs")]));

        let zones = service(rx).list();

        assert_eq!(zones.len(), 2);
    }

    #[test]
    fn list_orders_the_zones_by_name_instead_of_id() {
        let (_tx, rx) = watch::channel(snapshot(vec![
            zone("1", "Upstairs"),
            zone("2", "Downstairs"),
            zone("3", "Garden"),
            zone("4", "Attic"),
        ]));

        let zones = service(rx).list();

        assert_eq!(names(&zones), vec!["Attic", "Downstairs", "Garden", "Upstairs"]);
    }

    #[test]
    fn list_orders_zones_with_the_same_name_by_id() {
        let (_tx, rx) = watch::channel(snapshot(vec![zone("c", "Zone"), zone("a", "Zone"), zone("b", "Zone")]));

        let zones = service(rx).list();

        let ids: Vec<_> = zones.iter().map(|zone| zone.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn list_follows_the_latest_snapshot() {
        let (tx, rx) = watch::channel(snapshot(vec![zone("1", "Upstairs")]));
        let service = service(rx);

        tx.send(snapshot(vec![zone("1", "Upstairs"), zone("2", "Garden")])).unwrap();

        assert_eq!(names(&service.list()), vec!["Garden", "Upstairs"]);
    }

    #[test]
    fn retrieve_zone_returns_the_zone_with_the_id() {
        let (_tx, rx) = watch::channel(snapshot(vec![zone("1", "Garden"), zone("2", "Upstairs")]));

        let found = service(rx).retrieve_zone("2").unwrap();

        assert_eq!(found.name, "Upstairs");
    }

    #[test]
    fn retrieve_zone_returns_not_found_for_an_unknown_id() {
        let (_tx, rx) = watch::channel(snapshot(vec![zone("1", "Garden")]));

        let result = service(rx).retrieve_zone("unknown");

        assert!(matches!(result, Err(RetrieveZoneError::NotFound)));
    }

    #[test]
    fn retrieve_zone_does_not_match_on_the_name() {
        let (_tx, rx) = watch::channel(snapshot(vec![zone("1", "Garden")]));

        let result = service(rx).retrieve_zone("Garden");

        assert!(matches!(result, Err(RetrieveZoneError::NotFound)));
    }

    #[test]
    fn retrieve_zone_follows_the_latest_snapshot() {
        let (tx, rx) = watch::channel(StoreSnapshot::default());
        let service = service(rx);

        tx.send(snapshot(vec![zone("1", "Garden")])).unwrap();

        assert!(service.retrieve_zone("1").is_ok());
    }

    #[tokio::test]
    async fn create_zone_returns_the_stored_zone() {
        let Fixture { service, store_rx: _store_rx, .. } = create_service(snapshot_with(&["g1", "g2"], &["a", "b"]));

        let stored = service.create_zone("zone", "Downstairs", ids(&["g2", "g1"]), ids(&["b", "a"])).await.unwrap();

        assert_eq!(stored.id, "zone");
        assert_eq!(stored.name, "Downstairs");
        assert_eq!(stored.group_ids, set(&["g1", "g2"]));
        assert_eq!(stored.device_ids, set(&["a", "b"]));
        assert_eq!(stored.revision, 0);
    }

    #[tokio::test]
    async fn create_zone_persists_the_zone() {
        let Fixture { service, zone_store, store_rx: _store_rx } = create_service(snapshot_with(&["g"], &["a"]));

        service.create_zone("zone", "Downstairs", ids(&["g"]), ids(&["a"])).await.unwrap();

        let stored = zone_store.list().await.unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!((stored[0].id.as_str(), stored[0].name.as_str()), ("zone", "Downstairs"));
        assert_eq!(stored[0].group_ids, set(&["g"]));
        assert_eq!(stored[0].device_ids, set(&["a"]));
    }

    #[tokio::test]
    async fn create_zone_sends_a_zone_created_event_for_a_hearth_zone() {
        let Fixture { service, mut store_rx, .. } = create_service(snapshot_with(&["g1", "g2"], &["a", "b"]));

        service.create_zone("zone", "Downstairs", ids(&["g2", "g1", "g2"]), ids(&["b", "a"])).await.unwrap();

        let Event::ZoneCreated(zone) = store_rx.try_recv().expect("an event was sent") else {
            panic!("expected a ZoneCreated event");
        };
        assert_eq!(
            zone,
            Zone {
                id: "zone".to_string(),
                name: "Downstairs".to_string(),
                group_ids: set(&["g1", "g2"]),
                device_ids: set(&["a", "b"]),
                revision: 0,
                owner: Owner::Hearth,
                external_id: None,
            }
        );
        assert!(store_rx.try_recv().is_err(), "only one event must be sent");
    }

    #[tokio::test]
    async fn create_zone_deduplicates_the_group_and_device_ids() {
        let Fixture { service, store_rx: _store_rx, .. } = create_service(snapshot_with(&["g"], &["a"]));

        let stored = service.create_zone("zone", "Downstairs", ids(&["g", "g"]), ids(&["a", "a"])).await.unwrap();

        assert_eq!(stored.group_ids, set(&["g"]));
        assert_eq!(stored.device_ids, set(&["a"]));
    }

    #[rstest]
    #[case::groups_only(&["g"], &[])]
    #[case::devices_only(&[], &["a"])]
    #[tokio::test]
    async fn create_zone_accepts_a_zone_with_only_groups_or_only_devices(
        #[case] group_ids: &[&str],
        #[case] device_ids: &[&str],
    ) {
        let Fixture { service, store_rx: _store_rx, .. } = create_service(snapshot_with(&["g"], &["a"]));

        let result = service.create_zone("zone", "Downstairs", ids(group_ids), ids(device_ids)).await;

        assert!(result.is_ok(), "got {result:?}");
    }

    #[tokio::test]
    async fn create_zone_rejects_a_zone_without_groups_and_devices() {
        let Fixture { service, zone_store, mut store_rx } = create_service(StoreSnapshot::default());

        let result = service.create_zone("zone", "Empty", vec![], vec![]).await;

        assert!(matches!(result, Err(CreateZoneError::ZoneEmpty)), "got {result:?}");
        assert!(zone_store.list().await.unwrap().is_empty(), "nothing must be stored");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn create_zone_rejects_an_existing_id() {
        let snapshot =
            StoreSnapshot { zones: snapshot(vec![zone("zone", "Existing")]).zones, ..snapshot_with(&[], &["a"]) };
        let Fixture { service, zone_store, mut store_rx } = create_service(snapshot);

        let result = service.create_zone("zone", "Downstairs", vec![], ids(&["a"])).await;

        assert!(matches!(result, Err(CreateZoneError::IdAlreadyExists)), "got {result:?}");
        assert!(zone_store.list().await.unwrap().is_empty(), "nothing must be stored");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[rstest]
    #[case::same_case("Downstairs")]
    #[case::different_case("downstairs")]
    #[tokio::test]
    async fn create_zone_rejects_a_name_in_use_by_another_zone(#[case] name: &str) {
        let snapshot =
            StoreSnapshot { zones: snapshot(vec![zone("other", "Downstairs")]).zones, ..snapshot_with(&[], &["a"]) };
        let Fixture { service, zone_store, mut store_rx } = create_service(snapshot);

        let result = service.create_zone("zone", name, vec![], ids(&["a"])).await;

        assert!(matches!(result, Err(CreateZoneError::NameAlreadyExists)), "got {result:?}");
        assert!(zone_store.list().await.unwrap().is_empty(), "nothing must be stored");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn create_zone_rejects_a_name_stored_but_not_yet_in_the_snapshot() {
        let Fixture { service, zone_store, mut store_rx } = create_service(snapshot_with(&[], &["a"]));
        zone_store.insert("other", "Downstairs", &set(&[]), &set(&["a"])).await.unwrap();

        let result = service.create_zone("zone", "downstairs", vec![], ids(&["a"])).await;

        assert!(matches!(result, Err(CreateZoneError::NameAlreadyExists)), "got {result:?}");
        assert_eq!(zone_store.list().await.unwrap().len(), 1);
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn create_zone_rejects_unknown_groups_and_lists_all_of_them_sorted() {
        let Fixture { service, zone_store, mut store_rx } = create_service(snapshot_with(&["known"], &[]));

        let result = service.create_zone("zone", "Downstairs", ids(&["z", "known", "a"]), vec![]).await;

        let Err(err @ CreateZoneError::UnknownGroups { .. }) = result else {
            panic!("expected UnknownGroups, got {result:?}");
        };
        assert_eq!(err.to_string(), "unknown group ids: 'a, z'");
        assert!(zone_store.list().await.unwrap().is_empty(), "nothing must be stored");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn create_zone_rejects_unknown_devices_and_lists_all_of_them_sorted() {
        let Fixture { service, zone_store, mut store_rx } = create_service(snapshot_with(&[], &["known"]));

        let result = service.create_zone("zone", "Downstairs", vec![], ids(&["z", "known", "a"])).await;

        let Err(err @ CreateZoneError::UnknownDevices { .. }) = result else {
            panic!("expected UnknownDevices, got {result:?}");
        };
        assert_eq!(err.to_string(), "unknown device ids: 'a, z'");
        assert!(zone_store.list().await.unwrap().is_empty(), "nothing must be stored");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn create_zone_does_not_mistake_a_group_id_for_a_device_id_or_the_other_way_around() {
        let Fixture { service, store_rx: _store_rx, .. } = create_service(snapshot_with(&["g"], &["a"]));

        let result = service.create_zone("zone", "Downstairs", ids(&["a"]), ids(&["g"])).await;

        assert!(matches!(result, Err(CreateZoneError::UnknownGroups { .. })), "got {result:?}");
    }

    #[tokio::test]
    async fn create_zone_reports_unknown_groups_before_unknown_devices() {
        let Fixture { service, store_rx: _store_rx, .. } = create_service(StoreSnapshot::default());

        let result = service.create_zone("zone", "Downstairs", ids(&["g"]), ids(&["a"])).await;

        assert!(matches!(result, Err(CreateZoneError::UnknownGroups { .. })), "got {result:?}");
    }

    #[tokio::test]
    async fn create_zone_fails_without_storing_when_the_store_is_gone() {
        let Fixture { service, zone_store, store_rx } = create_service(snapshot_with(&[], &["a"]));
        drop(store_rx);

        let result = service.create_zone("zone", "Downstairs", vec![], ids(&["a"])).await;

        assert!(matches!(result, Err(CreateZoneError::StoreUnavailable)), "got {result:?}");
        assert!(zone_store.list().await.unwrap().is_empty(), "nothing must be stored");
    }

    #[tokio::test]
    async fn create_zone_returns_an_internal_error_when_the_database_fails() {
        let (service, mut store_rx) = create_service_with_broken_database(snapshot_with(&[], &["a"])).await;

        let result = service.create_zone("zone", "Downstairs", vec![], ids(&["a"])).await;

        assert!(matches!(result, Err(CreateZoneError::Internal(_))), "got {result:?}");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }
}
