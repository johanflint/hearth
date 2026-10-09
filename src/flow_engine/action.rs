use crate::domain::property::{NumberProperty, Property, PropertyError, ValidatedValue};
use crate::domain::{Problem, find_device, find_property};
use crate::flow_engine::Value;
use crate::flow_engine::action_registry::{ACTION_REGISTRY, known_actions};
use crate::flow_engine::context::Context;
use crate::flow_engine::expression::evaluate;
use crate::flow_engine::property_command::{Operation, PropertyCommand, ResolvedPropertyCommand};
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

impl ControlDeviceAction {
    #[cfg(test)]
    pub fn new(device_id: String, property: HashMap<String, PropertyCommand>) -> ControlDeviceAction {
        ControlDeviceAction { device_id, property }
    }

    fn validate_command(&self, snapshot: &StoreSnapshot, property_id: &str, property: &dyn Property, command: &PropertyCommand) -> Result<(), Problem> {
        let device_id = self.device_id.clone();
        let property_id = property_id.to_string();

        if property.readonly() {
            return Err(Problem::ReadOnlyProperty { device_id, property_id });
        }

        let actual = command.value.value_kind(snapshot)?;
        let expected = property.value_kind();

        if expected != actual {
            return Err(Problem::IncompatibleValue { device_id, property_id, expected, actual });
        }

        let (Operation::Set, Some(Value::Number(value))) = (&command.operation, command.value.constant_value()) else {
            return Ok(());
        };
        let Some(number_property) = property.as_any().downcast_ref::<NumberProperty>() else {
            return Ok(());
        };
        let value = *value;
        match number_property.validate_value(value) {
            ValidatedValue::Clamped(minimum, PropertyError::ValueTooSmall) => Err(Problem::ValueTooSmall { device_id, property_id, value, minimum }),
            ValidatedValue::Clamped(maximum, PropertyError::ValueTooLarge) => Err(Problem::ValueTooLarge { device_id, property_id, value, maximum }),
            ValidatedValue::Valid(_) | ValidatedValue::Clamped(..) | ValidatedValue::Invalid(_) => Ok(()),
        }
    }
}

pub type CommandMap = HashMap<String, HashMap<String, ResolvedPropertyCommand>>;

