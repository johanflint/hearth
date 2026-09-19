use crate::hue::domain::Owner;
use chrono::Utc;
use serde::Deserialize;

// API: https://developers.meethue.com/develop/hue-api-v2/api-reference/#resource_motion_get
#[derive(Debug, Deserialize)]
pub struct MotionGet {
    pub id: String,
    pub owner: Owner,
    pub enabled: bool,
    pub motion: Motion,
    pub sensitivity: Sensitivity,
    pub r#type: MotionType,
}

#[derive(Debug, Deserialize)]
pub struct Motion {
    // motion and motion_valid are deprecated and thus left out
    pub motion_report: Option<MotionReport>,
}

#[derive(Debug, Deserialize)]
pub struct MotionReport {
    pub changed: chrono::DateTime<Utc>,
    pub motion: bool,
}

#[derive(Debug, Deserialize)]
pub struct Sensitivity {
    pub status: SensitivityStatus,
    pub sensitivity: u64, // int 0 to sensitivity_max
    pub sensitivity_max: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SensitivityStatus {
    Set,
    Changing,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MotionType {
    Motion
}

#[derive(Debug, Deserialize)]
pub struct MotionChanged {
    pub id: String,
    pub owner: Owner,
    pub enabled: Option<bool>,
    pub motion: Option<Motion>,
    pub sensitivity: Option<SensitivityChanged>,
    pub r#type: Option<MotionType>,
}

#[derive(Debug, Deserialize)]
pub struct SensitivityChanged {
    pub status: Option<SensitivityStatus>,
    pub sensitivity: Option<u64>,
}
