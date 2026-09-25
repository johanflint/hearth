use crate::app_config::AppConfig;
use crate::domain::device::Device;
use crate::hue::domain::{ButtonGet, DeviceGet, DevicePowerGet, HueResponse, LightGet, LightLevelGet, MotionGet, ZigbeeConnectivityGet};
use crate::hue::enrich_devices::enrich_devices;
use crate::hue::map_lights::{MapLightsError, map_lights};
use crate::hue::map_motion_sensors::{MapMotionSensorsError, map_motion_sensors};
use crate::hue::map_remotes::{MapRemotesError, map_remotes};
use reqwest::{Client, StatusCode};
use std::collections::{HashMap, HashSet};
use thiserror::Error;
use tracing::{debug, info, instrument, warn};

#[instrument(skip_all)]
pub async fn discover(client: &Client, config: &AppConfig) -> Result<Vec<Device>, DiscoverError> {
    info!("Retrieving Hue devices...");

    let hue_url = config.hue().url();
    let response = client
        .get(format!("{}/clip/v2/resource/device", hue_url))
        .send()
        .await?
        .error_for_status()
        .map_err(to_discover_error)?;

    let hue_response = response.json::<HueResponse<DeviceGet>>().await?;
    info!("Retrieving Hue devices... OK, {} found", hue_response.data.len());

    let response = client
        .get(format!("{}/clip/v2/resource/light", hue_url))
        .send()
        .await?
        .error_for_status()
        .map_err(to_discover_error)?;

    let light_response = response.json::<HueResponse<LightGet>>().await?;
    info!("Retrieving lights... OK, {} found", light_response.data.len());

    let response = client
        .get(format!("{}/clip/v2/resource/motion", hue_url))
        .send()
        .await?
        .error_for_status()
        .map_err(to_discover_error)?;

    let motion_response = response.json::<HueResponse<MotionGet>>().await?;
    info!("Retrieving motion sensors... OK, {} found", motion_response.data.len());

    let response = client
        .get(format!("{}/clip/v2/resource/light_level", hue_url))
        .send()
        .await?
        .error_for_status()
        .map_err(to_discover_error)?;

    let light_levels_response = response.json::<HueResponse<LightLevelGet>>().await?;
    debug!("Retrieving lights levels... OK, {} found", light_levels_response.data.len()); // Using debug as these aren't devices but services

    let response = client
        .get(format!("{}/clip/v2/resource/button", hue_url))
        .send()
        .await?
        .error_for_status()
        .map_err(to_discover_error)?;

    let button_response = response.json::<HueResponse<ButtonGet>>().await?;
    // Count the devices, not the buttons that button_response contains
    let remotes: HashSet<&str> = button_response.data.iter().map(|b| b.owner.rid.as_str()).collect();
    info!("Retrieving remotes... OK, {} found", remotes.len());

    let response = client
        .get(format!("{}/clip/v2/resource/zigbee_connectivity", hue_url))
        .send()
        .await?
        .error_for_status()
        .map_err(to_discover_error)?;

    let zigbee_connectivity_response = response.json::<HueResponse<ZigbeeConnectivityGet>>().await?;
    debug!("Retrieving zigbee connectivity... OK, {} found", zigbee_connectivity_response.data.len());

    let response = client
        .get(format!("{}/clip/v2/resource/device_power", hue_url))
        .send()
        .await?
        .error_for_status()
        .map_err(to_discover_error)?;

    let device_power_response = response.json::<HueResponse<DevicePowerGet>>().await?;
    debug!("Retrieving device power... OK, {} found", device_power_response.data.len());

    let mut device_map = hue_response.data.into_iter().map(|device| (device.id.clone(), device)).collect();

    let mut devices = vec![];
    let lights = map_lights(light_response.data, &mut device_map)?;
    let motion_sensors = map_motion_sensors(motion_response.data, light_levels_response.data, &mut device_map)?;
    let buttons = map_remotes(button_response.data, &mut device_map)?;

    devices.extend(lights);
    devices.extend(motion_sensors);
    devices.extend(buttons);

    // Enrich the devices with connectivity information
    enrich_devices(zigbee_connectivity_response.data, device_power_response.data, &mut devices);

    if !device_map.is_empty() {
        log_unmapped_devices(&device_map);
    }

    Ok(devices)
}

fn to_discover_error(e: reqwest::Error) -> DiscoverError {
    if let (Some(status), Some(url)) = (e.status(), e.url()) {
        DiscoverError::UnexpectedResponse(status, url.to_string())
    } else {
        DiscoverError::ClientError(e)
    }
}

#[instrument(skip_all)]
fn log_unmapped_devices(device_map: &HashMap<String, DeviceGet>) {
    let unmapped_devices = device_map
        .iter()
        .map(|(_, d)| format!("- {} {} '{}'", d.product_data.manufacturer_name, d.product_data.product_name, d.metadata.name))
        .collect::<Vec<String>>()
        .join("\n");
    warn!("⚠️ Ignored {} unsupported Hue devices:\n{}", device_map.len(), unmapped_devices);
}

