use crate::hue::domain::hue_response::Owner;
use serde::Deserialize;

// API: https://developers.meethue.com/develop/hue-api-v2/api-reference/#resource_light_get
#[derive(Debug, Deserialize)]
pub struct LightGet {
    pub id: String,
    pub owner: Owner,
    pub on: On,
    pub dimming: Option<Dimming>,
    pub color: Option<Color>,
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

#[derive(Debug, Deserialize)]
pub struct Color {
    pub xy: Xy,
    pub gamut: Option<ColorGamut>,
    pub gamut_type: GamutType,
}

#[derive(Debug, Deserialize)]
pub struct Xy {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Deserialize)]
pub struct ColorGamut {
    pub red: Xy,
    pub green: Xy,
    pub blue: Xy,
}

#[derive(Debug, Deserialize)]
pub enum GamutType {
    A,
    B,
    C,
}
