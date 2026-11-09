use crate::app_config::{AppConfig, Mqtt as MqttAppConfig};
use crate::mqtt_security::{MqttSecurity, SecurityError};
use crate::shelly::domain::{Component, ComponentsResult, DeviceInfo, MqttConfig, MqttStatus};
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
use thiserror::Error;
use tokio::sync::watch::Receiver as WatchReceiver;
use tokio::task;
use tracing::{debug, info, warn};
use uuid::Uuid;

const INITIAL_DELAY: Duration = Duration::from_secs(30);
const MAX_DELAY: Duration = Duration::from_secs(15 * 60);
// Rounds a provisioned device may stay disconnected before it gets new credentials, in case the broker lost its account
const PENDING_ROUNDS_BEFORE_REPROVISIONING: u32 = 3;

pub fn provision_announced_devices(
    client: &Client,
    config: Arc<AppConfig>,
    mqtt_security: Arc<MqttSecurity>,
    mut probed_rx: WatchReceiver<ProbedDeviceMap>,
) {
    let client = client.clone();
    task::spawn(async move {
        let mut seen = HashSet::new();
        loop {
            let probed_devices = take_new(&probed_rx.borrow_and_update(), &mut seen);
            for device in probed_devices {
                let probed_rx = probed_rx.clone();
                let client = client.clone();
                let config = Arc::clone(&config);
                let mqtt_security = Arc::clone(&mqtt_security);
                task::spawn(async move {
                    provision_with_retry(device, &config, &client, &mqtt_security, &probed_rx).await;
                });
            }

            if probed_rx.changed().await.is_err() {
                break; // Prober stopped
            }
        }
    });
}

async fn provision_with_retry(
    mut device: Arc<ProbedDevice>,
    config: &AppConfig,
    client: &Client,
    mqtt_security: &MqttSecurity,
    probed_rx: &WatchReceiver<ProbedDeviceMap>,
) {
    let Some(mut state) = mqtt_state(&device.components) else {
        debug!(device_id = device.info.id, "MQTT is not supported for this device");
        return;
    };

    let mut delay = INITIAL_DELAY;
    let mut pending_rounds = 0;
    loop {
        let force = pending_rounds >= PENDING_ROUNDS_BEFORE_REPROVISIONING;
        match provision(&device, &state, config, client, mqtt_security, force).await {
            Ok(Provisioning::Connected) => return,
            Ok(Provisioning::Pending) if force => pending_rounds = 0,
            Ok(Provisioning::Pending) => pending_rounds += 1,
            Err(err) => {
                warn!(
                    device_id = device.info.id,
                    "⚠️ Provisioning '{}' failed, retrying in {delay:?}: {err}", device.info.id
                );
            }
        }

        (device, state) = refresh_until_verified(&device, &mut delay, client, probed_rx).await;
    }
}

// Waits until the device reports its MQTT state at its current address. A failed or unverified refresh never leads
// to provisioning, because a stale address may belong to another device.
async fn refresh_until_verified(
    device: &Arc<ProbedDevice>,
    delay: &mut Duration,
    client: &Client,
    probed_rx: &WatchReceiver<ProbedDeviceMap>,
) -> (Arc<ProbedDevice>, MqttState) {
    loop {
        tokio::time::sleep(*delay).await;
        *delay = (*delay * 2).min(MAX_DELAY);

        // The prober may have found a new address in the meantime
        let current = probed_rx.borrow().get(&device.info.id).cloned().unwrap_or_else(|| Arc::clone(device));
        match fetch_mqtt_state(client, &current.address, &current.info.id).await {
            Ok(Some(state)) => return (current, state),
            Ok(None) => debug!(device_id = device.info.id, "MQTT component is gone, skipping provisioning"),
            Err(err) => warn!(
                device_id = device.info.id,
                "⚠️ Refreshing '{}' failed, skipping provisioning: {err}", device.info.id
            ),
        }
    }
}

