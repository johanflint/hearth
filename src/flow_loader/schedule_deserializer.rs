use crate::flow_engine::{Schedule, WeekdayCondition};
use serde::de::{Error, Unexpected};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::str::FromStr;

impl<'de> Deserialize<'de> for Schedule {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;

        match value {
            Value::String(cron) => {
                // Validate the cron expression
                match cron::Schedule::from_str(&cron) {
                    Ok(_) => Ok(Schedule::Cron(cron)),
                    Err(_e) => Err(Error::invalid_value(Unexpected::Str(&cron), &"a valid cron expression")),
                }
            }
            Value::Object(map) => {
                let event = map.get("event").and_then(|v| v.as_str()).ok_or_else(|| Error::custom("missing or invalid field 'event'"))?;
                let when_value = map.get("when").ok_or_else(|| Error::custom("missing or invalid field 'when'"))?;
                let when = WeekdayCondition::deserialize(when_value).map_err(Error::custom)?;
                let offset = map.get("offset").and_then(|v| v.as_i64()).unwrap_or(0);

                match event {
                    "sunrise" => Ok(Schedule::Sunrise { when, offset }),
                    "sunset" => Ok(Schedule::Sunset { when, offset }),
                    _ => Err(Error::custom(format!("unknown schedule event '{}'", event))),
                }
            }
            _ => Err(Error::custom("a string containing a cron expression or an object with 'event', 'when' and 'offset'")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Weekday::*;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use serde_json::json;

    #[test]
    fn deserializes_a_cron_expression() {
        let parsed: Schedule = serde_json::from_value(json!("0 12 * * * *")).unwrap();
        let expected = Schedule::Cron("0 12 * * * *".to_string());
        assert_eq!(parsed, expected);
    }

    #[test]
    fn deserialize_fails_on_invalid_cron_expression() {
        let parsed = serde_json::from_value::<Schedule>(json!("0 12 * * *"));
        let err = parsed.expect_err("expected an error but got Ok");
        let msg = err.to_string();
        let expected_message = "expected a valid cron expression";
        assert!(msg.contains(expected_message), "Expected error message to contain '{expected_message}', but got '{msg}'");
    }

    #[rstest]
    #[case::with_cron_expression(
        json!("0 12 * * * *"),
        Schedule::Cron("0 12 * * * *".to_string())
    )]
    #[case::with_offset(
        json!({
            "event": "sunrise",
            "when": "Wednesday",
            "offset": 5
        }),
        Schedule::Sunrise { when: WeekdayCondition::Specific(Wednesday), offset: 5 }
    )]
    #[rstest]
    #[case::with_negative_offset(
        json!({
            "event": "sunrise",
            "when": "Wednesday",
            "offset": -5
        }),
        Schedule::Sunrise { when: WeekdayCondition::Specific(Wednesday), offset: -5 }
    )]
    #[case::without_offset(
        json!({
            "event": "sunrise",
            "when": "Wednesday-Saturday"
        }),
        Schedule::Sunrise { when: WeekdayCondition::Range { start: Wednesday, end: Saturday }, offset: 0 }
    )]
    fn deserializes_valid_values(#[case] json: Value, #[case] expected: Schedule) {
        let parsed: Schedule = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, expected);
    }

    #[rstest]
    #[case::no_event(
        json!({
            "when": "Wednesday-Saturday"
        }),
        "missing or invalid field 'event'"
    )]
    #[case::no_when(
        json!({
            "event": "sunrise",
        }),
        "missing or invalid field 'when'"
    )]
    #[case::unknown_event(
        json!({
            "event": "noon",
            "when": "Wednesday-Saturday"
        }),
        "unknown schedule event 'noon'"
    )]
    #[case::invalid_when(
        json!({
            "event": "noon",
            "when": 42
        }),
        "expected a string like 'Monday', 'Mon-Fri'"
    )]
    #[case::invalid_cron_expression(
        json!(42),
        "a string containing a cron expression or an object with 'event', 'when' and 'offset'"
    )]
    fn deserialize_fails_for_invalid_cases(#[case] json: Value, #[case] expected_message: &str) {
        let parsed: Result<Schedule, _> = serde_json::from_value(json);
        let err = parsed.expect_err("expected an error but got Ok");
        let msg = err.to_string();
        assert!(msg.contains(expected_message), "Expected error message to contain '{expected_message}', but got '{msg}'");
    }
}
