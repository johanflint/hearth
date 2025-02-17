use std::collections::HashMap;

#[derive(PartialEq, Debug)]
pub struct Device {
    pub id: String,
    pub r#type: DeviceType,
    pub manufacturer: String,
    pub model_id: String,
    pub product_name: String,
    pub name: String,
    pub properties: HashMap<String, String>,
    pub external_id: Option<String>,
    pub address: Option<String>,
}

#[derive(PartialEq, Debug)]
pub enum DeviceType {
    Light,
}
