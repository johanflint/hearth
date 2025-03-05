use crate::hue::hue_response::Owner;
use serde::Deserialize;

// API: https://developers.meethue.com/develop/hue-api-v2/api-reference/#resource_light_get
#[derive(Debug, Deserialize)]
pub struct LightGet {
    pub id: String,
    pub owner: Owner,
    pub on: On,
}

#[derive(Debug, Deserialize)]
pub struct On {
    pub on: bool,
}
