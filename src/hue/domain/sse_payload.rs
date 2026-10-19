use crate::hue::domain::{
    ButtonChanged, ContactChanged, DevicePowerChanged, LightChanged, LightLevelChanged, MotionChanged, RoomChanged,
    TemperatureChanged, ZigbeeConnectivityChanged,
};
use serde::{Deserialize, Deserializer};
use serde_json::{Value, to_string_pretty};

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct ServerSentEventPayload {
    pub id: String,
    pub r#type: DataType,
    #[serde(rename = "creationtime")]
    pub creation_time: String,
    pub data: Vec<ChangedProperty>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DataType {
    Add,
    Update,
    Delete,
    Error,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChangedProperty {
    Light(LightChanged),
    Motion(MotionChanged),
    LightLevel(LightLevelChanged),
    Button(ButtonChanged),
    ZigbeeConnectivity(ZigbeeConnectivityChanged),
    DevicePower(DevicePowerChanged),
    Temperature(TemperatureChanged),
    Contact(ContactChanged),
    Room(RoomChanged),
    #[serde(untagged)]
    Unknown(UnknownProperty),
}

#[derive(Debug)]
pub struct UnknownProperty {
    pub property_type: String,
    pub value: String,
}

impl<'de> Deserialize<'de> for UnknownProperty {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut value = Value::deserialize(deserializer)?;
        let string_value = to_string_pretty(&value).unwrap();

        let property_type = match value.as_object_mut().and_then(|map| map.get_mut("type")) {
            Some(type_value) => match type_value.take() {
                Value::String(property_type) => property_type,
                _ => return Err(serde::de::Error::missing_field("type")),
            },
            None => return Err(serde::de::Error::missing_field("type")),
        };

        Ok(UnknownProperty { property_type, value: string_value })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hue::domain::ContactState;

    #[test]
    fn deserialize_a_known_property_type() -> Result<(), serde_json::Error> {
        let json = r#"
        [
          {
            "creationtime": "2025-03-07T19:13:41Z",
            "data": [
              {
                "id": "31e6d98c-09ca-4538-b4ea-b57c8c540b3e",
                "id_v1": "/lights/22",
                "on": {
                  "on": false
                },
                "owner": {
                  "rid": "84a3be14-5d90-4165-ac64-818b7981bb32",
                  "rtype": "device"
                },
                "service_id": 0,
                "type": "light"
              }
            ],
            "id": "11c2f169-9c29-444b-9ef6-4868f6d2daf6",
            "type": "update"
          }
        ]
        "#;

        let result = serde_json::from_str::<Vec<ServerSentEventPayload>>(json)?;
        assert_eq!(result.len(), 1);
        let first_result = &result[0];
        assert_eq!(first_result.data.len(), 1);
        assert!(matches!(&first_result.data[0], ChangedProperty::Light(LightChanged { .. })));

        Ok(())
    }

    #[test]
    fn deserialize_a_light_level_property() -> Result<(), serde_json::Error> {
        let json = r#"
        [
          {
            "creationtime": "2026-09-23T15:37:21Z",
            "data": [
              {
                "id": "a3f1c2d4-6b7e-4c1a-9d2e-5f8b0c3a7e11",
                "owner": {
                  "rid": "dbe91174-8fe5-4309-b7d7-3f15ec1e57d3",
                  "rtype": "device"
                },
                "light": {
                  "light_level": 12000,
                  "light_level_valid": true,
                  "light_level_report": {
                    "changed": "2026-09-23T15:37:21.003Z",
                    "light_level": 12000
                  }
                },
                "type": "light_level"
              }
            ],
            "id": "11c2f169-9c29-444b-9ef6-4868f6d2daf6",
            "type": "update"
          }
        ]
        "#;

        let result = serde_json::from_str::<Vec<ServerSentEventPayload>>(json)?;
        let ChangedProperty::LightLevel(LightLevelChanged { id, owner, light }) = &result[0].data[0] else {
            panic!("expected a LightLevel variant");
        };
        assert_eq!(id, "a3f1c2d4-6b7e-4c1a-9d2e-5f8b0c3a7e11");
        assert_eq!(owner.rid, "dbe91174-8fe5-4309-b7d7-3f15ec1e57d3");
        let report =
            light.as_ref().and_then(|l| l.light_level_report.as_ref()).expect("light_level_report should be present");
        assert_eq!(report.light_level, 12000);

        Ok(())
    }

    #[test]
    fn deserialize_a_device_power_property() -> Result<(), serde_json::Error> {
        let json = r#"
        [
          {
            "creationtime": "2025-03-07T19:13:41Z",
            "data": [
              {
                "id": "5568ab93-2bef-4739-a667-8beb87898f78",
                "owner": {
                  "rid": "3a3225cb-dcda-46fb-8f21-00a8c76024bc",
                  "rtype": "device"
                },
                "power_state": {
                  "battery_level": 56,
                  "battery_state": "normal"
                },
                "type": "device_power"
              }
            ],
            "id": "11c2f169-9c29-444b-9ef6-4868f6d2daf6",
            "type": "update"
          }
        ]
        "#;

        let result = serde_json::from_str::<Vec<ServerSentEventPayload>>(json)?;
        assert_eq!(result.len(), 1);
        let first_result = &result[0];
        assert_eq!(first_result.data.len(), 1);
        assert!(matches!(&first_result.data[0], ChangedProperty::DevicePower(DevicePowerChanged { .. })));
        let ChangedProperty::DevicePower(DevicePowerChanged { id, owner, power_state }) = &first_result.data[0] else {
            panic!("expected a DevicePower variant");
        };
        assert_eq!(id, "5568ab93-2bef-4739-a667-8beb87898f78");
        assert_eq!(owner.rid, "3a3225cb-dcda-46fb-8f21-00a8c76024bc");
        let power_state = power_state.as_ref().expect("power_state should be present");
        assert_eq!(power_state.battery_level, Some(56));
        assert_eq!(power_state.battery_state, Some("normal".to_string()));

        Ok(())
    }

    #[test]
    fn deserialize_a_temperature_property() -> Result<(), serde_json::Error> {
        let json = r#"
        [
          {
            "creationtime": "2026-09-23T15:37:21Z",
            "data": [
              {
                "id": "197f1e17-f04f-471d-b410-77b9267826e9",
                "owner": {
                  "rid": "dbe91174-8fe5-4309-b7d7-3f15ec1e57d3",
                  "rtype": "device"
                },
                "temperature": {
                  "temperature": 21.34,
                  "temperature_valid": true,
                  "temperature_report": {
                    "changed": "2026-09-23T15:37:21.003Z",
                    "temperature": 21.34
                  }
                },
                "type": "temperature"
              }
            ],
            "id": "11c2f169-9c29-444b-9ef6-4868f6d2daf6",
            "type": "update"
          }
        ]
        "#;

        let result = serde_json::from_str::<Vec<ServerSentEventPayload>>(json)?;
        let ChangedProperty::Temperature(TemperatureChanged { id, owner, temperature }) = &result[0].data[0] else {
            panic!("expected a Temperature variant");
        };
        assert_eq!(id, "197f1e17-f04f-471d-b410-77b9267826e9");
        assert_eq!(owner.rid, "dbe91174-8fe5-4309-b7d7-3f15ec1e57d3");
        let report = temperature
            .as_ref()
            .and_then(|t| t.temperature_report.as_ref())
            .expect("temperature_report should be present");
        assert_eq!(report.temperature, 21.34);

        Ok(())
    }

    #[test]
    fn deserialize_a_contact_property() -> Result<(), serde_json::Error> {
        let json = r#"
        [
          {
            "creationtime": "2026-09-23T15:37:22Z",
            "data": [
              {
                "id": "6d8f3b41-5b3c-4a52-8d1a-9f1b6a0c7e21",
                "owner": {
                  "rid": "c0a5a9b4-1b0a-4d6f-8d3a-2f4d6c1e9b77",
                  "rtype": "device"
                },
                "contact_report": {
                  "changed": "2026-09-23T15:37:22.004Z",
                  "state": "no_contact"
                },
                "type": "contact"
              }
            ],
            "id": "11c2f169-9c29-444b-9ef6-4868f6d2daf6",
            "type": "update"
          }
        ]
        "#;

        let result = serde_json::from_str::<Vec<ServerSentEventPayload>>(json)?;
        let ChangedProperty::Contact(ContactChanged { id, owner, contact_report }) = &result[0].data[0] else {
            panic!("expected a Contact variant");
        };
        assert_eq!(id, "6d8f3b41-5b3c-4a52-8d1a-9f1b6a0c7e21");
        assert_eq!(owner.rid, "c0a5a9b4-1b0a-4d6f-8d3a-2f4d6c1e9b77");
        assert_eq!(contact_report.as_ref().expect("contact_report should be present").state, ContactState::NoContact);

        Ok(())
    }

    #[test]
    fn deserialize_an_unknown_property_types_returns_unknown() -> Result<(), serde_json::Error> {
        let json = r#"
        [
          {
            "creationtime": "2025-03-07T19:13:41Z",
            "data": [
              {
                "id": "31e6d98c-09ca-4538-b4ea-b57c8c540b3e",
                "id_v1": "/lights/22",
                "on": {
                  "on": false
                },
                "owner": {
                  "rid": "84a3be14-5d90-4165-ac64-818b7981bb32",
                  "rtype": "device"
                },
                "service_id": 0,
                "type": "unknown"
              }
            ],
            "id": "11c2f169-9c29-444b-9ef6-4868f6d2daf6",
            "type": "update"
          }
        ]
        "#;

        let result = serde_json::from_str::<Vec<ServerSentEventPayload>>(json)?;
        assert_eq!(result.len(), 1);
        let first_result = &result[0];
        assert_eq!(first_result.data.len(), 1);
        assert!(
            matches!(&first_result.data[0], ChangedProperty::Unknown(UnknownProperty { property_type, .. }) if property_type == "unknown")
        );

        Ok(())
    }
}
