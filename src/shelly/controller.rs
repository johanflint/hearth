use crate::domain::property::{BooleanProperty, Property, PropertyType};
use crate::domain::{Command, Controller, Device, DeviceType, Value};
use crate::flow_engine::property_command::ResolvedPropertyCommand;
use crate::metrics::Metric;
use crate::shelly::prober::ProbedDeviceMap;
use async_trait::async_trait;
use metrics::counter;
use reqwest::Client;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::watch::Receiver as WatchReceiver;
use tracing::{info, instrument, warn};

pub const CONTROLLER_ID: &str = "shelly";

#[derive(Debug)]
pub struct ShellyController {
    client: Client,
    probed_rx: WatchReceiver<ProbedDeviceMap>,
}

#[async_trait]
impl Controller for ShellyController {
    fn id(&self) -> &'static str {
        CONTROLLER_ID
    }

    #[instrument(skip_all)]
    async fn execute(&self, command: Command) {
        match command {
            Command::ControlDevice { device, property } => {
                if device.r#type == DeviceType::Switch {
                    self.control_device_switch(device, property).await
                }
            }
            Command::ControlGroup { .. } => {} // Doesn't exist for Shelly's
            Command::ControlZone { .. } => {}  // Doesn't exist for Shelly's
        }
    }
}

impl ShellyController {
    pub fn new(client: Client, probed_rx: WatchReceiver<ProbedDeviceMap>) -> Self {
        Self { client, probed_rx }
    }

    async fn control_device_switch(
        &self,
        device: Arc<Device>,
        property: Arc<HashMap<String, ResolvedPropertyCommand>>,
    ) {
        let Some(probed_device) = self.probed_rx.borrow().get(&device.id).cloned() else {
            warn!(device_id = device.id, "⚠️ Device is not found");
            return;
        };

        let Some(on_property) = device.get_property_of_type::<BooleanProperty>(PropertyType::On) else {
            warn!(device_id = device.id, "⚠️ Switch has no on property");
            return;
        };

        let Some(switch_id) =
            on_property.external_id().and_then(|id| id.strip_prefix("switch:")).and_then(|id| id.parse::<u16>().ok())
        else {
            warn!(device_id = device.id, "⚠️ Switch on property has no component id");
            return;
        };

        let Some(on) = property.get(on_property.name()).and_then(|pc| match pc.value {
            Value::Boolean(value) => Some(value),
            _ => None,
        }) else {
            warn!(device_id = device.id, "⚠️ Switch on property is missing from the command");
            return;
        };

        let on_text = if on { "on" } else { "off" };
        info!(device_id = device.id, "🟢 Turn {} switch '{}'", on_text, device.name);

        let current_address = probed_device.address;
        let request_result = self
            .client
            .post(format!("http://{current_address}/rpc/Switch.Set"))
            .json(&SwitchSetRequest { id: switch_id, on })
            .send()
            .await;

        let response = match request_result {
            Ok(response) => response,
            Err(e) => {
                counter_command_dispatches("failure", "n/a".to_string());
                warn!(device_id = device.id, "⚠️ Unable to control the switch: {:?}", e);
                return;
            }
        };

        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if status.is_success() {
            counter_command_dispatches("success", "n/a".to_string());
            return;
        }

        counter_command_dispatches("failure", status.as_u16().to_string());
        warn!(device_id = device.id, status_code = %status, "⚠️ Unable to control the switch, request failed: {:?}", body);
    }
}

fn counter_command_dispatches(result: &'static str, status: String) {
    counter!(Metric::DeviceCommandDispatches.name(), "system" => "shelly", "result" => result, "status" => status)
        .increment(1);
}

#[derive(Debug, Serialize)]
struct SwitchSetRequest {
    id: u16,
    on: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shelly::map_shelly::map_shelly;
    use crate::shelly::test_support::{PLUS_1_ID, probed_device, switch_component};
    use mockito::{Matcher, Mock, ServerGuard};
    use serde_json::json;
    use tokio::sync::watch;

