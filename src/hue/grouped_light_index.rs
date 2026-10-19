use crate::hue::domain::{Owner, RoomChanged, RoomGet};
use std::collections::{BTreeSet, HashMap};
use std::sync::RwLock;

type RoomId = String;
type DeviceId = String;
type GroupedLightId = String;

#[derive(Default, Debug)]
pub struct GroupedLightIndex {
    state: RwLock<State>,
}

impl GroupedLightIndex {
    pub fn replace_all(&self, rooms: HashMap<RoomId, Room>) {
        let state = State::new(rooms); // built before taking the lock
        *self.state.write().expect("grouped light index lock poisoned") = state;
    }

    pub fn apply(&self, room_id: RoomId, patch: RoomPatch) {
        let mut state = self.state.write().expect("grouped light index lock poisoned");
        let room = state.rooms.entry(room_id).or_default();
        if let Some(members) = patch.members {
            room.members = members;
        }
        if let Some(grouped_light_id) = patch.grouped_light_id {
            room.grouped_light_id = Some(grouped_light_id);
        }
        state.rebuild();
    }

    pub fn remove(&self, room_id: &str) {
        let mut state = self.state.write().expect("grouped light index lock poisoned");
        if state.rooms.remove(room_id).is_some() {
            state.rebuild();
        }
    }

    /// The grouped light of the room that has exactly these devices.
    #[allow(dead_code)]
    pub fn find(&self, device_ids: &BTreeSet<DeviceId>) -> Option<GroupedLightId> {
        self.state.read().expect("grouped light index lock poisoned").by_members.get(device_ids).cloned()
    }
}

#[derive(Default, Debug)]
struct State {
    rooms: HashMap<RoomId, Room>,
    by_members: HashMap<BTreeSet<DeviceId>, GroupedLightId>,
}

impl State {
    fn new(rooms: HashMap<RoomId, Room>) -> Self {
        let mut state = State { rooms, by_members: HashMap::new() };
        state.rebuild();
        state
    }

    fn rebuild(&mut self) {
        self.by_members = self
            .rooms
            .values()
            .filter_map(|room| match &room.grouped_light_id {
                Some(grouped_light_id) if !room.members.is_empty() => {
                    Some((room.members.clone(), grouped_light_id.clone()))
                }
                _ => None,
            })
            .collect()
    }
}

#[derive(Default, Debug)]
pub(super) struct Room {
    members: BTreeSet<DeviceId>,
    grouped_light_id: Option<GroupedLightId>,
}

impl From<RoomGet> for Room {
    fn from(room: RoomGet) -> Self {
        Room { members: device_ids(room.children), grouped_light_id: grouped_light_id(room.services) }
    }
}

/// The fields of a room that an SSE add or update event carries; `None` means unchanged.
#[derive(Debug)]
pub(super) struct RoomPatch {
    members: Option<BTreeSet<DeviceId>>,
    grouped_light_id: Option<GroupedLightId>,
}

impl From<RoomChanged> for RoomPatch {
    fn from(room: RoomChanged) -> Self {
        RoomPatch { members: room.children.map(device_ids), grouped_light_id: room.services.and_then(grouped_light_id) }
    }
}

fn device_ids(children: Vec<Owner>) -> BTreeSet<DeviceId> {
    children.into_iter().map(|child| child.rid).collect()
}

