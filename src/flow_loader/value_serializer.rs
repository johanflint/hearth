use crate::domain::{Number, Value};
use serde::ser::{Error, SerializeMap};
use serde::{Serialize, Serializer};

impl Serialize for Value {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Value::Boolean(value) => serializer.serialize_bool(*value),
            Value::Number(Number::PositiveInt(value)) => serializer.serialize_u64(*value),
            Value::Number(Number::NegativeInt(value)) => serializer.serialize_i64(*value),
            Value::Number(Number::Float(value)) => serializer.serialize_f64(*value),
            Value::String(value) => serializer.serialize_str(value),
            Value::Color(color) => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("type", "color")?;
                map.serialize_entry("value", color)?;
                map.end()
            }
            // The deserializer would read it back as a Value::String
            Value::DateTime(_) => Err(S::Error::custom("a date-time can't be stored")),
            Value::None => Err(S::Error::custom("none can't be stored")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Color;
    use crate::domain::property::CartesianCoordinate;
    use chrono::Utc;
    use rstest::rstest;
    use serde_json::json;

    #[rstest]
    #[case::boolean(Value::Boolean(true), json!(true))]
    #[case::positive_int(Value::Number(Number::PositiveInt(1337)), json!(1337))]
    #[case::negative_int(Value::Number(Number::NegativeInt(-1337)), json!(-1337))]
    #[case::float(Value::Number(Number::Float(13.37)), json!(13.37))]
    #[case::string(Value::String("hearth".to_string()), json!("hearth"))]
    #[case::color_hex(Value::Color(Color::Hex("#ff0000".to_string())), json!({ "type": "color", "value": "#ff0000" }))]
    #[case::color_rgb(Value::Color(Color::RGB(255, 0, 0)), json!({ "type": "color", "value": { "r": 255, "g": 0, "b": 0 } }))]
    #[case::color_xy_brightness(Value::Color(Color::CIE_xyY { xy: CartesianCoordinate::new(0.675, 0.322), brightness: 0.2126 }), json!({ "type": "color", "value": { "x": 0.675, "y": 0.322, "brightness": 0.2126 } }))]
    fn serialize_values(#[case] value: Value, #[case] expected: serde_json::Value) {
        let json = serde_json::to_value(&value).unwrap();

        assert_eq!(json, expected);
        assert_eq!(serde_json::from_value::<Value>(json).unwrap(), value, "must deserialize to the same value");
    }

    #[rstest]
    #[case::date_time(Value::DateTime(Utc::now()), "a date-time can't be stored")]
    #[case::none(Value::None, "none can't be stored")]
    fn serialize_unsupported_values(#[case] value: Value, #[case] expected: &str) {
        let error = serde_json::to_value(&value).unwrap_err();

        assert_eq!(error.to_string(), expected);
    }
}
