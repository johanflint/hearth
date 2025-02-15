use crate::app_config::AppConfig;
use crate::domain::device::Device;
use crate::hue::device_get::DeviceGet;
use crate::hue::hue_response::HueResponse;
use reqwest::Client;
use serde::Deserialize;
use std::error::Error;
use tracing::{info, instrument};

#[instrument(skip(client, config))]
pub async fn observe(client: &Client, config: &AppConfig) -> Result<Vec<Device>, Box<dyn Error>> {
    info!("Retrieving Hue devices...");

    let hue_url = config.hue().url();
    let response = client
        .get(format!("{}/clip/v2/resource/device", hue_url))
        .header("hue-application-key", config.hue().application_key())
        .send()
        .await?
        .error_for_status()?;

    let hue_response = response.json::<HueResponse<DeviceGet>>().await?;
    info!("Retrieving Hue devices... OK, {} found", hue_response.data.len());

    let devices = hue_response
        .data
        .into_iter()
        .map(|device_get| Device { id: device_get.id })
        .collect::<Vec<Device>>();

    Ok(devices)
}
