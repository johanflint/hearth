use crate::domain::property::{BooleanProperty, EnumProperty, Property, PropertyType};
use crate::domain::{Connectivity, Device, DeviceType};
use crate::shelly::controller::CONTROLLER_ID;
use crate::shelly::domain::{Component, PresenceZoneStatus, SwitchStatus};
use crate::shelly::prober::ProbedDevice;
use std::collections::HashMap;
use strum::IntoEnumIterator;
use tracing::debug;

pub fn map_shelly(probed_device: &ProbedDevice) -> Option<Device> {
    let Some(MappedProperties { device_type, mut properties }) =
        map_switch(probed_device).or_else(|| map_presence_sensor(probed_device))
    else {
        debug!("⚠️ Skipping Shelly device '{}', it has no supported components", probed_device.info.id);
        return None;
    };

    let allowed_values: Vec<String> = Connectivity::iter().map(|c| c.to_string()).collect();
    let connectivity_property = Box::new(
        EnumProperty::new(
            "connectivity".to_string(),
            PropertyType::Connectivity,
            true,
            None,
            Some(Connectivity::Connected.to_string()),
            allowed_values,
        )
        .expect("connectivity status is always one of the declared allowed values"),
    );
    properties.insert(connectivity_property.name().to_owned(), connectivity_property);

    let name = match &probed_device.info.name {
        Some(name) => name.clone(),
        None => probed_device.info.id.clone(),
    };
    Some(Device {
        id: probed_device.info.id.clone(),
        r#type: device_type,
        manufacturer: "Shelly Europe Ltd.".to_string(),
        model_id: probed_device.info.model.clone(),
        product_name: probed_device.info.app.clone(),
        name,
        claimable: true,
        properties,
        external_id: None,
        address: Some(probed_device.address.to_string()),
        controller_id: Some(CONTROLLER_ID),
    })
}

fn map_switch(probed_device: &ProbedDevice) -> Option<MappedProperties> {
    let mut properties = HashMap::new();

    for component in &probed_device.components {
        match component {
            Component::Switch { status: SwitchStatus { id, output } } => {
                let switch_property: Box<dyn Property> = Box::new(BooleanProperty::new(
                    format!("switch{}", id),
                    PropertyType::On,
                    false,
                    Some(format!("switch:{}", id)),
                    *output,
                ));

                properties.insert(switch_property.name().to_owned(), switch_property);
            }
            Component::Mqtt { .. } | Component::PresenceZone { .. } | Component::Unsupported => {}
        }
    }

    if properties.is_empty() {
        return None;
    }
    Some(MappedProperties { device_type: DeviceType::Switch, properties })
}

fn map_presence_sensor(probed_device: &ProbedDevice) -> Option<MappedProperties> {
    let mut properties = HashMap::new();

    for component in &probed_device.components {
        match component {
            Component::PresenceZone { status: PresenceZoneStatus { id, value, num_objects: _ } } => {
                let presence_property: Box<dyn Property> = Box::new(BooleanProperty::new(
                    format!("presenceZone{}", id),
                    PropertyType::Presence,
                    true,
                    Some(format!("presencezone:{}", id)),
                    *value,
                ));

                properties.insert(presence_property.name().to_owned(), presence_property);
            }
            Component::Mqtt { .. } | Component::Switch { .. } | Component::Unsupported => {}
        }
    }

    if properties.is_empty() {
        return None;
    }

    Some(MappedProperties { device_type: DeviceType::PresenceSensor, properties })
}

#[derive(Debug)]
struct MappedProperties {
    device_type: DeviceType,
    properties: HashMap<String, Box<dyn Property>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::property::PropertyType;
    use crate::shelly::test_support::{
        PLUS_1_ID, ble_component, presence_zone_component, probed_device, probed_plus_1, switch_component,
    };
    use pretty_assertions::assert_eq;

