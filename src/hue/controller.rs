use crate::app_config::AppConfig;
use crate::domain::commands::Command;
use crate::domain::controller::Controller;
use crate::domain::device::DeviceType;
use crate::domain::property::{BooleanProperty, NumberProperty, Property, PropertyType};
use crate::flow_engine::property_value::PropertyValue;
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
            Command::ControlDevice { device: device_lock, property } => {
                let device = device_lock.read().await;

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
                        property.get(brightness_property.name()).and_then(|pv| match pv {
                            PropertyValue::SetNumberValue(value) => value.as_f64(),
                            _ => None,
                        })
                    });

                    let request = LightRequest::new(on, brightness);
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
                            let body = response.text().await;
                            warn!(device_id = device.id, status_code = %status, "⚠️ Unable to control the light, request to the Hue bridge failed. Response: {:?}", body);
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
