use crate::domain::property::{Property, PropertyLocator, PropertyType};
use std::collections::HashMap;

#[derive(Clone, PartialEq, Debug)]
pub struct Device {
    pub id: String,
    pub r#type: DeviceType,
    pub manufacturer: String,
    pub model_id: String,
    pub product_name: String,
    pub name: String,
    pub properties: HashMap<String, Box<dyn Property>>,
    pub external_id: Option<String>,
    pub address: Option<String>,
    pub controller_id: Option<&'static str>,
}

impl Device {
    pub fn get_property<T: 'static + Property>(&self, name: &str) -> Option<&T> {
        self.properties.get(name).and_then(|v| v.as_any().downcast_ref::<T>())
    }

    pub fn get_property_of_type<T: 'static + Property>(&self, property_type: PropertyType) -> Option<&T> {
        self.properties
            .iter()
            .find(|(_key, v)| v.property_type() == property_type)
            .and_then(|(_, v)| v.as_any().downcast_ref::<T>())
    }

    pub fn resolve_property<T: 'static + Property>(&self, locator: &PropertyLocator) -> Option<&T> {
        match locator {
            PropertyLocator::Name(name) => self.get_property(name),
            PropertyLocator::ExternalId(external_id) => {
                // Require exactly one match to avoid updating the wrong property
                let mut matches = self.properties
                    .values()
                    .filter_map(|p| p.as_any().downcast_ref::<T>())
                    .filter(|p| p.external_id() == Some(external_id));
                let first = matches.next()?;
                matches.next().is_none().then_some(first)
            }
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum DeviceType {
    Light,
    MotionSensor,
    Remote,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::property::EnumProperty;
    use crate::test_support::DeviceBuilder;

    fn enum_property(name: &str, external_id: &str, value: &str) -> Box<dyn Property> {
        Box::new(EnumProperty::new(name.to_string(), PropertyType::Button, true, Some(external_id.to_string()), Some(value.to_string()), vec![value.to_string()]).unwrap())
    }

    #[test]
    fn resolve_property_returns_the_property_when_the_external_id_is_unique() {
        let device = DeviceBuilder::new("device")
            .with_properties(vec![enum_property("button1", "unique-external-id", "short_release")])
            .build();

        let resolved = device.resolve_property::<EnumProperty>(&PropertyLocator::ExternalId("unique-external-id".to_string()));

        assert_eq!(resolved.map(Property::name), Some("button1"));
    }

    #[test]
    fn resolve_property_returns_none_when_multiple_properties_of_the_same_type_share_an_external_id() {
        let external_id = "shared-external-id";
        let device = DeviceBuilder::new("device")
            .with_properties(vec![enum_property("button1", external_id, "short_release"), enum_property("button2", external_id, "long_press")])
            .build();

        let resolved = device.resolve_property::<EnumProperty>(&PropertyLocator::ExternalId(external_id.to_string()));

        assert_eq!(resolved, None);
    }
}