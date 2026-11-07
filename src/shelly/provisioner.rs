use crate::app_config::{AppConfig, Mqtt as MqttAppConfig};
use crate::shelly::domain::{Component, ComponentsResult, MqttConfig, MqttStatus};
use crate::shelly::fetch::{FetchError, call, fetch};
use crate::shelly::prober::{ProbedDevice, ProbedDeviceMap};
use crate::shelly::take_new::take_new;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch::Receiver as WatchReceiver;
use tokio::task;
use tracing::{debug, info, warn};

const INITIAL_DELAY: Duration = Duration::from_secs(30);
const MAX_DELAY: Duration = Duration::from_secs(15 * 60);

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
                    provision_with_retry(&device, &config, &client).await;
                });
            }

            if probed_rx.changed().await.is_err() {
                break; // Prober stopped
            }
        }
    });
}

async fn provision_with_retry(device: &ProbedDevice, config: &AppConfig, client: &Client) {
    let Some(mut state) = mqtt_state(&device.components) else {
        debug!(device_id = device.info.id, "MQTT is not supported for this device");
        return;
    };

    let mut delay = INITIAL_DELAY;
    loop {
        match provision(device, &state, config, client).await {
            Ok(Provisioning::Connected) => return,
            Ok(Provisioning::Pending) => {}
            Err(err) => {
                warn!(
                    device_id = device.info.id,
                    "⚠️ Provisioning '{}' failed, retrying in {delay:?}: {err}", device.info.id
                );
            }
        }

        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(MAX_DELAY);

        match fetch_mqtt_state(client, &device.address).await {
            Ok(Some(fresh_state)) => state = fresh_state,
            Ok(None) => debug!(device_id = device.info.id, "MQTT component is gone, keeping the last known state"),
            Err(err) => debug!(device_id = device.info.id, "Fetching MQTT state failed: {err}"),
        }
    }
}

