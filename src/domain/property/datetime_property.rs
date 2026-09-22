use crate::domain::property::{Property, PropertyError, PropertyType};
use chrono::{DateTime, Utc};
use std::any::Any;

#[derive(Clone, PartialEq, Debug)]
pub struct DateTimeProperty {
    name: String,
    property_type: PropertyType,
    readonly: bool,
    external_id: Option<String>,
    value: Option<DateTime<Utc>>,
}

impl DateTimeProperty {
    pub fn new(name: String, property_type: PropertyType, readonly: bool, external_id: Option<String>, value: Option<DateTime<Utc>>) -> Self {
        DateTimeProperty {
            name,
            property_type,
            readonly,
            external_id,
            value,
        }
    }

    pub fn value(&self) -> Option<DateTime<Utc>> {
        self.value
    }

    // This function does not check the readonly value as the value comes from an observer and the system
    // must be in sync with the observed system.
    pub fn set_value(&mut self, value: Option<DateTime<Utc>>) -> Result<(), PropertyError> {
        self.value = value;
        Ok(())
    }
}

impl Property for DateTimeProperty {
    fn name(&self) -> &str {
        &self.name
    }

    fn property_type(&self) -> PropertyType {
        self.property_type
    }

    fn readonly(&self) -> bool {
        self.readonly
    }

    fn external_id(&self) -> Option<&str> {
        self.external_id.as_deref()
    }

    fn value_string(&self) -> String {
        self.value.map(|v| v.to_string()).unwrap_or(String::new())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn eq_dyn(&self, other: &dyn Property) -> bool {
        other.as_any().downcast_ref::<DateTimeProperty>().map_or(false, |o| self == o)
    }

    fn clone_box(&self) -> Box<dyn Property> {
        Box::new(self.clone())
    }
}