    #[test]
    fn maps_a_plus_1_to_a_switch_device() {
        let device = map_shelly(&probed_plus_1()).unwrap();

        assert_eq!(device.id, PLUS_1_ID);
        assert_eq!(device.r#type, DeviceType::Switch);
        assert_eq!(device.manufacturer, "Shelly Europe Ltd.");
        assert_eq!(device.model_id, "SNSW-001X16EU");
        assert_eq!(device.product_name, "Plus1");
        assert_eq!(device.name, "Hallway");
        assert_eq!(device.address.as_deref(), Some("192.168.1.35:80"));
        assert_eq!(device.properties.len(), 2);
    }

    #[test]
    fn maps_a_device_to_a_connected_readonly_connectivity_property() {
        let device = map_shelly(&probed_plus_1()).unwrap();

        let property = device.get_property::<EnumProperty>("connectivity").unwrap();
        assert_eq!(property.value(), Some("connected"));
        assert_eq!(property.property_type(), PropertyType::Connectivity);
        assert!(property.readonly());
    }

    #[test]
    fn maps_a_switch_to_a_boolean_property_with_the_component_key_as_external_id() {
        let device = map_shelly(&probed_device(PLUS_1_ID, &[ble_component(), switch_component(0, true)])).unwrap();

        let property = device.get_property::<BooleanProperty>("switch0").unwrap();
        assert!(property.value());
        assert_eq!(property.property_type(), PropertyType::On);
        assert!(!property.readonly());
        assert_eq!(property.external_id(), Some("switch:0"));
    }

    #[test]
    fn maps_every_switch_to_its_own_property() {
        let device =
            map_shelly(&probed_device(PLUS_1_ID, &[switch_component(0, true), switch_component(1, false)])).unwrap();

        assert_eq!(device.properties.len(), 3);
        assert!(device.get_property::<BooleanProperty>("switch0").unwrap().value());
        assert!(!device.get_property::<BooleanProperty>("switch1").unwrap().value());
        assert_eq!(device.get_property::<BooleanProperty>("switch1").unwrap().external_id(), Some("switch:1"));
    }

    #[test]
    fn returns_none_for_a_device_without_supported_components() {
        let device = map_shelly(&probed_device(PLUS_1_ID, &[ble_component()]));

        assert_eq!(device, None);
    }

    #[test]
    fn maps_a_device_with_presence_zones_to_a_presence_sensor() {
        let components = [ble_component(), presence_zone_component(200, true), presence_zone_component(201, false)];

        let device = map_shelly(&probed_device("shellypresence-e8f60a7937ec", &components)).unwrap();

        assert_eq!(device.r#type, DeviceType::PresenceSensor);
        assert_eq!(device.properties.len(), 3); // 2 zones and connectivity
        assert!(device.get_property::<EnumProperty>("connectivity").is_some());
    }

    #[test]
    fn maps_a_presence_zone_to_a_readonly_boolean_property_with_the_component_key_as_external_id() {
        let device = map_shelly(&probed_device(PLUS_1_ID, &[presence_zone_component(200, true)])).unwrap();

        let property = device.get_property::<BooleanProperty>("presenceZone200").unwrap();
        assert!(property.value());
        assert_eq!(property.property_type(), PropertyType::Presence);
        assert!(property.readonly());
        assert_eq!(property.external_id(), Some("presencezone:200"));
    }

    #[test]
    fn maps_every_presence_zone_to_its_own_property() {
        let components = [presence_zone_component(200, true), presence_zone_component(201, false)];

        let device = map_shelly(&probed_device(PLUS_1_ID, &components)).unwrap();

        assert!(device.get_property::<BooleanProperty>("presenceZone200").unwrap().value());
        assert!(!device.get_property::<BooleanProperty>("presenceZone201").unwrap().value());
        assert_eq!(
            device.get_property::<BooleanProperty>("presenceZone201").unwrap().external_id(),
            Some("presencezone:201")
        );
    }

    #[test]
    fn maps_a_device_with_a_switch_and_a_presence_zone_to_a_switch_without_the_zone() {
        let components = [presence_zone_component(200, true), switch_component(0, false)];

        let device = map_shelly(&probed_device(PLUS_1_ID, &components)).unwrap();

        assert_eq!(device.r#type, DeviceType::Switch);
        assert!(device.get_property::<BooleanProperty>("presenceZone200").is_none());
    }
}
