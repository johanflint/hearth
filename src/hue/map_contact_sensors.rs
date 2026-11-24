use crate::domain::property::{BooleanProperty, DateTimeProperty, EnumProperty, Property, PropertyType};
use crate::domain::{ContactState, Device, DeviceType};
use crate::hue::controller::CONTROLLER_ID;
use crate::hue::domain::{ContactGet, ContactState as HueContactState, DeviceGet, TamperGet, TamperState};
use std::collections::HashMap;
use strum::IntoEnumIterator;
use thiserror::Error;

pub fn map_contact_sensors(
    sensors: Vec<ContactGet>,
    tamper_states: Vec<TamperGet>,
    device_map: &mut HashMap<String, DeviceGet>,
) -> Result<Vec<Device>, MapContactSensorsError> {
    let mut tamper_by_owner: HashMap<String, TamperGet> =
        tamper_states.into_iter().map(|tamper| (tamper.owner.rid.clone(), tamper)).collect();

    sensors
        .into_iter()
        .map(|sensor| {
            let tamper = tamper_by_owner.remove(&sensor.owner.rid);

            let device_id = sensor.owner.rid;
            let device_get =
                device_map.remove(&device_id).ok_or(MapContactSensorsError::UnknownDevice { device_id })?;

            let mut properties = HashMap::with_capacity(4);

            let value = sensor.contact_report.as_ref().map(|report| ContactState::from(&report.state).to_string());
            let allowed_values = ContactState::iter().map(|s| s.to_string()).collect();
            let contact_property: Box<dyn Property> = Box::new(
                EnumProperty::new(
                    "contact".to_string(),
                    PropertyType::ContactSensor,
                    true,
                    Some(sensor.id.clone()),
                    value,
                    allowed_values,
                )
                .expect("contact value is always one of the allowed values"),
            );
            properties.insert(contact_property.name().to_owned(), contact_property);

            let contact_last_changed_property = Box::new(DateTimeProperty::new(
                "contactLastChanged".to_string(),
                PropertyType::ContactSensorLastChanged,
                true,
                None,
                sensor.contact_report.map(|report| report.changed),
            ));
            properties.insert(contact_last_changed_property.name().to_owned(), contact_last_changed_property);

            if let Some(tamper) = tamper {
                let tampered = tamper.tamper_reports.iter().any(|report| report.state == TamperState::Tampered);
                let tampered_property = Box::new(BooleanProperty::new(
                    "tampered".to_string(),
                    PropertyType::Tampered,
                    true,
                    Some(tamper.id),
                    tampered,
                ));
                properties.insert(tampered_property.name().to_owned(), tampered_property);

                let tampered_last_changed_property = Box::new(DateTimeProperty::new(
                    "tamperedLastChanged".to_string(),
                    PropertyType::TamperedLastChanged,
                    true,
                    None,
                    tamper.tamper_reports.iter().map(|report| report.changed).max(),
                ));
                properties.insert(tampered_last_changed_property.name().to_owned(), tampered_last_changed_property);
            }

            Ok(Device {
                id: device_get.id,
                r#type: DeviceType::ContactSensor,
                manufacturer: device_get.product_data.manufacturer_name,
                model_id: device_get.product_data.model_id,
                product_name: device_get.product_data.product_name,
                name: device_get.metadata.name,
                claimable: false,
                properties,
                external_id: None,
                address: None,
                controller_id: Some(CONTROLLER_ID),
            })
        })
        .collect()
}

impl From<&HueContactState> for ContactState {
    fn from(state: &HueContactState) -> Self {
        match state {
            HueContactState::Contact => ContactState::Closed,
            HueContactState::NoContact => ContactState::Open,
        }
    }
}

