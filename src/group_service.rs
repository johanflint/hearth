use crate::domain::{Event, Group, GroupChanged, GroupOwner};
use crate::flow_registry::{FlowRegistry, FlowSummary};
use crate::group_store::{DeleteError, GroupStore, GroupStoreError, InsertError, StoredGroup, UpdateError};
use crate::store::StoreSnapshot;
use std::collections::BTreeSet;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::mpsc::Sender;
use tokio::sync::watch::Receiver;

#[derive(Debug)]
pub struct GroupService {
    snapshot_rx: Receiver<StoreSnapshot>,
    group_store: Arc<GroupStore>,
    flow_registry: Arc<FlowRegistry>,
    store_tx: Sender<Event>,
}

impl GroupService {
    pub fn new(
        snapshot_rx: Receiver<StoreSnapshot>,
        group_store: Arc<GroupStore>,
        flow_registry: Arc<FlowRegistry>,
        store_tx: Sender<Event>,
    ) -> Self {
        GroupService { snapshot_rx, group_store, flow_registry, store_tx }
    }

    pub fn list(&self) -> Vec<Arc<Group>> {
        let store_snapshot = self.snapshot_rx.borrow().clone(); // Cheap: clones the Arc<GroupMap>
        let mut groups: Vec<_> = store_snapshot.groups.values().cloned().collect();
        // First sort by name, then by id
        groups.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
        groups
    }

    pub fn retrieve_group(&self, id: &str) -> Result<Arc<Group>, RetrieveGroupError> {
        self.snapshot_rx.borrow().groups.get(id).cloned().ok_or(RetrieveGroupError::NotFound)
    }

    pub async fn create_group(
        &self,
        id: &str,
        name: &str,
        device_ids: Vec<String>,
    ) -> Result<StoredGroup, CreateGroupError> {
        if device_ids.is_empty() {
            return Err(CreateGroupError::GroupEmpty);
        }

        let store_snapshot = self.snapshot_rx.borrow().clone(); // Cheap
        if store_snapshot.groups.contains_key(id) {
            return Err(CreateGroupError::IdAlreadyExists);
        }
        if store_snapshot.groups.values().any(|group| group.name.eq_ignore_ascii_case(name)) {
            return Err(CreateGroupError::NameAlreadyExists);
        }
        let group = Group {
            id: id.to_string(),
            name: name.to_string(),
            device_ids: device_ids.into_iter().collect(), // BTreeSet dedupes
            revision: 0,
            owner: GroupOwner::Hearth,
            external_id: None,
        };

        let unknown_device_ids: BTreeSet<String> = group.unknown_device_ids(&store_snapshot.devices).cloned().collect();
        if !unknown_device_ids.is_empty() {
            return Err(CreateGroupError::UnknownDevices { device_ids: unknown_device_ids });
        }

        let permit = self.store_tx.reserve().await.map_err(|_| CreateGroupError::StoreUnavailable)?;

        let stored_group = self.group_store.insert(id, name, &group.device_ids).await.map_err(|err| match err {
            InsertError::NameAlreadyExists => CreateGroupError::NameAlreadyExists,
            InsertError::Store(err) => CreateGroupError::Internal(err),
        })?;

        permit.send(Event::GroupCreated(Group { revision: stored_group.revision, ..group }));
        Ok(stored_group)
    }

    pub async fn update_group(
        &self,
        id: &str,
        base_revision: u64,
        name: &str,
        device_ids: Vec<String>,
    ) -> Result<StoredGroup, UpdateGroupError> {
        if device_ids.is_empty() {
            return Err(UpdateGroupError::GroupEmpty);
        }

        let snapshot = self.snapshot_rx.borrow().clone();
        let unknown_device_ids: BTreeSet<String> =
            device_ids.iter().filter(|id| !snapshot.devices.contains_key(*id)).cloned().collect();
        if !unknown_device_ids.is_empty() {
            return Err(UpdateGroupError::UnknownDevices { device_ids: unknown_device_ids });
        }

        let device_ids: BTreeSet<String> = device_ids.into_iter().collect(); // BTreeSet dedupes
        let permit = self.store_tx.reserve().await.map_err(|_| UpdateGroupError::StoreUnavailable)?;
        let stored_group = self.group_store.update(id, base_revision, name, &device_ids).await?;

        permit.send(Event::GroupChanged(GroupChanged {
            id: id.to_string(),
            name: Some(name.to_string()),
            device_ids: Some(device_ids),
            revision: Some(stored_group.revision),
            external_id: None,
        }));
        Ok(stored_group)
    }

