use std::any::Any;
use std::fmt::Debug;
use thiserror::Error;

#[allow(dead_code)]
pub trait Property: Debug + Send + Sync {
    fn name(&self) -> &str;
    fn property_type(&self) -> PropertyType;
    fn readonly(&self) -> bool;
    fn external_id(&self) -> Option<&str>;

    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn eq_dyn(&self, other: &dyn Property) -> bool;
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
    #[error("value is smaller than the minimum value")]
    ValueTooLarge,
    #[error("value is smaller than the minimum value")]
    IncorrectValueType,
}
