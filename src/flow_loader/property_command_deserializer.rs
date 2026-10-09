use crate::flow_engine::property_command::{Operation, PropertyCommand};
use crate::flow_engine::{Expression, Value};
use serde::de::Error;
use serde::{Deserialize, Deserializer};
use std::time::Duration;

impl<'de> Deserialize<'de> for PropertyCommand {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let command = serde_json::Value::deserialize(deserializer)?;

        let operation = command.get("operation").ok_or_else(|| Error::custom("missing field 'operation'"))?;
        let operation = Operation::deserialize(operation).map_err(|e| Error::custom(format!("invalid field 'operation': {e}")))?;

        let value = match (command.get("value"), &operation) {
            (Some(value), _) => Expression::deserialize(value).map_err(|e| Error::custom(format!("invalid field 'value': {e}")))?,
            (None, Operation::Toggle) => Expression::Literal { value: Value::None },
            (None, _) => return Err(Error::custom("missing field 'value'")),
        };
        let transition = command
            .get("transition")
            .map(humantime_serde::deserialize::<Duration, _>)
            .transpose()
            .map_err(|e| Error::custom(format!("invalid field 'transition': {e}")))?;

        Ok(PropertyCommand { operation, value, transition })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Number;
    use crate::flow_engine::Expression::Literal;
    use crate::test_support::property_command;
    use rstest::rstest;
    use serde_json::json;

    #[test]
    fn deserialize_set_boolean_value_without_transition() {
        let json = r#"
          {
            "operation": "set",
            "value": {
                "type": "literal",
                "value": true
            }
          }
        "#;

        let response = serde_json::from_str::<PropertyCommand>(json);
        assert_eq!(response.unwrap(), property_command(Operation::Set, Value::Boolean(true)));
    }

    #[test]
    fn deserialize_set_boolean_value_with_transition() {
        let json = r#"
          {
            "operation": "set",
            "value": {
                "type": "literal",
                "value": true
            },
            "transition": "10m"
          }
        "#;

        let response = serde_json::from_str::<PropertyCommand>(json);
        assert_eq!(response.unwrap(), PropertyCommand { operation: Operation::Set, value: Literal { value: Value::Boolean(true) }, transition: Some(Duration::from_secs(600)) });
    }

    #[test]
    fn deserialize_property_command_returns_error_for_an_invalid_transition() {
        let json = r#"
          {
            "operation": "set",
            "value": {
                "type": "literal",
                "value": true
            },
            "transition": "soon"
          }
        "#;

        let response = serde_json::from_str::<PropertyCommand>(json);
        assert_eq!(response.unwrap_err().to_string(), "invalid field 'transition': invalid value: string \"soon\", expected a duration");
    }

    #[test]
    fn deserialize_property_command_returns_error_for_a_missing_value() {
        let json = r#"
          {
            "operation": "set",
            "transition": "10m"
          }
        "#;

        let response = serde_json::from_str::<PropertyCommand>(json);
        assert_eq!(response.unwrap_err().to_string(), "missing field 'value'");
    }

    #[test]
    fn deserialize_property_command_returns_error_for_a_missing_operation() {
        let json = r#"
          {
            "value": {
                "type": "literal",
                "value": true
            }
          }
        "#;

        let response = serde_json::from_str::<PropertyCommand>(json);
        assert_eq!(response.unwrap_err().to_string(), "missing field 'operation'");
    }

    #[test]
    fn deserialize_property_command_returns_error_for_an_unknown_operation() {
        let json = r#"
          {
            "operation": "flip",
            "value": {
                "type": "literal",
                "value": true
            }
          }
        "#;

        let response = serde_json::from_str::<PropertyCommand>(json);
        assert_eq!(response.unwrap_err().to_string(), "invalid field 'operation': unknown variant `flip`, expected one of `set`, `toggle`, `increment`, `decrement`");
    }

    #[test]
    fn deserialize_property_command_returns_error_for_an_invalid_value() {
        let json = r#"
          {
            "operation": "set",
            "value": true
          }
        "#;

        let response = serde_json::from_str::<PropertyCommand>(json);
        assert!(response.unwrap_err().to_string().starts_with("invalid field 'value': "));
    }

    #[test]
    fn deserialize_toggle_property_command_without_value() {
        let json = r#"
          {
            "operation": "toggle"
          }
        "#;

        let response = serde_json::from_str::<PropertyCommand>(json);
        assert_eq!(response.unwrap(), property_command(Operation::Toggle, Value::None));
    }

    #[rstest]
    #[case::set("set", Operation::Set)]
    #[case::toggle("toggle", Operation::Toggle)]
    #[case::increment("increment", Operation::Increment)]
    #[case::decrement("decrement", Operation::Decrement)]
    fn deserialize_property_command_operations(#[case] operation: &str, #[case] expected: Operation) {
        let json = json!({
            "operation": operation,
            "value": { "type": "literal", "value": 5 }
        });

        let response = serde_json::from_value::<PropertyCommand>(json);
        assert_eq!(response.unwrap(), property_command(expected, Value::Number(Number::PositiveInt(5))));
    }
}
