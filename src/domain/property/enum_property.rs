use crate::domain::property::{Property, PropertyError, PropertyType};
use std::any::Any;

#[derive(Clone, PartialEq, Debug)]
pub struct EnumProperty {
    name: String,
    property_type: PropertyType,
    readonly: bool,
    external_id: Option<String>,
    value: Option<String>,
    allowed_values: Vec<String>,
}

impl EnumProperty {
    pub fn new(name: String, property_type: PropertyType, readonly: bool, external_id: Option<String>, value: Option<String>, allowed_values: Vec<String>) -> Result<Self, PropertyError> {
        if let Some(value) = &value {
            if allowed_values.is_empty() {
                return Err(PropertyError::EmptyAllowedValues);
            }
            if !allowed_values.contains(value) {
                return Err(PropertyError::UnknownValue { value: value.clone(), allowed_values });
            }
        }
        Ok(EnumProperty {
            name,
            property_type,
            readonly,
            external_id,
            value,
            allowed_values,
        })
    }

    pub fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }

    // This function does not check the readonly value as the value comes from an observer and the system
    // must be in sync with the observed system. It does ensure that the `value` is in the `allowed_values`
    // to ensure it remains a valid enum.
    pub fn set_value(&mut self, value: Option<String>) -> Result<(), PropertyError> {
        if let Some(v) = &value {
            if self.allowed_values.is_empty() {
                return Err(PropertyError::EmptyAllowedValues);
            }
            if !self.allowed_values.contains(v) {
                return Err(PropertyError::UnknownValue { value: v.clone(), allowed_values: self.allowed_values.clone() });
            }
        }

        self.value = value;
        Ok(())
    }
}

impl Property for EnumProperty {
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
        self.value.clone().unwrap_or_default()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn eq_dyn(&self, other: &dyn Property) -> bool {
        other.as_any().downcast_ref::<EnumProperty>().map_or(false, |o| self == o)
    }

    fn clone_box(&self) -> Box<dyn Property> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn property(initial_value: String, readonly: bool) -> Result<EnumProperty, PropertyError> {
        EnumProperty::new(
            "button0".to_string(),
            PropertyType::Button,
            readonly,
            None,
            Some(initial_value),
            vec!["initial_press".to_string(), "short_release".to_string()],
        )
    }

    #[test]
    fn new_returns_a_property_if_the_initial_value_is_valid() {
        let property = property("initial_press".to_string(), true);
        assert!(property.is_ok());
    }

    #[test]
    fn new_returns_a_property_if_the_initial_value_is_none() {
        let property = EnumProperty::new(
            "button0".to_string(),
            PropertyType::Button,
            true,
            None,
            None,
            vec!["initial_press".to_string(), "short_release".to_string()],
        );
        assert!(property.is_ok());
        assert_eq!(property.unwrap().value(), None);
    }

    #[test]
    fn new_returns_a_property_if_allowed_values_is_empty_and_value_is_none() {
        let property = EnumProperty::new("button0".to_string(), PropertyType::Button, true, None, None, vec![]);
        assert!(property.is_ok());
    }

    #[test]
    fn new_returns_an_error_if_the_initial_value_is_invalid() {
        let property = property("unknown_initial_value".to_string(), true);
        assert!(property.is_err());
        assert_eq!(property.unwrap_err(), PropertyError::UnknownValue {
            value: "unknown_initial_value".to_string(),
            allowed_values: vec!["initial_press".to_string(), "short_release".to_string()],
        });
    }

    #[test]
    fn new_returns_an_error_if_allowed_values_is_empty_and_value_is_some() {
        let property = EnumProperty::new("button0".to_string(), PropertyType::Button, true, None, Some("initial_press".to_string()), vec![]);
        assert_eq!(property.unwrap_err(), PropertyError::EmptyAllowedValues);
    }

    #[test]
    fn set_value_updates_the_value_if_the_property_is_editable() {
        let mut property = property("initial_press".to_string(), false).unwrap();
        let result = property.set_value(Some("short_release".to_string()));

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), ());
        assert_eq!(property.value, Some("short_release".to_string()));
    }

    #[test]
    fn set_value_updates_the_value_even_if_the_property_is_readonly() {
        let mut property = property("initial_press".to_string(), true).unwrap();
        let result = property.set_value(Some("short_release".to_string()));

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), ());
        assert_eq!(property.value, Some("short_release".to_string()));
    }

    #[test]
    fn set_value_accepts_none() {
        let mut property = property("initial_press".to_string(), false).unwrap();
        let result = property.set_value(None);

        assert!(result.is_ok());
        assert_eq!(property.value, None);
    }

    #[test]
    fn set_value_rejects_value_not_in_allowed_values() {
        let mut property = property("initial_press".to_string(), false).unwrap();
        let result = property.set_value(Some("not_in_allowed_values".to_string()));

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), PropertyError::UnknownValue {
            value: "not_in_allowed_values".to_string(),
            allowed_values: vec!["initial_press".to_string(), "short_release".to_string()],
        });
        assert_eq!(property.value, Some("initial_press".to_string()));
    }

    #[test]
    fn set_value_rejects_a_value_if_allowed_values_is_empty() {
        let mut property = EnumProperty::new("button0".to_string(), PropertyType::Button, true, None, None, vec![]).unwrap();
        let result = property.set_value(Some("initial_press".to_string()));
        assert_eq!(result.unwrap_err(), PropertyError::EmptyAllowedValues);
    }
}
