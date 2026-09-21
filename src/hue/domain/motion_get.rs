use crate::hue::domain::Owner;
use chrono::Utc;
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Serialize)]
pub struct MotionRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sensitivity: Option<SetSensitivity>,
}

impl MotionRequest {
    pub fn new(enabled: Option<bool>, sensitivity: Option<SetSensitivity>) -> Self {
        MotionRequest { enabled, sensitivity }
    }
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

#[derive(Debug, Serialize)]
pub struct SetSensitivity {
    pub sensitivity: u64,
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

#[cfg(test)]
mod tests {
    use crate::hue::domain::{MotionRequest, SetSensitivity};
    use serde_json::json;

    #[test]
    fn motion_request_serializes_enabled_only() {
        let request = MotionRequest::new(Some(true), None);

        assert_eq!(serde_json::to_value(request).unwrap(), json!({ "enabled": true }));
    }

    #[test]
    fn motion_request_serializes_sensitivity_only() {
        let request = MotionRequest::new(None, Some(SetSensitivity { sensitivity: 2 }));

        assert_eq!(serde_json::to_value(request).unwrap(), json!({ "sensitivity": { "sensitivity": 2 } }));
    }

    #[test]
    fn motion_request_serializes_both_fields() {
        let request = MotionRequest::new(Some(true), Some(SetSensitivity { sensitivity: 2 }));

        assert_eq!(serde_json::to_value(request).unwrap(), json!({ "enabled": true, "sensitivity": { "sensitivity": 2 } }));
    }

    #[test]
    fn motion_request_serializes_neither_field() {
        let request = MotionRequest::new(None, None);

        assert_eq!(serde_json::to_value(request).unwrap(), json!({}));
    }
}
