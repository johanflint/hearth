use crate::domain::{Event, Owner, Zone, ZoneChanged};
use crate::flow_registry::{FlowRegistry, FlowSummary};
use crate::store::StoreSnapshot;
use crate::zone_repository::{DeleteError, InsertError, StoredZone, UpdateError, ZoneRepository, ZoneRepositoryError};
use std::collections::BTreeSet;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::mpsc::Sender;
use tokio::sync::watch::Receiver;

#[derive(Debug)]
pub struct ZoneService {
    snapshot_rx: Receiver<StoreSnapshot>,
    zone_repository: Arc<ZoneRepository>,
    flow_registry: Arc<FlowRegistry>,
    store_tx: Sender<Event>,
}

impl ZoneService {
    pub fn new(
        snapshot_rx: Receiver<StoreSnapshot>,
        zone_repository: Arc<ZoneRepository>,
        flow_registry: Arc<FlowRegistry>,
        store_tx: Sender<Event>,
    ) -> Self {
        ZoneService { snapshot_rx, zone_repository, flow_registry, store_tx }
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

        let stored_zone = self.zone_repository.insert(id, name, &zone.group_ids, &zone.device_ids).await.map_err(
            |err| match err {
                InsertError::NameAlreadyExists => CreateZoneError::NameAlreadyExists,
                InsertError::Repository(err) => CreateZoneError::Internal(err),
            },
        )?;

        permit.send(Event::ZoneCreated(Zone { revision: stored_zone.revision, ..zone }));
        Ok(stored_zone)
    }

    pub async fn update_zone(
        &self,
        id: &str,
        base_revision: u64,
        name: &str,
        group_ids: Vec<String>,
        device_ids: Vec<String>,
    ) -> Result<StoredZone, UpdateZoneError> {
        let store_snapshot = self.snapshot_rx.borrow().clone(); // Cheap
        if store_snapshot.zones.get(id).is_some_and(|zone| zone.owner == Owner::Integration) {
            return Err(UpdateZoneError::NotOwned);
        }

        if store_snapshot.zones.values().any(|zone| zone.id != id && zone.name.eq_ignore_ascii_case(name)) {
            return Err(UpdateZoneError::NameAlreadyExists);
        }

        if group_ids.is_empty() && device_ids.is_empty() {
            return Err(UpdateZoneError::ZoneEmpty);
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
            return Err(UpdateZoneError::UnknownGroups { group_ids: unknown_group_ids });
        }

        let unknown_device_ids: BTreeSet<String> = zone.unknown_device_ids(&store_snapshot.devices).cloned().collect();
        if !unknown_device_ids.is_empty() {
            return Err(UpdateZoneError::UnknownDevices { device_ids: unknown_device_ids });
        }

        let permit = self.store_tx.reserve().await.map_err(|_| UpdateZoneError::StoreUnavailable)?;
        let stored_zone =
            self.zone_repository.update(id, base_revision, name, &zone.group_ids, &zone.device_ids).await?;
        permit.send(Event::ZoneChanged(ZoneChanged {
            id: id.to_string(),
            name: Some(name.to_string()),
            group_ids: Some(zone.group_ids),
            device_ids: Some(zone.device_ids),
            revision: Some(stored_zone.revision),
            external_id: None,
        }));
        Ok(stored_zone)
    }