#[derive(Error, Debug)]
pub enum DiscoverError {
    #[error("client error: {0}")]
    ClientError(#[from] reqwest::Error),
    #[error("unexpected status code {0} when calling {1}")]
    UnexpectedResponse(StatusCode, String),
    #[error(transparent)]
    MapLights(#[from] MapLightsError),
    #[error(transparent)]
    MapMotionSensors(#[from] MapMotionSensorsError),
    #[error(transparent)]
    MapRemotes(#[from] MapRemotesError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_config::AppConfigBuilder;
    use crate::domain::{BatteryState, Connectivity};
    use crate::domain::device::DeviceType;
    use crate::domain::property::{BooleanProperty, DateTimeProperty, EnumProperty, NumberProperty, Property, PropertyType, Unit};
    use crate::hue::client::new_client;
    use chrono::{TimeZone, Timelike, Utc};
    use pretty_assertions::assert_eq;
    use std::collections::HashMap;
    use strum::IntoEnumIterator;

    fn connectivity_property(status: &str) -> Box<dyn Property> {
        let allowed_connectivity_values: Vec<String> = Connectivity::iter().map(|c| c.as_str().to_string()).collect();
        Box::new(EnumProperty::new("connectivity".to_string(), PropertyType::Connectivity, true, None, Some(status.to_string()), allowed_connectivity_values.clone()).unwrap())
    }

    fn battery_level_property(battery_level: Option<u64>) -> Box<dyn Property> {
        Box::new(NumberProperty::builder("batteryLevel".to_string(), PropertyType::BatteryLevel, true)
            .unit(Unit::Percentage)
            .positive_int(battery_level, Some(0), Some(100))
            .build())
    }

    fn battery_state_property(state: Option<&str>) -> Box<dyn Property> {
        let allowed_battery_state_values: Vec<String> = BatteryState::iter().map(|s| s.as_str().to_string()).collect();
        Box::new(EnumProperty::new("batteryState".to_string(), PropertyType::BatteryState, true, None, state.map(str::to_string), allowed_battery_state_values).unwrap())
    }

    #[tokio::test]
    async fn discover_returns_mapped_devices() -> Result<(), DiscoverError> {
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
            .with_body(include_str!("../../tests/resources/hue_light_simplified_response.json"))
            .create_async()
            .await;

        server
            .mock("GET", "/clip/v2/resource/motion")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(include_str!("../../tests/resources/hue_motion_simplified_response.json"))
            .create_async()
            .await;

        server
            .mock("GET", "/clip/v2/resource/light_level")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(include_str!("../../tests/resources/hue_light_level_simplified_response.json"))
            .create_async()
            .await;

        server
            .mock("GET", "/clip/v2/resource/button")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(include_str!("../../tests/resources/hue_button_simplified_response.json"))
            .create_async()
            .await;

        server
            .mock("GET", "/clip/v2/resource/zigbee_connectivity")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(include_str!("../../tests/resources/hue_zigbee_connectivity_simplified_response.json"))
            .create_async()
            .await;

        server
            .mock("GET", "/clip/v2/resource/device_power")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(include_str!("../../tests/resources/hue_device_power_simplified_response.json"))
            .create_async()
            .await;

        let app_config = AppConfigBuilder::new().hue_url(server.url()).build();
        let client = new_client(&app_config).unwrap();

        let response = discover(&client, &app_config).await?;

        let on_property: Box<dyn Property> = Box::new(BooleanProperty::new(
            "on".to_string(),
            PropertyType::On,
            false,
            Some("703c7167-ff79-4fd4-a3d9-635b3f237a4f".to_string()),
            false,
        ));

        let enabled_property: Box<dyn Property> = Box::new(BooleanProperty::new(
            "enabled".to_string(),
            PropertyType::Enabled,
            false,
            Some("0af9eb8a-f38f-427c-b819-0c6850f55fe9".to_string()),
            false,
        ));

        let motion_property: Box<dyn Property> = Box::new(BooleanProperty::new(
            "motion".to_string(),
            PropertyType::Motion,
            true,
            Some("0af9eb8a-f38f-427c-b819-0c6850f55fe9".to_string()),
            false,
        ));

        let motion_last_changed_property: Box<dyn Property> = Box::new(DateTimeProperty::new(
            "motionLastChanged".to_string(),
            PropertyType::MotionLastChanged,
            true,
            None,
            Some(Utc.with_ymd_and_hms(2026, 9, 19, 19, 53, 59).unwrap().with_nanosecond(108_000_000).unwrap()),
        ));

        let sensitivity_property: Box<dyn Property> = Box::new(
            NumberProperty::builder("sensitivity".to_string(), PropertyType::MotionSensitivity, false)
                .external_id("0af9eb8a-f38f-427c-b819-0c6850f55fe9".to_string())
                .unit(Unit::None)
                .positive_int(Some(2), Some(0), Some(4))
                .build(),
        );

        let illuminance_property: Box<dyn Property> = Box::new(
            NumberProperty::builder("illuminance".to_string(), PropertyType::Illuminance, true)
                .unit(Unit::Lux)
                .float(Some(36.257679024119625), Some(0.0), None)
                .build()
        );

        let illuminance_last_changed_property: Box<dyn Property> = Box::new(DateTimeProperty::new(
            "illuminanceLastChanged".to_string(),
            PropertyType::IlluminanceLastChanged,
            true,
            None,
            Some("2026-09-23T15:37:20.001Z".parse().unwrap()),
        ));

        let button_property: Box<dyn Property> = Box::new(
            EnumProperty::new(
                "button1".to_string(),
                PropertyType::Button,
                true,
                Some("f72e36a1-50e1-4d01-9c04-c9d44327285e".to_string()),
                Some("short_release".to_string()),
                vec![
                    "initial_press".to_string(),
                    "repeat".to_string(),
                    "short_release".to_string(),
                    "long_release".to_string(),
                    "long_press".to_string(),
                ],
            ).unwrap(),
        );

        let button_last_changed_property: Box<dyn Property> = Box::new(DateTimeProperty::new(
            "button1LastChanged".to_string(),
            PropertyType::ButtonLastChanged,
            true,
            Some("f72e36a1-50e1-4d01-9c04-c9d44327285e".to_string()),
            Some("2026-09-20T18:36:08.948Z".parse().unwrap()),
        ));

        mock.assert();
        assert_eq!(response.len(), 3);
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
                    ("connectivity".to_string(), connectivity_property("connected")),
                    ("batteryLevel".to_string(), battery_level_property(Some(2))),
                    ("batteryState".to_string(), battery_state_property(Some("low"))),
                ]),
                external_id: None,
                address: None,
                controller_id: Some("hue"),
            }
        );
        assert_eq!(
            response[1],
            Device {
                id: "dbe91174-8fe5-4309-b7d7-3f15ec1e57d3".to_string(),
                r#type: DeviceType::MotionSensor,
                manufacturer: "Signify Netherlands B.V.".to_string(),
                model_id: "SML004".to_string(),
                product_name: "Hue outdoor motion sensor".to_string(),
                name: "Schuur sensor".to_string(),
                properties: HashMap::from([
                    (enabled_property.name().to_string(), enabled_property),
                    (motion_property.name().to_string(), motion_property),
                    (motion_last_changed_property.name().to_string(), motion_last_changed_property),
                    (sensitivity_property.name().to_string(), sensitivity_property),
                    (illuminance_property.name().to_string(), illuminance_property),
                    (illuminance_last_changed_property.name().to_string(), illuminance_last_changed_property),
                    ("connectivity".to_string(), connectivity_property("issues")),
                    ("batteryLevel".to_string(), battery_level_property(Some(30))),
                    ("batteryState".to_string(), battery_state_property(Some("normal"))),
                ]),
                external_id: None,
                address: None,
                controller_id: Some("hue"),
            }
        );
        assert_eq!(
            response[2],
            Device {
                id: "3a3225cb-dcda-46fb-8f21-00a8c76024bc".to_string(),
                r#type: DeviceType::Remote,
                manufacturer: "Signify Netherlands B.V.".to_string(),
                model_id: "RWL021".to_string(),
                product_name: "Hue dimmer switch".to_string(),
                name: "Dimmer".to_string(),
                properties: HashMap::from([
                    (button_property.name().to_string(), button_property),
                    (button_last_changed_property.name().to_string(), button_last_changed_property),
                    ("connectivity".to_string(), connectivity_property("unknown")),
                    ("batteryLevel".to_string(), battery_level_property(Some(1))),
                    ("batteryState".to_string(), battery_state_property(Some("critical"))),
                ]),
                external_id: None,
                address: None,
                controller_id: Some("hue"),
            }
        );

        Ok(())
    }

    #[tokio::test]
    async fn discover_returns_an_error_for_an_unexpected_response() -> Result<(), DiscoverError> {
        let mut server = mockito::Server::new_async().await;

        let mock = server.mock("GET", "/clip/v2/resource/device").with_status(400).create_async().await;

        let client = Client::new();

        let app_config = AppConfigBuilder::new().hue_url(server.url()).build();

        let response = discover(&client, &app_config).await;
        assert!(response.is_err());

        match response {
            Err(DiscoverError::UnexpectedResponse(StatusCode::BAD_REQUEST, url)) => {
                assert_eq!(url, format!("{}/clip/v2/resource/device", server.url()))
            }
            _ => panic!("unexpected response"),
        }

        mock.assert();
        Ok(())
    }
}
