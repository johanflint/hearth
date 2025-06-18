use crate::domain::Number;

#[derive(Clone, PartialEq, Debug)]
pub enum PropertyValue {
    SetBooleanValue(bool),
    ToggleBooleanValue,
    SetNumberValue(Number),
    IncrementNumberValue(Number),
    DecrementNumberValue(Number),
}
