use crate::domain::property::{BooleanProperty, Property, PropertyType};
use crate::domain::{Device, Event, ZoneChanged};
use crate::hue::domain::{DataType, ZoneChanged as HueZoneChanged};
use crate::hue::map_rooms::grouped_light_id;
use crate::store::StoreSnapshot;

pub fn map_zones_changed(zone: HueZoneChanged, data_type: DataType, snapshot: &StoreSnapshot) -> Vec<Event> {
    match data_type {
        DataType::Add | DataType::Update => upsert(zone, snapshot),
        DataType::Delete => vec![Event::ZoneRemoved { zone_id: zone.id }],
        DataType::Error => vec![],
    }
}

fn upsert(zone: HueZoneChanged, snapshot: &StoreSnapshot) -> Vec<Event> {
    let device_ids = zone
        .children
        .map(|children| children.iter().filter_map(|light| device_id_for_light(&light.rid, snapshot)).collect());
    let external_id = zone.services.as_deref().and_then(grouped_light_id);
    let zone_changed =
        ZoneChanged { id: zone.id, name: zone.metadata.and_then(|metadata| metadata.name), device_ids, external_id };
    vec![Event::ZoneChanged(zone_changed)]
}

fn device_id_for_light(light_id: &str, snapshot: &StoreSnapshot) -> Option<String> {
    snapshot.devices.values().find(|device| has_light_id(device, light_id)).map(|device| device.id.clone())
}

fn has_light_id(device: &Device, light_id: &str) -> bool {
    device
        .get_property_of_type::<BooleanProperty>(PropertyType::On)
        .is_some_and(|on_property| on_property.external_id() == Some(light_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hue::domain::{Owner, ZoneMetadataChanged};
    use crate::test_support::DeviceBuilder;
    use std::collections::BTreeSet;
    use std::sync::Arc;

    fn owner(rid: &str, rtype: &str) -> Owner {
        Owner { rid: rid.to_string(), rtype: rtype.to_string() }
    }

    fn lamp(device_id: &str, light_id: &str) -> Device {
        let on_property: Box<dyn Property> = Box::new(BooleanProperty::new(
            "on".to_string(),
            PropertyType::On,
            false,
            Some(light_id.to_string()),
            false,
        ));
        DeviceBuilder::new(device_id).with_properties(vec![on_property]).build()
    }

    fn snapshot() -> StoreSnapshot {
        let devices = [lamp("device-a", "light-a"), lamp("device-b", "light-b"), DeviceBuilder::new("sensor").build()];
        StoreSnapshot {
            devices: Arc::new(devices.into_iter().map(|device| (device.id.clone(), Arc::new(device))).collect()),
            ..Default::default()
        }
    }

    fn zone(name: Option<&str>, children: Option<&[&str]>) -> HueZoneChanged {
        HueZoneChanged {
            id: "zone".to_string(),
            children: children.map(|rids| rids.iter().map(|rid| owner(rid, "light")).collect()),
            services: None,
            metadata: name.map(|name| ZoneMetadataChanged { name: Some(name.to_string()) }),
        }
    }

    fn zone_changed(name: Option<&str>, device_ids: Option<&[&str]>) -> Event {
        Event::ZoneChanged(ZoneChanged {
            id: "zone".to_string(),
            name: name.map(str::to_string),
            device_ids: device_ids.map(|ids| ids.iter().map(|id| id.to_string()).collect()),
            external_id: None,
        })
    }

    #[test]
    fn add_maps_the_name_and_the_lights_to_devices() {
        let events =
            map_zones_changed(zone(Some("Downstairs"), Some(&["light-a", "light-b"])), DataType::Add, &snapshot());

        assert_eq!(events, vec![zone_changed(Some("Downstairs"), Some(&["device-a", "device-b"]))]);
    }

    #[test]
    fn update_with_only_a_name_leaves_the_devices_unset() {
        let events = map_zones_changed(zone(Some("Upstairs"), None), DataType::Update, &snapshot());

        assert_eq!(events, vec![zone_changed(Some("Upstairs"), None)]);
    }

    #[test]
    fn update_with_only_children_leaves_the_name_unset() {
        let events = map_zones_changed(zone(None, Some(&["light-a"])), DataType::Update, &snapshot());

        assert_eq!(events, vec![zone_changed(None, Some(&["device-a"]))]);
    }

    #[test]
    fn children_unknown_to_the_store_are_ignored() {
        let events = map_zones_changed(zone(None, Some(&["light-a", "unknown"])), DataType::Update, &snapshot());

        assert_eq!(events, vec![zone_changed(None, Some(&["device-a"]))]);
    }

    #[test]
    fn a_device_id_is_not_mistaken_for_a_light_id() {
        let events = map_zones_changed(zone(None, Some(&["device-a"])), DataType::Update, &snapshot());

        assert_eq!(events, vec![zone_changed(None, Some(&[]))]);
    }

    #[test]
    fn devices_without_an_on_property_are_never_matched() {
        let events = map_zones_changed(zone(None, Some(&["sensor"])), DataType::Update, &snapshot());

        assert_eq!(events, vec![zone_changed(None, Some(&[]))]);
    }

    #[test]
    fn children_without_known_lights_map_to_empty_devices() {
        let events = map_zones_changed(zone(None, Some(&["unknown"])), DataType::Update, &snapshot());

        assert_eq!(events, vec![zone_changed(None, Some(&[]))]);
    }

    #[test]
    fn a_light_listed_twice_maps_to_one_device() {
        let events = map_zones_changed(zone(None, Some(&["light-a", "light-a"])), DataType::Update, &snapshot());

        assert_eq!(events, vec![zone_changed(None, Some(&["device-a"]))]);
    }

    #[test]
    fn add_maps_the_grouped_light_to_the_external_id() {
        let zone = HueZoneChanged {
            services: Some(vec![owner("grouped-light", "grouped_light")]),
            ..zone(Some("Downstairs"), Some(&["light-a"]))
        };

        let events = map_zones_changed(zone, DataType::Add, &snapshot());

        assert_eq!(
            events,
            vec![Event::ZoneChanged(ZoneChanged {
                id: "zone".to_string(),
                name: Some("Downstairs".to_string()),
                device_ids: Some(BTreeSet::from(["device-a".to_string()])),
                external_id: Some("grouped-light".to_string()),
            })]
        );
    }

    #[test]
    fn services_without_a_grouped_light_leave_the_external_id_unset() {
        let zone = HueZoneChanged {
            services: Some(vec![owner("motion-service", "motion")]),
            ..zone(None, Some(&["light-a"]))
        };

        let events = map_zones_changed(zone, DataType::Update, &snapshot());

        assert_eq!(events, vec![zone_changed(None, Some(&["device-a"]))]);
    }

    #[test]
    fn delete_removes_the_zone() {
        let events = map_zones_changed(zone(None, None), DataType::Delete, &snapshot());

        assert_eq!(events, vec![Event::ZoneRemoved { zone_id: "zone".to_string() }]);
    }

    #[test]
    fn error_maps_to_no_events() {
        let events = map_zones_changed(zone(Some("Downstairs"), Some(&["light-a"])), DataType::Error, &snapshot());

        assert_eq!(events, vec![]);
    }
}
