use crate::hue::domain::hue_response::Owner;
use serde::Deserialize;

// API: https://developers.meethue.com/develop/hue-api-v2/api-reference/#resource_light_get
#[derive(Debug, Deserialize)]
pub struct LightGet {
    pub id: String,
    pub owner: Owner,
    pub on: On,
    pub dimming: Option<Dimming>,
}

#[derive(Debug, Deserialize)]
pub struct On {
    pub on: bool,
}

#[derive(Debug, Deserialize)]
pub struct Dimming {
    pub brightness: f64,
    pub min_dim_level: Option<f64>,
}