async fn fetch_mqtt_state(client: &Client, address: &SocketAddr) -> Result<Option<MqttState>, FetchError> {
    let config = fetch::<ComponentsResult>(
        client,
        address,
        "Shelly.GetComponents",
        &[("keys", r#"["mqtt"]"#), ("include", r#"["config","status"]"#)],
    )
    .await?;

    Ok(mqtt_state(&config.components))
}

#[derive(Debug)]
enum Provisioning {
    Connected,
    Pending,
}

async fn provision(
    device: &ProbedDevice,
    mqtt_state: &MqttState,
    config: &AppConfig,
    client: &Client,
) -> Result<Provisioning, FetchError> {
    let desired = MqttSettings::desired(&device.info.id, config.mqtt());
    let settings_match = MqttSettings::actual(&device.info.id, &mqtt_state.config) == desired;
    if settings_match && mqtt_state.status.connected {
        info!(device_id = device.info.id, "⚙️ Shelly device '{}' is provisioned and connected", device.info.id);
        return Ok(Provisioning::Connected);
    }

    if settings_match {
        warn!(
            device_id = device.info.id,
            "⚠️ Shelly device '{}' has the correct MQTT settings but is not connected yet", device.info.id
        );

        return Ok(Provisioning::Pending);
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

    Ok(Provisioning::Pending)
}

fn mqtt_state(components: &[Component]) -> Option<MqttState> {
    components.iter().find_map(|component| match component {
        Component::Mqtt { config, status } => Some(MqttState { config: config.clone(), status: status.clone() }),
        _ => None,
    })
}

#[derive(Debug, Clone, PartialEq)]
struct MqttState {
    config: MqttConfig,
    status: MqttStatus,
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
    use mockito::{Matcher, Mock, Server, ServerGuard};
    use pretty_assertions::assert_eq;
    use reqwest::StatusCode;
    use rstest::rstest;
    use serde_json::{Value, json};
    use std::net::SocketAddr;
    use tokio::time::Instant;

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

    fn state(connected: bool, config: Value) -> MqttState {
        let device = probed_device(PLUS_1_ID, &[mqtt_component(connected, config)]);
        mqtt_state(&device.components).expect("device should have an MQTT component")
    }

    fn actual_settings(config: Value) -> MqttSettings {
        MqttSettings::actual(PLUS_1_ID, &state(true, config).config)
    }

    fn unprovisioned_config() -> Value {
        with(provisioned_config(), "enable", json!(false))
    }

    // Returns the mock unregistered, so the caller can set expectations before `create_async`
    fn set_config_mock(server: &mut ServerGuard, restart_required: bool) -> Mock {
        server
            .mock("POST", "/rpc/MQTT.SetConfig")
            .with_body(json!({ "restart_required": restart_required }).to_string())
    }

    // Returns the mock unregistered, so the caller can set expectations before `create_async`
    fn mqtt_state_mock(server: &mut ServerGuard, connected: bool, config: Value) -> Mock {
        server
            .mock("GET", "/rpc/Shelly.GetComponents")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("keys".to_string(), r#"["mqtt"]"#.to_string()),
                Matcher::UrlEncoded("include".to_string(), r#"["config","status"]"#.to_string()),
            ]))
            .with_body(format!(r#"{{"components":[{}]}}"#, mqtt_component(connected, config)))
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

    #[test]
    fn mqtt_state_is_none_without_an_mqtt_component() {
        assert_eq!(mqtt_state(&probed_device(PLUS_1_ID, &[ble_component()]).components), None);
    }

    #[tokio::test]
    async fn provision_leaves_a_provisioned_and_connected_device_alone() {
        let mut server = Server::new_async().await;
        let set_config = server.mock("POST", "/rpc/MQTT.SetConfig").expect(0).create_async().await;
        let reboot = server.mock("POST", "/rpc/Shelly.Reboot").expect(0).create_async().await;
        let device = device(server.socket_address(), &[]);

        let result = provision(&device, &state(true, provisioned_config()), &app_config(), &Client::new()).await;

        assert!(matches!(result, Ok(Provisioning::Connected)), "got {result:?}");
        set_config.assert_async().await;
        reboot.assert_async().await;
    }

    // Waits for the device to connect, the next commits push fresh credentials here
    #[tokio::test]
    async fn provision_leaves_a_provisioned_but_disconnected_device_pending() {
        let mut server = Server::new_async().await;
        let set_config = server.mock("POST", "/rpc/MQTT.SetConfig").expect(0).create_async().await;
        let reboot = server.mock("POST", "/rpc/Shelly.Reboot").expect(0).create_async().await;
        let device = device(server.socket_address(), &[]);

        let result = provision(&device, &state(false, provisioned_config()), &app_config(), &Client::new()).await;

        assert!(matches!(result, Ok(Provisioning::Pending)), "got {result:?}");
        set_config.assert_async().await;
        reboot.assert_async().await;
    }

    // The MQTT status reflects the old settings, so it doesn't count until the new settings are applied
    #[tokio::test]
    async fn provision_configures_a_connected_device_with_other_settings() {
        let mut server = Server::new_async().await;
        let set_config = set_config_mock(&mut server, false).create_async().await;
        let device = device(server.socket_address(), &[]);
        let other_server = with(provisioned_config(), "server", json!("10.0.0.1:1883"));

        let result = provision(&device, &state(true, other_server), &app_config(), &Client::new()).await;

        assert!(matches!(result, Ok(Provisioning::Pending)), "got {result:?}");
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
        let device = device(server.socket_address(), &[]);

        let result = provision(&device, &state(false, unprovisioned_config()), &app_config(), &Client::new()).await;

        assert!(matches!(result, Ok(Provisioning::Pending)), "got {result:?}");
        set_config.assert_async().await;
        reboot.assert_async().await;
    }

    #[tokio::test]
    async fn provision_does_not_reboot_when_no_restart_is_required() {
        let mut server = Server::new_async().await;
        let set_config = set_config_mock(&mut server, false).create_async().await;
        let reboot = server.mock("POST", "/rpc/Shelly.Reboot").expect(0).create_async().await;
        let device = device(server.socket_address(), &[]);
        let no_status_notifications = with(provisioned_config(), "status_ntf", json!(false));

        let result = provision(&device, &state(false, no_status_notifications), &app_config(), &Client::new()).await;

        assert!(matches!(result, Ok(Provisioning::Pending)), "got {result:?}");
        set_config.assert_async().await;
        reboot.assert_async().await;
    }

    #[tokio::test]
    async fn provision_fails_when_the_device_rejects_the_configuration() {
        let mut server = Server::new_async().await;
        server.mock("POST", "/rpc/MQTT.SetConfig").with_status(401).create_async().await;
        let reboot = server.mock("POST", "/rpc/Shelly.Reboot").expect(0).create_async().await;
        let device = device(server.socket_address(), &[]);

        let result = provision(&device, &state(false, unprovisioned_config()), &app_config(), &Client::new()).await;

        assert!(matches!(result, Err(FetchError::UnexpectedResponse(StatusCode::UNAUTHORIZED, _))), "got {result:?}");
        reboot.assert_async().await;
    }

    #[tokio::test]
    async fn provision_fails_when_the_reboot_fails() {
        let mut server = Server::new_async().await;
        set_config_mock(&mut server, true).create_async().await;
        server.mock("POST", "/rpc/Shelly.Reboot").with_status(500).create_async().await;
        let device = device(server.socket_address(), &[]);

        let result = provision(&device, &state(false, unprovisioned_config()), &app_config(), &Client::new()).await;

        assert!(
            matches!(result, Err(FetchError::UnexpectedResponse(StatusCode::INTERNAL_SERVER_ERROR, _))),
            "got {result:?}"
        );
    }

    #[tokio::test]
    async fn fetch_mqtt_state_returns_the_current_config_and_status() {
        let mut server = Server::new_async().await;
        let mock = mqtt_state_mock(&mut server, true, provisioned_config()).create_async().await;

        let fetched = fetch_mqtt_state(&Client::new(), &server.socket_address()).await.unwrap();

        assert_eq!(fetched, Some(state(true, provisioned_config())));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn fetch_mqtt_state_returns_none_without_an_mqtt_component() {
        let mut server = Server::new_async().await;
        server
            .mock("GET", "/rpc/Shelly.GetComponents")
            .match_query(Matcher::Any)
            .with_body(r#"{"components":[]}"#)
            .create_async()
            .await;

        let fetched = fetch_mqtt_state(&Client::new(), &server.socket_address()).await.unwrap();

        assert_eq!(fetched, None);
    }

    #[tokio::test]
    async fn fetch_mqtt_state_fails_on_an_unexpected_status() {
        let mut server = Server::new_async().await;
        server.mock("GET", "/rpc/Shelly.GetComponents").match_query(Matcher::Any).with_status(500).create_async().await;

        let result = fetch_mqtt_state(&Client::new(), &server.socket_address()).await;

        assert!(
            matches!(result, Err(FetchError::UnexpectedResponse(StatusCode::INTERNAL_SERVER_ERROR, _))),
            "got {result:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn provision_with_retry_leaves_a_device_without_mqtt_alone() {
        let mut server = Server::new_async().await;
        let any_get = server.mock("GET", Matcher::Any).expect(0).create_async().await;
        let any_post = server.mock("POST", Matcher::Any).expect(0).create_async().await;
        let device = device(server.socket_address(), &[ble_component()]);

        provision_with_retry(&device, &app_config(), &Client::new()).await;

        any_get.assert_async().await;
        any_post.assert_async().await;
    }

    #[tokio::test(start_paused = true)]
    async fn provision_with_retry_stops_at_once_for_a_connected_device() {
        let mut server = Server::new_async().await;
        let any_get = server.mock("GET", Matcher::Any).expect(0).create_async().await;
        let any_post = server.mock("POST", Matcher::Any).expect(0).create_async().await;
        let device = device(server.socket_address(), &[mqtt_component(true, provisioned_config())]);
        let start = Instant::now();

        provision_with_retry(&device, &app_config(), &Client::new()).await;

        assert_eq!(start.elapsed(), Duration::ZERO);
        any_get.assert_async().await;
        any_post.assert_async().await;
    }

    #[tokio::test(start_paused = true)]
    async fn provision_with_retry_stops_once_the_device_is_connected() {
        let mut server = Server::new_async().await;
        let set_config = set_config_mock(&mut server, true).create_async().await;
        let reboot = server.mock("POST", "/rpc/Shelly.Reboot").with_body("null").create_async().await;
        let refresh = mqtt_state_mock(&mut server, true, provisioned_config()).create_async().await;
        let device = device(server.socket_address(), &[mqtt_component(false, unprovisioned_config())]);
        let start = Instant::now();

        provision_with_retry(&device, &app_config(), &Client::new()).await;

        assert_eq!(start.elapsed(), INITIAL_DELAY);
        set_config.assert_async().await;
        reboot.assert_async().await;
        refresh.assert_async().await;
    }

    #[tokio::test(start_paused = true)]
    async fn provision_with_retry_provisions_again_while_the_settings_differ() {
        let mut server = Server::new_async().await;
        let set_config = set_config_mock(&mut server, false).expect(2).create_async().await;
        let still_unprovisioned =
            mqtt_state_mock(&mut server, false, unprovisioned_config()).expect(1).create_async().await;
        let connected = mqtt_state_mock(&mut server, true, provisioned_config()).create_async().await;
        let device = device(server.socket_address(), &[mqtt_component(false, unprovisioned_config())]);

        provision_with_retry(&device, &app_config(), &Client::new()).await;

        set_config.assert_async().await;
        still_unprovisioned.assert_async().await;
        connected.assert_async().await;
    }

    #[tokio::test(start_paused = true)]
    async fn provision_with_retry_waits_for_a_provisioned_device_to_connect() {
        let mut server = Server::new_async().await;
        let set_config = server.mock("POST", "/rpc/MQTT.SetConfig").expect(0).create_async().await;
        let disconnected = mqtt_state_mock(&mut server, false, provisioned_config()).expect(1).create_async().await;
        let connected = mqtt_state_mock(&mut server, true, provisioned_config()).create_async().await;
        let device = device(server.socket_address(), &[mqtt_component(false, provisioned_config())]);

        provision_with_retry(&device, &app_config(), &Client::new()).await;

        set_config.assert_async().await;
        disconnected.assert_async().await;
        connected.assert_async().await;
    }

    #[tokio::test(start_paused = true)]
    async fn provision_with_retry_retries_after_a_failed_provisioning() {
        let mut server = Server::new_async().await;
        let failure = server.mock("POST", "/rpc/MQTT.SetConfig").with_status(500).expect(1).create_async().await;
        let success = set_config_mock(&mut server, false).create_async().await;
        let still_unprovisioned =
            mqtt_state_mock(&mut server, false, unprovisioned_config()).expect(1).create_async().await;
        let connected = mqtt_state_mock(&mut server, true, provisioned_config()).create_async().await;
        let device = device(server.socket_address(), &[mqtt_component(false, unprovisioned_config())]);

        provision_with_retry(&device, &app_config(), &Client::new()).await;

        failure.assert_async().await;
        success.assert_async().await;
        still_unprovisioned.assert_async().await;
        connected.assert_async().await;
    }

    // A failed refresh keeps the last known state, so the device is provisioned again
    #[tokio::test(start_paused = true)]
    async fn provision_with_retry_keeps_the_last_state_when_the_refresh_fails() {
        let mut server = Server::new_async().await;
        let set_config = set_config_mock(&mut server, false).expect(2).create_async().await;
        let failed_refresh = server
            .mock("GET", "/rpc/Shelly.GetComponents")
            .match_query(Matcher::Any)
            .with_status(500)
            .expect(1)
            .create_async()
            .await;
        let connected = mqtt_state_mock(&mut server, true, provisioned_config()).create_async().await;
        let device = device(server.socket_address(), &[mqtt_component(false, unprovisioned_config())]);

        provision_with_retry(&device, &app_config(), &Client::new()).await;

        set_config.assert_async().await;
        failed_refresh.assert_async().await;
        connected.assert_async().await;
    }

    #[tokio::test(start_paused = true)]
    async fn provision_with_retry_doubles_the_delay_up_to_the_maximum() {
        let mut server = Server::new_async().await;
        set_config_mock(&mut server, false).create_async().await;
        let disconnected = mqtt_state_mock(&mut server, false, provisioned_config()).expect(6).create_async().await;
        let connected = mqtt_state_mock(&mut server, true, provisioned_config()).create_async().await;
        let device = device(server.socket_address(), &[mqtt_component(false, unprovisioned_config())]);
        let start = Instant::now();

        provision_with_retry(&device, &app_config(), &Client::new()).await;

        // 30s, 1m, 2m, 4m, 8m, then capped at 15m for the last two rounds
        let expected = Duration::from_secs(30 + 60 + 2 * 60 + 4 * 60 + 8 * 60) + 2 * MAX_DELAY;
        assert_eq!(start.elapsed(), expected);
        disconnected.assert_async().await;
        connected.assert_async().await;
    }
}