#[derive(Error, Debug)]
enum ProvisionError {
    #[error(transparent)]
    Fetch(#[from] FetchError),
    #[error(transparent)]
    Security(#[from] SecurityError),
}

#[derive(Error, Debug)]
enum RefreshError {
    #[error(transparent)]
    Fetch(#[from] FetchError),
    #[error("{address} is device '{found}', expected '{expected}'")]
    OtherDevice { address: SocketAddr, expected: String, found: String },
}

async fn fetch_mqtt_state(
    client: &Client,
    address: &SocketAddr,
    expected_id: &str,
) -> Result<Option<MqttState>, RefreshError> {
    let info = fetch::<DeviceInfo>(client, address, "Shelly.GetDeviceInfo", &[]).await?;
    if info.id != expected_id {
        return Err(RefreshError::OtherDevice { address: *address, expected: expected_id.to_string(), found: info.id });
    }

    let components = fetch::<ComponentsResult>(
        client,
        address,
        "Shelly.GetComponents",
        &[("keys", r#"["mqtt"]"#), ("include", r#"["config","status"]"#)],
    )
    .await?;

    Ok(mqtt_state(&components.components))
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
    mqtt_security: &MqttSecurity,
    force: bool,
) -> Result<Provisioning, ProvisionError> {
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

        if !force {
            return Ok(Provisioning::Pending);
        }
    }

    info!(device_id = device.info.id, "⚙️ Provisioning Shelly device '{}'...", device.info.id);
    let password = Uuid::new_v4().simple().to_string();
    mqtt_security
        .ensure_account(&format!("shelly-{}", &device.info.id), &device.info.id, &device.info.id, &password)
        .await?;

    let request = SetConfigRequest { config: SetConfig { settings: &desired, pass: &password } };
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
            user: Some(device_id.to_string()),
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
    use crate::mqtt::MqttService;
    use crate::shelly::test_support::{PLUS_1_DEVICE_INFO, PLUS_1_ID, ble_component, mqtt_component, probed_device};
    use mockito::{Matcher, Mock, Server, ServerGuard};
    use pretty_assertions::assert_eq;
    use reqwest::StatusCode;
    use rstest::rstest;
    use serde_json::{Value, json};
    use std::net::SocketAddr;
    use tokio::sync::watch;
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
            "user": PLUS_1_ID,
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

    // Returns the mock unregistered, so the caller can set expectations before `create_async`
    fn device_info_mock(server: &mut ServerGuard) -> Mock {
        server.mock("GET", "/rpc/Shelly.GetDeviceInfo").with_body(PLUS_1_DEVICE_INFO)
    }

    // Not connected: the tests using it must not reach the broker
    fn mqtt_security() -> MqttSecurity {
        MqttSecurity::new(MqttService::unconnected()).expect("subscribing is queued, not sent")
    }

    async fn provision_device(
        device: &ProbedDevice,
        state: &MqttState,
        force: bool,
    ) -> Result<Provisioning, ProvisionError> {
        provision(device, state, &app_config(), &Client::new(), &mqtt_security(), force).await
    }

    async fn provision_with_retry_for(device: ProbedDevice) {
        let (_tx, probed_rx) = watch::channel(ProbedDeviceMap::default());
        provision_with_retry(Arc::new(device), &app_config(), &Client::new(), &mqtt_security(), &probed_rx).await;
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
            user: Some(PLUS_1_ID.to_string()),
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
            "user": PLUS_1_ID,
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

        let result = provision_device(&device, &state(true, provisioned_config()), false).await;

        assert!(matches!(result, Ok(Provisioning::Connected)), "got {result:?}");
        set_config.assert_async().await;
        reboot.assert_async().await;
    }

    #[tokio::test]
    async fn provision_leaves_a_provisioned_but_disconnected_device_pending() {
        let mut server = Server::new_async().await;
        let set_config = server.mock("POST", "/rpc/MQTT.SetConfig").expect(0).create_async().await;
        let reboot = server.mock("POST", "/rpc/Shelly.Reboot").expect(0).create_async().await;
        let device = device(server.socket_address(), &[]);

        let result = provision_device(&device, &state(false, provisioned_config()), false).await;

        assert!(matches!(result, Ok(Provisioning::Pending)), "got {result:?}");
        set_config.assert_async().await;
        reboot.assert_async().await;
    }

    #[tokio::test]
    async fn provision_forced_leaves_a_provisioned_and_connected_device_alone() {
        let mut server = Server::new_async().await;
        let set_config = server.mock("POST", "/rpc/MQTT.SetConfig").expect(0).create_async().await;
        let device = device(server.socket_address(), &[]);

        let result = provision_device(&device, &state(true, provisioned_config()), true).await;

        assert!(matches!(result, Ok(Provisioning::Connected)), "got {result:?}");
        set_config.assert_async().await;
    }

    #[tokio::test]
    async fn fetch_mqtt_state_returns_the_current_config_and_status() {
        let mut server = Server::new_async().await;
        let info = device_info_mock(&mut server).create_async().await;
        let mock = mqtt_state_mock(&mut server, true, provisioned_config()).create_async().await;

        let fetched = fetch_mqtt_state(&Client::new(), &server.socket_address(), PLUS_1_ID).await.unwrap();

        assert_eq!(fetched, Some(state(true, provisioned_config())));
        info.assert_async().await;
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn fetch_mqtt_state_returns_none_without_an_mqtt_component() {
        let mut server = Server::new_async().await;
        device_info_mock(&mut server).create_async().await;
        server
            .mock("GET", "/rpc/Shelly.GetComponents")
            .match_query(Matcher::Any)
            .with_body(r#"{"components":[]}"#)
            .create_async()
            .await;

        let fetched = fetch_mqtt_state(&Client::new(), &server.socket_address(), PLUS_1_ID).await.unwrap();

        assert_eq!(fetched, None);
    }

    #[tokio::test]
    async fn fetch_mqtt_state_fails_for_another_device() {
        let mut server = Server::new_async().await;
        device_info_mock(&mut server).create_async().await;
        let components =
            server.mock("GET", "/rpc/Shelly.GetComponents").match_query(Matcher::Any).expect(0).create_async().await;

        let result = fetch_mqtt_state(&Client::new(), &server.socket_address(), "shellyplus1-112233445566").await;

        assert!(
            matches!(result, Err(RefreshError::OtherDevice { ref expected, ref found, .. })
                if expected == "shellyplus1-112233445566" && found == PLUS_1_ID),
            "got {result:?}"
        );
        components.assert_async().await;
    }

    #[tokio::test]
    async fn fetch_mqtt_state_fails_when_the_device_info_cannot_be_fetched() {
        let mut server = Server::new_async().await;
        server.mock("GET", "/rpc/Shelly.GetDeviceInfo").with_status(500).create_async().await;

        let result = fetch_mqtt_state(&Client::new(), &server.socket_address(), PLUS_1_ID).await;

        assert!(
            matches!(
                result,
                Err(RefreshError::Fetch(FetchError::UnexpectedResponse(StatusCode::INTERNAL_SERVER_ERROR, _)))
            ),
            "got {result:?}"
        );
    }

    #[tokio::test]
    async fn fetch_mqtt_state_fails_on_an_unexpected_status() {
        let mut server = Server::new_async().await;
        device_info_mock(&mut server).create_async().await;
        server.mock("GET", "/rpc/Shelly.GetComponents").match_query(Matcher::Any).with_status(500).create_async().await;

        let result = fetch_mqtt_state(&Client::new(), &server.socket_address(), PLUS_1_ID).await;

        assert!(
            matches!(
                result,
                Err(RefreshError::Fetch(FetchError::UnexpectedResponse(StatusCode::INTERNAL_SERVER_ERROR, _)))
            ),
            "got {result:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn provision_with_retry_leaves_a_device_without_mqtt_alone() {
        let mut server = Server::new_async().await;
        let any_get = server.mock("GET", Matcher::Any).expect(0).create_async().await;
        let any_post = server.mock("POST", Matcher::Any).expect(0).create_async().await;
        let device = device(server.socket_address(), &[ble_component()]);

        provision_with_retry_for(device).await;

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

        provision_with_retry_for(device).await;

        assert_eq!(start.elapsed(), Duration::ZERO);
        any_get.assert_async().await;
        any_post.assert_async().await;
    }

    #[tokio::test(start_paused = true)]
    async fn provision_with_retry_waits_for_a_provisioned_device_to_connect() {
        let mut server = Server::new_async().await;
        let set_config = server.mock("POST", "/rpc/MQTT.SetConfig").expect(0).create_async().await;
        device_info_mock(&mut server).create_async().await;
        let disconnected = mqtt_state_mock(&mut server, false, provisioned_config()).expect(1).create_async().await;
        let connected = mqtt_state_mock(&mut server, true, provisioned_config()).create_async().await;
        let device = device(server.socket_address(), &[mqtt_component(false, provisioned_config())]);

        provision_with_retry_for(device).await;

        set_config.assert_async().await;
        disconnected.assert_async().await;
        connected.assert_async().await;
    }
}
