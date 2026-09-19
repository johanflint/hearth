use crate::domain::property::{Property, PropertyType};
use chrono::Utc;
use std::any::Any;

#[derive(Clone, PartialEq, Debug)]
pub struct DateTimeProperty {
    name: String,
    property_type: PropertyType,
    readonly: bool,
    external_id: Option<String>,
    value: chrono::DateTime<Utc>,
}

impl DateTimeProperty {
    pub fn new(name: String, property_type: PropertyType, readonly: bool, external_id: Option<String>, value: chrono::DateTime<Utc>) -> Self {
        DateTimeProperty {
            name,
            property_type,
            readonly,
            external_id,
            value,
        }
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
        self.value.to_string()
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
