use crate::domain::{Event, Group, GroupOwner};
use crate::group_store::{GroupStore, GroupStoreError, InsertError, StoredGroup};
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
    store_tx: Sender<Event>,
}

impl GroupService {
    pub fn new(snapshot_rx: Receiver<StoreSnapshot>, group_store: Arc<GroupStore>, store_tx: Sender<Event>) -> Self {
        GroupService { snapshot_rx, group_store, store_tx }
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
}

#[derive(Debug, Error)]
pub enum RetrieveGroupError {
    #[error("group not found")]
    NotFound,
}

#[derive(Debug, Error)]
pub enum CreateGroupError {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use crate::domain::GroupOwner;
    use crate::test_support::DeviceBuilder;
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
        GroupService::new(rx, Arc::new(GroupStore::new(database)), store_tx)
    }

    /// Bind `store_rx` when the service sends events: dropping it, e.g. by leaving it out with `..`,
    /// makes the store unavailable.
    struct Fixture {
        service: GroupService,
        group_store: Arc<GroupStore>,
        store_rx: mpsc::Receiver<Event>,
    }

    fn create_service(snapshot: StoreSnapshot) -> Fixture {
        let database = Database::open(Path::new(":memory:")).expect("failed to open in-memory database");
        let group_store = Arc::new(GroupStore::new(database));
        let (store_tx, store_rx) = mpsc::channel(8);
        let (_tx, rx) = watch::channel(snapshot);
        let service = GroupService::new(rx, Arc::clone(&group_store), store_tx);
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
    async fn create_group_allows_a_group_without_devices() {
        let Fixture { service, store_rx: _store_rx, .. } = create_service(StoreSnapshot::default());

        let stored = service.create_group("group", "Empty", vec![]).await.unwrap();

        assert!(stored.device_ids.is_empty());
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

        let result = service.create_group("group", "Kitchen", vec![]).await;

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

        let result = service.create_group("group", name, vec![]).await;

        assert!(matches!(result, Err(CreateGroupError::NameAlreadyExists)));
        assert!(group_store.list().await.unwrap().is_empty(), "nothing must be stored");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn create_group_rejects_a_name_stored_but_not_yet_in_the_snapshot() {
        let Fixture { service, group_store, mut store_rx } = create_service(StoreSnapshot::default());
        group_store.insert("other", "Kitchen", &BTreeSet::new()).await.unwrap();

        let result = service.create_group("group", "kitchen", vec![]).await;

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
        let Fixture { service, group_store, store_rx } = create_service(StoreSnapshot::default());
        drop(store_rx);

        let result = service.create_group("group", "Kitchen", vec![]).await;

        assert!(matches!(result, Err(CreateGroupError::StoreUnavailable)));
        assert!(group_store.list().await.unwrap().is_empty(), "nothing must be stored");
    }
}
