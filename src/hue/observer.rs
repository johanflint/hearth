use crate::app_config::AppConfig;
use crate::domain::device::{Device, DeviceType};
use crate::hue::device_get::DeviceGet;
use crate::hue::hue_response::HueResponse;
use reqwest::Client;
use std::collections::HashMap;
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
        .map(|device_get| Device {
            id: device_get.id,
            r#type: DeviceType::Light,
            manufacturer: device_get.product_data.manufacturer_name,
            model_id: device_get.product_data.model_id,
            product_name: device_get.product_data.product_name,
            name: device_get.metadata.name,
            properties: HashMap::new(),
            external_id: None,
            address: None,
        })
        .collect::<Vec<Device>>();

    Ok(devices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_config::Hue;

    #[tokio::test]
    async fn observe_returns_mapped_devices() -> Result<(), Box<dyn Error>> {
        let mut server = mockito::Server::new_async().await;

        let mock = server
            .mock("GET", "/clip/v2/resource/device")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(include_str!("../../tests/resources/hue_device_response.json"))
            .match_header("hue-application-key", "key")
            .create_async()
            .await;

        let client = Client::new();

        let app_config = AppConfig {
            hue: Hue {
                url: server.url(),
                retry_ms: 100,
                max_delay_ms: 200,
                application_key: "key".to_string(),
            },
        };

        let response = observe(&client, &app_config).await?;

        mock.assert();
        assert_eq!(response.len(), 1);
        assert_eq!(
            response[0],
            Device {
                id: "079e0321-7e18-46bc-bc16-fcbc3dd09e30".to_string(),
                r#type: DeviceType::Light,
                manufacturer: "Signify Netherlands B.V.".to_string(),
                model_id: "LWA004".to_string(),
                product_name: "Hue filament bulb".to_string(),
                name: "Woonkamer".to_string(),
                properties: HashMap::new(),
                external_id: None,
                address: None,
            }
        );

        Ok(())
    }
}
