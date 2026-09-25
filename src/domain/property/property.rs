use std::any::Any;
use std::fmt::Debug;
use thiserror::Error;

#[allow(dead_code)]
pub trait Property: Debug + Send + Sync {
    fn name(&self) -> &str;
    fn property_type(&self) -> PropertyType;
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

// Semantic property type
#[derive(PartialEq, Debug, Clone, Copy)]
pub enum PropertyType {
    Brightness,
    Button,
    ButtonLastChanged,
    Color,
    ColorTemperature,
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
    #[error("missing property")]
    MissingProperty,
    #[error("enum property must have at least oe allowed value")]
    EmptyAllowedValues,
    #[error("unknown value '{value}', allowed values: '{}'", allowed_values.join(", "))]
    UnknownValue { value: String, allowed_values: Vec<String> },
}