    pub async fn delete_zone(&self, id: &str, base_revision: u64) -> Result<(), DeleteZoneError> {
        let store_snapshot = self.snapshot_rx.borrow().clone(); // Cheap
        let zone = store_snapshot.zones.get(id).ok_or(DeleteZoneError::NotFound)?;
        if zone.owner == Owner::Integration {
            return Err(DeleteZoneError::NotOwned);
        }
        if zone.revision != base_revision {
            return Err(DeleteZoneError::RevisionConflict { base_revision, current_revision: zone.revision });
        }

        let flows = self.flow_registry.flows_referencing_zone(id);
        if !flows.is_empty() {
            return Err(DeleteZoneError::InUse(flows));
        }

        let permit = self.store_tx.reserve().await.map_err(|_| DeleteZoneError::StoreUnavailable)?;
        self.zone_repository.delete(id, base_revision).await?;
        permit.send(Event::ZoneRemoved { zone_id: id.to_string() });

        Ok(())
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
    Internal(#[from] ZoneRepositoryError),
}

#[derive(Debug, Error)]
pub enum UpdateZoneError {
    #[error("zone is owned by an integration and cannot be updated")]
    NotOwned,
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
    #[error("zone not found")]
    NotFound,
    #[error(
        "revision conflict: update is based on revision {base_revision}, but the current revision is {current_revision}"
    )]
    RevisionConflict { base_revision: u64, current_revision: u64 },
    #[error(transparent)]
    Internal(#[from] ZoneRepositoryError),
}

impl From<UpdateError> for UpdateZoneError {
    fn from(err: UpdateError) -> Self {
        match err {
            UpdateError::NotFound => Self::NotFound,
            UpdateError::RevisionConflict { base_revision, current_revision } => {
                Self::RevisionConflict { base_revision, current_revision }
            }
            UpdateError::NameAlreadyExists => Self::NameAlreadyExists,
            UpdateError::Repository(err) => Self::Internal(err),
        }
    }
}

#[derive(Debug, Error)]
pub enum DeleteZoneError {
    #[error("zone not found")]
    NotFound,
    #[error("zone not owned")]
    NotOwned,
    #[error(
        "revision conflict: delete is based on revision {base_revision}, but the current revision is {current_revision}"
    )]
    RevisionConflict { base_revision: u64, current_revision: u64 },
    #[error("zone in use by {} flow(s)", .0.len())]
    InUse(Vec<FlowSummary>),
    #[error("store is unavailable")]
    StoreUnavailable,
    #[error(transparent)]
    Internal(#[from] ZoneRepositoryError),
}

