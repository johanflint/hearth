use crate::domain::property::Property;
use std::collections::HashMap;

#[derive(PartialEq, Debug)]
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

#[derive(PartialEq, Debug)]
pub enum DeviceType {
    Light,
}
