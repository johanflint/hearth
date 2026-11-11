use serde::Deserialize;

// API: https://shelly-api-docs.shelly.cloud/gen2/ComponentsAndServices/Shelly/#shellygetdeviceinfo
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct DeviceInfo {
    pub id: String,
    pub name: Option<String>,
    pub mac: String,
    pub model: String,
    #[serde(rename = "gen")]
    pub generation: u16,
    pub fw_id: String, // Firmware id
    pub ver: String,   // Version of the firmware
    pub app: String,
    pub profile: Option<String>, // Name of the device profile (only applicable for multi-profile devices)
    pub auth_en: bool,           // Whether authentication is enabled
    pub auth_domain: Option<String>, // Name of the domain or null if disabled
    pub discoverable: Option<bool>,
    pub provision: String,
}
