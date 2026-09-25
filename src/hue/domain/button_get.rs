use crate::hue::domain::Owner;
use chrono::{DateTime, Utc};
use serde::Deserialize;

// API: https://developers.meethue.com/develop/hue-api-v2/api-reference/#resource_button_get
#[derive(Debug, Deserialize)]
pub struct ButtonGet {
    pub id: String,
    pub owner: Owner,
    pub metadata: ButtonMetadata,
    pub button: Button,
}

#[derive(Debug, Deserialize)]
pub struct ButtonMetadata {
    pub control_id: u8, // >= 0 && <= 8
}

#[derive(Debug, Deserialize)]
pub struct Button {
    pub button_report: Option<ButtonReport>,
    pub repeat_interval: u64, // Duration between repeat events when holding the button in milliseconds
    pub event_values: Vec<String>, // All button events that this device supports
}

#[derive(Debug, Deserialize)]
pub struct ButtonReport {
    pub updated: DateTime<Utc>,
    pub event: String, // One of initial_press, repeat, short_release, long_release, double_short_release, long_press
}

#[derive(Debug, Deserialize)]
pub struct ButtonChanged {
    pub id: String,
    pub owner: Owner,
    pub button: Option<ButtonUpdate>,
}

#[derive(Debug, Deserialize)]
pub struct ButtonUpdate {
    pub button_report: Option<ButtonReport>,
}
