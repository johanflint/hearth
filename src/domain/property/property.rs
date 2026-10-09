use std::any::Any;
use std::fmt;
use std::fmt::{Debug, Formatter};
use thiserror::Error;

#[allow(dead_code)]
pub trait Property: Debug + Send + Sync {
    fn name(&self) -> &str;
    fn property_type(&self) -> PropertyType;
    fn value_kind(&self) -> ValueKind;
    /// Determines if the property may be set from a flow
    fn readonly(&self) -> bool;
    fn external_id(&self) -> Option<&str>;

    /// Returns a string representation of the value of the property
    fn value_string(&self) -> String;

    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn eq_dyn(&self, other: &dyn Property) -> bool;
    fn clone_box(&self) -> Box<dyn Property>;
}

impl Clone for Box<dyn Property> {
    fn clone(&self) -> Box<dyn Property> {
        self.clone_box()
    }
}

impl PartialEq for dyn Property {
    fn eq(&self, other: &Self) -> bool {
        self.eq_dyn(other)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ValueKind {
    Boolean,
    Color,
    DateTime,
    Enum,
    None,
    Number,
}

impl fmt::Display for ValueKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let kind = match self {
            ValueKind::Boolean => "boolean",
            ValueKind::Color => "color",
            ValueKind::DateTime => "datetime",
            ValueKind::Enum => "enum",
            ValueKind::Number => "number",
        };
        f.write_str(kind)
    }
}

// Semantic property type
#[derive(PartialEq, Debug, Clone, Copy)]
pub enum PropertyType {
    BatteryLevel,
    BatteryState,
    Brightness,
    Button,
    ButtonLastChanged,
    Color,
    ColorTemperature,
    Connectivity,
    Enabled,
    Illuminance,
    IlluminanceLastChanged,
    Motion,
    MotionLastChanged,
    MotionSensitivity,
    On,
}

// Identifies aproperty either by its well-known name or by the controller-owned
// external_id resource.
#[derive(PartialEq, Debug)]
pub enum PropertyLocator {
    Name(String),
    ExternalId(String),
}

#[derive(Error, PartialEq, Debug)]
pub enum PropertyError {
    #[error("unable to modify readonly property")]
    ReadOnly,
    #[error("value is smaller than the minimum value")]
    ValueTooSmall,
    #[error("value is larger than the maximum value")]
    ValueTooLarge,
    #[error("enum property must have at least oe allowed value")]
    EmptyAllowedValues,
    #[error("unknown value '{value}', allowed values: '{}'", allowed_values.join(", "))]
    UnknownValue { value: String, allowed_values: Vec<String> },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::property::{BooleanProperty, CartesianCoordinate, ColorProperty, DateTimeProperty, EnumProperty, NumberProperty};
    use rstest::rstest;

    #[rstest]
    #[case::boolean(ValueKind::Boolean, "boolean")]
    #[case::color(ValueKind::Color, "color")]
    #[case::datetime(ValueKind::DateTime, "datetime")]
    #[case::enumeration(ValueKind::Enum, "enum")]
    #[case::number(ValueKind::Number, "number")]
    fn value_kind_displays_in_lowercase(#[case] kind: ValueKind, #[case] expected: &str) {
        assert_eq!(kind.to_string(), expected);
    }

    #[rstest]
    #[case::boolean(Box::new(BooleanProperty::new("on".to_string(), PropertyType::On, false, None, true)), ValueKind::Boolean)]
    #[case::color(Box::new(ColorProperty::new("color".to_string(), PropertyType::Color, false, None, CartesianCoordinate::new(0.3, 0.3), None)), ValueKind::Color)]
    #[case::datetime(Box::new(DateTimeProperty::new("motion_last_changed".to_string(), PropertyType::MotionLastChanged, true, None, None)), ValueKind::DateTime)]
    #[case::enumeration(Box::new(EnumProperty::new("button".to_string(), PropertyType::Button, true, None, None, vec!["pressed".to_string()]).unwrap()), ValueKind::Enum)]
    #[case::number(Box::new(NumberProperty::builder("brightness".to_string(), PropertyType::Brightness, false).build()), ValueKind::Number)]
    fn property_value_kind_matches_its_value(#[case] property: Box<dyn Property>, #[case] expected: ValueKind) {
        assert_eq!(property.value_kind(), expected);
    }
}
