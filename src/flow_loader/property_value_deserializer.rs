use crate::domain::Number;
use crate::flow_engine::property_value::PropertyValue;
use serde::de::Error;
use serde::{Deserialize, Deserializer};
use serde_json::Number as JsonNumber;
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
                let value = value.index_mut("value").as_bool().ok_or_missing("value", "boolean")?;
                Ok(PropertyValue::SetBooleanValue(value))
            }
            "toggle" => Ok(PropertyValue::ToggleBooleanValue),
            "number" => {
                let value = value.index_mut("value").as_number().ok_or_missing("value", "number")?;
                Ok(PropertyValue::SetNumberValue(value.into()))
            }
            "increment" => {
                let value = value.index_mut("value").as_number().ok_or_missing("value", "number")?;
                Ok(PropertyValue::IncrementNumberValue(value.into()))
            }
            "decrement" => {
                let value = value.index_mut("value").as_number().ok_or_missing("value", "number")?;
                Ok(PropertyValue::DecrementNumberValue(value.into()))
            }
            _ => Err(Error::custom(format!("unknown property type '{}'", kind))),
        }
    }
}

impl From<&JsonNumber> for Number {
    fn from(value: &JsonNumber) -> Self {
        if let Some(int_value) = value.as_u64() {
            Number::PositiveInt(int_value)
        } else if let Some(int_value) = value.as_i64() {
            Number::NegativeInt(int_value)
        } else if let Some(float_value) = value.as_f64() {
            Number::Float(float_value)
        } else {
            panic!("Converting json value {} to Number failed", value)
        }
    }
}

trait OptionResultExt<T, E>
where
    E: Error + Sized,
{
    fn ok_or_missing(self, msg: &'static str, datatype: &'static str) -> Result<T, E>;
}

impl<T, E> OptionResultExt<T, E> for Option<T>
where
    E: Error + Sized,
{
    fn ok_or_missing(self, field: &'static str, datatype: &'static str) -> Result<T, E> {
        self.ok_or_else(|| Error::custom(format!("expected field '{}' of type '{}'", field, datatype)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

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

    #[test]
    fn deserialize_toggle_value() {
        let json = r#"
          {
            "type": "toggle"
          }
        "#;

        let response = serde_json::from_str::<PropertyValue>(json);
        assert!(response.is_ok());
        assert_eq!(response.unwrap(), PropertyValue::ToggleBooleanValue);
    }

    #[rstest]
    #[case::for_positive_int("1337", Number::PositiveInt(1337))]
    #[case::for_negative_int("-1337", Number::NegativeInt(-1337))]
    #[case::for_float("13.37", Number::Float(13.37))]
    fn deserialize_set_number_value(#[case] json_value: String, #[case] expected_number: Number) {
        let json = format!(
            r#"{{
                "type": "number",
                "value": {}
            }}"#,
            json_value
        );

        let response = serde_json::from_str::<PropertyValue>(&json);
        assert!(response.is_ok());
        assert_eq!(response.unwrap(), PropertyValue::SetNumberValue(expected_number));
    }

    #[rstest]
    #[case::for_positive_int("1337", Number::PositiveInt(1337))]
    #[case::for_negative_int("-1337", Number::NegativeInt(-1337))]
    #[case::for_float("13.37", Number::Float(13.37))]
    fn deserialize_increment_number_value(#[case] json_value: String, #[case] expected_number: Number) {
        let json = format!(
            r#"{{
                "type": "increment",
                "value": {}
            }}"#,
            json_value
        );

        let response = serde_json::from_str::<PropertyValue>(&json);
        assert!(response.is_ok());
        assert_eq!(response.unwrap(), PropertyValue::IncrementNumberValue(expected_number));
    }

    #[rstest]
    #[case::for_positive_int("1337", Number::PositiveInt(1337))]
    #[case::for_negative_int("-1337", Number::NegativeInt(-1337))]
    #[case::for_float("13.37", Number::Float(13.37))]
    fn deserialize_decrement_number_value(#[case] json_value: String, #[case] expected_number: Number) {
        let json = format!(
            r#"{{
                "type": "decrement",
                "value": {}
            }}"#,
            json_value
        );

        let response = serde_json::from_str::<PropertyValue>(&json);
        assert!(response.is_ok());
        assert_eq!(response.unwrap(), PropertyValue::DecrementNumberValue(expected_number));
    }
}
