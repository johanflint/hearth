use crate::flow_engine::Value;
use serde::{Deserialize, Deserializer};

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
            _ => Err(serde::de::Error::custom("expected the value to be a boolean, a number or a string")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Number;
    use crate::flow_engine::Expression;
    use crate::flow_engine::Expression::{EqualTo, Literal};
    use serde_json::json;

    #[test]
    fn deserialize_numbers() {
        let json = json!({
            "type": "equalTo",
            "lhs": {
              "type": "literal",
              "value": 42
            },
            "rhs": {
              "type": "literal",
              "value": 42.0
            }
        });

        let expression = serde_json::from_value::<Expression>(json).unwrap();
        let expected = EqualTo {
            lhs: Box::new(Literal {
                value: Value::Number(Number::PositiveInt(42)),
            }),
            rhs: Box::new(Literal {
                value: Value::Number(Number::Float(42.0)),
            }),
        };
        assert_eq!(expression, expected);
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
