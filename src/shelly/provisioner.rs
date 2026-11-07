use crate::app_config::{AppConfig, Mqtt as MqttAppConfig};
use crate::shelly::domain::{Component, MqttConfig, MqttStatus};
use crate::shelly::fetch::{FetchError, call};
use crate::shelly::prober::{ProbedDevice, ProbedDeviceMap};
use crate::shelly::take_new::take_new;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::watch::Receiver as WatchReceiver;
use tokio::task;
use tracing::{debug, info, warn};

pub fn provision_announced_devices(
    client: &Client,
    config: Arc<AppConfig>,
    mut probed_rx: WatchReceiver<ProbedDeviceMap>,
) {
    let client = client.clone();
    task::spawn(async move {
        let mut seen = HashSet::new();
        loop {
            let probed_devices = take_new(&probed_rx.borrow_and_update(), &mut seen);
            for device in probed_devices {
                let client = client.clone();
                let config = Arc::clone(&config);
                task::spawn(async move {
                    if let Err(err) = provision(&device, &config, &client).await {
                        warn!("⚠️ Provisioning '{}' failed: {err}", device.info.id);
                    };
                });
            }

            if probed_rx.changed().await.is_err() {
                break; // Prober stopped
            }
        }
    });
}

async fn provision(device: &ProbedDevice, config: &AppConfig, client: &Client) -> Result<(), FetchError> {
    let Some((mqtt_config, mqtt_status)) = mqtt_config(device) else {
        debug!(device_id = device.info.id, "MQTT is not supported for this device");
        return Ok(());
    };

    let desired = MqttSettings::desired(&device.info.id, config.mqtt());
    if MqttSettings::actual(&device.info.id, mqtt_config) == desired {
        if mqtt_status.connected {
            info!(device_id = device.info.id, "⚙️ Provisioning Shelly device '{}' not needed", device.info.id);
        } else {
            warn!(
                device_id = device.info.id,
                "⚠️ Shelly device '{}' has the correct MQTT settings but is not connected", device.info.id
            );
        }

        return Ok(());
    }

    info!(device_id = device.info.id, "⚙️ Provisioning Shelly device '{}'...", device.info.id);
    let request = SetConfigRequest { config: SetConfig { settings: &desired, pass: config.mqtt().password() } };
    let result: SetConfigResult = call(client, &device.address, "MQTT.SetConfig", &request).await?;

    if result.restart_required {
        info!(
            device_id = device.info.id,
            "⚙️ Provisioning Shelly device '{}'... OK, restarting to apply settings", device.info.id
        );
        call::<()>(client, &device.address, "Shelly.Reboot", &json!({})).await?;
    } else {
        info!(device_id = device.info.id, "⚙️ Provisioning Shelly device '{}'... OK", device.info.id);
    }
    Ok(())
}

fn mqtt_config(device: &ProbedDevice) -> Option<(&MqttConfig, &MqttStatus)> {
    device.components.iter().find_map(|component| match component {
        Component::Mqtt { config, status } => Some((config, status)),
        _ => None,
    })
}

#[derive(Debug, PartialEq, Serialize)]
struct MqttSettings {
    enable: bool,
    server: Option<String>, // hostname:port
    user: Option<String>,
    topic_prefix: String,
    rpc_ntf: bool,
    status_ntf: bool,
    enable_rpc: bool,
    enable_control: bool,
}

// `pass` is write-only on the device, so it's not part of MqttSettings
#[derive(Serialize)]
struct SetConfigRequest<'a> {
    config: SetConfig<'a>,
}

#[derive(Serialize)]
struct SetConfig<'a> {
    #[serde(flatten)]
    settings: &'a MqttSettings,
    pass: &'a str,
}

#[derive(Debug, Deserialize)]
struct SetConfigResult {
    restart_required: bool,
}

impl MqttSettings {
    fn desired(device_id: &str, config: &MqttAppConfig) -> Self {
        MqttSettings {
            enable: true,
            server: Some(format!("{}:{}", config.advertised_host(), config.advertised_port())),
            user: Some(config.username().to_string()),
            topic_prefix: device_id.to_string(),
            rpc_ntf: false,
            status_ntf: true,
            enable_rpc: false,
            enable_control: false,
        }
    }

