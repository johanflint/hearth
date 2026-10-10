use crate::flow_engine::{Expression, Value};
use serde::Deserialize;
use std::time::Duration;

#[derive(PartialEq, Deserialize, Debug)]
pub struct PropertyCommand {
    pub value: Expression,
    #[serde(default, with = "humantime_serde")]
    pub transition: Option<Duration>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct ResolvedPropertyCommand {
    pub value: Value,
    pub transition: Option<Duration>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Number;
    use crate::flow_engine::Expression::Literal;
    use crate::flow_engine::Value;
    use crate::test_support::property_command;
    use serde_json::json;

    #[test]
    fn deserialize_boolean_value_without_transition() {
        let json = r#"
          {
            "value": {
                "type": "literal",
                "value": true
            }
          }
        "#;

        let response = serde_json::from_str::<PropertyCommand>(json);
        assert_eq!(response.unwrap(), property_command(Value::Boolean(true)));
    }

    #[test]
    fn deserialize_boolean_value_with_transition() {
        let json = r#"
          {
            "value": {
                "type": "literal",
                "value": true
            },
            "transition": "10m"
          }
        "#;

        let response = serde_json::from_str::<PropertyCommand>(json);
        assert_eq!(response.unwrap(), PropertyCommand { value: Literal { value: Value::Boolean(true) }, transition: Some(Duration::from_secs(600)) });
    }

    #[test]
    fn deserialize_property_command_returns_error_for_an_invalid_transition() {
        let json = r#"
          {
            "value": {
                "type": "literal",
                "value": true
            },
            "transition": "soon"
          }
        "#;

        let response = serde_json::from_str::<PropertyCommand>(json);
        assert_eq!(response.unwrap_err().to_string(), "invalid value: string \"soon\", expected a duration at line 7 column 32");
    }

    #[test]
    fn deserialize_property_command_returns_error_for_a_missing_value() {
        let json = r#"
          {
            "transition": "10m"
          }
        "#;

        let response = serde_json::from_str::<PropertyCommand>(json);
        assert_eq!(response.unwrap_err().to_string(), "missing field `value` at line 4 column 11");
    }

    #[test]
    fn deserialize_property_command_returns_error_for_an_invalid_value() {
        let json = r#"
          {
            "value": true
          }
        "#;

        let response = serde_json::from_str::<PropertyCommand>(json);
        assert!(response.unwrap_err().to_string().starts_with("invalid type: boolean `true`, expected internally tagged enum Expression"));
    }

    #[test]
    fn deserialize_number_value() {
        let json = json!({
            "value": { "type": "literal", "value": 5 }
        });

        let response = serde_json::from_value::<PropertyCommand>(json);
        assert_eq!(response.unwrap(), property_command(Value::Number(Number::PositiveInt(5))));
    }
}
