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