    #[tokio::test]
    async fn execute_turns_the_switch_on() {
        let mut server = mockito::Server::new_async().await;
        let mock = switch_set_mock(&mut server, Matcher::Json(json!({"id": 0, "on": true})), 1).await;
        let controller = controller(&server, 0);

        controller.execute(control_device(switch_device(0), "switch0", Value::Boolean(true))).await;

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn execute_turns_the_switch_off() {
        let mut server = mockito::Server::new_async().await;
        let mock = switch_set_mock(&mut server, Matcher::Json(json!({"id": 0, "on": false})), 1).await;
        let controller = controller(&server, 0);

        controller.execute(control_device(switch_device(0), "switch0", Value::Boolean(false))).await;

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn execute_uses_the_component_id_of_the_switch() {
        let mut server = mockito::Server::new_async().await;
        let mock = switch_set_mock(&mut server, Matcher::Json(json!({"id": 1, "on": true})), 1).await;
        let controller = controller(&server, 1);

        controller.execute(control_device(switch_device(1), "switch1", Value::Boolean(true))).await;

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn execute_survives_a_failing_request() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/rpc/Switch.Set")
            .with_status(500)
            .with_body(r#"{"code":-103,"message":"Invalid argument 'id'!"}"#)
            .expect(1)
            .create_async()
            .await;
        let controller = controller(&server, 0);

        controller.execute(control_device(switch_device(0), "switch0", Value::Boolean(true))).await;

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn execute_sends_no_request_for_a_device_that_was_not_probed() {
        let mut server = mockito::Server::new_async().await;
        let mock = switch_set_mock(&mut server, Matcher::Any, 0).await;
        let controller = ShellyController::new(Client::new(), watch::channel(Arc::new(HashMap::new())).1);

        controller.execute(control_device(switch_device(0), "switch0", Value::Boolean(true))).await;

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn execute_sends_no_request_for_a_device_that_is_not_a_switch() {
        let mut server = mockito::Server::new_async().await;
        let mock = switch_set_mock(&mut server, Matcher::Any, 0).await;
        let controller = controller(&server, 0);
        let mut device = Arc::unwrap_or_clone(switch_device(0));
        device.r#type = DeviceType::Light;

        controller.execute(control_device(Arc::new(device), "switch0", Value::Boolean(true))).await;

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn execute_sends_no_request_when_the_command_has_no_value_for_the_switch() {
        let mut server = mockito::Server::new_async().await;
        let mock = switch_set_mock(&mut server, Matcher::Any, 0).await;
        let controller = controller(&server, 0);

        controller.execute(control_device(switch_device(0), "unknown", Value::Boolean(true))).await;

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn execute_sends_no_request_when_the_value_is_not_a_boolean() {
        let mut server = mockito::Server::new_async().await;
        let mock = switch_set_mock(&mut server, Matcher::Any, 0).await;
        let controller = controller(&server, 0);

        controller.execute(control_device(switch_device(0), "switch0", Value::String("on".to_string()))).await;

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn execute_sends_no_request_when_the_switch_has_no_component_id() {
        let mut server = mockito::Server::new_async().await;
        let mock = switch_set_mock(&mut server, Matcher::Any, 0).await;
        let controller = controller(&server, 0);
        let mut device = Arc::unwrap_or_clone(switch_device(0));
        device.properties.remove("switch0");
        device.properties.insert(
            "switch0".to_string(),
            Box::new(BooleanProperty::new("switch0".to_string(), PropertyType::On, false, None, false)),
        );

        controller.execute(control_device(Arc::new(device), "switch0", Value::Boolean(true))).await;

        mock.assert_async().await;
    }

    async fn switch_set_mock(server: &mut ServerGuard, body: Matcher, expected_requests: usize) -> Mock {
        server
            .mock("POST", "/rpc/Switch.Set")
            .match_body(body)
            .with_body(r#"{"was_on":false}"#)
            .expect(expected_requests)
            .create_async()
            .await
    }

    /// A controller that knows the device at the mock server
    fn controller(server: &ServerGuard, switch_id: u32) -> ShellyController {
        let mut probed = probed_device(PLUS_1_ID, &[switch_component(switch_id, false)]);
        probed.address = server.host_with_port().parse().unwrap();
        let probed_devices = Arc::new(HashMap::from([(PLUS_1_ID.to_string(), Arc::new(probed))]));
        ShellyController::new(Client::new(), watch::channel(probed_devices).1)
    }

    fn switch_device(switch_id: u32) -> Arc<Device> {
        Arc::new(map_shelly(&probed_device(PLUS_1_ID, &[switch_component(switch_id, false)])).unwrap())
    }

    fn control_device(device: Arc<Device>, property: &str, value: Value) -> Command {
        let property = HashMap::from([(property.to_string(), ResolvedPropertyCommand { value, transition: None })]);
        Command::ControlDevice { device, property: Arc::new(property) }
    }
}
