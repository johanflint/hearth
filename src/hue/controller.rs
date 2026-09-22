use crate::app_config::AppConfig;
use crate::domain::Number;
use crate::domain::color::Color;
use crate::domain::commands::Command;
use crate::domain::controller::Controller;
use crate::domain::device::{Device, DeviceType};
use crate::domain::property::{BooleanProperty, ColorProperty, NumberProperty, Property, PropertyError, PropertyType, ValidatedValue};
use crate::extensions::unsigned_ints_ext::MirekConversions;
use crate::flow_engine::property_value::PropertyValue;
use crate::hue::clip_to_gamut::clip_to_gamut;
use crate::hue::domain::{LightRequest, MotionRequest, On, SetSensitivity};
use crate::metrics::Metric;
use async_trait::async_trait;
use metrics::counter;
use reqwest::Client;
use serde::Serialize;
use std::collections::HashMap;
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
                match device.r#type {
                    DeviceType::Light => self.control_device_light(device, property).await,
                    DeviceType::MotionSensor => self.control_device_motion_sensor(device, property).await,
                }
            }
        }
    }
}

impl HueController {
    pub fn new(client: Client, config: Arc<AppConfig>) -> Self {
        HueController { client, config }
    }

    async fn control_device_light(&self, device: Arc<Device>, property: Arc<HashMap<String, PropertyValue>>) {
        let Some(on_property) = device.get_property_of_type::<BooleanProperty>(PropertyType::On) else {
            warn!(device_id = device.id, "⚠️ Light has no on property");
            return;
        };

        let Some(light_id) = on_property.external_id() else {
            warn!(device_id = device.id, "⚠️ Light on property has no Hue resource id");
            return;
        };

        let on = property.get(on_property.name()).and_then(|pv| match pv {
            PropertyValue::SetBooleanValue(value) => Some(On { on: *value }),
            PropertyValue::ToggleBooleanValue => Some(On { on: !on_property.value() }),
            _ => None,
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
                    PropertyValue::IncrementNumberValue(value) => (brightness_property.value().unwrap_or(Number::PositiveInt(0)) + value.clone()).as_f64(),
                    PropertyValue::DecrementNumberValue(value) => (brightness_property.value().unwrap_or(Number::PositiveInt(0)) - value.clone()).as_f64(),
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

        let color_temperature = device
            .get_property_of_type::<NumberProperty>(PropertyType::ColorTemperature)
            .and_then(|color_temperature_property| {
                property
                    .get(color_temperature_property.name())
                    .and_then(|pv| match pv {
                        PropertyValue::SetNumberValue(value) => value.as_u64(),
                        _ => None,
                    })
                    .and_then(|color_temperature| match color_temperature_property.validate_value(Number::PositiveInt(color_temperature)) {
                        ValidatedValue::Valid(value) => value.as_u64(),
                        ValidatedValue::Clamped(value, PropertyError::ValueTooSmall) => {
                            #[rustfmt::skip]
                                        warn!(device_id = device.id, ?color_temperature_property, "🌈 Color temperature value of '{}K' is too small, clamped to the minimum valid value of '{}K'", color_temperature, value);
                            value.as_u64()
                        }
                        ValidatedValue::Clamped(value, PropertyError::ValueTooLarge) => {
                            #[rustfmt::skip]
                                        warn!(device_id = device.id, ?color_temperature_property, "🌈 Color temperature value of '{}K' is too large, clamped to the maximim valid value of '{}K'", color_temperature, value);
                            value.as_u64()
                        }
                        ValidatedValue::Clamped(value, error) => {
                            warn!("🌈 Color temperature value of '{}K' is invalid, clamped to {}K", error, value);
                            value.as_u64()
                        }
                        ValidatedValue::Invalid(error) => {
                            warn!("🌈 Color temperature value is invalid: {}", error);
                            None
                        }
                    })
                    .map(|color_temperature| color_temperature.kelvin_to_mirek())
            });

        let color = device.get_property_of_type::<ColorProperty>(PropertyType::Color).and_then(|color_property| {
            property.get(color_property.name()).and_then(|pv| match pv {
                PropertyValue::SetColor(color) => match color.clone().to_cie_xyY() {
                    Ok(Color::CIE_xyY { xy, brightness: _ }) => color_property.gamut().map(|gamut| clip_to_gamut(xy.clone(), gamut)).or(Some(xy)),
                    Err(error) => {
                        warn!("🌈 Color value is invalid: {}", error);
                        None
                    }
                    _ => None,
                },
                _ => None,
            })
        });

        let request = LightRequest::new(on, brightness, color_temperature, color);
        let url = format!("{}/clip/v2/resource/light/{}", self.config.hue().url(), light_id);
        self.send_request(&request, &url, &device, "light").await;
    }

    async fn control_device_motion_sensor(&self, device: Arc<Device>, property: Arc<HashMap<String, PropertyValue>>) {
        let Some(motion_property) = device.get_property_of_type::<BooleanProperty>(PropertyType::Motion) else {
            warn!(device_id = device.id, "⚠️ Motion sensor has no motion property");
            return;
        };

        let Some(motion_sensor_id) = motion_property.external_id() else {
            warn!(device_id = device.id, "⚠️ Motion sensor motion property has no Hue resource id");
            return;
        };

        let enabled_property = device.get_property_of_type::<BooleanProperty>(PropertyType::Enabled).and_then(|enabled_property| {
            property.get(enabled_property.name())
                .and_then(|pv| match pv {
                    PropertyValue::SetBooleanValue(value) => Some(*value),
                    PropertyValue::ToggleBooleanValue => Some(!enabled_property.value()),
                    _ => None,
                })
        });

        let sensitivity = device.get_property_of_type::<NumberProperty>(PropertyType::MotionSensitivity).and_then(|sensitivity_property| {
            property
                .get(sensitivity_property.name())
                .and_then(|pv| match pv {
                    PropertyValue::SetNumberValue(value) => Some(value.clone()),
                    PropertyValue::IncrementNumberValue(value) => Some(sensitivity_property.value().unwrap_or(Number::PositiveInt(0)) + value.clone()),
                    PropertyValue::DecrementNumberValue(value) => Some(sensitivity_property.value().unwrap_or(Number::PositiveInt(0)) - value.clone()),
                    _ => None,
                })
                .and_then(|sensitivity| match sensitivity_property.validate_value(sensitivity) {
                    ValidatedValue::Valid(value) => value.as_u64(),
                    ValidatedValue::Clamped(value, PropertyError::ValueTooSmall) => {
                        #[rustfmt::skip]
                        warn!(device_id = device.id, ?sensitivity_property, "🎚️ Motion sensor sensitivity value of '{}' is too small, clamped to the minimum valid value of '{}'",
                    sensitivity, value);
                        value.as_u64()
                    }
                    ValidatedValue::Clamped(value, PropertyError::ValueTooLarge) => {
                        #[rustfmt::skip]
                        warn!(device_id = device.id, ?sensitivity_property, "🎚️ Motion sensor sensitivity value of '{}' is too large, clamped to the maximim valid value of '{}'", sensitivity, value);
                        value.as_u64()
                    }
                    ValidatedValue::Clamped(value, error) => {
                        warn!("🎚️ Motion sensor sensitivity value of '{}' is invalid, clamped to {}", error, value);
                        value.as_u64()
                    }
                    ValidatedValue::Invalid(error) => {
                        warn!("🎚️ Motion sensor sensitivity value is invalid: {}", error);
                        None
                    }
                })
                .map(|sensitivity| SetSensitivity { sensitivity })
        });

        let request = MotionRequest::new(enabled_property, sensitivity);
        let url = format!("{}/clip/v2/resource/motion/{}", self.config.hue().url(), motion_sensor_id);
        self.send_request(&request, &url, &device, "motion sensor").await;
    }

    async fn send_request<T: Serialize + ?Sized>(&self, request: &T, url: &str, device: &Device, device_kind: &str) {
        let request_result = self
            .client
            .put(url)
            .json(request)
            .send()
            .await;

        let (result, status) = match &request_result {
            Err(_) => ("failure", "n/a".to_string()),
            Ok(response) if response.status().is_success() => ("success", "n/a".to_string()),
            Ok(response) => ("failure", response.status().as_u16().to_string()),
        };
        counter!(Metric::DeviceCommandDispatches.name(), "system" => "hue", "result" => result, "status" => status).increment(1);

        match request_result {
            Err(e) => {
                warn!(device_id = device.id, "⚠️ Unable to control the {}: {:?}", device_kind, e);
            }
            Ok(response) if !response.status().is_success() => {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                warn!(device_id = device.id, status_code = %status, "⚠️ Unable to control the {}, request to the Hue bridge failed. Response: {}", device_kind, body);
            }
            _ => {}
        }
    }
}
