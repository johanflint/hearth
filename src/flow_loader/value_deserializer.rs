use crate::domain::Number;
use crate::domain::color::Color;
use crate::flow_engine::Value;
use serde::de::Error;
use serde::{Deserialize, Deserializer};
use serde_json::Number as JsonNumber;

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value: serde_json::Value = Deserialize::deserialize(deserializer)?;
        match value {
            serde_json::Value::Bool(value) => Ok(Value::Boolean(value)),
            serde_json::Value::Number(value) => Ok(Value::Number((&value).into())),
            serde_json::Value::String(value) => Ok(Value::String(value)),
            serde_json::Value::Object(object) => {
                match object.get("type").and_then(|kind| kind.as_str()) {
                    Some("color") => {
                        let value = object.get("value").ok_or_else(|| Error::missing_field("value"))?;
                        let color = Color::deserialize(value).map_err(|e| Error::custom(e.to_string()))?;
                        Ok(Value::Color(color))
                    },
                    Some(kind) => Err(Error::unknown_variant(kind, &["color"])),
                    None => Err(Error::missing_field("type")),
                }
            }
            _ => Err(Error::custom("expected the value to be a boolean, a color, a number or a string")),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Number;
    use crate::domain::property::CartesianCoordinate;
    use crate::flow_engine::Expression;
    use crate::flow_engine::Expression::{EqualTo, Literal};
    use rstest::rstest;
    use serde_json::json;

    #[test]
    fn deserialize_colors() {
        let json = json!({
            "type": "equalTo",
            "lhs": {
              "type": "literal",
              "value": { "type": "color", "value": "#FF0000" }
            },
            "rhs": {
              "type": "literal",
              "value": { "type": "color", "value": "#ff0000" }
            }
        });

        let expression = serde_json::from_value::<Expression>(json).unwrap();
        let expected = EqualTo {
            lhs: Box::new(Literal {
                value: Value::Color(Color::Hex("#ff0000".to_string())),
            }),
            rhs: Box::new(Literal {
                value: Value::Color(Color::Hex("#ff0000".to_string())),
            }),
        };
        assert_eq!(expression, expected);
    }

    #[rstest]
    #[case::hex(json!("#ff0000"), Color::Hex("#ff0000".to_string()))]
    #[case::rgb(json!({ "r": 255, "g": 0, "b": 0 }), Color::RGB(255, 0, 0))]
    #[case::xy_brightness(json!({ "x": 0.675, "y": 0.322, "brightness": 0.2126 }), Color::CIE_xyY { xy: CartesianCoordinate::new(0.675, 0.322), brightness: 0.2126 })]
    #[case::xy_luminance(json!({ "x": 0.675, "y": 0.322, "Y": 0.2126 }), Color::CIE_xyY { xy: CartesianCoordinate::new(0.675, 0.322), brightness: 0.2126 })]
    fn deserialize_color_formats(#[case] color: serde_json::Value, #[case] expected: Color) {
        let value = serde_json::from_value::<Value>(json!({ "type": "color", "value": color })).unwrap();

        assert_eq!(value, Value::Color(expected));
    }

    #[rstest]
    #[case::invalid_color(json!({ "type": "color", "value": "#f00" }), "invalid value: string \"#f00\", expected a 6-digit hex color")]
    #[case::missing_color_value(json!({ "type": "color" }), "missing field `value`")]
    #[case::unknown_type(json!({ "type": "temperature", "value": 20 }), "unknown variant `temperature`, expected `color`")]
    #[case::missing_type(json!({ "value": "#ff0000" }), "missing field `type`")]
    #[case::unsupported_value(json!([1, 2, 3]), "expected the value to be a boolean, a color, a number or a string")]
    fn deserialize_invalid_values(#[case] json: serde_json::Value, #[case] expected: &str) {
        let error = serde_json::from_value::<Value>(json).unwrap_err();

        assert_eq!(error.to_string(), expected);
    }

    #[rstest]
    #[case::positive_int(json!(1337), Number::PositiveInt(1337))]
    #[case::negative_int(json!(-1337), Number::NegativeInt(-1337))]
    #[case::float(json!(13.37), Number::Float(13.37))]
    fn deserialize_numbers(#[case] json: serde_json::Value, #[case] expected: Number) {
        assert_eq!(serde_json::from_value::<Value>(json).unwrap(), Value::Number(expected));
    }

    #[test]
    fn deserialize_strings() {
        let json = json!({
            "type": "equalTo",
            "lhs": {
              "type": "literal",
              "value": "short_release"
            },
            "rhs": {
              "type": "literal",
              "value": "short_release"
            }
        });

        let expression = serde_json::from_value::<Expression>(json).unwrap();
        let expected = EqualTo {
            lhs: Box::new(Literal {
                value: Value::String("short_release".to_string()),
            }),
            rhs: Box::new(Literal {
                value: Value::String("short_release".to_string()),
            }),
        };
        assert_eq!(expression, expected);
    }
}
