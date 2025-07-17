use crate::app_config::AppConfig;
use crate::domain::Number;
use crate::domain::color::Color;
use crate::domain::commands::Command;
use crate::domain::controller::Controller;
use crate::domain::device::DeviceType;
use crate::domain::property::{BooleanProperty, ColorProperty, NumberProperty, Property, PropertyError, PropertyType, ValidatedValue};
use crate::flow_engine::property_value::PropertyValue;
use crate::hue::clip_to_gamut::clip_to_gamut;
use crate::hue::domain::{LightRequest, On};
use async_trait::async_trait;
use reqwest::Client;
use std::sync::Arc;
use tracing::{info, instrument, warn};

#[derive(Debug)]
pub struct HueController {
    client: Client,
    config: Arc<AppConfig>,
}

pub const CONTROLLER_ID: &str = "hue";

#[async_trait]
impl Controller for HueController {
    fn id(&self) -> &'static str {
        CONTROLLER_ID
    }

    #[instrument(skip_all)]
    async fn execute(&self, command: Command) {
        match command {
            Command::ControlDevice { device, property } => {
                if device.r#type == DeviceType::Light {
                    let on_property = device.get_property_of_type::<BooleanProperty>(PropertyType::On).unwrap();
                    let on = device.get_property_of_type::<BooleanProperty>(PropertyType::On).and_then(|on_property| {
                        property.get(on_property.name()).and_then(|pv| match pv {
                            PropertyValue::SetBooleanValue(value) => Some(On { on: *value }),
                            PropertyValue::ToggleBooleanValue => Some(On { on: !on_property.value() }),
                            _ => None,
                        })
                    });

                    if let Some(value) = &on {
                        let on_text = if value.on { "on" } else { "off" };
                        info!(device_id = device.id, ?on_property, "🟢 Turn {} light '{}'", on_text, device.name);
                    }

                    let brightness = device.get_property_of_type::<NumberProperty>(PropertyType::Brightness).and_then(|brightness_property| {
                        property
                            .get(brightness_property.name())
                            .and_then(|pv| match pv {
                                PropertyValue::SetNumberValue(value) => value.as_f64(),
                                PropertyValue::IncrementNumberValue(value) => (brightness_property.value() + value.clone()).as_f64(),
                                PropertyValue::DecrementNumberValue(value) => (brightness_property.value() - value.clone()).as_f64(),
                                _ => None,
                            })
                            .and_then(|brightness| match brightness_property.validate_value(Number::Float(brightness)) {
                                ValidatedValue::Valid(value) => value.as_f64(),
                                ValidatedValue::Clamped(value, PropertyError::ValueTooSmall) => {
                                    #[rustfmt::skip]
                                    warn!(device_id = device.id, ?brightness_property, "🔅 Brightness value of '{}%' is too small, clamped to the minimum valid value of '{}%'", brightness, value);
                                    value.as_f64()
                                }
                                ValidatedValue::Clamped(value, PropertyError::ValueTooLarge) => {
                                    #[rustfmt::skip]
                                    warn!(device_id = device.id, ?brightness_property, "🔆 Brightness value of '{}%' is too large, clamped to the maximim valid value of '{}%'", brightness, value);
                                    value.as_f64()
                                }
                                ValidatedValue::Clamped(value, error) => {
                                    warn!("🔆 Brightness value of '{}%' is invalid, clamped to {}", error, value);
                                    value.as_f64()
                                }
                                ValidatedValue::Invalid(error) => {
                                    warn!("🔆 Brightness value is invalid: {}", error);
                                    None
                                }
                            })
                    });

                    let color = device.get_property_of_type::<ColorProperty>(PropertyType::Color).and_then(|color_property| {
                        property
                            .get(color_property.name())
                            .and_then(|pv| match pv {
                                PropertyValue::SetColor(color) => Some(color),
                                _ => None,
                            })
                            .map(|color| match Color::Hex(color.to_string()).to_cie_xyY() {
                                Ok(x) => Some(x),
                                Err(error) => {
                                    warn!("🌈 Color value is invalid: {}", error);
                                    None
                                }
                            })
                            .and_then(|color| match color {
                                Some(Color::CIE_xyY { xy, brightness: _ }) => color_property.gamut().map(|gamut| clip_to_gamut(xy.clone(), gamut)).or(Some(xy)),
                                _ => None,
                            })
                    });

                    let request = LightRequest::new(on, brightness, color);
                    let request_result = self
                        .client
                        .put(format!("{}/clip/v2/resource/light/{}", self.config.hue().url(), on_property.external_id().expect("")))
                        .json(&request)
                        .send()
                        .await;

                    match request_result {
                        Err(e) => {
                            warn!(device_id = device.id, "⚠️ Unable to control the light: {:?}", e);
                        }
                        Ok(response) if !response.status().is_success() => {
                            let status = response.status();
                            let body = response.text().await.unwrap_or_default();
                            warn!(device_id = device.id, status_code = %status, "⚠️ Unable to control the light, request to the Hue bridge failed. Response: {}", body);
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

impl HueController {
    pub fn new(client: Client, config: Arc<AppConfig>) -> Self {
        HueController { client, config }
    }
}
