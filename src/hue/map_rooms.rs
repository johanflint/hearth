use crate::domain::{Group, GroupOwner};
use crate::hue::domain::RoomGet;
use std::collections::{BTreeSet, HashSet};

pub fn map_rooms(rooms: Vec<RoomGet>, light_device_ids: &HashSet<String>) -> Vec<Group> {
    rooms
        .into_iter()
        .filter_map(|room| {
            // Only allow lights
            let device_ids: BTreeSet<String> = room
                .children
                .iter()
                .filter(|child| light_device_ids.contains(&child.rid))
                .map(|child| child.rid.clone())
                .collect();

            if device_ids.is_empty() {
                return None;
            }

            Some(Group {
                id: room.id,
                name: room.metadata.name,
                device_ids,
                revision: 0,
                owner: GroupOwner::Integration,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hue::domain::{Metadata, Owner};
    use pretty_assertions::assert_eq;

    fn owner(rid: &str, rtype: &str) -> Owner {
        Owner { rid: rid.to_string(), rtype: rtype.to_string() }
    }

    fn room(id: &str, name: &str, children: &[&str]) -> RoomGet {
        RoomGet {
            id: id.to_string(),
            children: children.iter().map(|rid| owner(rid, "device")).collect(),
            services: vec![owner("grouped-light", "grouped_light")],
            metadata: Metadata { name: name.to_string(), archetype: "other".to_string() },
        }
    }

    fn light_device_ids(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    fn device_ids(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    #[test]
    fn map_rooms_maps_a_room_to_an_integration_owned_group() {
        let rooms = vec![room("room-1", "Living room", &["lamp-1", "lamp-2"])];

        let groups = map_rooms(rooms, &light_device_ids(&["lamp-1", "lamp-2"]));

        assert_eq!(
            groups,
            vec![Group {
                id: "room-1".to_string(),
                name: "Living room".to_string(),
                device_ids: device_ids(&["lamp-1", "lamp-2"]),
                revision: 0,
                owner: GroupOwner::Integration,
            }]
        );
    }

    #[test]
    fn map_rooms_keeps_only_the_children_that_are_lights() {
        let rooms = vec![room("room-1", "Office", &["lamp", "motion-sensor", "remote"])];

        let groups = map_rooms(rooms, &light_device_ids(&["lamp"]));

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].device_ids, device_ids(&["lamp"]));
    }

    #[test]
    fn map_rooms_skips_a_room_without_lights() {
        let rooms = vec![room("room-1", "Hallway", &["motion-sensor"]), room("room-2", "Empty", &[])];

        let groups = map_rooms(rooms, &light_device_ids(&["lamp"]));

        assert!(groups.is_empty(), "got {groups:?}");
    }

    #[test]
    fn map_rooms_ignores_lights_that_are_in_another_room() {
        let rooms = vec![room("room-1", "Kitchen", &["lamp-1"]), room("room-2", "Bedroom", &["lamp-2"])];

        let groups = map_rooms(rooms, &light_device_ids(&["lamp-1", "lamp-2"]));

        let ids: Vec<(&str, &BTreeSet<String>)> = groups.iter().map(|g| (g.id.as_str(), &g.device_ids)).collect();
        assert_eq!(ids, vec![("room-1", &device_ids(&["lamp-1"])), ("room-2", &device_ids(&["lamp-2"]))]);
    }

    #[test]
    fn map_rooms_deduplicates_children_listed_twice() {
        let rooms = vec![room("room-1", "Kitchen", &["lamp", "lamp"])];

        let groups = map_rooms(rooms, &light_device_ids(&["lamp"]));

        assert_eq!(groups[0].device_ids, device_ids(&["lamp"]));
    }

    #[test]
    fn map_rooms_returns_nothing_without_rooms() {
        assert!(map_rooms(vec![], &light_device_ids(&["lamp"])).is_empty());
    }
}
