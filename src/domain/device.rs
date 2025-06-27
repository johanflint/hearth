use crate::domain::property::{Property, PropertyType};
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
}

#[derive(Clone, PartialEq, Debug)]
pub enum DeviceType {
    Light,
}
