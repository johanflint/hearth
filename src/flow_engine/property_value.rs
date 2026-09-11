use crate::domain::Number;
use crate::domain::color::Color;

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
}
