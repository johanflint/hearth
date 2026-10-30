use crate::app_config::AppConfig;
use crate::domain::property::{
    BooleanProperty, ColorProperty, NumberProperty, Property, PropertyError, PropertyType, ValidatedValue,
};
use crate::domain::{Color, Command, Controller, Device, DeviceType, Group};
use crate::domain::{Number, Value};
use crate::extensions::unsigned_ints_ext::MirekConversions;
use crate::flow_engine::property_command::ResolvedPropertyCommand;
use crate::hue::clip_to_gamut::clip_to_gamut;
use crate::hue::domain::{LightRequest, MotionRequest, On, SetSensitivity};
use crate::hue::pending_writes::PendingWrites;
use crate::metrics::Metric;
use async_trait::async_trait;
use humantime_serde::re::humantime::format_duration;
use metrics::counter;
use reqwest::Client;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{info, instrument, warn};

#[derive(Debug)]
pub struct HueController {
    client: Client,
    pending_writes: Arc<PendingWrites>,
    config: Arc<AppConfig>,
}

pub const CONTROLLER_ID: &str = "hue";

const MAX_TRANSITION: Duration = Duration::from_secs(3600);

#[async_trait]
impl Controller for HueController {
    fn id(&self) -> &'static str {
        CONTROLLER_ID
    }

    #[instrument(skip_all)]
    async fn execute(&self, command: Command) {
        match command {
            Command::ControlDevice { device, property } => match device.r#type {
                DeviceType::Light => self.control_device_light(device, property).await,
                DeviceType::MotionSensor => self.control_device_motion_sensor(device, property).await,
                DeviceType::Remote => {}
                DeviceType::ContactSensor => {}
            },
            Command::ControlGroup { group, devices, property } => self.control_group(&group, &devices, &property).await,
            Command::ControlZone { .. } => {}
        }
    }
}

impl HueController {
    pub fn new(client: Client, pending_writes: Arc<PendingWrites>, config: Arc<AppConfig>) -> Self {
        HueController { client, pending_writes, config }
    }

