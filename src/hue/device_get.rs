use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct DeviceGet {
    pub id: String,
    pub product_data: ProductData,
    pub metadata: Metadata,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct ProductData {
    pub model_id: String,
    pub manufacturer_name: String,
    pub product_name: String,
    pub product_archetype: String,
    pub certified: bool,
    pub software_version: String, // pattern: \d+\.\d+\.\d+
    pub hardware_platform_type: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct Metadata {
    pub name: String,
    pub archetype: String,
}