fn grouped_light_id(services: Vec<Owner>) -> Option<GroupedLightId> {
    services.into_iter().find(|service| service.rtype == "grouped_light").map(|service| service.rid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn owners(rtype: &str, rids: &[&str]) -> Value {
        Value::Array(rids.iter().map(|rid| json!({ "rid": rid, "rtype": rtype })).collect())
    }

    fn members(devices: &[&str]) -> BTreeSet<DeviceId> {
        devices.iter().map(|device| device.to_string()).collect()
    }

    fn room_get(children: &[&str], services: &[(&str, &str)]) -> RoomGet {
        let services: Vec<_> = services.iter().map(|(rid, rtype)| json!({ "rid": rid, "rtype": rtype })).collect();
        serde_json::from_value(json!({
            "id": "room-1",
            "children": owners("device", children),
            "services": services,
            "metadata": { "name": "Room", "archetype": "other" }
        }))
        .unwrap()
    }

    fn room_changed(children: Option<&[&str]>, services: Option<&[&str]>) -> RoomChanged {
        let mut json = json!({ "id": "room-1", "type": "room" });
        if let Some(children) = children {
            json["children"] = owners("device", children);
        }
        if let Some(services) = services {
            json["services"] = owners("grouped_light", services);
        }
        serde_json::from_value(json).unwrap()
    }

    fn patch(children: Option<&[&str]>, grouped_light_id: Option<&str>) -> RoomPatch {
        RoomPatch { members: children.map(members), grouped_light_id: grouped_light_id.map(str::to_string) }
    }

    fn index_with_kitchen() -> GroupedLightIndex {
        let index = GroupedLightIndex::default();
        index.replace_all(HashMap::from([(
            "kitchen".to_string(),
            Room { members: members(&["lamp-a", "lamp-b"]), grouped_light_id: Some("grouped-light-1".to_string()) },
        )]));
        index
    }

    #[test]
    fn room_from_room_get_maps_children_and_grouped_light() {
        let room = Room::from(room_get(
            &["device-b", "device-a"],
            &[("other-service", "something_else"), ("grouped-light-1", "grouped_light")],
        ));

        assert_eq!(room.members, members(&["device-a", "device-b"]));
        assert_eq!(room.grouped_light_id, Some("grouped-light-1".to_string()));
    }

    #[test]
    fn room_from_room_get_without_grouped_light() {
        let room = Room::from(room_get(&["device-a"], &[("other-service", "something_else")]));

        assert_eq!(room.members, members(&["device-a"]));
        assert_eq!(room.grouped_light_id, None);
    }

    #[test]
    fn patch_from_room_changed_with_all_fields() {
        let patch = RoomPatch::from(room_changed(Some(&["device-b", "device-a"]), Some(&["grouped-light-1"])));

        assert_eq!(patch.members, Some(members(&["device-a", "device-b"])));
        assert_eq!(patch.grouped_light_id, Some("grouped-light-1".to_string()));
    }

    #[test]
    fn patch_from_room_changed_leaves_absent_fields_unset() {
        let patch = RoomPatch::from(room_changed(None, None));

        assert_eq!(patch.members, None);
        assert_eq!(patch.grouped_light_id, None);
    }

    #[test]
    fn patch_from_room_changed_ignores_services_without_grouped_light() {
        let changed: RoomChanged = serde_json::from_value(json!({
            "id": "room-1",
            "services": owners("something_else", &["other-service"])
        }))
        .unwrap();

        assert_eq!(RoomPatch::from(changed).grouped_light_id, None);
    }

    #[test]
    fn find_returns_the_grouped_light_of_the_room_with_exactly_these_devices() {
        let index = index_with_kitchen();

        assert_eq!(index.find(&members(&["lamp-b", "lamp-a"])), Some("grouped-light-1".to_string()));
    }

    #[test]
    fn find_returns_none_for_a_subset_or_superset_of_the_members() {
        let index = index_with_kitchen();

        assert_eq!(index.find(&members(&["lamp-a"])), None);
        assert_eq!(index.find(&members(&["lamp-a", "lamp-b", "lamp-c"])), None);
    }

    #[test]
    fn find_returns_none_on_an_empty_index() {
        assert_eq!(GroupedLightIndex::default().find(&members(&["lamp-a"])), None);
    }

    #[test]
    fn find_ignores_rooms_without_a_grouped_light() {
        let index = GroupedLightIndex::default();
        index.replace_all(HashMap::from([(
            "kitchen".to_string(),
            Room { members: members(&["lamp-a"]), grouped_light_id: None },
        )]));

        assert_eq!(index.find(&members(&["lamp-a"])), None);
    }

    #[test]
    fn find_ignores_rooms_without_members() {
        let index = GroupedLightIndex::default();
        index.replace_all(HashMap::from([(
            "kitchen".to_string(),
            Room { members: members(&[]), grouped_light_id: Some("grouped-light-1".to_string()) },
        )]));

        assert_eq!(index.find(&members(&[])), None);
    }

    #[test]
    fn replace_all_discards_the_previous_rooms() {
        let index = index_with_kitchen();

        index.replace_all(HashMap::from([(
            "living-room".to_string(),
            Room { members: members(&["lamp-c"]), grouped_light_id: Some("grouped-light-2".to_string()) },
        )]));

        assert_eq!(index.find(&members(&["lamp-a", "lamp-b"])), None);
        assert_eq!(index.find(&members(&["lamp-c"])), Some("grouped-light-2".to_string()));
    }

    #[test]
    fn apply_adds_an_unknown_room() {
        let index = GroupedLightIndex::default();

        index.apply("kitchen".to_string(), patch(Some(&["lamp-a"]), Some("grouped-light-1")));

        assert_eq!(index.find(&members(&["lamp-a"])), Some("grouped-light-1".to_string()));
    }

    #[test]
    fn apply_replaces_the_members_of_a_known_room() {
        let index = index_with_kitchen();

        index.apply("kitchen".to_string(), patch(Some(&["lamp-a", "lamp-b", "lamp-c"]), None));

        assert_eq!(index.find(&members(&["lamp-a", "lamp-b"])), None);
        assert_eq!(index.find(&members(&["lamp-a", "lamp-b", "lamp-c"])), Some("grouped-light-1".to_string()));
    }

    #[test]
    fn apply_keeps_the_grouped_light_when_only_the_members_change() {
        let index = index_with_kitchen();

        index.apply("kitchen".to_string(), patch(Some(&["lamp-a"]), None));

        assert_eq!(index.find(&members(&["lamp-a"])), Some("grouped-light-1".to_string()));
    }

    #[test]
    fn apply_keeps_the_members_when_only_the_grouped_light_changes() {
        let index = index_with_kitchen();

        index.apply("kitchen".to_string(), patch(None, Some("grouped-light-2")));

        assert_eq!(index.find(&members(&["lamp-a", "lamp-b"])), Some("grouped-light-2".to_string()));
    }

    #[test]
    fn apply_completes_a_room_that_was_missing_a_field() {
        let index = GroupedLightIndex::default();
        index.apply("kitchen".to_string(), patch(Some(&["lamp-a"]), None));
        assert_eq!(index.find(&members(&["lamp-a"])), None);

        index.apply("kitchen".to_string(), patch(None, Some("grouped-light-1")));

        assert_eq!(index.find(&members(&["lamp-a"])), Some("grouped-light-1".to_string()));
    }

    #[test]
    fn apply_does_not_affect_other_rooms() {
        let index = index_with_kitchen();

        index.apply("living-room".to_string(), patch(Some(&["lamp-c"]), Some("grouped-light-2")));

        assert_eq!(index.find(&members(&["lamp-a", "lamp-b"])), Some("grouped-light-1".to_string()));
        assert_eq!(index.find(&members(&["lamp-c"])), Some("grouped-light-2".to_string()));
    }

    #[test]
    fn remove_deletes_the_room() {
        let index = index_with_kitchen();

        index.remove("kitchen");

        assert_eq!(index.find(&members(&["lamp-a", "lamp-b"])), None);
    }

    #[test]
    fn remove_of_an_unknown_room_changes_nothing() {
        let index = index_with_kitchen();

        index.remove("garage");

        assert_eq!(index.find(&members(&["lamp-a", "lamp-b"])), Some("grouped-light-1".to_string()));
    }
}
