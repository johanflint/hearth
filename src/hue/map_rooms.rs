use crate::hue::domain::RoomGet;
use crate::hue::grouped_light_index::Room;
use std::collections::HashMap;

pub fn map_rooms(rooms: Vec<RoomGet>) -> HashMap<String, Room> {
    rooms.into_iter().map(|room| (room.id.clone(), Room::from(room))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hue::grouped_light_index::GroupedLightIndex;
    use serde_json::json;
    use std::collections::BTreeSet;

    fn room(id: &str, children: &[&str], services: &[(&str, &str)]) -> RoomGet {
        let children: Vec<_> = children.iter().map(|rid| json!({ "rid": rid, "rtype": "device" })).collect();
        let services: Vec<_> = services.iter().map(|(rid, rtype)| json!({ "rid": rid, "rtype": rtype })).collect();
        serde_json::from_value(json!({
            "id": id,
            "children": children,
            "services": services,
            "metadata": { "name": "Room", "archetype": "other" }
        }))
        .unwrap()
    }

    fn members(devices: &[&str]) -> BTreeSet<String> {
        devices.iter().map(|device| device.to_string()).collect()
    }

    // The fields of Room are private, so the mapped rooms are inspected through the index
    fn index_of(rooms: HashMap<String, Room>) -> GroupedLightIndex {
        let index = GroupedLightIndex::default();
        index.replace_all(rooms);
        index
    }

    #[test]
    fn maps_every_room_by_its_id() {
        let rooms = vec![
            room("room-1", &["device-a"], &[("grouped-light-1", "grouped_light")]),
            room("room-2", &["device-b", "device-c"], &[("grouped-light-2", "grouped_light")]),
        ];

        let result = map_rooms(rooms);

        let mut ids: Vec<_> = result.keys().cloned().collect();
        ids.sort();
        assert_eq!(ids, vec!["room-1".to_string(), "room-2".to_string()]);
    }

    #[test]
    fn maps_the_children_of_a_room_to_its_grouped_light() {
        let rooms = vec![room("room-1", &["device-b", "device-a"], &[("grouped-light-1", "grouped_light")])];

        let index = index_of(map_rooms(rooms));

        assert_eq!(index.find(&members(&["device-a", "device-b"])), Some("grouped-light-1".to_string()));
    }

    #[test]
    fn picks_the_grouped_light_from_the_services() {
        let rooms = vec![room(
            "room-1",
            &["device-a"],
            &[("other-service", "something_else"), ("grouped-light-1", "grouped_light")],
        )];

        let index = index_of(map_rooms(rooms));

        assert_eq!(index.find(&members(&["device-a"])), Some("grouped-light-1".to_string()));
    }

    #[test]
    fn keeps_rooms_without_a_grouped_light() {
        let rooms = vec![room("room-1", &["device-a"], &[("other-service", "something_else")])];

        let result = map_rooms(rooms);

        assert!(result.contains_key("room-1"));
        assert_eq!(index_of(result).find(&members(&["device-a"])), None);
    }

    #[test]
    fn keeps_rooms_without_children() {
        let rooms = vec![room("room-1", &[], &[("grouped-light-1", "grouped_light")])];

        let result = map_rooms(rooms);

        assert!(result.contains_key("room-1"));
        assert_eq!(index_of(result).find(&members(&[])), None);
    }

    #[test]
    fn maps_no_rooms_to_an_empty_map() {
        assert!(map_rooms(vec![]).is_empty());
    }
}
