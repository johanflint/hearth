use crate::hue::domain::LightChanged;
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::ops::IndexMut;

#[derive(Debug, Deserialize)]
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
    #[serde(untagged)]
    Unknown(UnknownProperty),
}

#[derive(Debug)]
pub struct UnknownProperty {
    pub property_type: String,
}

impl<'de> Deserialize<'de> for UnknownProperty {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut value = Value::deserialize(deserializer)?;
        match value.index_mut("type").take() {
            Value::String(property_type) => Ok(UnknownProperty { property_type }),
            _ => Err(serde::de::Error::missing_field("type")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(matches!(&first_result.data[0], ChangedProperty::Unknown(UnknownProperty { property_type }) if property_type == "unknown"));

        Ok(())
    }
}
