use crate::app_config::AppConfig;
use crate::domain::device::Device;
use crate::hue::domain::{DeviceGet, HueResponse, LightGet};
use crate::hue::map_lights::map_lights;
use reqwest::{Client, StatusCode};
use std::collections::HashMap;
use thiserror::Error;
use tracing::{info, instrument, warn};

#[instrument(skip_all)]
pub async fn observe(client: &Client, config: &AppConfig) -> Result<Vec<Device>, ObserverError> {
    info!("Retrieving Hue devices...");

    let hue_url = config.hue().url();
    let response = client
        .get(format!("{}/clip/v2/resource/device", hue_url))
        .send()
        .await?
        .error_for_status()
        .map_err(|e| ObserverError::UnexpectedResponse(e.status().unwrap(), e.url().unwrap().to_string()))?;

    let hue_response = response.json::<HueResponse<DeviceGet>>().await?;
    info!("Retrieving Hue devices... OK, {} found", hue_response.data.len());

    let response = client
        .get(format!("{}/clip/v2/resource/light", hue_url))
        .send()
        .await?
        .error_for_status()
        .map_err(|e| ObserverError::UnexpectedResponse(e.status().unwrap(), e.url().unwrap().to_string()))?;

    let light_response = response.json::<HueResponse<LightGet>>().await?;
    info!("Retrieving lights... OK, {} found", light_response.data.len());

    let mut device_map = hue_response.data.into_iter().map(|device| (device.id.clone(), device)).collect();
    let devices = map_lights(light_response.data, &mut device_map).unwrap();

    if !device_map.is_empty() {
        log_unmapped_devices(&device_map);
    }

    Ok(devices)
}

#[instrument(skip_all)]
fn log_unmapped_devices(device_map: &HashMap<String, DeviceGet>) {
    let unmapped_devices = device_map
        .iter()
        .map(|(_, d)| {
            format!(
                "- {} {} '{}'",
                d.product_data.manufacturer_name, d.product_data.product_name, d.metadata.name
            )
        })
        .collect::<Vec<String>>()
        .join("\n");
    warn!("⚠️ Ignored {} unsupported Hue devices:\n{}", device_map.len(), unmapped_devices);
}

#[derive(Error, Debug)]
pub enum ObserverError {
    #[error("client error: {0}")]
    ClientError(#[from] reqwest::Error),
    #[error("unexpected status code {0} when calling {1}")]
    UnexpectedResponse(StatusCode, String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_config::AppConfigBuilder;
    use crate::domain::device::DeviceType;
    use crate::domain::property::{BooleanProperty, NumberProperty, Property, PropertyType, Unit};
    use crate::hue::client::new_client;
    use std::collections::HashMap;

    #[tokio::test]
    async fn observe_returns_mapped_devices() -> Result<(), ObserverError> {
        let mut server = mockito::Server::new_async().await;

        let mock = server
            .mock("GET", "/clip/v2/resource/device")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(include_str!("../../tests/resources/hue_device_response.json"))
            .match_header("hue-application-key", "key")
            .create_async()
            .await;

        server
            .mock("GET", "/clip/v2/resource/light")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(include_str!("../../tests/resources/hue_light_response.json"))
            .create_async()
            .await;

        let app_config = AppConfigBuilder::new().hue_url(server.url()).build();
        let client = new_client(&app_config).unwrap();

        let response = observe(&client, &app_config).await?;

        let on_property: Box<dyn Property> = Box::new(BooleanProperty::new(
            "on".to_string(),
            PropertyType::On,
            false,
            Some("703c7167-ff79-4fd4-a3d9-635b3f237a4f".to_string()),
            false,
        ));

        let brightness_property: Box<dyn Property> = Box::new(
            NumberProperty::builder("brightness".to_string(), PropertyType::Brightness, false)
                .external_id("703c7167-ff79-4fd4-a3d9-635b3f237a4f".to_string())
                .unit(Unit::Percentage)
                .float(19.37, Some(2.0), Some(100.0))
                .build(),
        );

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
                properties: HashMap::from([
                    (on_property.name().to_string(), on_property),
                    (brightness_property.name().to_string(), brightness_property)
                ]),
                external_id: None,
                address: None,
            }
        );

        Ok(())
    }

    #[tokio::test]
    async fn observe_returns_an_error_for_an_unexpected_response() -> Result<(), ObserverError> {
        let mut server = mockito::Server::new_async().await;

        let mock = server.mock("GET", "/clip/v2/resource/device").with_status(400).create_async().await;

        let client = Client::new();

        let app_config = AppConfigBuilder::new().hue_url(server.url()).build();

        let response = observe(&client, &app_config).await;
        assert!(response.is_err());

        match response {
            Err(ObserverError::UnexpectedResponse(StatusCode::BAD_REQUEST, url)) => {
                assert_eq!(url, format!("{}/clip/v2/resource/device", server.url()))
            }
            _ => panic!("unexpected response"),
        }

        mock.assert();
        Ok(())
    }
}
