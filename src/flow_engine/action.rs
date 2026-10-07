use crate::domain::{Problem, find_device, find_property};
use crate::flow_engine::action_registry::{ACTION_REGISTRY, known_actions};
use crate::flow_engine::context::Context;
use crate::flow_engine::property_value::PropertyCommand;
use crate::flow_engine::scope::Scope;
use crate::store::StoreSnapshot;
use action_macros::register_action;
use async_trait::async_trait;
use serde::{Deserialize, Deserializer};
use std::any::Any;
use std::collections::HashMap;
use std::fmt::Debug;
use tracing::{error, info, instrument, warn};

#[async_trait]
pub trait Action: Debug + Send + Sync {
    fn kind(&self) -> &'static str;

    async fn execute(&self, context: &Context, scope: &mut Scope);

    fn validate(&self, _snapshot: &StoreSnapshot) -> Vec<Problem> {
        vec![]
    }

    fn as_any(&self) -> &dyn Any;
}

impl<'de> Deserialize<'de> for Box<dyn Action> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value: serde_json::Value = Deserialize::deserialize(deserializer)?;
        let kind = value.get("type").and_then(|v| v.as_str()).ok_or_else(|| serde::de::Error::custom("missing field 'type'"))?;

        let registry = ACTION_REGISTRY.read().unwrap();
        if let Some(action) = registry.get(kind) {
            action(&value).map_err(serde::de::Error::custom)
        } else {
            Err(serde::de::Error::custom(format!(
                "unknown action type '{}', known types: {}",
                kind,
                known_actions().join(", ")
            )))
        }
    }
}

#[derive(Debug, Deserialize, Default, PartialEq)]
#[register_action]
pub struct LogAction {
    message: String,
}

#[cfg(test)]
impl LogAction {
    pub fn new(message: String) -> LogAction {
        LogAction { message }
    }
}