impl From<DeleteError> for DeleteZoneError {
    fn from(err: DeleteError) -> Self {
        match err {
            DeleteError::NotFound => Self::NotFound,
            DeleteError::RevisionConflict { base_revision, current_revision } => {
                Self::RevisionConflict { base_revision, current_revision }
            }
            DeleteError::Repository(err) => Self::Internal(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use crate::domain::{Group, Owner};
    use crate::flow_engine::VersionedFlow;
    use crate::test_support::{DeviceBuilder, flow_using_zones};
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
        ZoneService::new(rx, Arc::new(ZoneRepository::new(database)), Arc::new(FlowRegistry::new(vec![])), store_tx)
    }

    /// Bind `store_rx` when the service reserves a permit: dropping it, e.g. by leaving it out with `..`,
    /// makes the store unavailable.
    struct Fixture {
        service: ZoneService,
        zone_repository: Arc<ZoneRepository>,
        store_rx: mpsc::Receiver<Event>,
    }

    fn create_service(snapshot: StoreSnapshot) -> Fixture {
        create_service_with_flows(snapshot, vec![])
    }

    fn create_service_with_flows(snapshot: StoreSnapshot, flows: Vec<VersionedFlow>) -> Fixture {
        let database = Database::open(Path::new(":memory:")).expect("failed to open in-memory database");
        let zone_repository = Arc::new(ZoneRepository::new(database));
        let (store_tx, store_rx) = mpsc::channel(8);
        let (_tx, rx) = watch::channel(snapshot);
        let flow_registry = Arc::new(FlowRegistry::new(flows));
        let service = ZoneService::new(rx, Arc::clone(&zone_repository), flow_registry, store_tx);
        Fixture { service, zone_repository, store_rx }
    }

    /// A service whose `zones` table is gone, so every database call fails
    async fn create_service_with_broken_database(snapshot: StoreSnapshot) -> (ZoneService, mpsc::Receiver<Event>) {
        let database = Database::open(Path::new(":memory:")).expect("failed to open in-memory database");
        let (store_tx, store_rx) = mpsc::channel(8);
        let (_tx, rx) = watch::channel(snapshot);
        let flow_registry = Arc::new(FlowRegistry::new(vec![]));
        let service = ZoneService::new(rx, Arc::new(ZoneRepository::new(database.clone())), flow_registry, store_tx);
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
        let Fixture { service, zone_repository, store_rx: _store_rx } = create_service(snapshot_with(&["g"], &["a"]));

        service.create_zone("zone", "Downstairs", ids(&["g"]), ids(&["a"])).await.unwrap();

        let stored = zone_repository.list().await.unwrap();
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
        let Fixture { service, zone_repository, mut store_rx } = create_service(StoreSnapshot::default());

        let result = service.create_zone("zone", "Empty", vec![], vec![]).await;

        assert!(matches!(result, Err(CreateZoneError::ZoneEmpty)), "got {result:?}");
        assert!(zone_repository.list().await.unwrap().is_empty(), "nothing must be stored");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn create_zone_rejects_an_existing_id() {
        let snapshot =
            StoreSnapshot { zones: snapshot(vec![zone("zone", "Existing")]).zones, ..snapshot_with(&[], &["a"]) };
        let Fixture { service, zone_repository, mut store_rx } = create_service(snapshot);

        let result = service.create_zone("zone", "Downstairs", vec![], ids(&["a"])).await;

        assert!(matches!(result, Err(CreateZoneError::IdAlreadyExists)), "got {result:?}");
        assert!(zone_repository.list().await.unwrap().is_empty(), "nothing must be stored");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[rstest]
    #[case::same_case("Downstairs")]
    #[case::different_case("downstairs")]
    #[tokio::test]
    async fn create_zone_rejects_a_name_in_use_by_another_zone(#[case] name: &str) {
        let snapshot =
            StoreSnapshot { zones: snapshot(vec![zone("other", "Downstairs")]).zones, ..snapshot_with(&[], &["a"]) };
        let Fixture { service, zone_repository, mut store_rx } = create_service(snapshot);

        let result = service.create_zone("zone", name, vec![], ids(&["a"])).await;

        assert!(matches!(result, Err(CreateZoneError::NameAlreadyExists)), "got {result:?}");
        assert!(zone_repository.list().await.unwrap().is_empty(), "nothing must be stored");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn create_zone_rejects_a_name_stored_but_not_yet_in_the_snapshot() {
        let Fixture { service, zone_repository, mut store_rx } = create_service(snapshot_with(&[], &["a"]));
        zone_repository.insert("other", "Downstairs", &set(&[]), &set(&["a"])).await.unwrap();

        let result = service.create_zone("zone", "downstairs", vec![], ids(&["a"])).await;

        assert!(matches!(result, Err(CreateZoneError::NameAlreadyExists)), "got {result:?}");
        assert_eq!(zone_repository.list().await.unwrap().len(), 1);
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn create_zone_rejects_unknown_groups_and_lists_all_of_them_sorted() {
        let Fixture { service, zone_repository, mut store_rx } = create_service(snapshot_with(&["known"], &[]));

        let result = service.create_zone("zone", "Downstairs", ids(&["z", "known", "a"]), vec![]).await;

        let Err(err @ CreateZoneError::UnknownGroups { .. }) = result else {
            panic!("expected UnknownGroups, got {result:?}");
        };
        assert_eq!(err.to_string(), "unknown group ids: 'a, z'");
        assert!(zone_repository.list().await.unwrap().is_empty(), "nothing must be stored");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn create_zone_rejects_unknown_devices_and_lists_all_of_them_sorted() {
        let Fixture { service, zone_repository, mut store_rx } = create_service(snapshot_with(&[], &["known"]));

        let result = service.create_zone("zone", "Downstairs", vec![], ids(&["z", "known", "a"])).await;

        let Err(err @ CreateZoneError::UnknownDevices { .. }) = result else {
            panic!("expected UnknownDevices, got {result:?}");
        };
        assert_eq!(err.to_string(), "unknown device ids: 'a, z'");
        assert!(zone_repository.list().await.unwrap().is_empty(), "nothing must be stored");
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
        let Fixture { service, zone_repository, store_rx } = create_service(snapshot_with(&[], &["a"]));
        drop(store_rx);

        let result = service.create_zone("zone", "Downstairs", vec![], ids(&["a"])).await;

        assert!(matches!(result, Err(CreateZoneError::StoreUnavailable)), "got {result:?}");
        assert!(zone_repository.list().await.unwrap().is_empty(), "nothing must be stored");
    }

    #[tokio::test]
    async fn create_zone_returns_an_internal_error_when_the_database_fails() {
        let (service, mut store_rx) = create_service_with_broken_database(snapshot_with(&[], &["a"])).await;

        let result = service.create_zone("zone", "Downstairs", vec![], ids(&["a"])).await;

        assert!(matches!(result, Err(CreateZoneError::Internal(_))), "got {result:?}");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    fn hearth_zone(id: &str, name: &str) -> Arc<Zone> {
        Arc::new(Zone { owner: Owner::Hearth, ..Arc::unwrap_or_clone(zone(id, name)) })
    }

    /// A service with a stored Hearth zone "zone" named "Downstairs" at revision 0 with group "g" and device "a",
    /// that is also in the snapshot next to `other_zones`. The snapshot knows the given groups and devices.
    async fn create_service_with_zone_and_snapshot_zones(
        group_ids: &[&str],
        device_ids: &[&str],
        other_zones: Vec<Arc<Zone>>,
    ) -> Fixture {
        let stored = Arc::new(Zone {
            group_ids: set(&["g"]),
            device_ids: set(&["a"]),
            ..(*hearth_zone("zone", "Downstairs")).clone()
        });
        let zones = snapshot(other_zones.into_iter().chain([stored]).collect()).zones;
        let fixture = create_service(StoreSnapshot { zones, ..snapshot_with(group_ids, device_ids) });
        fixture.zone_repository.insert("zone", "Downstairs", &set(&["g"]), &set(&["a"])).await.unwrap();
        fixture
    }

    async fn create_service_with_zone(group_ids: &[&str], device_ids: &[&str]) -> Fixture {
        create_service_with_zone_and_snapshot_zones(group_ids, device_ids, vec![]).await
    }

    async fn assert_zone_unchanged(zone_repository: &ZoneRepository) {
        let stored = zone_repository.list().await.unwrap();
        let zone = stored.iter().find(|zone| zone.id == "zone").expect("the zone is stored");
        assert_eq!((zone.name.as_str(), zone.revision), ("Downstairs", 0), "the zone must be unchanged");
        assert_eq!((&zone.group_ids, &zone.device_ids), (&set(&["g"]), &set(&["a"])));
    }

    #[tokio::test]
    async fn update_zone_returns_the_stored_zone_with_the_next_revision() {
        let Fixture { service, store_rx: _store_rx, .. } = create_service_with_zone(&["g", "h"], &["a", "b"]).await;

        let stored =
            service.update_zone("zone", 0, "Upstairs", ids(&["h", "g", "h"]), ids(&["b", "a", "b"])).await.unwrap();

        assert_eq!(stored.id, "zone");
        assert_eq!(stored.name, "Upstairs");
        assert_eq!(stored.group_ids, set(&["g", "h"]));
        assert_eq!(stored.device_ids, set(&["a", "b"]));
        assert_eq!(stored.revision, 1);
    }

    #[tokio::test]
    async fn update_zone_persists_the_changes() {
        let Fixture { service, zone_repository, store_rx: _store_rx } =
            create_service_with_zone(&["g", "h"], &["a", "b"]).await;

        service.update_zone("zone", 0, "Upstairs", ids(&["h"]), ids(&["b"])).await.unwrap();

        let stored = zone_repository.list().await.unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!((stored[0].name.as_str(), stored[0].revision), ("Upstairs", 1));
        assert_eq!((&stored[0].group_ids, &stored[0].device_ids), (&set(&["h"]), &set(&["b"])));
    }

    #[tokio::test]
    async fn update_zone_sends_a_zone_changed_event_with_the_new_revision() {
        let Fixture { service, mut store_rx, .. } = create_service_with_zone(&["g", "h"], &["a", "b"]).await;

        service.update_zone("zone", 0, "Upstairs", ids(&["h", "g"]), ids(&["b", "a"])).await.unwrap();

        let event = store_rx.try_recv().expect("an event was sent");
        assert_eq!(
            event,
            Event::ZoneChanged(ZoneChanged {
                id: "zone".to_string(),
                name: Some("Upstairs".to_string()),
                group_ids: Some(set(&["g", "h"])),
                device_ids: Some(set(&["a", "b"])),
                revision: Some(1),
                external_id: None,
            })
        );
        assert!(store_rx.try_recv().is_err(), "only one event must be sent");
    }

    #[rstest]
    #[case::only_groups(&["g"], &[])]
    #[case::only_devices(&[], &["a"])]
    #[tokio::test]
    async fn update_zone_accepts_a_zone_with_only_groups_or_only_devices(
        #[case] group_ids: &[&str],
        #[case] device_ids: &[&str],
    ) {
        let Fixture { service, store_rx: _store_rx, .. } = create_service_with_zone(&["g"], &["a"]).await;

        let stored = service.update_zone("zone", 0, "Upstairs", ids(group_ids), ids(device_ids)).await.unwrap();

        assert_eq!((stored.group_ids, stored.device_ids), (set(group_ids), set(device_ids)));
    }

    #[rstest]
    #[case::same_case("Downstairs")]
    #[case::different_case("downstairs")]
    #[tokio::test]
    async fn update_zone_allows_the_zone_to_keep_its_own_name(#[case] name: &str) {
        let Fixture { service, store_rx: _store_rx, .. } = create_service_with_zone(&["g"], &["a"]).await;

        let stored = service.update_zone("zone", 0, name, ids(&["g"]), vec![]).await.unwrap();

        assert_eq!(stored.name, name);
    }

    #[tokio::test]
    async fn update_zone_rejects_a_zone_without_groups_and_devices() {
        let Fixture { service, zone_repository, mut store_rx } = create_service_with_zone(&["g"], &["a"]).await;

        let result = service.update_zone("zone", 0, "Upstairs", vec![], vec![]).await;

        assert!(matches!(result, Err(UpdateZoneError::ZoneEmpty)), "got {result:?}");
        assert_zone_unchanged(&zone_repository).await;
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn update_zone_rejects_unknown_groups_and_lists_all_of_them_sorted() {
        let Fixture { service, zone_repository, mut store_rx } =
            create_service_with_zone(&["known", "g"], &["a"]).await;

        let result = service.update_zone("zone", 0, "Upstairs", ids(&["z", "known", "b"]), vec![]).await;

        let Err(err @ UpdateZoneError::UnknownGroups { .. }) = result else {
            panic!("expected UnknownGroups, got {result:?}");
        };
        assert_eq!(err.to_string(), "unknown group ids: 'b, z'");
        assert_zone_unchanged(&zone_repository).await;
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn update_zone_rejects_unknown_devices_and_lists_all_of_them_sorted() {
        let Fixture { service, zone_repository, mut store_rx } =
            create_service_with_zone(&["g"], &["known", "a"]).await;

        let result = service.update_zone("zone", 0, "Upstairs", vec![], ids(&["z", "known", "b"])).await;

        let Err(err @ UpdateZoneError::UnknownDevices { .. }) = result else {
            panic!("expected UnknownDevices, got {result:?}");
        };
        assert_eq!(err.to_string(), "unknown device ids: 'b, z'");
        assert_zone_unchanged(&zone_repository).await;
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn update_zone_does_not_mistake_a_group_id_for_a_device_id_or_the_other_way_around() {
        let Fixture { service, store_rx: _store_rx, .. } = create_service_with_zone(&["g"], &["a"]).await;

        let result = service.update_zone("zone", 0, "Upstairs", ids(&["a"]), ids(&["g"])).await;

        assert!(matches!(result, Err(UpdateZoneError::UnknownGroups { .. })), "got {result:?}");
    }

    #[tokio::test]
    async fn update_zone_reports_unknown_groups_before_unknown_devices() {
        let Fixture { service, store_rx: _store_rx, .. } = create_service_with_zone(&["g"], &["a"]).await;

        let result = service.update_zone("zone", 0, "Upstairs", ids(&["x"]), ids(&["y"])).await;

        assert!(matches!(result, Err(UpdateZoneError::UnknownGroups { .. })), "got {result:?}");
    }

    #[tokio::test]
    async fn update_zone_returns_not_found_for_an_unknown_id() {
        let Fixture { service, zone_repository, mut store_rx } = create_service_with_zone(&["g"], &["a"]).await;

        let result = service.update_zone("unknown", 0, "Upstairs", ids(&["g"]), vec![]).await;

        assert!(matches!(result, Err(UpdateZoneError::NotFound)), "got {result:?}");
        assert_eq!(zone_repository.list().await.unwrap().len(), 1, "update must not create a zone");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[rstest]
    #[case::behind(1)]
    #[case::ahead(5)]
    #[tokio::test]
    async fn update_zone_returns_a_revision_conflict_for_a_mismatching_base_revision(#[case] base_revision: u64) {
        let Fixture { service, zone_repository, mut store_rx } = create_service_with_zone(&["g"], &["a"]).await;

        let result = service.update_zone("zone", base_revision, "Upstairs", ids(&["g"]), vec![]).await;

        assert!(
            matches!(
                result,
                Err(UpdateZoneError::RevisionConflict { base_revision: b, current_revision: 0 }) if b == base_revision
            ),
            "got {result:?}"
        );
        assert_zone_unchanged(&zone_repository).await;
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[rstest]
    #[case::same_case("Other")]
    #[case::different_case("oTHER")]
    #[tokio::test]
    async fn update_zone_rejects_a_name_in_use_by_another_zone(#[case] name: &str) {
        let Fixture { service, zone_repository, mut store_rx } = create_service_with_zone(&["g"], &["a"]).await;
        zone_repository.insert("other", "Other", &set(&["g"]), &set(&[])).await.unwrap();

        let result = service.update_zone("zone", 0, name, ids(&["g"]), vec![]).await;

        assert!(matches!(result, Err(UpdateZoneError::NameAlreadyExists)), "got {result:?}");
        assert_zone_unchanged(&zone_repository).await;
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[rstest]
    #[case::same_case("Hue zone")]
    #[case::different_case("hUE zONE")]
    #[tokio::test]
    async fn update_zone_rejects_a_name_in_use_by_an_integration_zone(#[case] name: &str) {
        let Fixture { service, zone_repository, mut store_rx } =
            create_service_with_zone_and_snapshot_zones(&["g"], &["a"], vec![zone("hue", "Hue zone")]).await;

        let result = service.update_zone("zone", 0, name, ids(&["g"]), vec![]).await;

        assert!(matches!(result, Err(UpdateZoneError::NameAlreadyExists)), "got {result:?}");
        assert_zone_unchanged(&zone_repository).await;
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn update_zone_rejects_a_zone_owned_by_an_integration() {
        let Fixture { service, zone_repository, mut store_rx } =
            create_service_with_zone_and_snapshot_zones(&["g"], &["a"], vec![zone("hue", "Hue zone")]).await;

        let result = service.update_zone("hue", 0, "Renamed", ids(&["g"]), vec![]).await;

        assert!(matches!(result, Err(UpdateZoneError::NotOwned)), "got {result:?}");
        assert_zone_unchanged(&zone_repository).await;
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn update_zone_rejects_a_zone_owned_by_an_integration_before_checking_the_name_and_members() {
        let Fixture { service, store_rx: _store_rx, .. } =
            create_service_with_zone_and_snapshot_zones(&["g"], &["a"], vec![zone("hue", "Hue zone")]).await;

        let result = service.update_zone("hue", 0, "Downstairs", ids(&["x"]), ids(&["y"])).await;

        assert!(matches!(result, Err(UpdateZoneError::NotOwned)), "got {result:?}");
    }

    #[tokio::test]
    async fn update_zone_fails_without_storing_when_the_store_is_gone() {
        let Fixture { service, zone_repository, store_rx } = create_service_with_zone(&["g"], &["a"]).await;
        drop(store_rx);

        let result = service.update_zone("zone", 0, "Upstairs", ids(&["g"]), vec![]).await;

        assert!(matches!(result, Err(UpdateZoneError::StoreUnavailable)), "got {result:?}");
        assert_zone_unchanged(&zone_repository).await;
    }

    #[tokio::test]
    async fn update_zone_returns_an_internal_error_when_the_database_fails() {
        let (service, mut store_rx) = create_service_with_broken_database(snapshot_with(&["g"], &["a"])).await;

        let result = service.update_zone("zone", 0, "Upstairs", ids(&["g"]), vec![]).await;

        assert!(matches!(result, Err(UpdateZoneError::Internal(_))), "got {result:?}");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    /// A service with a stored Hearth zone "zone" named "Downstairs" at revision 0, that is also in the snapshot
    async fn create_service_for_delete() -> Fixture {
        create_service_for_delete_with_flows(vec![]).await
    }

    async fn create_service_for_delete_with_flows(flows: Vec<VersionedFlow>) -> Fixture {
        let fixture = create_service_with_flows(snapshot(vec![hearth_zone("zone", "Downstairs")]), flows);
        fixture.zone_repository.insert("zone", "Downstairs", &set(&["g"]), &set(&[])).await.unwrap();
        fixture
    }

    #[tokio::test]
    async fn delete_zone_removes_the_stored_zone() {
        let Fixture { service, zone_repository, store_rx: _store_rx } = create_service_for_delete().await;

        service.delete_zone("zone", 0).await.unwrap();

        assert!(zone_repository.list().await.unwrap().is_empty(), "the zone must be deleted");
    }

    #[tokio::test]
    async fn delete_zone_only_removes_the_given_zone() {
        let Fixture { service, zone_repository, store_rx: _store_rx } = create_service_for_delete().await;
        zone_repository.insert("other", "Other", &set(&["g"]), &set(&[])).await.unwrap();

        service.delete_zone("zone", 0).await.unwrap();

        let stored = zone_repository.list().await.unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].id, "other");
    }

    #[tokio::test]
    async fn delete_zone_sends_a_zone_removed_event() {
        let Fixture { service, mut store_rx, .. } = create_service_for_delete().await;

        service.delete_zone("zone", 0).await.unwrap();

        assert_eq!(store_rx.try_recv().expect("an event was sent"), Event::ZoneRemoved { zone_id: "zone".to_string() });
        assert!(store_rx.try_recv().is_err(), "only one event must be sent");
    }

    #[tokio::test]
    async fn delete_zone_returns_not_found_for_an_unknown_id() {
        let Fixture { service, zone_repository, mut store_rx } = create_service_for_delete().await;

        let result = service.delete_zone("unknown", 0).await;

        assert!(matches!(result, Err(DeleteZoneError::NotFound)), "got {result:?}");
        assert_eq!(zone_repository.list().await.unwrap().len(), 1, "nothing must be deleted");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn delete_zone_returns_not_found_when_the_zone_is_in_the_snapshot_but_not_stored() {
        let Fixture { service, mut store_rx, .. } = create_service(snapshot(vec![hearth_zone("zone", "Downstairs")]));

        let result = service.delete_zone("zone", 0).await;

        assert!(matches!(result, Err(DeleteZoneError::NotFound)), "got {result:?}");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn delete_zone_rejects_a_zone_owned_by_an_integration() {
        let Fixture { service, mut store_rx, .. } = create_service(snapshot(vec![zone("zone", "Downstairs")]));

        let result = service.delete_zone("zone", 0).await;

        assert!(matches!(result, Err(DeleteZoneError::NotOwned)), "got {result:?}");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[rstest]
    #[case::behind(1)]
    #[case::ahead(5)]
    #[tokio::test]
    async fn delete_zone_returns_a_revision_conflict_for_a_mismatching_base_revision(#[case] base_revision: u64) {
        let Fixture { service, zone_repository, mut store_rx } = create_service_for_delete().await;

        let result = service.delete_zone("zone", base_revision).await;

        assert!(
            matches!(
                result,
                Err(DeleteZoneError::RevisionConflict { base_revision: b, current_revision: 0 }) if b == base_revision
            ),
            "got {result:?}"
        );
        assert_eq!(zone_repository.list().await.unwrap().len(), 1, "nothing must be deleted");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn delete_zone_returns_a_revision_conflict_when_the_snapshot_is_behind_the_database() {
        let Fixture { service, zone_repository, mut store_rx } = create_service_for_delete().await;
        zone_repository.update("zone", 0, "Downstairs", &set(&["g"]), &set(&[])).await.unwrap();

        let result = service.delete_zone("zone", 0).await;

        assert!(
            matches!(result, Err(DeleteZoneError::RevisionConflict { base_revision: 0, current_revision: 1 })),
            "got {result:?}"
        );
        assert_eq!(zone_repository.list().await.unwrap().len(), 1, "nothing must be deleted");
        assert!(store_rx.try_recv().is_err(), "no event must be sent after a failed delete");
    }

    #[tokio::test]
    async fn delete_zone_fails_without_deleting_when_the_store_is_gone() {
        let Fixture { service, zone_repository, store_rx } = create_service_for_delete().await;
        drop(store_rx);

        let result = service.delete_zone("zone", 0).await;

        assert!(matches!(result, Err(DeleteZoneError::StoreUnavailable)), "got {result:?}");
        assert_eq!(zone_repository.list().await.unwrap().len(), 1, "nothing must be deleted");
    }

    #[tokio::test]
    async fn delete_zone_returns_an_internal_error_when_the_database_fails() {
        let (service, mut store_rx) =
            create_service_with_broken_database(snapshot(vec![hearth_zone("zone", "Downstairs")])).await;

        let result = service.delete_zone("zone", 0).await;

        assert!(matches!(result, Err(DeleteZoneError::Internal(_))), "got {result:?}");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn delete_zone_returns_in_use_with_the_flows_using_the_zone_ordered_by_id() {
        let flows = vec![
            flow_using_zones("b", "Flow B", 2, &["zone"]),
            flow_using_zones("a", "Flow A", 0, &["other", "zone"]),
            flow_using_zones("c", "Flow C", 1, &["other"]),
        ];
        let Fixture { service, zone_repository, mut store_rx } = create_service_for_delete_with_flows(flows).await;

        let result = service.delete_zone("zone", 0).await;

        let Err(err @ DeleteZoneError::InUse(_)) = result else {
            panic!("expected InUse, got {result:?}");
        };
        assert_eq!(err.to_string(), "zone in use by 2 flow(s)");
        let DeleteZoneError::InUse(summaries) = err else { unreachable!() };
        let summaries: Vec<_> = summaries.iter().map(|s| (s.id.as_str(), s.name.as_str(), s.revision)).collect();
        assert_eq!(summaries, [("a", "Flow A", 0), ("b", "Flow B", 2)]);
        assert_eq!(zone_repository.list().await.unwrap().len(), 1, "nothing must be deleted");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn delete_zone_returns_not_found_before_checking_the_flows() {
        let flows = vec![flow_using_zones("a", "Flow A", 0, &["unknown"])];
        let Fixture { service, .. } = create_service_with_flows(snapshot(vec![]), flows);

        let result = service.delete_zone("unknown", 0).await;

        assert!(matches!(result, Err(DeleteZoneError::NotFound)), "got {result:?}");
    }

    #[tokio::test]
    async fn delete_zone_rejects_a_zone_owned_by_an_integration_before_checking_the_flows() {
        let flows = vec![flow_using_zones("a", "Flow A", 0, &["hue"])];
        let Fixture { service, .. } = create_service_with_flows(snapshot(vec![zone("hue", "Hue zone")]), flows);

        let result = service.delete_zone("hue", 0).await;

        assert!(matches!(result, Err(DeleteZoneError::NotOwned)), "got {result:?}");
    }

    #[tokio::test]
    async fn delete_zone_returns_a_revision_conflict_before_checking_the_flows() {
        let flows = vec![flow_using_zones("a", "Flow A", 0, &["zone"])];
        let Fixture { service, .. } = create_service_for_delete_with_flows(flows).await;

        let result = service.delete_zone("zone", 1).await;

        assert!(
            matches!(result, Err(DeleteZoneError::RevisionConflict { base_revision: 1, current_revision: 0 })),
            "got {result:?}"
        );
    }

    #[tokio::test]
    async fn delete_zone_ignores_flows_using_other_zones() {
        let flows = vec![flow_using_zones("a", "Flow A", 0, &["other"])];
        let Fixture { service, zone_repository, store_rx: _store_rx } =
            create_service_for_delete_with_flows(flows).await;

        service.delete_zone("zone", 0).await.unwrap();

        assert!(zone_repository.list().await.unwrap().is_empty(), "the zone must be deleted");
    }
}
