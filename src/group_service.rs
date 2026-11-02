use crate::domain::{Group, GroupOwner};
use crate::group_store::{GroupStore, GroupStoreError, InsertError, StoredGroup};
use crate::store::StoreSnapshot;
use std::collections::BTreeSet;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::watch::Receiver;

#[derive(Debug)]
pub struct GroupService {
    snapshot_rx: Receiver<StoreSnapshot>,
    group_store: Arc<GroupStore>,
}

impl GroupService {
    pub fn new(snapshot_rx: Receiver<StoreSnapshot>, group_store: Arc<GroupStore>) -> Self {
        GroupService { snapshot_rx, group_store }
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

        self.group_store.insert(id, name, &group.device_ids).await.map_err(|err| match err {
            InsertError::NameAlreadyExists => CreateGroupError::NameAlreadyExists,
            InsertError::Store(err) => CreateGroupError::Internal(err),
        })
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
    #[error("a preset with this name already exists")]
    NameAlreadyExists,
    #[error("unknown device ids: '{}'", device_ids.iter().map(String::as_str).collect::<Vec<_>>().join(", "))]
    UnknownDevices { device_ids: BTreeSet<String> },
    #[error(transparent)]
    Internal(#[from] GroupStoreError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use crate::domain::GroupOwner;
    use std::collections::{BTreeSet, HashMap};
    use std::path::Path;
    use tokio::sync::watch;

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
        GroupService::new(rx, Arc::new(GroupStore::new(database)))
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
}
