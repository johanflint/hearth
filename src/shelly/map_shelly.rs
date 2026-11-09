use crate::domain::property::{BooleanProperty, EnumProperty, Property, PropertyType};
use crate::domain::{Connectivity, Device, DeviceType};
use crate::shelly::domain::{Component, SwitchStatus};
use crate::shelly::prober::ProbedDevice;
use std::collections::HashMap;
use strum::IntoEnumIterator;

pub fn map_shelly(probed_device: &ProbedDevice) -> Option<Device> {
    let mut properties = HashMap::with_capacity(2);

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
            Component::Mqtt { .. } | Component::Unsupported => {}
        }
    }

    if properties.is_empty() {
        return None;
    }

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

    Some(Device {
        id: probed_device.info.id.clone(),
        r#type: DeviceType::Switch,
        manufacturer: "Shelly Europe Ltd.".to_string(),
        model_id: probed_device.info.model.clone(),
        product_name: probed_device.info.app.clone(),
        name: probed_device.info.name.clone(),
        claimable: true,
        properties,
        external_id: None,
        address: Some(probed_device.address.to_string()),
        controller_id: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::property::PropertyType;
    use crate::shelly::test_support::{PLUS_1_ID, ble_component, probed_device, probed_plus_1, switch_component};
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
    fn returns_none_for_a_device_without_switches() {
        let device = map_shelly(&probed_device(PLUS_1_ID, &[ble_component()]));

        assert_eq!(device, None);
    }
}