    pub async fn delete_group(&self, id: &str, base_revision: u64) -> Result<(), DeleteGroupError> {
        let store_snapshot = self.snapshot_rx.borrow().clone(); // Cheap
        let group = store_snapshot.groups.get(id).ok_or(DeleteGroupError::NotFound)?;
        if group.owner == GroupOwner::Integration {
            return Err(DeleteGroupError::NotOwned);
        }
        if group.revision != base_revision {
            return Err(DeleteGroupError::RevisionConflict { base_revision, current_revision: group.revision });
        }

        let flows = self.flow_registry.flows_referencing_group(id);
        if !flows.is_empty() {
            return Err(DeleteGroupError::InUse(flows));
        }

        let permit = self.store_tx.reserve().await.map_err(|_| DeleteGroupError::StoreUnavailable)?;
        self.group_store.delete(id, base_revision).await?;
        permit.send(Event::GroupRemoved { group_id: id.to_string() });

        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum RetrieveGroupError {
    #[error("group not found")]
    NotFound,
}

#[derive(Debug, Error)]
pub enum CreateGroupError {
    #[error("a group must contain at least one device")]
    GroupEmpty,
    #[error("a group with this id already exists")]
    IdAlreadyExists,
    #[error("a group with this name already exists")]
    NameAlreadyExists,
    #[error("unknown device ids: '{}'", device_ids.iter().map(String::as_str).collect::<Vec<_>>().join(", "))]
    UnknownDevices { device_ids: BTreeSet<String> },
    #[error("store is unavailable")]
    StoreUnavailable,
    #[error(transparent)]
    Internal(#[from] GroupStoreError),
}

#[derive(Debug, Error)]
pub enum UpdateGroupError {
    #[error("a group must contain at least one device")]
    GroupEmpty,
    #[error("unknown device ids: '{}'", device_ids.iter().map(String::as_str).collect::<Vec<_>>().join(", "))]
    UnknownDevices { device_ids: BTreeSet<String> },
    #[error("group not found")]
    NotFound,
    #[error(
        "revision conflict: update is based on revision {base_revision}, but the current revision is {current_revision}"
    )]
    RevisionConflict { base_revision: u64, current_revision: u64 },
    #[error("a group with this name already exists")]
    NameAlreadyExists,
    #[error("store is unavailable")]
    StoreUnavailable,
    #[error(transparent)]
    Internal(#[from] GroupStoreError),
}

impl From<UpdateError> for UpdateGroupError {
    fn from(err: UpdateError) -> Self {
        match err {
            UpdateError::NotFound => Self::NotFound,
            UpdateError::RevisionConflict { base_revision, current_revision } => {
                Self::RevisionConflict { base_revision, current_revision }
            }
            UpdateError::NameAlreadyExists => Self::NameAlreadyExists,
            UpdateError::Store(err) => Self::Internal(err),
        }
    }
}

#[derive(Debug, Error)]
pub enum DeleteGroupError {
    #[error("group in use by {} flow(s)", .0.len())]
    InUse(Vec<FlowSummary>),
    #[error("group not found")]
    NotFound,
    #[error("group not owned")]
    NotOwned,
    #[error(
        "revision conflict: delete is based on revision {base_revision}, but the current revision is {current_revision}"
    )]
    RevisionConflict { base_revision: u64, current_revision: u64 },
    #[error("store is unavailable")]
    StoreUnavailable,
    #[error(transparent)]
    Internal(#[from] GroupStoreError),
}

impl From<DeleteError> for DeleteGroupError {
    fn from(err: DeleteError) -> Self {
        match err {
            DeleteError::NotFound => Self::NotFound,
            DeleteError::RevisionConflict { base_revision, current_revision } => {
                Self::RevisionConflict { base_revision, current_revision }
            }
            DeleteError::Store(err) => Self::Internal(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use crate::domain::GroupOwner;
    use crate::flow_engine::VersionedFlow;
    use crate::test_support::{DeviceBuilder, flow_using_groups};
    use rstest::rstest;
    use std::collections::{BTreeSet, HashMap};
    use std::path::Path;
    use tokio::sync::{mpsc, watch};

    fn group(id: &str, name: &str) -> Arc<Group> {
        Arc::new(Group {
            id: id.to_string(),
            name: name.to_string(),
            device_ids: BTreeSet::new(),
            revision: 0,
            owner: GroupOwner::Integration,
            external_id: None,
        })
    }

    fn snapshot(groups: Vec<Arc<Group>>) -> StoreSnapshot {
        let groups: HashMap<_, _> = groups.into_iter().map(|group| (group.id.clone(), group)).collect();
        StoreSnapshot { groups: Arc::new(groups), ..Default::default() }
    }

    fn service(rx: Receiver<StoreSnapshot>) -> GroupService {
        let database = Database::open(Path::new(":memory:")).expect("failed to open in-memory database");
        let (store_tx, _store_rx) = mpsc::channel(1);
        GroupService::new(rx, Arc::new(GroupStore::new(database)), Arc::new(FlowRegistry::new(vec![])), store_tx)
    }

    /// Bind `store_rx` when the service sends events: dropping it, e.g. by leaving it out with `..`,
    /// makes the store unavailable.
    struct Fixture {
        service: GroupService,
        group_store: Arc<GroupStore>,
        store_rx: mpsc::Receiver<Event>,
    }

    fn create_service(snapshot: StoreSnapshot) -> Fixture {
        create_service_with_flows(snapshot, vec![])
    }

    fn create_service_with_flows(snapshot: StoreSnapshot, flows: Vec<VersionedFlow>) -> Fixture {
        let database = Database::open(Path::new(":memory:")).expect("failed to open in-memory database");
        let group_store = Arc::new(GroupStore::new(database));
        let (store_tx, store_rx) = mpsc::channel(8);
        let (_tx, rx) = watch::channel(snapshot);
        let service = GroupService::new(rx, Arc::clone(&group_store), Arc::new(FlowRegistry::new(flows)), store_tx);
        Fixture { service, group_store, store_rx }
    }

    fn snapshot_with_devices(device_ids: &[&str]) -> StoreSnapshot {
        let devices: HashMap<_, _> =
            device_ids.iter().map(|id| (id.to_string(), Arc::new(DeviceBuilder::new(id).build()))).collect();
        StoreSnapshot { devices: Arc::new(devices), ..Default::default() }
    }

    fn ids(device_ids: &[&str]) -> Vec<String> {
        device_ids.iter().map(|id| id.to_string()).collect()
    }

    fn set(device_ids: &[&str]) -> BTreeSet<String> {
        device_ids.iter().map(|id| id.to_string()).collect()
    }

    fn names(groups: &[Arc<Group>]) -> Vec<&str> {
        groups.iter().map(|group| group.name.as_str()).collect()
    }

    #[test]
    fn list_returns_nothing_for_an_empty_snapshot() {
        let (_tx, rx) = watch::channel(StoreSnapshot::default());

        assert!(service(rx).list().is_empty());
    }

    #[test]
    fn list_returns_all_groups_in_the_snapshot() {
        let (_tx, rx) = watch::channel(snapshot(vec![group("1", "Kitchen"), group("2", "Hall")]));

        let groups = service(rx).list();

        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn list_orders_the_groups_by_name_instead_of_id() {
        let (_tx, rx) = watch::channel(snapshot(vec![
            group("1", "Living room"),
            group("2", "Bedroom"),
            group("3", "Kitchen"),
            group("4", "Attic"),
        ]));

        let groups = service(rx).list();

        assert_eq!(names(&groups), vec!["Attic", "Bedroom", "Kitchen", "Living room"]);
    }

    #[test]
    fn list_follows_the_latest_snapshot() {
        let (tx, rx) = watch::channel(snapshot(vec![group("1", "Kitchen")]));
        let service = service(rx);

        tx.send(snapshot(vec![group("1", "Kitchen"), group("2", "Hall")])).unwrap();

        assert_eq!(names(&service.list()), vec!["Hall", "Kitchen"]);
    }

    #[test]
    fn list_orders_groups_with_the_same_name_by_id() {
        let (_tx, rx) = watch::channel(snapshot(vec![group("c", "Room"), group("a", "Room"), group("b", "Room")]));

        let groups = service(rx).list();

        let ids: Vec<_> = groups.iter().map(|group| group.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn retrieve_group_returns_the_group_with_the_id() {
        let (_tx, rx) = watch::channel(snapshot(vec![group("1", "Kitchen"), group("2", "Hall")]));

        let found = service(rx).retrieve_group("2").unwrap();

        assert_eq!(found.name, "Hall");
    }

    #[test]
    fn retrieve_group_returns_not_found_for_an_unknown_id() {
        let (_tx, rx) = watch::channel(snapshot(vec![group("1", "Kitchen")]));

        let result = service(rx).retrieve_group("unknown");

        assert!(matches!(result, Err(RetrieveGroupError::NotFound)));
    }

    #[test]
    fn retrieve_group_does_not_match_on_the_name() {
        let (_tx, rx) = watch::channel(snapshot(vec![group("1", "Kitchen")]));

        let result = service(rx).retrieve_group("Kitchen");

        assert!(matches!(result, Err(RetrieveGroupError::NotFound)));
    }

    #[test]
    fn retrieve_group_follows_the_latest_snapshot() {
        let (tx, rx) = watch::channel(StoreSnapshot::default());
        let service = service(rx);

        tx.send(snapshot(vec![group("1", "Kitchen")])).unwrap();

        assert!(service.retrieve_group("1").is_ok());
    }

    #[tokio::test]
    async fn create_group_returns_the_stored_group() {
        let Fixture { service, store_rx: _store_rx, .. } = create_service(snapshot_with_devices(&["a", "b"]));

        let stored = service.create_group("group", "Kitchen", ids(&["b", "a"])).await.unwrap();

        assert_eq!(stored.id, "group");
        assert_eq!(stored.name, "Kitchen");
        assert_eq!(stored.device_ids, set(&["a", "b"]));
        assert_eq!(stored.revision, 0);
    }

    #[tokio::test]
    async fn create_group_persists_the_group() {
        let Fixture { service, group_store, store_rx: _store_rx } = create_service(snapshot_with_devices(&["a"]));

        service.create_group("group", "Kitchen", ids(&["a"])).await.unwrap();

        let stored = group_store.list().await.unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!((stored[0].id.as_str(), stored[0].name.as_str()), ("group", "Kitchen"));
        assert_eq!(stored[0].device_ids, set(&["a"]));
    }

    #[tokio::test]
    async fn create_group_deduplicates_the_device_ids() {
        let Fixture { service, store_rx: _store_rx, .. } = create_service(snapshot_with_devices(&["a"]));

        let stored = service.create_group("group", "Kitchen", ids(&["a", "a"])).await.unwrap();

        assert_eq!(stored.device_ids, set(&["a"]));
    }

    #[tokio::test]
    async fn create_group_rejects_a_group_without_devices() {
        let Fixture { service, group_store, mut store_rx } = create_service(StoreSnapshot::default());

        let result = service.create_group("group", "Empty", vec![]).await;

        assert!(matches!(result, Err(CreateGroupError::GroupEmpty)), "got {result:?}");
        assert!(group_store.list().await.unwrap().is_empty(), "nothing must be stored");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn create_group_sends_a_group_created_event_for_a_hearth_group() {
        let Fixture { service, mut store_rx, .. } = create_service(snapshot_with_devices(&["a"]));

        service.create_group("group", "Kitchen", ids(&["a"])).await.unwrap();

        let Event::GroupCreated(group) = store_rx.try_recv().expect("an event was sent") else {
            panic!("expected a GroupCreated event");
        };
        assert_eq!(
            group,
            Group {
                id: "group".to_string(),
                name: "Kitchen".to_string(),
                device_ids: set(&["a"]),
                revision: 0,
                owner: GroupOwner::Hearth,
                external_id: None,
            }
        );
        assert!(store_rx.try_recv().is_err(), "only one event must be sent");
    }

    #[tokio::test]
    async fn create_group_rejects_an_existing_id() {
        let snapshot = snapshot(vec![group("group", "Existing")]);
        let Fixture { service, group_store, mut store_rx } = create_service(snapshot);

        let result = service.create_group("group", "Kitchen", ids(&["a"])).await;

        assert!(matches!(result, Err(CreateGroupError::IdAlreadyExists)));
        assert!(group_store.list().await.unwrap().is_empty(), "nothing must be stored");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[rstest]
    #[case::same_case("Kitchen")]
    #[case::different_case("kitchen")]
    #[tokio::test]
    async fn create_group_rejects_a_name_in_use_by_another_group(#[case] name: &str) {
        let Fixture { service, group_store, mut store_rx } = create_service(snapshot(vec![group("other", "Kitchen")]));

        let result = service.create_group("group", name, ids(&["a"])).await;

        assert!(matches!(result, Err(CreateGroupError::NameAlreadyExists)));
        assert!(group_store.list().await.unwrap().is_empty(), "nothing must be stored");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn create_group_rejects_a_name_stored_but_not_yet_in_the_snapshot() {
        let Fixture { service, group_store, mut store_rx } = create_service(snapshot_with_devices(&["a"]));
        group_store.insert("other", "Kitchen", &set(&["a"])).await.unwrap();

        let result = service.create_group("group", "kitchen", ids(&["a"])).await;

        assert!(matches!(result, Err(CreateGroupError::NameAlreadyExists)));
        assert_eq!(group_store.list().await.unwrap().len(), 1);
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn create_group_rejects_unknown_devices_and_lists_all_of_them_sorted() {
        let Fixture { service, group_store, mut store_rx } = create_service(snapshot_with_devices(&["known"]));

        let result = service.create_group("group", "Kitchen", ids(&["z", "known", "a"])).await;

        let Err(err @ CreateGroupError::UnknownDevices { .. }) = result else {
            panic!("expected UnknownDevices, got {result:?}");
        };
        assert_eq!(err.to_string(), "unknown device ids: 'a, z'");
        assert!(group_store.list().await.unwrap().is_empty(), "nothing must be stored");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn create_group_fails_without_storing_when_the_store_is_gone() {
        let Fixture { service, group_store, store_rx } = create_service(snapshot_with_devices(&["a"]));
        drop(store_rx);

        let result = service.create_group("group", "Kitchen", ids(&["a"])).await;

        assert!(matches!(result, Err(CreateGroupError::StoreUnavailable)));
        assert!(group_store.list().await.unwrap().is_empty(), "nothing must be stored");
    }

    /// A service with a stored group "group" named "Kitchen" with device "a", and the given devices in the snapshot
    async fn create_service_with_group(device_ids: &[&str]) -> Fixture {
        let fixture = create_service(snapshot_with_devices(device_ids));
        fixture.group_store.insert("group", "Kitchen", &set(&["a"])).await.unwrap();
        fixture
    }

    #[tokio::test]
    async fn update_group_returns_the_stored_group_with_the_next_revision() {
        let Fixture { service, store_rx: _store_rx, .. } = create_service_with_group(&["a", "b"]).await;

        let stored = service.update_group("group", 0, "Hall", ids(&["b", "a", "b"])).await.unwrap();

        assert_eq!(stored.id, "group");
        assert_eq!(stored.name, "Hall");
        assert_eq!(stored.device_ids, set(&["a", "b"]));
        assert_eq!(stored.revision, 1);
    }

    #[tokio::test]
    async fn update_group_persists_the_changes() {
        let Fixture { service, group_store, store_rx: _store_rx } = create_service_with_group(&["a", "b"]).await;

        service.update_group("group", 0, "Hall", ids(&["b"])).await.unwrap();

        let stored = group_store.list().await.unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!((stored[0].name.as_str(), stored[0].revision), ("Hall", 1));
        assert_eq!(stored[0].device_ids, set(&["b"]));
    }

    #[tokio::test]
    async fn update_group_sends_a_group_changed_event_with_the_new_revision() {
        let Fixture { service, mut store_rx, .. } = create_service_with_group(&["a", "b"]).await;

        service.update_group("group", 0, "Hall", ids(&["b", "a"])).await.unwrap();

        let event = store_rx.try_recv().expect("an event was sent");
        assert_eq!(
            event,
            Event::GroupChanged(GroupChanged {
                id: "group".to_string(),
                name: Some("Hall".to_string()),
                device_ids: Some(set(&["a", "b"])),
                revision: Some(1),
                external_id: None,
            })
        );
        assert!(store_rx.try_recv().is_err(), "only one event must be sent");
    }

    #[tokio::test]
    async fn update_group_allows_the_group_to_keep_its_own_name() {
        let Fixture { service, store_rx: _store_rx, .. } = create_service_with_group(&["a"]).await;

        let stored = service.update_group("group", 0, "kitchen", ids(&["a"])).await.unwrap();

        assert_eq!(stored.name, "kitchen");
    }

    #[tokio::test]
    async fn update_group_rejects_a_group_without_devices() {
        let Fixture { service, group_store, mut store_rx } = create_service_with_group(&["a"]).await;

        let result = service.update_group("group", 0, "Hall", vec![]).await;

        assert!(matches!(result, Err(UpdateGroupError::GroupEmpty)), "got {result:?}");
        let stored = group_store.list().await.unwrap();
        assert_eq!((stored[0].name.as_str(), stored[0].revision), ("Kitchen", 0), "the group must be unchanged");
        assert_eq!(stored[0].device_ids, set(&["a"]));
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn update_group_rejects_unknown_devices_and_lists_all_of_them_sorted() {
        let Fixture { service, group_store, mut store_rx } = create_service_with_group(&["known", "a"]).await;

        let result = service.update_group("group", 0, "Hall", ids(&["z", "known", "b"])).await;

        let Err(err @ UpdateGroupError::UnknownDevices { .. }) = result else {
            panic!("expected UnknownDevices, got {result:?}");
        };
        assert_eq!(err.to_string(), "unknown device ids: 'b, z'");
        let stored = group_store.list().await.unwrap();
        assert_eq!((stored[0].name.as_str(), stored[0].revision), ("Kitchen", 0), "the group must be unchanged");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn update_group_returns_not_found_for_an_unknown_id() {
        let Fixture { service, group_store, mut store_rx } = create_service_with_group(&["a"]).await;

        let result = service.update_group("unknown", 0, "Hall", ids(&["a"])).await;

        assert!(matches!(result, Err(UpdateGroupError::NotFound)), "got {result:?}");
        assert_eq!(group_store.list().await.unwrap().len(), 1, "update must not create a group");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[rstest]
    #[case::behind(1)]
    #[case::ahead(5)]
    #[tokio::test]
    async fn update_group_returns_a_revision_conflict_for_a_mismatching_base_revision(#[case] base_revision: u64) {
        let Fixture { service, group_store, mut store_rx } = create_service_with_group(&["a"]).await;

        let result = service.update_group("group", base_revision, "Hall", ids(&["a"])).await;

        assert!(
            matches!(
                result,
                Err(UpdateGroupError::RevisionConflict { base_revision: b, current_revision: 0 }) if b == base_revision
            ),
            "got {result:?}"
        );
        assert_eq!(group_store.list().await.unwrap()[0].name, "Kitchen", "the group must be unchanged");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[rstest]
    #[case::same_case("Other")]
    #[case::different_case("oTHER")]
    #[tokio::test]
    async fn update_group_rejects_a_name_in_use_by_another_group(#[case] name: &str) {
        let Fixture { service, group_store, mut store_rx } = create_service_with_group(&["a"]).await;
        group_store.insert("other", "Other", &BTreeSet::new()).await.unwrap();

        let result = service.update_group("group", 0, name, ids(&["a"])).await;

        assert!(matches!(result, Err(UpdateGroupError::NameAlreadyExists)), "got {result:?}");
        assert_eq!(group_store.list().await.unwrap()[0].name, "Kitchen", "the group must be unchanged");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn update_group_fails_without_storing_when_the_store_is_gone() {
        let Fixture { service, group_store, store_rx } = create_service_with_group(&["a"]).await;
        drop(store_rx);

        let result = service.update_group("group", 0, "Hall", ids(&["a"])).await;

        assert!(matches!(result, Err(UpdateGroupError::StoreUnavailable)), "got {result:?}");
        let stored = group_store.list().await.unwrap();
        assert_eq!((stored[0].name.as_str(), stored[0].revision), ("Kitchen", 0), "nothing must be stored");
    }

    fn hearth_group(id: &str, name: &str, revision: u64) -> Arc<Group> {
        Arc::new(Group { owner: GroupOwner::Hearth, revision, ..Arc::unwrap_or_clone(group(id, name)) })
    }

    /// A service with a stored Hearth group "group" named "Kitchen" at revision 0, that is also in the snapshot
    async fn create_service_for_delete() -> Fixture {
        let fixture = create_service(snapshot(vec![hearth_group("group", "Kitchen", 0)]));
        fixture.group_store.insert("group", "Kitchen", &BTreeSet::new()).await.unwrap();
        fixture
    }

    #[tokio::test]
    async fn delete_group_removes_the_stored_group() {
        let Fixture { service, group_store, store_rx: _store_rx } = create_service_for_delete().await;

        service.delete_group("group", 0).await.unwrap();

        assert!(group_store.list().await.unwrap().is_empty(), "the group must be deleted");
    }

    #[tokio::test]
    async fn delete_group_only_removes_the_given_group() {
        let Fixture { service, group_store, store_rx: _store_rx } = create_service_for_delete().await;
        group_store.insert("other", "Other", &BTreeSet::new()).await.unwrap();

        service.delete_group("group", 0).await.unwrap();

        let stored = group_store.list().await.unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].id, "other");
    }

    #[tokio::test]
    async fn delete_group_sends_a_group_removed_event() {
        let Fixture { service, mut store_rx, .. } = create_service_for_delete().await;

        service.delete_group("group", 0).await.unwrap();

        assert_eq!(
            store_rx.try_recv().expect("an event was sent"),
            Event::GroupRemoved { group_id: "group".to_string() }
        );
        assert!(store_rx.try_recv().is_err(), "only one event must be sent");
    }

    #[tokio::test]
    async fn delete_group_returns_not_found_for_an_unknown_id() {
        let Fixture { service, group_store, mut store_rx } = create_service_for_delete().await;

        let result = service.delete_group("unknown", 0).await;

        assert!(matches!(result, Err(DeleteGroupError::NotFound)), "got {result:?}");
        assert_eq!(group_store.list().await.unwrap().len(), 1, "nothing must be deleted");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn delete_group_returns_not_found_when_the_group_is_in_the_snapshot_but_not_stored() {
        let Fixture { service, mut store_rx, .. } = create_service(snapshot(vec![hearth_group("group", "Kitchen", 0)]));

        let result = service.delete_group("group", 0).await;

        assert!(matches!(result, Err(DeleteGroupError::NotFound)), "got {result:?}");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn delete_group_rejects_a_group_owned_by_an_integration() {
        let Fixture { service, mut store_rx, .. } = create_service(snapshot(vec![group("group", "Kitchen")]));

        let result = service.delete_group("group", 0).await;

        assert!(matches!(result, Err(DeleteGroupError::NotOwned)), "got {result:?}");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[rstest]
    #[case::behind(1)]
    #[case::ahead(5)]
    #[tokio::test]
    async fn delete_group_returns_a_revision_conflict_for_a_mismatching_base_revision(#[case] base_revision: u64) {
        let Fixture { service, group_store, mut store_rx } = create_service_for_delete().await;

        let result = service.delete_group("group", base_revision).await;

        assert!(
            matches!(
                result,
                Err(DeleteGroupError::RevisionConflict { base_revision: b, current_revision: 0 }) if b == base_revision
            ),
            "got {result:?}"
        );
        assert_eq!(group_store.list().await.unwrap().len(), 1, "nothing must be deleted");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn delete_group_returns_a_revision_conflict_when_the_snapshot_is_behind_the_database() {
        let Fixture { service, group_store, mut store_rx } = create_service_for_delete().await;
        group_store.update("group", 0, "Kitchen", &BTreeSet::new()).await.unwrap();

        let result = service.delete_group("group", 0).await;

        assert!(
            matches!(result, Err(DeleteGroupError::RevisionConflict { base_revision: 0, current_revision: 1 })),
            "got {result:?}"
        );
        assert_eq!(group_store.list().await.unwrap().len(), 1, "nothing must be deleted");
        assert!(store_rx.try_recv().is_err(), "no event must be sent after a failed delete");
    }

    #[tokio::test]
    async fn delete_group_fails_without_deleting_when_the_store_is_gone() {
        let Fixture { service, group_store, store_rx } = create_service_for_delete().await;
        drop(store_rx);

        let result = service.delete_group("group", 0).await;

        assert!(matches!(result, Err(DeleteGroupError::StoreUnavailable)), "got {result:?}");
        assert_eq!(group_store.list().await.unwrap().len(), 1, "nothing must be deleted");
    }

    #[tokio::test]
    async fn delete_group_returns_an_internal_error_when_the_database_fails() {
        let database = Database::open(Path::new(":memory:")).expect("failed to open in-memory database");
        let (store_tx, mut store_rx) = mpsc::channel(8);
        let (_tx, rx) = watch::channel(snapshot(vec![hearth_group("group", "Kitchen", 0)]));
        let flow_registry = Arc::new(FlowRegistry::new(vec![]));
        let service = GroupService::new(rx, Arc::new(GroupStore::new(database.clone())), flow_registry, store_tx);
        database
            .call(|conn| conn.execute("DROP TABLE groups", []).map_err(crate::database::DatabaseError::from))
            .await
            .unwrap();

        let result = service.delete_group("group", 0).await;

        assert!(matches!(result, Err(DeleteGroupError::Internal(_))), "got {result:?}");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn delete_group_returns_in_use_with_the_flows_using_the_group_ordered_by_id() {
        let flows = vec![
            flow_using_groups("b", "Flow B", 2, &["group"]),
            flow_using_groups("a", "Flow A", 0, &["other", "group"]),
            flow_using_groups("c", "Flow C", 1, &["other"]),
        ];
        let Fixture { service, group_store, mut store_rx } =
            create_service_with_flows(snapshot(vec![hearth_group("group", "Kitchen", 0)]), flows);
        group_store.insert("group", "Kitchen", &BTreeSet::new()).await.unwrap();

        let result = service.delete_group("group", 0).await;

        let Err(err @ DeleteGroupError::InUse(_)) = result else {
            panic!("expected InUse, got {result:?}");
        };
        assert_eq!(err.to_string(), "group in use by 2 flow(s)");
        let DeleteGroupError::InUse(summaries) = err else { unreachable!() };
        let summaries: Vec<_> = summaries.iter().map(|s| (s.id.as_str(), s.name.as_str(), s.revision)).collect();
        assert_eq!(summaries, [("a", "Flow A", 0), ("b", "Flow B", 2)]);
        assert_eq!(group_store.list().await.unwrap().len(), 1, "nothing must be deleted");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn delete_group_ignores_flows_using_other_groups() {
        let flows = vec![flow_using_groups("a", "Flow A", 0, &["other"])];
        let Fixture { service, group_store, store_rx: _store_rx } =
            create_service_with_flows(snapshot(vec![hearth_group("group", "Kitchen", 0)]), flows);
        group_store.insert("group", "Kitchen", &BTreeSet::new()).await.unwrap();

        service.delete_group("group", 0).await.unwrap();

        assert!(group_store.list().await.unwrap().is_empty(), "the group must be deleted");
    }
}
