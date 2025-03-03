use crate::hue::hue_response::Owner;
use serde::Deserialize;

// API: https://developers.meethue.com/develop/hue-api-v2/api-reference/#resource_light_get
#[derive(Debug, Deserialize)]
pub struct LightGet {
    id: String,
    owner: Owner,
    on: On,
}

#[derive(Debug, Deserialize)]
pub struct On {
    on: bool,
}