fn clamp_to_range(device_id: &str, property_id: &str, property: &dyn Property, value: Value) -> Value {
    let (Value::Number(number), Some(number_property)) = (&value, property.as_any().downcast_ref::<NumberProperty>()) else {
        return value;
    };

    match number_property.validate_value(*number) {
        ValidatedValue::Clamped(clamped, error) => {
            info!(device_id, property_id, "⚠️ Invalid value '{}' for property '{}' ({}), clamped to '{}'", number, property_id, error, clamped);
            Value::Number(clamped)
        }
        ValidatedValue::Valid(_) | ValidatedValue::Invalid(_) => value,
    }
}

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
            let Some(property) = device.properties.get(property_id) else {
                warn!(device_id = self.device_id, property_id, "Unable to control device '{}'... unknown property '{}'", self.device_id, property_id);
                continue;
            };

            let value = match evaluate(&property_command.value, context) {
                Ok(value) => value,
                Err(err) => {
                    warn!(device_id = self.device_id, property_id, "⚠️ Evaluating expression... failed: {}", err);
                    continue;
                }
            };
            let value = clamp_to_range(&self.device_id, property_id, property.as_ref(), value);

            let resolved_command = ResolvedPropertyCommand { operation: property_command.operation.clone(), value, transition: property_command.transition };
            let result = device_command_map.insert(property_id.clone(), resolved_command);
            if let Some(previous_value) = result {
                warn!(
                    device_id = self.device_id,
                    "⚠️ Overriding property '{}' for device '{}', it was set by another node to '{:?}'", property_id, device.name, previous_value.value
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
            .filter_map(|(property_id, command)| {
                find_property(device, property_id).and_then(|property| self.validate_command(snapshot, property_id, property, command)).err()
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
    use crate::flow_engine::Expression;
    use crate::store::DeviceMap;
    use crate::test_support::{DeviceBuilder, property_command};
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use std::io;
    use std::sync::Arc;
    use std::time::Duration;

    fn snapshot() -> StoreSnapshot {
        let device = DeviceBuilder::new("lamp")
            .with_boolean_property("on", true)
            .with_properties(vec![
                Box::new(BooleanProperty::new("motion".to_string(), PropertyType::Motion, true, None, false)),
                Box::new(NumberProperty::builder("brightness".to_string(), PropertyType::Brightness, false).positive_int(Some(50), Some(1), Some(100)).build()),
                Box::new(NumberProperty::builder("temperature".to_string(), PropertyType::ColorTemperature, false).build()),
                Box::new(ColorProperty::new("color".to_string(), PropertyType::Color, false, None, CartesianCoordinate::new(0.3, 0.3), None)),
            ])
            .build();
        let devices: DeviceMap = HashMap::from([(device.id.clone(), Arc::new(device))]);
        StoreSnapshot { devices: Arc::new(devices) }
    }

    fn control_device(device_id: &str, property_ids: &[&str]) -> ControlDeviceAction {
        let property = property_ids.iter().map(|id| (id.to_string(), property_command(Operation::Set, Value::Boolean(true)))).collect();
        ControlDeviceAction::new(device_id.to_string(), property)
    }

    fn control_device_with(device_id: &str, property_id: &str, command: PropertyCommand) -> ControlDeviceAction {
        ControlDeviceAction::new(device_id.to_string(), HashMap::from([(property_id.to_string(), command)]))
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
                    "operation": "set",
                    "value": {
                      "type": "literal",
                      "value": true
                    }
                }
            }
        }"#;

        let node = serde_json::from_str::<Box<dyn Action>>(json)?;

        let expected = ControlDeviceAction {
            device_id: "42".to_string(),
            property: HashMap::from([("fan".to_string(), property_command(Operation::Set, Value::Boolean(true)))]),
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
    #[case::set_boolean("on", property_command(Operation::Set, Value::Boolean(true)))]
    #[case::set_number("brightness", property_command(Operation::Set, Value::Number(number(50))))]
    #[case::set_number_to_the_minimum("brightness", property_command(Operation::Set, Value::Number(number(1))))]
    #[case::set_number_to_the_maximum("brightness", property_command(Operation::Set, Value::Number(number(100))))]
    #[case::set_float_within_the_range("brightness", property_command(Operation::Set, Value::Number(Number::Float(99.5))))]
    #[case::set_float_to_the_minimum("brightness", property_command(Operation::Set, Value::Number(Number::Float(1.0))))]
    #[case::set_float_to_the_maximum("brightness", property_command(Operation::Set, Value::Number(Number::Float(100.0))))]
    #[case::set_number_without_a_range("temperature", property_command(Operation::Set, Value::Number(number(1_000_000))))]
    #[case::set_color("color", property_command(Operation::Set, Value::Color(red())))]
    fn control_device_validate_accepts_a_value_that_matches_the_property(#[case] property_id: &str, #[case] command: PropertyCommand) {
        assert_eq!(control_device_with("lamp", property_id, command).validate(&snapshot()), []);
    }

    #[rstest]
    #[case::number_for_boolean("on", property_command(Operation::Set, Value::Number(number(50))), ValueKind::Boolean, ValueKind::Number)]
    #[case::color_for_boolean("on", property_command(Operation::Set, Value::Color(red())), ValueKind::Boolean, ValueKind::Color)]
    #[case::boolean_for_number("brightness", property_command(Operation::Set, Value::Boolean(true)), ValueKind::Number, ValueKind::Boolean)]
    fn control_device_validate_reports_an_incompatible_value(
        #[case] property_id: &str,
        #[case] command: PropertyCommand,
        #[case] expected: ValueKind,
        #[case] actual: ValueKind,
    ) {
        let problems = control_device_with("lamp", property_id, command).validate(&snapshot());

        assert_eq!(problems, [Problem::IncompatibleValue { device_id: "lamp".to_string(), property_id: property_id.to_string(), expected, actual }]);
    }

    #[test]
    fn control_device_validate_reports_a_readonly_property_instead_of_an_incompatible_value() {
        let problems = control_device_with("lamp", "motion", property_command(Operation::Set, Value::Number(number(50)))).validate(&snapshot());

        assert_eq!(problems, [Problem::ReadOnlyProperty { device_id: "lamp".to_string(), property_id: "motion".to_string() }]);
    }

    #[rstest]
    #[case::positive_int(number(0))]
    #[case::negative_int(Number::NegativeInt(-1))]
    #[case::float(Number::Float(0.5))]
    fn control_device_validate_reports_a_value_below_the_minimum(#[case] value: Number) {
        let problems = control_device_with("lamp", "brightness", property_command(Operation::Set, Value::Number(value))).validate(&snapshot());

        assert_eq!(problems, [Problem::ValueTooSmall { device_id: "lamp".to_string(), property_id: "brightness".to_string(), value, minimum: number(1) }]);
    }

    #[rstest]
    #[case::positive_int(number(101))]
    #[case::float(Number::Float(100.5))]
    fn control_device_validate_reports_a_value_above_the_maximum(#[case] value: Number) {
        let problems = control_device_with("lamp", "brightness", property_command(Operation::Set, Value::Number(value))).validate(&snapshot());

        assert_eq!(problems, [Problem::ValueTooLarge { device_id: "lamp".to_string(), property_id: "brightness".to_string(), value, maximum: number(100) }]);
    }

    #[test]
    fn control_device_validate_skips_the_range_for_a_value_that_is_only_known_at_runtime() {
        let command = PropertyCommand {
            operation: Operation::Set,
            value: Expression::PropertyValue { device_id: "lamp".to_string(), property_id: "temperature".to_string() },
            transition: None,
        };

        assert_eq!(control_device_with("lamp", "brightness", command).validate(&snapshot()), []);
    }

    #[test]
    fn control_device_validate_reports_a_range_problem_for_each_property() {
        let action = ControlDeviceAction::new(
            "lamp".to_string(),
            HashMap::from([
                ("brightness".to_string(), property_command(Operation::Set, Value::Number(number(101)))),
                ("on".to_string(), property_command(Operation::Set, Value::Boolean(true))),
                ("missing".to_string(), property_command(Operation::Set, Value::Boolean(true))),
            ]),
        );

        let mut problems = action.validate(&snapshot());
        problems.sort_by_key(|problem| problem.to_string());

        assert_eq!(
            problems,
            [
                Problem::UnknownProperty { device_id: "lamp".to_string(), property_id: "missing".to_string() },
                Problem::ValueTooLarge { device_id: "lamp".to_string(), property_id: "brightness".to_string(), value: number(101), maximum: number(100) },
            ]
        );
    }

    async fn execute(action: &ControlDeviceAction) -> Option<CommandMap> {
        let context = Context::builder().snapshot(snapshot()).build();
        let mut scope = Scope::new();
        action.execute(&context, &mut scope).await;
        scope.get::<CommandMap>("command_map").cloned()
    }

    #[tokio::test]
    async fn execute_resolves_the_expression_against_the_snapshot() {
        let brightness_plus_ten = Expression::Add {
            lhs: Box::new(Expression::PropertyValue { device_id: "lamp".to_string(), property_id: "brightness".to_string() }),
            rhs: Box::new(Expression::Literal { value: Value::Number(number(10)) }),
        };
        let transition = Some(Duration::from_secs(1));
        let action = control_device_with("lamp", "brightness", PropertyCommand { operation: Operation::Set, value: brightness_plus_ten, transition });

        let command_map = execute(&action).await;

        let expected = ResolvedPropertyCommand { operation: Operation::Set, value: Value::Number(number(60)), transition };
        assert_eq!(command_map, Some(HashMap::from([("lamp".to_string(), HashMap::from([("brightness".to_string(), expected)]))])));
    }

    #[rstest]
    #[case::too_large(Expression::Add{ lhs: brightness(), rhs: Box::new(Expression::Literal { value: Value::Number(number(60)) }) }, number(100))]
    #[case::too_small(Expression::Subtract{ lhs: brightness(), rhs: Box::new(Expression::Literal { value: Value::Number(number(60)) }) }, number(1))]
    #[tokio::test]
    async fn execute_clamps_a_calculated_value_to_the_property_range(#[case] value: Expression, #[case] expected: Number) {
        let action = control_device_with("lamp", "brightness", PropertyCommand { operation: Operation::Set, value, transition: None });

        let command_map = execute(&action).await;

        let expected = ResolvedPropertyCommand { operation: Operation::Set, value: Value::Number(expected), transition: None };
        assert_eq!(command_map, Some(HashMap::from([("lamp".to_string(), HashMap::from([("brightness".to_string(), expected)]))])));
    }

    fn brightness() -> Box<Expression> {
        Box::new(Expression::PropertyValue { device_id: "lamp".to_string(), property_id: "brightness".to_string() })
    }

    #[tokio::test]
    async fn execute_skips_a_property_whose_expression_fails_to_evaluate() {
        let invalid = Expression::Add {
            lhs: Box::new(Expression::Literal { value: Value::Boolean(true) }),
            rhs: Box::new(Expression::Literal { value: Value::Number(number(10)) }),
        };
        let action = control_device_with("lamp", "brightness", PropertyCommand { operation: Operation::Set, value: invalid, transition: None });

        let command_map = execute(&action).await;

        assert_eq!(command_map, Some(HashMap::from([("lamp".to_string(), HashMap::new())])));
    }

    #[tokio::test]
    async fn execute_skips_an_unknown_property() {
        let command_map = execute(&control_device("lamp", &["unknown"])).await;

        assert_eq!(command_map, Some(HashMap::from([("lamp".to_string(), HashMap::new())])));
    }

    #[tokio::test]
    async fn execute_ignores_an_unknown_device() {
        let command_map = execute(&control_device("unknown", &["on"])).await;

        assert_eq!(command_map, None);
    }
}
