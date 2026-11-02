use crate::domain::Group;
use crate::store::StoreSnapshot;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::watch::Receiver;

#[derive(Debug)]
pub struct GroupService {
    snapshot_rx: Receiver<StoreSnapshot>,
}

impl GroupService {
    pub fn new(snapshot_rx: Receiver<StoreSnapshot>) -> Self {
        GroupService { snapshot_rx }
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
}

#[derive(Debug, Error)]
pub enum RetrieveGroupError {
    #[error("group not found")]
    NotFound,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::GroupOwner;
    use std::collections::{BTreeSet, HashMap};
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

    fn names(groups: &[Arc<Group>]) -> Vec<&str> {
        groups.iter().map(|group| group.name.as_str()).collect()
    }

    #[test]
    fn list_returns_nothing_for_an_empty_snapshot() {
        let (_tx, rx) = watch::channel(StoreSnapshot::default());

        assert!(GroupService::new(rx).list().is_empty());
    }

    #[test]
    fn list_returns_all_groups_in_the_snapshot() {
        let (_tx, rx) = watch::channel(snapshot(vec![group("1", "Kitchen"), group("2", "Hall")]));

        let groups = GroupService::new(rx).list();

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

        let groups = GroupService::new(rx).list();

        assert_eq!(names(&groups), vec!["Attic", "Bedroom", "Kitchen", "Living room"]);
    }

    #[test]
    fn list_follows_the_latest_snapshot() {
        let (tx, rx) = watch::channel(snapshot(vec![group("1", "Kitchen")]));
        let service = GroupService::new(rx);

        tx.send(snapshot(vec![group("1", "Kitchen"), group("2", "Hall")])).unwrap();

        assert_eq!(names(&service.list()), vec!["Hall", "Kitchen"]);
    }

    #[test]
    fn list_orders_groups_with_the_same_name_by_id() {
        let (_tx, rx) = watch::channel(snapshot(vec![group("c", "Room"), group("a", "Room"), group("b", "Room")]));

        let groups = GroupService::new(rx).list();

        let ids: Vec<_> = groups.iter().map(|group| group.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn retrieve_group_returns_the_group_with_the_id() {
        let (_tx, rx) = watch::channel(snapshot(vec![group("1", "Kitchen"), group("2", "Hall")]));

        let found = GroupService::new(rx).retrieve_group("2").unwrap();

        assert_eq!(found.name, "Hall");
    }

    #[test]
    fn retrieve_group_returns_not_found_for_an_unknown_id() {
        let (_tx, rx) = watch::channel(snapshot(vec![group("1", "Kitchen")]));

        let result = GroupService::new(rx).retrieve_group("unknown");

        assert!(matches!(result, Err(RetrieveGroupError::NotFound)));
    }

    #[test]
    fn retrieve_group_does_not_match_on_the_name() {
        let (_tx, rx) = watch::channel(snapshot(vec![group("1", "Kitchen")]));

        let result = GroupService::new(rx).retrieve_group("Kitchen");

        assert!(matches!(result, Err(RetrieveGroupError::NotFound)));
    }

    #[test]
    fn retrieve_group_follows_the_latest_snapshot() {
        let (tx, rx) = watch::channel(StoreSnapshot::default());
        let service = GroupService::new(rx);

        tx.send(snapshot(vec![group("1", "Kitchen")])).unwrap();

        assert!(service.retrieve_group("1").is_ok());
    }
}
