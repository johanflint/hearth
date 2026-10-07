use crate::domain::Number;
use crate::domain::color::Color;
use crate::domain::property::ValueKind;
use std::time::Duration;

#[derive(Clone, PartialEq, Debug)]
pub struct PropertyCommand {
    pub value: PropertyValue,
    pub transition: Option<Duration>,
}

#[cfg(test)]
impl From<PropertyValue> for PropertyCommand {
    fn from(value: PropertyValue) -> Self {
        PropertyCommand { value, transition: None }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum PropertyValue {
    SetBooleanValue(bool),
    ToggleBooleanValue,
    SetNumberValue(Number),
    IncrementNumberValue(Number),
    DecrementNumberValue(Number),
    SetColor(Color),
}

#[derive(Clone, PartialEq, Debug)]
pub(crate) enum ConflictMergeSemantics {
    /// Equal proposals from different flows can be collapsed into a single dispatch
    DeduplicateIfEqual,
    /// If two [PropertyValue]s are semantically distinct, do not mean agreement (e.g. two toggles or increments are
    /// not the same as applying one
    Conflict,
}

impl PropertyValue {
    pub(crate) fn conflict_merge_semantics(&self) -> ConflictMergeSemantics {
        match self {
            PropertyValue::SetBooleanValue(_) | PropertyValue::SetNumberValue(_) | PropertyValue::SetColor(_) => ConflictMergeSemantics::DeduplicateIfEqual,
            PropertyValue::ToggleBooleanValue | PropertyValue::IncrementNumberValue(_) | PropertyValue::DecrementNumberValue(_) => ConflictMergeSemantics::Conflict,
        }
    }

    pub(crate) fn value_kind(&self) -> ValueKind {
        match self {
            PropertyValue::SetBooleanValue(_) | PropertyValue::ToggleBooleanValue => ValueKind::Boolean,
            PropertyValue::SetNumberValue(_) | PropertyValue::IncrementNumberValue(_) | PropertyValue::DecrementNumberValue(_) => ValueKind::Number,
            PropertyValue::SetColor(_) => ValueKind::Color,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::set_boolean(PropertyValue::SetBooleanValue(true), ValueKind::Boolean)]
    #[case::toggle_boolean(PropertyValue::ToggleBooleanValue, ValueKind::Boolean)]
    #[case::set_number(PropertyValue::SetNumberValue(Number::PositiveInt(50)), ValueKind::Number)]
    #[case::increment_number(PropertyValue::IncrementNumberValue(Number::PositiveInt(10)), ValueKind::Number)]
    #[case::decrement_number(PropertyValue::DecrementNumberValue(Number::PositiveInt(10)), ValueKind::Number)]
    #[case::set_color(PropertyValue::SetColor(Color::Hex("#ff0000".to_string())), ValueKind::Color)]
    fn value_kind_maps_each_property_value(#[case] value: PropertyValue, #[case] expected: ValueKind) {
        assert_eq!(value.value_kind(), expected);
    }
}
