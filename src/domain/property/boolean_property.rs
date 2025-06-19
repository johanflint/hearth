use crate::domain::property::{Property, PropertyError, PropertyType};
use std::any::Any;

#[derive(PartialEq, Debug)]
pub struct BooleanProperty {
    name: String,
    property_type: PropertyType,
    readonly: bool,
    external_id: Option<String>,
    value: bool,
}

impl BooleanProperty {
    pub fn new(name: String, property_type: PropertyType, readonly: bool, external_id: Option<String>, value: bool) -> Self {
        BooleanProperty {
            name,
            property_type,
            readonly,
            external_id,
            value,
        }
    }

    pub fn value(&self) -> bool {
        self.value
    }

    pub fn set_value(&mut self, value: bool) -> Result<(), PropertyError> {
        if self.readonly {
            return Err(PropertyError::ReadOnly);
        }

        self.value = value;
        Ok(())
    }
}

impl Property for BooleanProperty {
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
        other.as_any().downcast_ref::<BooleanProperty>().map_or(false, |o| self == o)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_value_returns_the_value_if_property_is_editable() {
        let mut property = BooleanProperty {
            name: "on".to_string(),
            property_type: PropertyType::On,
            readonly: false,
            external_id: None,
            value: false,
        };

        let result = property.set_value(true);

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), ());
        assert_eq!(property.value, true);
    }

    #[test]
    fn set_value_returns_an_error_if_property_is_readonly() {
        let mut property = BooleanProperty {
            name: "on".to_string(),
            property_type: PropertyType::On,
            readonly: true,
            external_id: None,
            value: false,
        };

        let result = property.set_value(false);

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), PropertyError::ReadOnly);
        assert_eq!(property.value, false);
    }
}
