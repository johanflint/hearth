use crate::domain::{DeviceType, Event, GroupChanged};
use crate::hue::domain::{DataType, RoomChanged};
use crate::hue::map_rooms::grouped_light_id;
use crate::store::StoreSnapshot;
use std::collections::BTreeSet;

pub fn map_rooms_changed(room: RoomChanged, data_type: DataType, snapshot: &StoreSnapshot) -> Vec<Event> {
    match data_type {
        DataType::Add | DataType::Update => upsert(room, snapshot),
        DataType::Delete => vec![Event::GroupRemoved { group_id: room.id }],
        DataType::Error => vec![],
    }
}

fn upsert(room: RoomChanged, snapshot: &StoreSnapshot) -> Vec<Event> {
    // Only allow lights
    let device_ids: Option<BTreeSet<String>> = room.children.map(|children| {
        children
            .into_iter()
            .filter(|child| snapshot.devices.get(&child.rid).is_some_and(|device| device.r#type == DeviceType::Light))
            .map(|child| child.rid)
            .collect::<BTreeSet<_>>()
    });
    let external_id = room.services.as_deref().and_then(grouped_light_id);
    let group_changed =
        GroupChanged { id: room.id, name: room.metadata.and_then(|metadata| metadata.name), device_ids, external_id };
    vec![Event::GroupChanged(group_changed)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Device;
    use crate::hue::domain::{Owner, RoomMetadataChanged};
    use crate::test_support::DeviceBuilder;
    use std::sync::Arc;

    fn owner(rid: &str) -> Owner {
        Owner { rid: rid.to_string(), rtype: "device".to_string() }
    }

    fn motion_sensor(id: &str) -> Device {
        Device { r#type: DeviceType::MotionSensor, ..DeviceBuilder::new(id).build() }
    }

    fn snapshot() -> StoreSnapshot {
        let devices =
            [DeviceBuilder::new("lamp-a").build(), DeviceBuilder::new("lamp-b").build(), motion_sensor("sensor")];
        StoreSnapshot {
            devices: Arc::new(devices.into_iter().map(|device| (device.id.clone(), Arc::new(device))).collect()),
            ..Default::default()
        }
    }

    fn service(rid: &str, rtype: &str) -> Owner {
        Owner { rid: rid.to_string(), rtype: rtype.to_string() }
    }

    fn room(name: Option<&str>, children: Option<&[&str]>) -> RoomChanged {
        RoomChanged {
            id: "room".to_string(),
            children: children.map(|rids| rids.iter().map(|rid| owner(rid)).collect()),
            services: None,
            metadata: name.map(|name| RoomMetadataChanged { name: Some(name.to_string()) }),
        }
    }

    fn group_changed(name: Option<&str>, device_ids: Option<&[&str]>) -> Event {
        Event::GroupChanged(GroupChanged {
            id: "room".to_string(),
            name: name.map(str::to_string),
            device_ids: device_ids.map(|ids| ids.iter().map(|id| id.to_string()).collect()),
            external_id: None,
        })
    }

    #[test]
    fn add_maps_the_name_and_the_lights() {
        let events = map_rooms_changed(room(Some("Kitchen"), Some(&["lamp-a", "lamp-b"])), DataType::Add, &snapshot());

        assert_eq!(events, vec![group_changed(Some("Kitchen"), Some(&["lamp-a", "lamp-b"]))]);
    }

    #[test]
    fn update_with_only_a_name_leaves_the_devices_unset() {
        let events = map_rooms_changed(room(Some("Cooking"), None), DataType::Update, &snapshot());

        assert_eq!(events, vec![group_changed(Some("Cooking"), None)]);
    }

    #[test]
    fn update_with_only_children_leaves_the_name_unset() {
        let events = map_rooms_changed(room(None, Some(&["lamp-a"])), DataType::Update, &snapshot());

        assert_eq!(events, vec![group_changed(None, Some(&["lamp-a"]))]);
    }

    #[test]
    fn children_that_are_not_lights_are_ignored() {
        let events = map_rooms_changed(room(None, Some(&["lamp-a", "sensor"])), DataType::Update, &snapshot());

        assert_eq!(events, vec![group_changed(None, Some(&["lamp-a"]))]);
    }

    #[test]
    fn children_unknown_to_the_store_are_ignored() {
        let events = map_rooms_changed(room(None, Some(&["lamp-a", "unknown"])), DataType::Update, &snapshot());

        assert_eq!(events, vec![group_changed(None, Some(&["lamp-a"]))]);
    }

    #[test]
    fn children_without_lights_map_to_empty_devices() {
        let events = map_rooms_changed(room(None, Some(&["sensor"])), DataType::Update, &snapshot());

        assert_eq!(events, vec![group_changed(None, Some(&[]))]);
    }

    #[test]
    fn add_maps_the_grouped_light_to_the_external_id() {
        let room = RoomChanged {
            services: Some(vec![service("grouped-light", "grouped_light")]),
            ..room(Some("Kitchen"), Some(&["lamp-a"]))
        };

        let events = map_rooms_changed(room, DataType::Add, &snapshot());

        assert_eq!(
            events,
            vec![Event::GroupChanged(GroupChanged {
                id: "room".to_string(),
                name: Some("Kitchen".to_string()),
                device_ids: Some(BTreeSet::from(["lamp-a".to_string()])),
                external_id: Some("grouped-light".to_string()),
            })]
        );
    }

    #[test]
    fn services_without_a_grouped_light_leave_the_external_id_unset() {
        let room =
            RoomChanged { services: Some(vec![service("motion-service", "motion")]), ..room(None, Some(&["lamp-a"])) };

        let events = map_rooms_changed(room, DataType::Update, &snapshot());

        assert_eq!(events, vec![group_changed(None, Some(&["lamp-a"]))]);
    }

    #[test]
    fn delete_removes_the_group() {
        let events = map_rooms_changed(room(None, None), DataType::Delete, &snapshot());

        assert_eq!(events, vec![Event::GroupRemoved { group_id: "room".to_string() }]);
    }

    #[test]
    fn error_maps_to_no_events() {
        let events = map_rooms_changed(room(Some("Kitchen"), Some(&["lamp-a"])), DataType::Error, &snapshot());

        assert_eq!(events, vec![]);
    }
}
