use crate::domain::property::{BooleanProperty, Property, PropertyType};
use crate::domain::{Device, DeviceType};
use crate::shelly::domain::{Component, ComponentsResult, DeviceInfo, SwitchStatus};
use std::collections::HashMap;

pub fn map_shelly(device_info: DeviceInfo, component_result: ComponentsResult, address: String) -> Option<Device> {
    let mut properties = HashMap::with_capacity(1);

    for component in component_result.components {
        match component {
            Component::Switch { status: SwitchStatus { id, output } } => {
                let switch_property: Box<dyn Property> = Box::new(BooleanProperty::new(
                    format!("switch{}", id),
                    PropertyType::On,
                    false,
                    Some(format!("switch:{}", id)),
                    output,
                ));

                properties.insert(switch_property.name().to_owned(), switch_property);
            }
            Component::Unsupported => {}
        }
    }

    if properties.is_empty() {
        return None;
    }

    Some(Device {
        id: device_info.id,
        r#type: DeviceType::Switch,
        manufacturer: "Shelly Europe Ltd.".to_string(),
        model_id: device_info.model,
        product_name: device_info.app,
        name: device_info.name,
        claimable: true,
        properties,
        external_id: None,
        address: Some(address),
        controller_id: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::property::PropertyType;
    use pretty_assertions::assert_eq;

    fn plus_1_device_info() -> DeviceInfo {
        serde_json::from_str(include_str!("../../tests/resources/shelly_device_info_plus1_response.json")).unwrap()
    }

    fn components(components: &[String]) -> ComponentsResult {
        serde_json::from_str(&format!(r#"{{"components":[{}]}}"#, components.join(","))).unwrap()
    }

    fn switch_component(id: u32, output: bool) -> String {
        format!(r#"{{"key":"switch:{id}","status":{{"id":{id},"output":{output}}},"config":{{}}}}"#)
    }

    fn ble_component() -> String {
        r#"{"key":"ble","status":{},"config":{"enable":false}}"#.to_string()
    }

    #[test]
    fn maps_a_plus_1_to_a_switch_device() {
        let components: ComponentsResult =
            serde_json::from_str(include_str!("../../tests/resources/shelly_components_plus1_response.json")).unwrap();

        let device = map_shelly(plus_1_device_info(), components, "192.168.1.35:80".to_string()).unwrap();

        assert_eq!(device.id, "shellyplus1-aabbccddeeff");
        assert_eq!(device.r#type, DeviceType::Switch);
        assert_eq!(device.manufacturer, "Shelly Europe Ltd.");
        assert_eq!(device.model_id, "SNSW-001X16EU");
        assert_eq!(device.product_name, "Plus1");
        assert_eq!(device.name, "Hallway");
        assert_eq!(device.address.as_deref(), Some("192.168.1.35:80"));
        assert_eq!(device.properties.len(), 1);
    }

    #[test]
    fn maps_a_switch_to_a_boolean_property_with_the_component_key_as_external_id() {
        let device = map_shelly(
            plus_1_device_info(),
            components(&[ble_component(), switch_component(0, true)]),
            "192.168.1.35:80".to_string(),
        )
        .unwrap();

        let property = device.get_property::<BooleanProperty>("switch0").unwrap();
        assert!(property.value());
        assert_eq!(property.property_type(), PropertyType::On);
        assert!(!property.readonly());
        assert_eq!(property.external_id(), Some("switch:0"));
    }

    #[test]
    fn maps_every_switch_to_its_own_property() {
        let device = map_shelly(
            plus_1_device_info(),
            components(&[switch_component(0, true), switch_component(1, false)]),
            "192.168.1.35:80".to_string(),
        )
        .unwrap();

        assert_eq!(device.properties.len(), 2);
        assert!(device.get_property::<BooleanProperty>("switch0").unwrap().value());
        assert!(!device.get_property::<BooleanProperty>("switch1").unwrap().value());
        assert_eq!(device.get_property::<BooleanProperty>("switch1").unwrap().external_id(), Some("switch:1"));
    }

    #[test]
    fn returns_none_for_a_device_without_switches() {
        let device = map_shelly(plus_1_device_info(), components(&[ble_component()]), "192.168.1.35:80".to_string());

        assert_eq!(device, None);
    }
}
