use std::any::Any;
use std::fmt::Debug;
use thiserror::Error;

#[allow(dead_code)]
pub trait Property: Debug + Send + Sync {
    fn name(&self) -> &str;
    fn property_type(&self) -> PropertyType;
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
    Color,
    ColorTemperature,
    On,
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
}
