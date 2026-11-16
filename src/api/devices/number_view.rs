use crate::domain::Number;
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(super) enum NumberView {
    PositiveInt(u64),
    NegativeInt(i64),
    Float(f64),
}

impl From<Number> for NumberView {
    fn from(number: Number) -> Self {
        match number {
            Number::PositiveInt(n) => Self::PositiveInt(n),
            Number::NegativeInt(n) => Self::NegativeInt(n),
            Number::Float(n) => Self::Float(n),
        }
    }
}
