use crate::hue::domain::RoomGet;
use std::collections::{BTreeSet, HashMap};

type DeviceId = String;
type GroupedLightId = String;

pub fn map_rooms(rooms: Vec<RoomGet>) -> HashMap<BTreeSet<DeviceId>, GroupedLightId> {
    rooms
        .into_iter()
        .filter_map(|room| {
            let grouped_light_id = room.services.into_iter().find(|s| s.rtype == "grouped_light")?.rid;
            let members: BTreeSet<_> = room.children.into_iter().map(|resource| resource.rid).collect();
            (!members.is_empty()).then_some((members, grouped_light_id))
        })
        .fold(HashMap::new(), |mut map, (members, grouped_light_id)| {
            map.insert(members, grouped_light_id);
            map
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn room(children: &[&str], services: &[(&str, &str)]) -> RoomGet {
        let children: Vec<_> = children.iter().map(|rid| json!({ "rid": rid, "rtype": "device" })).collect();
        let services: Vec<_> = services.iter().map(|(rid, rtype)| json!({ "rid": rid, "rtype": rtype })).collect();
        serde_json::from_value(json!({
            "id": "room-id",
            "children": children,
            "services": services,
            "metadata": { "name": "Room", "archetype": "other" }
        }))
        .unwrap()
    }

    #[test]
    fn maps_the_children_of_a_room_to_its_grouped_light() {
        let rooms = vec![room(&["device-b", "device-a"], &[("grouped-light-1", "grouped_light")])];

        let result = map_rooms(rooms);

        assert_eq!(
            result,
            HashMap::from([(
                BTreeSet::from(["device-a".to_string(), "device-b".to_string()]),
                "grouped-light-1".to_string()
            )])
        );
    }

    #[test]
    fn picks_the_grouped_light_from_the_services() {
        let rooms = vec![room(&["device-a"], &[("other-service", "something_else"), ("grouped-light-1", "grouped_light")])];

        let result = map_rooms(rooms);

        assert_eq!(result.get(&BTreeSet::from(["device-a".to_string()])), Some(&"grouped-light-1".to_string()));
    }

    #[test]
    fn skips_rooms_without_a_grouped_light() {
        let rooms = vec![room(&["device-a"], &[("other-service", "something_else")]), room(&["device-b"], &[])];

        assert!(map_rooms(rooms).is_empty());
    }

    #[test]
    fn skips_rooms_without_children() {
        let rooms = vec![room(&[], &[("grouped-light-1", "grouped_light")])];

        assert!(map_rooms(rooms).is_empty());
    }

    #[test]
    fn maps_every_room() {
        let rooms = vec![
            room(&["device-a"], &[("grouped-light-1", "grouped_light")]),
            room(&["device-b", "device-c"], &[("grouped-light-2", "grouped_light")]),
        ];

        let result = map_rooms(rooms);

        assert_eq!(result.len(), 2);
        assert_eq!(result.get(&BTreeSet::from(["device-a".to_string()])), Some(&"grouped-light-1".to_string()));
        assert_eq!(
            result.get(&BTreeSet::from(["device-b".to_string(), "device-c".to_string()])),
            Some(&"grouped-light-2".to_string())
        );
    }
}
