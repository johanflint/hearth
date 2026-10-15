use crate::domain::Number;
use crate::domain::color::Color;
use crate::domain::property::ValueKind;
use chrono::{DateTime, Utc};
use std::fmt;

#[derive(Eq, PartialEq, Hash, Debug, Clone)]
pub enum Value {
    Boolean(bool),
    Color(Color),
    DateTime(DateTime<Utc>),
    Number(Number),
    String(String),
    None,
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Boolean(value) => write!(f, "{value}"),
            Value::Color(color) => write!(f, "{color}"),
            Value::DateTime(value) => f.write_str(&value.to_rfc3339()),
            Value::Number(value) => write!(f, "{value}"),
            Value::String(value) => write!(f, "{value:?}"),
            Value::None => f.write_str("none"),
        }
    }
}

impl Value {
    pub fn kind(&self) -> ValueKind {
        match self {
            Value::Boolean(_) => ValueKind::Boolean,
            Value::Color(_) => ValueKind::Color,
            Value::DateTime(_) => ValueKind::DateTime,
            Value::Number(_) => ValueKind::Number,
            Value::None => ValueKind::None,
            Value::String(_) => ValueKind::Enum,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use rstest::rstest;

    fn date_time() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 15, 8, 30, 0).unwrap()
    }

    #[rstest]
    #[case::boolean(Value::Boolean(true), "true")]
    #[case::color(Value::Color(Color::RGB(255, 128, 0)), "RGB(255, 128, 0)")]
    #[case::date_time(Value::DateTime(date_time()), "2026-10-15T08:30:00+00:00")]
    #[case::positive_number(Value::Number(Number::PositiveInt(42)), "42")]
    #[case::negative_number(Value::Number(Number::NegativeInt(-7)), "-7")]
    #[case::float_number(Value::Number(Number::Float(0.5)), "0.5")]
    #[case::string_is_quoted(Value::String("short_release".to_string()), "\"short_release\"")]
    #[case::none(Value::None, "none")]
    fn displays_the_value(#[case] value: Value, #[case] expected: &str) {
        assert_eq!(value.to_string(), expected);
    }

    #[rstest]
    #[case::boolean(Value::Boolean(true), ValueKind::Boolean)]
    #[case::color(Value::Color(Color::RGB(255, 128, 0)), ValueKind::Color)]
    #[case::date_time(Value::DateTime(date_time()), ValueKind::DateTime)]
    #[case::number(Value::Number(Number::PositiveInt(42)), ValueKind::Number)]
    #[case::string_is_an_enum(Value::String("short_release".to_string()), ValueKind::Enum)]
    #[case::none(Value::None, ValueKind::None)]
    fn kind_matches_the_value(#[case] value: Value, #[case] expected: ValueKind) {
        assert_eq!(value.kind(), expected);
    }
}
