use crate::flow_engine::property_value::PropertyValue;
use serde::de::Error;
use serde::{Deserialize, Deserializer};
use std::ops::IndexMut;

impl<'de> Deserialize<'de> for PropertyValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut value: serde_json::Value = Deserialize::deserialize(deserializer)?;
        let kind = value.get("type").and_then(|v| v.as_str()).ok_or_else(|| Error::custom("missing field 'type'"))?;

        match kind {
            "boolean" => {
                let value = value
                    .index_mut("value")
                    .as_bool()
                    .ok_or_else(|| Error::custom(format!("expected field '{}' of type '{}'", "value", "boolean")))?;
                Ok(PropertyValue::SetBooleanValue(value))
            }
            "toggle" => Ok(PropertyValue::ToggleBooleanValue),
            _ => Err(Error::custom(format!("unknown property type '{}'", kind))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_set_boolean_value() {
        let json = r#"
          {
            "type": "boolean",
            "value": true
          }
        "#;

        let response = serde_json::from_str::<PropertyValue>(json);
        assert!(response.is_ok());
        assert_eq!(response.unwrap(), PropertyValue::SetBooleanValue(true));
    }
}
