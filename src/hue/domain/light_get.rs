use crate::hue::domain::hue_response::Owner;
use serde::Deserialize;

// API: https://developers.meethue.com/develop/hue-api-v2/api-reference/#resource_light_get
#[derive(Debug, Deserialize)]
pub struct LightGet {
    pub id: String,
    pub owner: Owner,
    pub on: On,
    pub dimming: Option<Dimming>,
    pub color_temperature: Option<ColorTemperature>,
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

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct ColorTemperature {
    pub mirek: Option<u64>, // >= 153 && <= 500, color temperature in mirek or null when the light color is not in the ct spectrum
    pub mirek_valid: bool,
    pub mirek_schema: MirekSchema,
}

#[derive(Debug, Deserialize)]
pub struct MirekSchema {
    pub mirek_minimum: u64,
    pub mirek_maximum: u64,
}

#[allow(dead_code)]
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

#[derive(Debug, Deserialize)]
pub struct LightChanged {
    pub id: String,
    pub owner: Owner,
    pub on: Option<On>,
    pub dimming: Option<Dimming>,
    pub color_temperature: Option<ChangedColorTemperature>,
    pub color: Option<ChangedColor>,
}

#[derive(Debug, Deserialize)]
pub struct ChangedColorTemperature {
    pub mirek: u64, // >= 153 && <= 500, color temperature in mirek or null when the light color is not in the ct spectrum
    pub mirek_valid: bool,
}

#[derive(Debug, Deserialize)]
pub struct ChangedColor {
    pub xy: Xy,
    pub gamut: Option<ColorGamut>,
}