    fn actual(device_id: &str, config: &MqttConfig) -> Self {
        MqttSettings {
            enable: config.enable,
            server: config.server.clone(),
            user: config.user.clone(),
            topic_prefix: config.topic_prefix.clone().unwrap_or_else(|| device_id.to_string()),
            rpc_ntf: config.rpc_ntf,
            status_ntf: config.status_ntf,
            enable_rpc: config.enable_rpc,
            enable_control: config.enable_control,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_config::AppConfigBuilder;
    use crate::shelly::test_support::{PLUS_1_ID, ble_component, mqtt_component, probed_device};
    use axum::http::StatusCode;
    use mockito::{Matcher, Server};
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use serde_json::{Value, json};
    use std::net::SocketAddr;

    // The advertised host and port of AppConfigBuilder, which differ from the broker's own address
    const ADVERTISED_SERVER: &str = "192.168.1.100:1883";

    fn app_config() -> AppConfig {
        AppConfigBuilder::new().build()
    }

    // The config of a device that is provisioned the way hearth wants it
    fn provisioned_config() -> Value {
        json!({
            "enable": true,
            "server": ADVERTISED_SERVER,
            "client_id": null,
            "user": "hearth",
            "topic_prefix": PLUS_1_ID,
            "rpc_ntf": false,
            "status_ntf": true,
            "enable_rpc": false,
            "enable_control": false,
        })
    }

    fn with(mut config: Value, key: &str, value: Value) -> Value {
        config[key] = value;
        config
    }

    fn device(address: SocketAddr, components: &[String]) -> ProbedDevice {
        ProbedDevice { address, ..probed_device(PLUS_1_ID, components) }
    }

    fn actual_settings(config: Value) -> MqttSettings {
        let device = probed_device(PLUS_1_ID, &[mqtt_component(true, config)]);
        let (config, _) = mqtt_config(&device).expect("device should have an MQTT component");
        MqttSettings::actual(PLUS_1_ID, config)
    }

    fn desired_settings() -> MqttSettings {
        MqttSettings::desired(PLUS_1_ID, app_config().mqtt())
    }

    #[test]
    fn desired_settings_use_the_app_config_and_the_device_id() {
        let settings = desired_settings();

        let expected = MqttSettings {
            enable: true,
            server: Some(ADVERTISED_SERVER.to_string()),
            user: Some("hearth".to_string()),
            topic_prefix: PLUS_1_ID.to_string(),
            rpc_ntf: false,
            status_ntf: true,
            enable_rpc: false,
            enable_control: false,
        };
        assert_eq!(settings, expected);
    }

    #[test]
    fn actual_settings_equal_the_desired_settings_of_a_provisioned_device() {
        assert_eq!(actual_settings(provisioned_config()), desired_settings());
    }

    #[test]
    fn actual_settings_default_the_topic_prefix_to_the_device_id() {
        let settings = actual_settings(with(provisioned_config(), "topic_prefix", Value::Null));

        assert_eq!(settings.topic_prefix, PLUS_1_ID);
    }

    #[test]
    fn actual_settings_ignore_the_client_id() {
        let settings = actual_settings(with(provisioned_config(), "client_id", json!("custom")));

        assert_eq!(settings, desired_settings());
    }

    #[rstest]
    #[case::disabled("enable", json!(false))]
    #[case::no_server("server", Value::Null)]
    #[case::other_server("server", json!("10.0.0.1:1883"))]
    #[case::other_port("server", json!("192.168.1.100:8883"))]
    #[case::broker_address_instead_of_advertised("server", json!("127.0.0.1:1883"))]
    #[case::no_user("user", Value::Null)]
    #[case::other_user("user", json!("someone"))]
    #[case::other_topic_prefix("topic_prefix", json!("custom"))]
    #[case::rpc_notifications("rpc_ntf", json!(true))]
    #[case::no_status_notifications("status_ntf", json!(false))]
    #[case::rpc_enabled("enable_rpc", json!(true))]
    #[case::control_enabled("enable_control", json!(true))]
    fn actual_settings_differ_from_the_desired_settings(#[case] key: &str, #[case] value: Value) {
        assert_ne!(actual_settings(with(provisioned_config(), key, value)), desired_settings());
    }

    #[test]
    fn the_password_is_sent_next_to_the_settings() {
        let settings = desired_settings();
        let request = SetConfigRequest { config: SetConfig { settings: &settings, pass: "secret" } };

        let expected = json!({ "config": {
            "enable": true,
            "server": ADVERTISED_SERVER,
            "user": "hearth",
            "topic_prefix": PLUS_1_ID,
            "rpc_ntf": false,
            "status_ntf": true,
            "enable_rpc": false,
            "enable_control": false,
            "pass": "secret",
        }});
        assert_eq!(serde_json::to_value(&request).unwrap(), expected);
    }

    #[tokio::test]
    async fn provision_leaves_a_provisioned_device_alone() {
        let mut server = Server::new_async().await;
        let set_config = server.mock("POST", "/rpc/MQTT.SetConfig").expect(0).create_async().await;
        let reboot = server.mock("POST", "/rpc/Shelly.Reboot").expect(0).create_async().await;
        let device = device(server.socket_address(), &[mqtt_component(true, provisioned_config())]);

        provision(&device, &app_config(), &Client::new()).await.unwrap();

        set_config.assert_async().await;
        reboot.assert_async().await;
    }

    #[tokio::test]
    async fn provision_leaves_a_provisioned_but_disconnected_device_alone() {
        let mut server = Server::new_async().await;
        let set_config = server.mock("POST", "/rpc/MQTT.SetConfig").expect(0).create_async().await;
        let reboot = server.mock("POST", "/rpc/Shelly.Reboot").expect(0).create_async().await;
        let device = device(server.socket_address(), &[mqtt_component(false, provisioned_config())]);

        provision(&device, &app_config(), &Client::new()).await.unwrap();

        set_config.assert_async().await;
        reboot.assert_async().await;
    }

    #[tokio::test]
    async fn provision_leaves_a_device_without_mqtt_alone() {
        let mut server = Server::new_async().await;
        let set_config = server.mock("POST", "/rpc/MQTT.SetConfig").expect(0).create_async().await;
        let device = device(server.socket_address(), &[ble_component()]);

        provision(&device, &app_config(), &Client::new()).await.unwrap();

        set_config.assert_async().await;
    }

    #[tokio::test]
    async fn provision_configures_the_device_and_reboots_it() {
        let mut server = Server::new_async().await;
        let set_config = server
            .mock("POST", "/rpc/MQTT.SetConfig")
            .match_body(Matcher::AllOf(vec![
                Matcher::PartialJson(json!({ "config": {
                    "enable": true,
                    "server": ADVERTISED_SERVER,
                    "user": "hearth",
                    "topic_prefix": PLUS_1_ID,
                    "rpc_ntf": false,
                    "status_ntf": true,
                    "enable_rpc": false,
                    "enable_control": false,
                }})),
                Matcher::Regex(r#""pass":"[^"]+""#.to_string()),
            ]))
            .with_body(r#"{"restart_required":true}"#)
            .create_async()
            .await;
        let reboot = server.mock("POST", "/rpc/Shelly.Reboot").with_body("null").create_async().await;
        let device = device(
            server.socket_address(),
            &[mqtt_component(false, with(provisioned_config(), "enable", json!(false)))],
        );

        provision(&device, &app_config(), &Client::new()).await.unwrap();

        set_config.assert_async().await;
        reboot.assert_async().await;
    }

    #[tokio::test]
    async fn provision_does_not_reboot_when_no_restart_is_required() {
        let mut server = Server::new_async().await;
        let set_config =
            server.mock("POST", "/rpc/MQTT.SetConfig").with_body(r#"{"restart_required":false}"#).create_async().await;
        let reboot = server.mock("POST", "/rpc/Shelly.Reboot").expect(0).create_async().await;
        let device = device(
            server.socket_address(),
            &[mqtt_component(false, with(provisioned_config(), "status_ntf", json!(false)))],
        );

        provision(&device, &app_config(), &Client::new()).await.unwrap();

        set_config.assert_async().await;
        reboot.assert_async().await;
    }

    #[tokio::test]
    async fn provision_fails_when_the_device_rejects_the_configuration() {
        let mut server = Server::new_async().await;
        server.mock("POST", "/rpc/MQTT.SetConfig").with_status(401).create_async().await;
        let reboot = server.mock("POST", "/rpc/Shelly.Reboot").expect(0).create_async().await;
        let device = device(
            server.socket_address(),
            &[mqtt_component(false, with(provisioned_config(), "enable", json!(false)))],
        );

        let error = provision(&device, &app_config(), &Client::new()).await.unwrap_err();

        assert!(matches!(error, FetchError::UnexpectedResponse(StatusCode::UNAUTHORIZED, _)), "got {error:?}");
        reboot.assert_async().await;
    }

    #[tokio::test]
    async fn provision_fails_when_the_reboot_fails() {
        let mut server = Server::new_async().await;
        server.mock("POST", "/rpc/MQTT.SetConfig").with_body(r#"{"restart_required":true}"#).create_async().await;
        server.mock("POST", "/rpc/Shelly.Reboot").with_status(500).create_async().await;
        let device = device(
            server.socket_address(),
            &[mqtt_component(false, with(provisioned_config(), "enable", json!(false)))],
        );

        let error = provision(&device, &app_config(), &Client::new()).await.unwrap_err();

        assert!(matches!(error, FetchError::UnexpectedResponse(StatusCode::INTERNAL_SERVER_ERROR, _)), "got {error:?}");
    }
}
