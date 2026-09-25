use crate::domain::Connectivity;
use crate::domain::device::Device;
use crate::domain::property::{EnumProperty, Property, PropertyType};
use crate::hue::domain::ZigbeeConnectivityGet;
use std::collections::HashMap;
use strum::IntoEnumIterator;

pub fn enrich_devices(connectivity_list: Vec<ZigbeeConnectivityGet>, devices: &mut Vec<Device>) {
    let connectivity_map: HashMap<String, Connectivity> = connectivity_list.into_iter()
        .map(|c| (c.owner.rid, map_connectivity_status(&c.status)))
        .collect();

    for device in devices {
        let status = connectivity_map.get(&device.id).unwrap_or_else(|| &Connectivity::Unknown);
        let allowed_values = Connectivity::iter().map(|c| c.as_str().to_string()).collect();

        let connectivity_property = Box::new(
            // The allowed values always contains every `Connectivity` variant and `status` is always one of these, so it can never fail
            EnumProperty::new("connectivity".to_string(), PropertyType::Connectivity, true, None, Some(status.as_str().to_string()), allowed_values)
                .expect("connectivity status is always one of the declared allowed values")
        );

        device.properties.insert(connectivity_property.name().to_string(), connectivity_property);
    }
}

fn map_connectivity_status(status: &str) -> Connectivity {
    match status {
        "connected" => Connectivity::Connected,
        "connectivity_issue" | "unidirectional_incoming" => Connectivity::Issues,
        "disconnected" => Connectivity::Disconnected,
        _ => Connectivity::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hue::domain::Owner;
    use crate::test_support::DeviceBuilder;
    use rstest::rstest;

    #[test]
    fn enrich_devices_adds_a_connectivity_property_matched_by_owner_rid() {
        let device = DeviceBuilder::new("device").build();
        let mut devices = vec![device];
        let connectivity_list = vec![ZigbeeConnectivityGet {
            id: "connectivity".to_string(),
            owner: Owner { rid: "device".to_string(), rtype: "device".to_string() },
            status: "connected".to_string()
        }];

        enrich_devices(connectivity_list, &mut devices);

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

        enrich_devices(vec![], &mut devices);

        let property = devices[0].properties.get("connectivity").unwrap().as_any().downcast_ref::<EnumProperty>().unwrap();
        assert_eq!(property.value(), Some("unknown"));
    }

    #[rstest]
    #[case::maps_connected("connected", Connectivity::Connected)]
    #[case::maps_connectivity_issue("connectivity_issue", Connectivity::Issues)]
    #[case::maps_unidirectional_incoming("unidirectional_incoming", Connectivity::Issues)]
    #[case::maps_disconnected("disconnected", Connectivity::Disconnected)]
    #[case::maps_unknown("some_unrecognized_value", Connectivity::Unknown)]
    fn test_map_connectivity_status(#[case] value: String, #[case] expected: Connectivity) {
        let result = map_connectivity_status(&value);
        assert_eq!(result, expected);
    }
}