use crate::domain::{Group, GroupOwner};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

/// Represents a zone that can contain groups and devices. Zones may overlap.
#[derive(Clone, PartialEq, Debug)]
pub struct Zone {
    pub id: String,
    pub name: String,
    pub group_ids: BTreeSet<String>,
    pub device_ids: BTreeSet<String>,
    pub revision: u64,
    pub owner: GroupOwner,
    pub external_id: Option<String>,
}

impl Zone {
    /// All devices of a zone including its groups. Unknown groups are skipped.
    pub fn members(&self, groups: &HashMap<String, Arc<Group>>) -> BTreeSet<String> {
        let mut members = self.device_ids.clone();
        for group in self.group_ids.iter().filter_map(|id| groups.get(id)) {
            members.extend(group.device_ids.iter().cloned());
        }
        members
    }

    pub fn unknown_group_ids<'a>(
        &'a self,
        groups: &'a HashMap<String, Arc<Group>>,
    ) -> impl Iterator<Item = &'a String> {
        self.group_ids.iter().filter(|id| !groups.contains_key(*id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn group(id: &str, device_ids: &[&str]) -> Arc<Group> {
        Arc::new(Group {
            id: id.to_string(),
            name: id.to_string(),
            device_ids: device_ids.iter().map(|id| id.to_string()).collect(),
            revision: 0,
            owner: GroupOwner::Integration,
            external_id: None,
        })
    }

    fn groups(groups: Vec<Arc<Group>>) -> HashMap<String, Arc<Group>> {
        groups.into_iter().map(|group| (group.id.clone(), group)).collect()
    }

    fn zone(device_ids: &[&str], group_ids: &[&str]) -> Zone {
        Zone {
            id: "downstairs".to_string(),
            name: "Downstairs".to_string(),
            group_ids: group_ids.iter().map(|id| id.to_string()).collect(),
            device_ids: device_ids.iter().map(|id| id.to_string()).collect(),
            revision: 0,
            owner: GroupOwner::Hearth,
            external_id: None,
        }
    }

    fn ids(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    #[test]
    fn members_returns_the_devices_of_the_zone() {
        let zone = zone(&["lamp1", "lamp2"], &[]);

        assert_eq!(zone.members(&HashMap::new()), ids(&["lamp1", "lamp2"]));
    }

    #[test]
    fn members_includes_the_devices_of_the_groups() {
        let zone = zone(&["lamp1"], &["room"]);
        let groups = groups(vec![group("room", &["lamp2", "lamp3"])]);

        assert_eq!(zone.members(&groups), ids(&["lamp1", "lamp2", "lamp3"]));
    }

    #[test]
    fn members_returns_the_devices_of_a_zone_that_only_contains_groups() {
        let zone = zone(&[], &["room", "hall"]);
        let groups = groups(vec![group("room", &["lamp1"]), group("hall", &["lamp2"])]);

        assert_eq!(zone.members(&groups), ids(&["lamp1", "lamp2"]));
    }

    #[test]
    fn members_returns_a_device_that_is_in_the_zone_and_a_group_once() {
        let zone = zone(&["lamp1", "lamp2"], &["room", "hall"]);
        let groups = groups(vec![group("room", &["lamp2", "lamp3"]), group("hall", &["lamp3"])]);

        assert_eq!(zone.members(&groups), ids(&["lamp1", "lamp2", "lamp3"]));
    }

    #[test]
    fn members_skips_an_unknown_group() {
        let zone = zone(&["lamp1"], &["ghost", "room"]);
        let groups = groups(vec![group("room", &["lamp2"])]);

        assert_eq!(zone.members(&groups), ids(&["lamp1", "lamp2"]));
    }

    #[test]
    fn members_ignores_the_groups_that_the_zone_does_not_refer_to() {
        let zone = zone(&["lamp1"], &[]);
        let groups = groups(vec![group("room", &["lamp2"])]);

        assert_eq!(zone.members(&groups), ids(&["lamp1"]));
    }

    #[test]
    fn members_of_an_empty_zone_is_empty() {
        assert!(zone(&[], &[]).members(&HashMap::new()).is_empty());
    }

    fn unknown_group_ids(zone: &Zone, groups: &HashMap<String, Arc<Group>>) -> Vec<String> {
        zone.unknown_group_ids(groups).cloned().collect()
    }

    #[test]
    fn unknown_group_ids_is_empty_when_every_group_is_known() {
        let zone = zone(&["lamp1"], &["room", "hall"]);
        let groups = groups(vec![group("room", &["lamp2"]), group("hall", &["lamp3"])]);

        assert!(unknown_group_ids(&zone, &groups).is_empty());
    }

    #[test]
    fn unknown_group_ids_returns_the_groups_that_are_not_in_the_store() {
        let zone = zone(&[], &["ghost", "room", "phantom"]);
        let groups = groups(vec![group("room", &["lamp1"])]);

        assert_eq!(unknown_group_ids(&zone, &groups), ["ghost", "phantom"]);
    }

    #[test]
    fn unknown_group_ids_returns_every_group_when_the_store_has_none() {
        let zone = zone(&[], &["room", "hall"]);

        assert_eq!(unknown_group_ids(&zone, &HashMap::new()), ["hall", "room"]);
    }

    #[test]
    fn unknown_group_ids_is_empty_for_a_zone_without_groups() {
        let zone = zone(&["lamp1"], &[]);

        assert!(unknown_group_ids(&zone, &HashMap::new()).is_empty());
    }

    #[test]
    fn unknown_group_ids_ignores_unknown_devices() {
        let zone = zone(&["ghost"], &[]);

        assert!(unknown_group_ids(&zone, &HashMap::new()).is_empty());
    }
}