    fn light_request(
        device: &Device,
        property: &HashMap<String, ResolvedPropertyCommand>,
    ) -> Option<(String, LightRequest)> {
        let Some(on_property) = device.get_property_of_type::<BooleanProperty>(PropertyType::On) else {
            warn!(device_id = device.id, "⚠️ Light has no on property");
            return None;
        };

        let Some(light_id) = on_property.external_id() else {
            warn!(device_id = device.id, "⚠️ Light on property has no Hue resource id");
            return None;
        };

        let transition = property
            .values()
            .filter_map(|pc| pc.transition)
            .max()
            .map(|transition| clamp_transition(&device.id, transition));

        let on = property.get(on_property.name()).and_then(|pc| match pc.value {
            Value::Boolean(value) => Some(On { on: value }),
            _ => None,
        });

        if let Some(value) = &on {
            let on_text = if value.on { "on" } else { "off" };
            let transition_text = transition.map_or("".to_string(), |t| format!(" over {}", format_duration(t)));
            info!(device_id = device.id, "🟢 Turn {} light '{}'{}", on_text, device.name, transition_text);
        }

        let brightness = device.get_property_of_type::<NumberProperty>(PropertyType::Brightness).and_then(|brightness_property| {
            property
                .get(brightness_property.name())
                .and_then(|pc| match pc.value {
                    Value::Number(value) => value.as_f64(),
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
                    .and_then(|pc| match pc.value {
                        Value::Number(value) => Some(value),
                        _ => None,
                    })
                    .and_then(|color_temperature| match color_temperature_property.validate_value(color_temperature) {
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
            property.get(color_property.name()).and_then(|pc| match &pc.value {
                Value::Color(color) => match color.clone().to_cie_xyY() {
                    Ok(Color::CIE_xyY { xy, brightness: _ }) => {
                        color_property.gamut().map(|gamut| clip_to_gamut(xy.clone(), gamut)).or(Some(xy))
                    }
                    Err(error) => {
                        warn!("🌈 Color value is invalid: {}", error);
                        None
                    }
                    _ => None,
                },
                _ => None,
            })
        });

        Some((light_id.to_string(), LightRequest::new(on, brightness, color_temperature, color, transition)))
    }

    async fn control_device_light(&self, device: Arc<Device>, property: Arc<HashMap<String, ResolvedPropertyCommand>>) {
        let Some((light_id, request)) = Self::light_request(&device, &property) else {
            return;
        };
        self.send_light_request(&device, &light_id, &request).await;
    }

    async fn control_group(
        &self,
        group: &Group,
        devices: &[Arc<Device>],
        property: &HashMap<String, ResolvedPropertyCommand>,
    ) {
        // A light that can't be controlled is skipped, the same as when it is controlled on its own
        let mut light_writes = Vec::new();
        for device in devices {
            if let Some((light_id, request)) = Self::light_request(device, property) {
                light_writes.push(LightWrite { device: device.clone(), light_id, request });
            }
        }

        let Some(first) = light_writes.first() else { return };

        let Some(grouped_light_id) = &group.external_id else {
            return self.control_lights_separately(&light_writes).await;
        };

        // The group would also control the lights that were skipped
        if light_writes.len() != devices.len() {
            return self.control_lights_separately(&light_writes).await;
        }

        // Clamping and gamut are per light, so the requests can differ even though the properties are the same
        if light_writes.iter().any(|light_write| light_write.request != first.request) {
            return self.control_lights_separately(&light_writes).await;
        }

        let now = Instant::now();
        for light_write in &light_writes {
            self.pending_writes.record(&light_write.light_id, &first.request, now);
        }

        info!(group_id = group.id, "🏠 Control group '{}' with {} lights", group.name, devices.len());
        let url = format!("{}/clip/v2/resource/grouped_light/{}", self.config.hue().url(), grouped_light_id);
        self.send_request(&first.request, &url, &first.device, "group").await;
    }

    async fn control_lights_separately(&self, light_writes: &[LightWrite]) {
        for light_write in light_writes {
            self.send_light_request(&light_write.device, &light_write.light_id, &light_write.request).await;
        }
    }

    async fn send_light_request(&self, device: &Device, light_id: &str, request: &LightRequest) {
        self.pending_writes.record(light_id, request, Instant::now());

        let url = format!("{}/clip/v2/resource/light/{}", self.config.hue().url(), light_id);
        self.send_request(request, &url, device, "light").await;
    }

    async fn control_device_motion_sensor(
        &self,
        device: Arc<Device>,
        property: Arc<HashMap<String, ResolvedPropertyCommand>>,
    ) {
        let Some(motion_property) = device.get_property_of_type::<BooleanProperty>(PropertyType::Motion) else {
            warn!(device_id = device.id, "⚠️ Motion sensor has no motion property");
            return;
        };

        let Some(motion_sensor_id) = motion_property.external_id() else {
            warn!(device_id = device.id, "⚠️ Motion sensor motion property has no Hue resource id");
            return;
        };

        let enabled_property =
            device.get_property_of_type::<BooleanProperty>(PropertyType::Enabled).and_then(|enabled_property| {
                property.get(enabled_property.name()).and_then(|pc| match pc.value {
                    Value::Boolean(value) => Some(value),
                    _ => None,
                })
            });

        let sensitivity = device.get_property_of_type::<NumberProperty>(PropertyType::MotionSensitivity).and_then(|sensitivity_property| {
            property
                .get(sensitivity_property.name())
                .and_then(|pc| match pc.value {
                    Value::Number(value) => Some(value),
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
        let request_result = self.client.put(url).json(request).send().await;

        let (result, status) = match &request_result {
            Err(_) => ("failure", "n/a".to_string()),
            Ok(response) if response.status().is_success() => ("success", "n/a".to_string()),
            Ok(response) => ("failure", response.status().as_u16().to_string()),
        };
        counter!(Metric::DeviceCommandDispatches.name(), "system" => "hue", "result" => result, "status" => status)
            .increment(1);

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

fn clamp_transition(device_id: &str, transition: Duration) -> Duration {
    if transition > MAX_TRANSITION {
        warn!(
            "⚠️ Transition of '{}' is too long for device '{}', clamped to the maximum value of '{}'",
            format_duration(transition),
            device_id,
            format_duration(MAX_TRANSITION)
        );
        return MAX_TRANSITION;
    }

    transition
}

// The request for one light of a group, so it is only built once
struct LightWrite {
    device: Arc<Device>,
    light_id: String,
    request: LightRequest,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_config::AppConfigBuilder;
    use crate::domain::GroupOwner;
    use crate::domain::property::{CartesianCoordinate, Gamut};
    use crate::hue::client::new_client;
    use crate::hue::domain::LightChanged;
    use crate::test_support::DeviceBuilder;
    use mockito::{Matcher, Mock, ServerGuard};
    use rstest::rstest;
    use serde_json::json;
    use std::fmt::Debug;
    use std::sync::Mutex;
    use tracing::Subscriber;
    use tracing::field::{Field, Visit};
    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::{Context, SubscriberExt};

    const GROUPED_LIGHT_ID: &str = "grouped-light";

    #[tokio::test]
    async fn control_group_sends_one_group_request_when_every_light_gets_the_same_request() {
        let mut server = mockito::Server::new_async().await;
        let group_mock = turn_on_mock(&mut server, "grouped_light", GROUPED_LIGHT_ID, 1).await;
        let light1_mock = turn_on_mock(&mut server, "light", "light1", 0).await;
        let light2_mock = turn_on_mock(&mut server, "light", "light2", 0).await;
        let (controller, pending_writes) = controller(&server);
        let devices = vec![light("device1", Some("light1"), None), light("device2", Some("light2"), None)];
        let logs = LogMessages::default();

        {
            let _guard = logs.capture();
            controller.execute(control_group(Some(GROUPED_LIGHT_ID), devices, turn_on())).await;
        }

        group_mock.assert_async().await;
        light1_mock.assert_async().await;
        light2_mock.assert_async().await;
        assert_eq!(logs.count("🟢 Turn on light"), 2);
        assert_eq!(logs.count("🏠 Control group"), 1);
        assert_eq!(logs.count("Unable to control"), 0);
        assert_recorded_once(&pending_writes, "light1");
        assert_recorded_once(&pending_writes, "light2");
    }

    #[tokio::test]
    async fn control_group_sends_a_request_per_light_when_the_group_has_no_grouped_light() {
        let mut server = mockito::Server::new_async().await;
        let light1_mock = turn_on_mock(&mut server, "light", "light1", 1).await;
        let light2_mock = turn_on_mock(&mut server, "light", "light2", 1).await;
        let (controller, pending_writes) = controller(&server);
        let devices = vec![light("device1", Some("light1"), None), light("device2", Some("light2"), None)];
        let logs = LogMessages::default();

        {
            let _guard = logs.capture();
            controller.execute(control_group(None, devices, turn_on())).await;
        }

        light1_mock.assert_async().await;
        light2_mock.assert_async().await;
        assert_eq!(logs.count("🟢 Turn on light"), 2);
        assert_eq!(logs.count("🏠 Control group"), 0);
        assert_eq!(logs.count("Unable to control"), 0);
        assert_recorded_once(&pending_writes, "light1");
        assert_recorded_once(&pending_writes, "light2");
    }

    #[tokio::test]
    async fn control_group_sends_a_request_per_light_when_the_requests_differ_per_light() {
        let mut server = mockito::Server::new_async().await;
        let group_mock = turn_on_mock(&mut server, "grouped_light", GROUPED_LIGHT_ID, 0).await;
        let light1_mock = turn_on_mock(&mut server, "light", "light1", 1).await;
        let light2_mock = turn_on_mock(&mut server, "light", "light2", 1).await;
        let (controller, pending_writes) = controller(&server);
        // The color is outside both gamuts, so it is clipped to a different color per light
        let devices =
            vec![light("device1", Some("light1"), Some(gamut_a())), light("device2", Some("light2"), Some(gamut_c()))];
        let mut property = turn_on();
        let color = Color::CIE_xyY { xy: CartesianCoordinate::new(0.1, 0.9), brightness: 1.0 };
        property.insert("color".to_string(), ResolvedPropertyCommand { value: Value::Color(color), transition: None });
        let logs = LogMessages::default();

        {
            let _guard = logs.capture();
            controller.execute(control_group(Some(GROUPED_LIGHT_ID), devices, property)).await;
        }

        group_mock.assert_async().await;
        light1_mock.assert_async().await;
        light2_mock.assert_async().await;
        assert_eq!(logs.count("🟢 Turn on light"), 2);
        assert_eq!(logs.count("🏠 Control group"), 0);
        assert_eq!(logs.count("Unable to control"), 0);
        assert_recorded_once(&pending_writes, "light1");
        assert_recorded_once(&pending_writes, "light2");
    }

    #[tokio::test]
    async fn control_group_sends_a_request_per_light_when_a_light_cannot_be_controlled() {
        let mut server = mockito::Server::new_async().await;
        let group_mock = turn_on_mock(&mut server, "grouped_light", GROUPED_LIGHT_ID, 0).await;
        let light1_mock = turn_on_mock(&mut server, "light", "light1", 1).await;
        let (controller, pending_writes) = controller(&server);
        let devices = vec![light("device1", Some("light1"), None), light("device2", None, None)];
        let logs = LogMessages::default();

        {
            let _guard = logs.capture();
            controller.execute(control_group(Some(GROUPED_LIGHT_ID), devices, turn_on())).await;
        }

        group_mock.assert_async().await;
        light1_mock.assert_async().await;
        assert_eq!(logs.count("🟢 Turn on light"), 1);
        assert_eq!(logs.count("⚠️ Light on property has no Hue resource id"), 1);
        assert_eq!(logs.count("🏠 Control group"), 0);
        assert_eq!(logs.count("Unable to control"), 0);
        assert_recorded_once(&pending_writes, "light1");
    }

    #[tokio::test]
    async fn control_group_sends_nothing_when_no_light_can_be_controlled() {
        let mut server = mockito::Server::new_async().await;
        let group_mock = turn_on_mock(&mut server, "grouped_light", GROUPED_LIGHT_ID, 0).await;
        let (controller, _) = controller(&server);
        let devices = vec![light("device1", None, None), light("device2", None, None)];
        let logs = LogMessages::default();

        {
            let _guard = logs.capture();
            controller.execute(control_group(Some(GROUPED_LIGHT_ID), devices, turn_on())).await;
        }

        group_mock.assert_async().await;
        assert_eq!(logs.count("⚠️ Light on property has no Hue resource id"), 2);
        assert_eq!(logs.count("Unable to control"), 0);
    }

    async fn turn_on_mock(server: &mut ServerGuard, resource: &str, id: &str, expected_requests: usize) -> Mock {
        server
            .mock("PUT", format!("/clip/v2/resource/{resource}/{id}").as_str())
            .match_body(Matcher::PartialJson(json!({ "on": { "on": true } })))
            .with_status(200)
            .expect(expected_requests)
            .create_async()
            .await
    }

    fn controller(server: &ServerGuard) -> (HueController, Arc<PendingWrites>) {
        let config = AppConfigBuilder::new().hue_url(server.url()).build();
        let client = new_client(&config).unwrap();
        let pending_writes = Arc::new(PendingWrites::default());
        (HueController::new(client, pending_writes.clone(), Arc::new(config)), pending_writes)
    }

    fn light(device_id: &str, light_id: Option<&str>, gamut: Option<Gamut>) -> Arc<Device> {
        let on = BooleanProperty::new("on".to_string(), PropertyType::On, false, light_id.map(str::to_string), false);
        let xy = CartesianCoordinate::new(0.3, 0.3);
        let color = ColorProperty::new("color".to_string(), PropertyType::Color, false, None, xy, gamut);
        Arc::new(DeviceBuilder::new(device_id).with_properties(vec![Box::new(on), Box::new(color)]).build())
    }

    fn gamut_a() -> Gamut {
        Gamut::new(
            CartesianCoordinate::new(0.704, 0.296),
            CartesianCoordinate::new(0.2151, 0.7106),
            CartesianCoordinate::new(0.138, 0.08),
        )
    }

    fn gamut_c() -> Gamut {
        Gamut::new(
            CartesianCoordinate::new(0.6915, 0.3083),
            CartesianCoordinate::new(0.17, 0.7),
            CartesianCoordinate::new(0.1532, 0.0475),
        )
    }

    fn turn_on() -> HashMap<String, ResolvedPropertyCommand> {
        HashMap::from([("on".to_string(), ResolvedPropertyCommand { value: Value::Boolean(true), transition: None })])
    }

    fn control_group(
        external_id: Option<&str>,
        devices: Vec<Arc<Device>>,
        property: HashMap<String, ResolvedPropertyCommand>,
    ) -> Command {
        let group = Group {
            id: "room".to_string(),
            name: "Room".to_string(),
            device_ids: devices.iter().map(|device| device.id.clone()).collect(),
            revision: 1,
            owner: GroupOwner::Integration,
            external_id: external_id.map(str::to_string),
        };
        Command::ControlGroup { group: Arc::new(group), devices, property: Arc::new(property) }
    }

    // A pending write is consumed when it is confirmed, so a second confirm only succeeds if it was recorded twice
    fn assert_recorded_once(pending_writes: &PendingWrites, light_id: &str) {
        let change: LightChanged = serde_json::from_value(json!({
            "id": light_id,
            "owner": { "rid": "device", "rtype": "device" },
            "on": { "on": true },
        }))
        .unwrap();

        assert!(pending_writes.confirm(&change, Instant::now()), "no pending write for '{light_id}'");
        assert!(!pending_writes.confirm(&change, Instant::now()), "more than one pending write for '{light_id}'");
    }

    // Collects the log messages on the current thread, so a test can check that something was logged only once
    #[derive(Clone, Default)]
    struct LogMessages(Arc<Mutex<Vec<String>>>);

    impl LogMessages {
        fn capture(&self) -> tracing::subscriber::DefaultGuard {
            tracing::subscriber::set_default(tracing_subscriber::registry().with(self.clone()))
        }

        fn count(&self, text: &str) -> usize {
            self.0.lock().unwrap().iter().filter(|message| message.contains(text)).count()
        }
    }

    impl<S: Subscriber> Layer<S> for LogMessages {
        fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
            let mut visitor = MessageVisitor::default();
            event.record(&mut visitor);
            self.0.lock().unwrap().push(visitor.0);
        }
    }

    #[derive(Default)]
    struct MessageVisitor(String);

    impl Visit for MessageVisitor {
        fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
            if field.name() == "message" {
                self.0 = format!("{value:?}");
            }
        }
    }

    #[rstest]
    #[case::shorter_than_the_maximum(Duration::from_secs(600), Duration::from_secs(600))]
    #[case::equal_to_the_maximum(MAX_TRANSITION, MAX_TRANSITION)]
    #[case::longer_than_the_maximum(Duration::from_secs(3 * 60 * 60), MAX_TRANSITION)]
    fn clamp_transition_limits_the_transition_to_the_maximum(#[case] transition: Duration, #[case] expected: Duration) {
        assert_eq!(clamp_transition("device", transition), expected);
    }
}
