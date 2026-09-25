use crate::domain::BatteryState;
use crate::domain::Connectivity;
use crate::domain::device::Device;
use crate::domain::property::{EnumProperty, NumberProperty, Property, PropertyType, Unit};
use crate::hue::connectivity::map_connectivity_status;
use crate::hue::domain::{DevicePowerGet, ZigbeeConnectivityGet};
use std::collections::HashMap;
use strum::IntoEnumIterator;

pub fn enrich_devices(connectivity_list: Vec<ZigbeeConnectivityGet>, device_power_list: Vec<DevicePowerGet>, devices: &mut Vec<Device>) {
    let connectivity_map: HashMap<String, Connectivity> = connectivity_list.into_iter()
        .map(|c| (c.owner.rid, map_connectivity_status(&c.status)))
        .collect();
    let device_power_map: HashMap<String, DevicePowerGet> = device_power_list.into_iter()
        .map(|p| (p.owner.rid.clone(), p))
        .collect();

    for device in devices {
        enrich_connectivity(device, &connectivity_map);
        enrich_device_power(device, &device_power_map);
    }
}

fn enrich_connectivity(device: &mut Device, connectivity_map: &HashMap<String, Connectivity>) {
    let status = connectivity_map.get(&device.id).unwrap_or_else(|| &Connectivity::Unknown);
    let allowed_values: Vec<String> = Connectivity::iter().map(|c| c.as_str().to_string()).collect();

    let connectivity_property = Box::new(
        // The allowed values always contains every `Connectivity` variant and `status` is always one of these, so it can never fail
        EnumProperty::new("connectivity".to_string(), PropertyType::Connectivity, true, None, Some(status.as_str().to_string()), allowed_values)
            .expect("connectivity status is always one of the declared allowed values")
    );
    device.properties.insert(connectivity_property.name().to_string(), connectivity_property);
}

fn enrich_device_power(device: &mut Device, device_power_map: &HashMap<String, DevicePowerGet>) {
    let Some(power_state) = device_power_map.get(&device.id).map(|p| &p.power_state) else {
        return;
    };

    let battery_level = power_state.battery_level.map(u64::from);
    let battery_level_property = Box::new(
        NumberProperty::builder("batteryLevel".to_string(), PropertyType::BatteryLevel, true)
            .unit(Unit::Percentage)
            .positive_int(battery_level, Some(0), Some(100))
            .build()
    );
    device.properties.insert(battery_level_property.name().to_string(), battery_level_property);

    let battery_state = battery_level.map(|b| BatteryState::from_percent(b).as_str().to_string());
    let allowed_values: Vec<String> = BatteryState::iter().map(|c| c.as_str().to_string()).collect();
    let battery_state_property = Box::new(
        EnumProperty::new("batteryState".to_string(), PropertyType::BatteryState, true, None, battery_state, allowed_values)
            .expect("battery state is always one of the declared allowed values")
    );
    device.properties.insert(battery_state_property.name().to_string(), battery_state_property);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hue::domain::{Owner, PowerState};
    use crate::test_support::DeviceBuilder;

    #[test]
    fn enrich_devices_adds_a_connectivity_property_matched_by_owner_rid() {
        let device = DeviceBuilder::new("device").build();
        let mut devices = vec![device];
        let connectivity_list = vec![ZigbeeConnectivityGet {
            id: "connectivity".to_string(),
            owner: Owner { rid: "device".to_string(), rtype: "device".to_string() },
            status: "connected".to_string()
        }];

        enrich_devices(connectivity_list, vec![], &mut devices);

        let property = devices[0].properties.get("connectivity").unwrap().as_any().downcast_ref::<EnumProperty>().unwrap();
        assert_eq!(property.name(), "connectivity");
        assert_eq!(property.property_type(), PropertyType::Connectivity);
        assert!(property.readonly());
        assert_eq!(property.external_id(), None);
        assert_eq!(property.value(), Some("connected"));
    }

    #[test]
    fn enrich_devices_defaults_to_unknown_when_no_connectivity_resource_matches() {
        let device = DeviceBuilder::new("device").build();
        let mut devices = vec![device];

        enrich_devices(vec![], vec![], &mut devices);

        let property = devices[0].properties.get("connectivity").unwrap().as_any().downcast_ref::<EnumProperty>().unwrap();
        assert_eq!(property.value(), Some("unknown"));
    }

    #[test]
    fn enrich_devices_adds_battery_level_property() {
        let device = DeviceBuilder::new("device").build();
        let mut devices = vec![device];
        let device_power_list = vec![DevicePowerGet {
            id: "power".to_string(),
            owner: Owner { rid: "device".to_string(), rtype: "device".to_string() },
            power_state: PowerState { battery_level: Some(76), battery_state: Some("low".to_string()) },
        }];

        enrich_devices(vec![], device_power_list, &mut devices);

        let battery_level = devices[0].properties.get("batteryLevel").unwrap().as_any().downcast_ref::<NumberProperty>().unwrap();
        assert_eq!(battery_level.property_type(), PropertyType::BatteryLevel);
        assert!(battery_level.readonly());
        assert_eq!(battery_level.as_u64(), Some(76));

        let battery_state = devices[0].properties.get("batteryState").unwrap().as_any().downcast_ref::<EnumProperty>().unwrap();
        assert_eq!(battery_state.property_type(), PropertyType::BatteryState);
        assert!(battery_state.readonly());
        assert_eq!(battery_state.value(), Some("normal"));
    }

    #[test]
    fn enrich_devices_defaults_battery_properties_to_none_when_device_power_resource_has_no_reading() {
        let device = DeviceBuilder::new("device").build();
        let mut devices = vec![device];
        let device_power_list = vec![DevicePowerGet {
            id: "power".to_string(),
            owner: Owner { rid: "device".to_string(), rtype: "device".to_string() },
            power_state: PowerState { battery_level: None, battery_state: None },
        }];

        enrich_devices(vec![], device_power_list, &mut devices);

        let battery_level = devices[0].properties.get("batteryLevel").unwrap().as_any().downcast_ref::<NumberProperty>().unwrap();
        assert_eq!(battery_level.as_u64(), None);

        let battery_state = devices[0].properties.get("batteryState").unwrap().as_any().downcast_ref::<EnumProperty>().unwrap();
        assert_eq!(battery_state.value(), None);
    }

    #[test]
    fn enrich_devices_omits_battery_properties_when_no_device_power_matches() {
        let device = DeviceBuilder::new("device").build();
        let mut devices = vec![device];

        enrich_devices(vec![], vec![], &mut devices);

        assert!(devices[0].properties.get("batteryLevel").is_none());
        assert!(devices[0].properties.get("batteryState").is_none());
    }
}