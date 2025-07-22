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
