use crate::hue::domain::Owner;
use serde::Deserialize;

// API: https://developers.meethue.com/develop/hue-api-v2/api-reference/#resource_device_power_get
#[derive(Debug, Deserialize)]
pub struct DevicePowerGet {
    pub id: String,
    pub owner: Owner,
    pub power_state: PowerState,
}

#[derive(Debug, Deserialize)]
pub struct PowerState {
    pub battery_level: Option<u8>, // 0-100
    pub battery_state: Option<String>, // One of normal, low, critical
}
