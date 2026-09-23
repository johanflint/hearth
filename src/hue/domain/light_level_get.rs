use crate::hue::domain::Owner;
use chrono::{DateTime, Utc};
use serde::Deserialize;

// API: https://developers.meethue.com/develop/hue-api-v2/api-reference/#resource_light_level_get
#[derive(Debug, Deserialize)]
pub struct LightLevelGet {
    pub id: String,
    pub owner: Owner,
    pub enabled: bool,
    pub light: Light,
}

#[derive(Debug, Deserialize)]
pub struct Light {
    pub light_level_report: Option<LightLevelReport>,
}

#[derive(Debug, Deserialize)]
pub struct LightLevelReport {
    pub changed: DateTime<Utc>,
    pub light_level: u64,
}