#[derive(Error, Debug)]
pub enum MapContactSensorsError {
    #[error("unknown device '{device_id}'")]
    UnknownDevice { device_id: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hue::domain::{Metadata, ProductData};
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use serde_json::{Value, json};

    const DEVICE_ID: &str = "c0a5a9b4-1b0a-4d6f-8d3a-2f4d6c1e9b77";
    const CONTACT_ID: &str = "6d8f3b41-5b3c-4a52-8d1a-9f1b6a0c7e21";
    const TAMPER_ID: &str = "3b7e1c94-2f6a-4d8e-9c15-7a0d4e8b6f32";

    fn device_map() -> HashMap<String, DeviceGet> {
        let device_get = DeviceGet {
            id: DEVICE_ID.to_string(),
            product_data: ProductData {
                model_id: "SOC001".to_string(),
                manufacturer_name: "Signify Netherlands B.V.".to_string(),
                product_name: "Hue secure contact sensor".to_string(),
                product_archetype: "unknown_archetype".to_string(),
                certified: true,
                software_version: "2.67.9".to_string(),
                hardware_platform_type: None,
            },
            metadata: Metadata { name: "Voordeur".to_string(), archetype: "unknown_archetype".to_string() },
        };

        HashMap::from([(device_get.id.clone(), device_get)])
    }

    fn contact_get(contact_report: Value) -> ContactGet {
        let mut json = json!({
            "id": CONTACT_ID,
            "owner": { "rid": DEVICE_ID, "rtype": "device" },
            "enabled": true,
            "type": "contact"
        });
        if !contact_report.is_null() {
            json["contact_report"] = contact_report;
        }

        serde_json::from_value(json).unwrap()
    }

    fn contact_property(value: Option<&str>) -> Box<dyn Property> {
        Box::new(
            EnumProperty::new(
                "contact".to_string(),
                PropertyType::ContactSensor,
                true,
                Some(CONTACT_ID.to_string()),
                value.map(str::to_string),
                ContactState::iter().map(|s| s.to_string()).collect(),
            )
            .unwrap(),
        )
    }

    fn contact_last_changed_property(value: Option<&str>) -> Box<dyn Property> {
        Box::new(DateTimeProperty::new(
            "contactLastChanged".to_string(),
            PropertyType::ContactSensorLastChanged,
            true,
            None,
            value.map(|v| v.parse().unwrap()),
        ))
    }

    #[test]
    fn maps_a_contact_sensor_to_a_device() {
        let mut device_map = device_map();
        let sensor = contact_get(json!({ "changed": "2026-09-23T15:37:22.004Z", "state": "contact" }));

        let result = map_contact_sensors(vec![sensor], vec![], &mut device_map).unwrap();

        assert_eq!(
            result,
            vec![Device {
                id: DEVICE_ID.to_string(),
                r#type: DeviceType::ContactSensor,
                manufacturer: "Signify Netherlands B.V.".to_string(),
                model_id: "SOC001".to_string(),
                product_name: "Hue secure contact sensor".to_string(),
                name: "Voordeur".to_string(),
                claimable: false,
                properties: HashMap::from([
                    ("contact".to_string(), contact_property(Some("closed"))),
                    ("contactLastChanged".to_string(), contact_last_changed_property(Some("2026-09-23T15:37:22.004Z"))),
                ]),
                external_id: None,
                address: None,
                controller_id: Some(CONTROLLER_ID),
            }]
        );
        assert!(device_map.is_empty());
    }

    #[rstest]
    #[case::closed(json!({ "changed": "2026-09-23T15:37:22.004Z", "state": "contact" }), Some("closed"))]
    #[case::open(json!({ "changed": "2026-09-23T15:37:22.004Z", "state": "no_contact" }), Some("open"))]
    #[case::unknown_without_report(Value::Null, None)]
    fn maps_the_contact_state(#[case] contact_report: Value, #[case] expected: Option<&str>) {
        let result = map_contact_sensors(vec![contact_get(contact_report)], vec![], &mut device_map()).unwrap();

        let property = result[0].get_property::<EnumProperty>("contact").unwrap();
        assert_eq!(property.value(), expected);
    }

    #[rstest]
    #[case::with_report(json!({ "changed": "2026-09-23T15:37:22.004Z", "state": "contact" }), Some("2026-09-23T15:37:22.004Z"))]
    #[case::without_report(Value::Null, None)]
    fn maps_the_contact_last_changed(#[case] contact_report: Value, #[case] expected: Option<&str>) {
        let result = map_contact_sensors(vec![contact_get(contact_report)], vec![], &mut device_map()).unwrap();

        let property = result[0].get_property::<DateTimeProperty>("contactLastChanged").unwrap();
        assert_eq!(property.value(), expected.map(|v| v.parse().unwrap()));
    }

    #[test]
    fn maps_a_sensor_without_a_report_to_unknown_properties() {
        let result = map_contact_sensors(vec![contact_get(Value::Null)], vec![], &mut device_map()).unwrap();

        assert_eq!(
            result[0].properties,
            HashMap::from([
                ("contact".to_string(), contact_property(None)),
                ("contactLastChanged".to_string(), contact_last_changed_property(None)),
            ])
        );
    }

    #[test]
    fn maps_no_sensors_to_no_devices() {
        let result = map_contact_sensors(vec![], vec![], &mut device_map()).unwrap();

        assert!(result.is_empty());
    }

    fn tamper_get(reports: Value) -> TamperGet {
        serde_json::from_value(json!({
            "id": TAMPER_ID,
            "owner": { "rid": DEVICE_ID, "rtype": "device" },
            "tamper_reports": reports,
            "type": "tamper"
        }))
        .unwrap()
    }

    fn contact_closed() -> ContactGet {
        contact_get(json!({ "changed": "2026-09-23T15:37:22.004Z", "state": "contact" }))
    }

    #[test]
    fn maps_the_tamper_properties() {
        let tamper = tamper_get(json!([
            { "changed": "2026-09-23T15:41:08.512Z", "source": "battery_door", "state": "tampered" }
        ]));

        let result = map_contact_sensors(vec![contact_closed()], vec![tamper], &mut device_map()).unwrap();

        let expected: Box<dyn Property> = Box::new(BooleanProperty::new(
            "tampered".to_string(),
            PropertyType::Tampered,
            true,
            Some(TAMPER_ID.to_string()),
            true,
        ));
        assert_eq!(&result[0].properties["tampered"], &expected);
        let last_changed = result[0].get_property::<DateTimeProperty>("tamperedLastChanged").unwrap();
        assert_eq!(last_changed.value(), Some("2026-09-23T15:41:08.512Z".parse().unwrap()));
    }

    #[rstest]
    #[case::tampered("tampered", true)]
    #[case::not_tampered("not_tampered", false)]
    fn maps_the_tampered_state(#[case] state: &str, #[case] expected: bool) {
        let tamper = tamper_get(json!([
            { "changed": "2026-09-23T15:41:08.512Z", "source": "battery_door", "state": state }
        ]));

        let result = map_contact_sensors(vec![contact_closed()], vec![tamper], &mut device_map()).unwrap();

        assert_eq!(result[0].get_property::<BooleanProperty>("tampered").unwrap().value(), expected);
    }

    #[test]
    fn maps_an_empty_tamper_report_list_to_not_tampered_without_a_time() {
        let result =
            map_contact_sensors(vec![contact_closed()], vec![tamper_get(json!([]))], &mut device_map()).unwrap();

        assert!(!result[0].get_property::<BooleanProperty>("tampered").unwrap().value());
        assert_eq!(result[0].get_property::<DateTimeProperty>("tamperedLastChanged").unwrap().value(), None);
    }

    #[test]
    fn omits_the_tamper_properties_without_a_tamper_resource() {
        let result = map_contact_sensors(vec![contact_closed()], vec![], &mut device_map()).unwrap();

        assert!(!result[0].properties.contains_key("tampered"));
        assert!(!result[0].properties.contains_key("tamperedLastChanged"));
    }

    #[test]
    fn ignores_the_tamper_resource_of_another_device() {
        let other = serde_json::from_value(json!({
            "id": TAMPER_ID,
            "owner": { "rid": "00000000-0000-0000-0000-000000000000", "rtype": "device" },
            "tamper_reports": [{ "changed": "2026-09-23T15:41:08.512Z", "source": "battery_door", "state": "tampered" }],
            "type": "tamper"
        }))
        .unwrap();

        let result = map_contact_sensors(vec![contact_closed()], vec![other], &mut device_map()).unwrap();

        assert!(!result[0].properties.contains_key("tampered"));
    }

    #[test]
    fn fails_for_an_unknown_device() {
        let sensor = contact_get(json!({ "changed": "2026-09-23T15:37:22.004Z", "state": "contact" }));

        let result = map_contact_sensors(vec![sensor], vec![], &mut HashMap::new());

        assert!(matches!(result, Err(MapContactSensorsError::UnknownDevice { device_id }) if device_id == DEVICE_ID));
    }
}