#[async_trait]
impl Action for LogAction {
    fn kind(&self) -> &'static str {
        "log"
    }

    #[instrument(fields(action = self.kind()), skip_all)]
    async fn execute(&self, _context: &Context, _scope: &mut Scope) {
        info!(target: "hearth::flow_log", "{}", self.message);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
#[register_action]
pub struct ControlDeviceAction {
    device_id: String,
    property: HashMap<String, PropertyCommand>,
}

#[cfg(test)]
impl ControlDeviceAction {
    pub fn new(device_id: String, property: HashMap<String, PropertyCommand>) -> ControlDeviceAction {
        ControlDeviceAction { device_id, property }
    }
}

pub type CommandMap = HashMap<String, HashMap<String, PropertyCommand>>;

#[async_trait]
impl Action for ControlDeviceAction {
    fn kind(&self) -> &'static str {
        "controlDevice"
    }

    #[instrument(fields(action = self.kind()), skip_all)]
    async fn execute(&self, context: &Context, scope: &mut Scope) {
        let snapshot = context.snapshot();
        let Some(device) = snapshot.devices.get(&self.device_id) else {
            warn!(
                device_id = self.device_id,
                "Unable to control unknown device '{}', ignoring action: {:?}", self.device_id, self.property
            );
            return;
        };

        let Some(command_map) = scope.ensure_entry_mut::<CommandMap, _>("command_map".to_string(), HashMap::new) else {
            error!("🛑 Incorrect type for the command map");
            return;
        };

        let device_command_map = command_map.entry(self.device_id.clone()).or_insert_with(HashMap::new);
        for (property_id, property_command) in self.property.iter() {
            let result = device_command_map.insert(property_id.clone(), property_command.clone());
            if let Some(previous_value) = result {
                warn!(
                    device_id = self.device_id,
                    "⚠️ Overriding property '{}' for device '{}', it was set by another node to '{:?}'", property_id, device.name, previous_value
                );
            }
        }
    }

    fn validate(&self, snapshot: &StoreSnapshot) -> Vec<Problem> {
        let device = match find_device(&self.device_id, snapshot) {
            Ok(device) => device,
            Err(problem) => return vec![problem],
        };

        self.property
            .iter()
            .filter_map(|(property_id, command)| match find_property(device, property_id) {
                Ok(property) if property.readonly() => Some(Problem::ReadOnlyProperty { device_id: self.device_id.to_string(), property_id: property_id.to_string() }),
                Ok(property) if command.value.value_kind() != property.value_kind() => Some(Problem::IncompatibleValue {
                    device_id: self.device_id.to_string(),
                    property_id: property_id.to_string(),
                    expected: property.value_kind(),
                    actual: command.value.value_kind(),
                }),
                Ok(_) => None,
                Err(problem) => Some(problem),
            })
            .collect()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Number;
    use crate::domain::color::Color;
    use crate::domain::property::{BooleanProperty, CartesianCoordinate, ColorProperty, NumberProperty, PropertyType, ValueKind};
    use crate::flow_engine::property_value::PropertyValue;
    use crate::flow_engine::property_value::PropertyValue::SetBooleanValue;
    use crate::store::DeviceMap;
    use crate::test_support::DeviceBuilder;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use std::io;
    use std::sync::Arc;

    fn snapshot() -> StoreSnapshot {
        let device = DeviceBuilder::new("lamp")
            .with_boolean_property("on", true)
            .with_properties(vec![
                Box::new(BooleanProperty::new("motion".to_string(), PropertyType::Motion, true, None, false)),
                Box::new(NumberProperty::builder("brightness".to_string(), PropertyType::Brightness, false).build()),
                Box::new(ColorProperty::new("color".to_string(), PropertyType::Color, false, None, CartesianCoordinate::new(0.3, 0.3), None)),
            ])
            .build();
        let devices: DeviceMap = HashMap::from([(device.id.clone(), Arc::new(device))]);
        StoreSnapshot { devices: Arc::new(devices) }
    }

    fn control_device(device_id: &str, property_ids: &[&str]) -> ControlDeviceAction {
        let property = property_ids.iter().map(|id| (id.to_string(), SetBooleanValue(true).into())).collect();
        ControlDeviceAction::new(device_id.to_string(), property)
    }

    fn control_device_with(device_id: &str, property_id: &str, value: PropertyValue) -> ControlDeviceAction {
        ControlDeviceAction::new(device_id.to_string(), HashMap::from([(property_id.to_string(), value.into())]))
    }

    fn number(value: u64) -> Number {
        Number::PositiveInt(value)
    }

    fn red() -> Color {
        Color::Hex("#ff0000".to_string())
    }

    #[test]
    fn deserialize_log_action() -> io::Result<()> {
        let json = r#"{
            "type": "log",
            "message": "Hello"
        }"#;

        let node = serde_json::from_str::<Box<dyn Action>>(json)?;

        let expected = LogAction { message: "Hello".to_string() };

        let action = node.as_any().downcast_ref::<LogAction>().unwrap();
        assert_eq!(&expected, action);

        Ok(())
    }

    #[test]
    fn deserialize_control_device_action() -> io::Result<()> {
        let json = r#"{
            "type": "controlDevice",
            "deviceId": "42",
            "property": {
                "fan": {
                    "type": "boolean",
                    "value": true
                }
            }
        }"#;

        let node = serde_json::from_str::<Box<dyn Action>>(json)?;

        let expected = ControlDeviceAction {
            device_id: "42".to_string(),
            property: HashMap::from([("fan".to_string(), SetBooleanValue(true).into())]),
        };

        let action = node.as_any().downcast_ref::<ControlDeviceAction>().unwrap();
        assert_eq!(&expected, action);

        Ok(())
    }

    #[test]
    fn deserialize_returns_error_if_type_is_missing() {
        let json = "{}";

        let node = serde_json::from_str::<Box<dyn Action>>(json);
        assert!(node.is_err());
        assert_eq!(node.unwrap_err().to_string(), "missing field 'type'");
    }

    #[test]
    fn deserialize_returns_error_for_invalid_type() {
        let json = r#"{
            "type": "UnknownAction"
        }"#;

        let node = serde_json::from_str::<Box<dyn Action>>(json);
        assert!(node.is_err());
        assert!(node.unwrap_err().to_string().starts_with("unknown action type 'UnknownAction', known types:"));
    }

    #[test]
    fn log_action_has_nothing_to_validate() {
        assert_eq!(LogAction::new("message".to_string()).validate(&StoreSnapshot::default()), []);
    }

    #[test]
    fn control_device_validate_accepts_a_known_device_and_property() {
        assert_eq!(control_device("lamp", &["on"]).validate(&snapshot()), []);
    }

    #[test]
    fn control_device_validate_reports_an_unknown_device_once_for_all_properties() {
        let problems = control_device("unknown", &["on", "brightness"]).validate(&snapshot());

        assert_eq!(problems, [Problem::UnknownDevice { device_id: "unknown".to_string() }]);
    }

    #[test]
    fn control_device_validate_reports_only_the_unknown_property() {
        let problems = control_device("lamp", &["on", "missing"]).validate(&snapshot());

        assert_eq!(problems, [Problem::UnknownProperty { device_id: "lamp".to_string(), property_id: "missing".to_string() }]);
    }

    #[test]
    fn control_device_validate_reports_only_the_readonly_property() {
        let problems = control_device("lamp", &["on", "motion"]).validate(&snapshot());

        assert_eq!(problems, [Problem::ReadOnlyProperty { device_id: "lamp".to_string(), property_id: "motion".to_string() }]);
    }

    #[rstest]
    #[case::set_boolean("on", SetBooleanValue(true))]
    #[case::toggle_boolean("on", PropertyValue::ToggleBooleanValue)]
    #[case::set_number("brightness", PropertyValue::SetNumberValue(number(50)))]
    #[case::increment_number("brightness", PropertyValue::IncrementNumberValue(number(10)))]
    #[case::decrement_number("brightness", PropertyValue::DecrementNumberValue(number(10)))]
    #[case::set_color("color", PropertyValue::SetColor(red()))]
    fn control_device_validate_accepts_a_value_that_matches_the_property(#[case] property_id: &str, #[case] value: PropertyValue) {
        assert_eq!(control_device_with("lamp", property_id, value).validate(&snapshot()), []);
    }

    #[rstest]
    #[case::number_for_boolean("on", PropertyValue::SetNumberValue(number(50)), ValueKind::Boolean, ValueKind::Number)]
    #[case::color_for_boolean("on", PropertyValue::SetColor(red()), ValueKind::Boolean, ValueKind::Color)]
    #[case::boolean_for_number("brightness", SetBooleanValue(true), ValueKind::Number, ValueKind::Boolean)]
    #[case::toggle_for_number("brightness", PropertyValue::ToggleBooleanValue, ValueKind::Number, ValueKind::Boolean)]
    #[case::increment_for_color("color", PropertyValue::IncrementNumberValue(number(10)), ValueKind::Color, ValueKind::Number)]
    fn control_device_validate_reports_an_incompatible_value(
        #[case] property_id: &str,
        #[case] value: PropertyValue,
        #[case] expected: ValueKind,
        #[case] actual: ValueKind,
    ) {
        let problems = control_device_with("lamp", property_id, value).validate(&snapshot());

        assert_eq!(problems, [Problem::IncompatibleValue { device_id: "lamp".to_string(), property_id: property_id.to_string(), expected, actual }]);
    }

    #[test]
    fn control_device_validate_reports_a_readonly_property_instead_of_an_incompatible_value() {
        let problems = control_device_with("lamp", "motion", PropertyValue::SetNumberValue(number(50))).validate(&snapshot());

        assert_eq!(problems, [Problem::ReadOnlyProperty { device_id: "lamp".to_string(), property_id: "motion".to_string() }]);
    }
}
