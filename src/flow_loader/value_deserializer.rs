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
            _ => Err(serde::de::Error::custom("expected the value to be a boolean or a number")),
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
    fn deserialize() {
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
        println!("{:#?}", expression);
        assert_eq!(expression, expected);
    }
}
