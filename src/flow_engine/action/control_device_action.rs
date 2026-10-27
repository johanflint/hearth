use crate::domain::property::{NumberProperty, Property, PropertyError, ValidatedValue};
use crate::domain::{ClaimMode, Problem, Value, find_device, find_property};
use crate::flow_engine::Expression;
use crate::flow_engine::action::Action;
use crate::flow_engine::action::device_command::{CommandMap, evaluate_commands};
use crate::flow_engine::context::Context;
use crate::flow_engine::property_command::PropertyCommand;
use crate::flow_engine::scope::Scope;
use crate::preset_registry::PresetRegistry;
use crate::store::StoreSnapshot;
use action_macros::register_action;
use async_trait::async_trait;
use serde::Deserialize;
use std::any::Any;
use std::collections::HashMap;
use tracing::{error, instrument, warn};

#[derive(Debug, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
#[register_action]
pub struct ControlDeviceAction {
    device_id: String,
    claim: Option<ClaimMode>,
    property: HashMap<String, PropertyCommand>,
}

impl ControlDeviceAction {
    #[cfg(test)]
    pub fn new(
        device_id: String,
        claim: Option<ClaimMode>,
        property: HashMap<String, PropertyCommand>,
    ) -> ControlDeviceAction {
        ControlDeviceAction { device_id, claim, property }
    }

    fn validate_command(
        &self,
        snapshot: &StoreSnapshot,
        preset_registry: &PresetRegistry,
        property_id: &str,
        property: &dyn Property,
        command: &PropertyCommand,
    ) -> Result<(), Problem> {
        let device_id = self.device_id.clone();
        let property_id = property_id.to_string();

        if property.readonly() {
            return Err(Problem::ReadOnlyProperty { device_id, property_id });
        }

        let actual = command.value.value_kind(snapshot, preset_registry)?;
        let expected = property.value_kind();

        if expected != actual {
            return Err(Problem::IncompatibleValue { device_id, property_id, expected, actual });
        }

        let preset = match &command.value {
            Expression::PresetValue { id } => preset_registry.by_id(id),
            _ => None,
        };
        let known_value = command.value.constant_value().or(preset.as_ref().map(|preset| &preset.value));
        let Some(Value::Number(value)) = known_value else {
            return Ok(());
        };
        let Some(number_property) = property.as_any().downcast_ref::<NumberProperty>() else {
            return Ok(());
        };
        let value = *value;
        match number_property.validate_value(value) {
            ValidatedValue::Clamped(minimum, PropertyError::ValueTooSmall) => {
                Err(Problem::ValueTooSmall { device_id, property_id, value, minimum })
            }
            ValidatedValue::Clamped(maximum, PropertyError::ValueTooLarge) => {
                Err(Problem::ValueTooLarge { device_id, property_id, value, maximum })
            }
            ValidatedValue::Valid(_) | ValidatedValue::Clamped(..) | ValidatedValue::Invalid(_) => Ok(()),
        }
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
                "⚠️ Unable to control unknown device '{}', ignoring action: {:?}", self.device_id, self.property
            );
            return;
        };

        let Some(command_map) = scope.ensure_entry_mut::<CommandMap, _>("command_map".to_string(), HashMap::new) else {
            error!("🛑 Incorrect type for the command map");
            return;
        };

        let device_command = command_map.entry(self.device_id.clone()).or_default();
        device_command.set_properties(device, &evaluate_commands(&self.property, context));
        device_command.set_claim(self.claim);
    }

    fn validate(&self, snapshot: &StoreSnapshot, preset_registry: &PresetRegistry) -> Vec<Problem> {
        let device = match find_device(&self.device_id, snapshot) {
            Ok(device) => device,
            Err(problem) => return vec![problem],
        };

        self.property
            .iter()
            .filter_map(|(property_id, command)| {
                find_property(device, property_id)
                    .and_then(|property| {
                        self.validate_command(snapshot, preset_registry, property_id, property, command)
                    })
                    .err()
            })
            .collect()
    }

    fn expressions(&self) -> Vec<&Expression> {
        self.property.values().map(|property| &property.value).collect()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Color;
    use crate::domain::Number;
    use crate::domain::Preset;
    use crate::domain::VersionedPreset;
    use crate::domain::property::{
        BooleanProperty, CartesianCoordinate, ColorProperty, NumberProperty, PropertyType, ValueKind,
    };
    use crate::flow_engine::Expression;
    use crate::flow_engine::action::device_command::DeviceCommand;
    use crate::flow_engine::property_command::ResolvedPropertyCommand;
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
                Box::new(
                    NumberProperty::builder("brightness".to_string(), PropertyType::Brightness, false)
                        .positive_int(Some(50), Some(1), Some(100))
                        .build(),
                ),
                Box::new(
                    NumberProperty::builder("temperature".to_string(), PropertyType::ColorTemperature, false).build(),
                ),
                Box::new(ColorProperty::new(
                    "color".to_string(),
                    PropertyType::Color,
                    false,
                    None,
                    CartesianCoordinate::new(0.3, 0.3),
                    None,
                )),
            ])
            .build();
        let devices: DeviceMap = HashMap::from([(device.id.clone(), Arc::new(device))]);
        StoreSnapshot { devices: Arc::new(devices), ..Default::default() }
    }

    fn preset_registry() -> Arc<PresetRegistry> {
        Arc::new(PresetRegistry::new(vec![]))
    }

    fn preset_registry_with_presets() -> Arc<PresetRegistry> {
        let preset = |id: &str, value| VersionedPreset {
            preset: Arc::new(Preset { id: id.to_string(), name: id.to_string(), value }),
            revision: 0,
        };
        Arc::new(PresetRegistry::new(vec![
            preset("dim", Value::Number(number(10))),
            preset("too_dark", Value::Number(number(0))),
            preset("too_bright", Value::Number(number(101))),
            preset("enabled", Value::Boolean(true)),
            preset("warm", Value::Color(red())),
        ]))
    }

    fn preset_command(id: &str) -> PropertyCommand {
        PropertyCommand { value: Expression::PresetValue { id: id.to_string() }, transition: None }
    }

    fn control_device(device_id: &str, property_ids: &[&str]) -> ControlDeviceAction {
        let property = property_ids.iter().map(|id| (id.to_string(), property_command(Value::Boolean(true)))).collect();
        ControlDeviceAction::new(device_id.to_string(), None, property)
    }

    fn control_device_with(device_id: &str, property_id: &str, command: PropertyCommand) -> ControlDeviceAction {
        ControlDeviceAction::new(device_id.to_string(), None, HashMap::from([(property_id.to_string(), command)]))
    }

    fn number(value: u64) -> Number {
        Number::PositiveInt(value)
    }

    fn red() -> Color {
        Color::Hex("#ff0000".to_string())
    }

    #[test]
    fn deserialize_control_device_action() -> io::Result<()> {
        let json = r#"{
            "type": "controlDevice",
            "deviceId": "42",
            "property": {
                "fan": {
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
            claim: None,
            property: HashMap::from([("fan".to_string(), property_command(Value::Boolean(true)))]),
        };

        let action = node.as_any().downcast_ref::<ControlDeviceAction>().unwrap();
        assert_eq!(&expected, action);

        Ok(())
    }

    #[test]
    fn deserialize_control_device_action_with_claim() -> io::Result<()> {
        let json = r#"{
            "type": "controlDevice",
            "deviceId": "42",
            "claim": "claim",
            "property": {
                "fan": {
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
            claim: Some(ClaimMode::Claim),
            property: HashMap::from([("fan".to_string(), property_command(Value::Boolean(true)))]),
        };

        let action = node.as_any().downcast_ref::<ControlDeviceAction>().unwrap();
        assert_eq!(&expected, action);

        Ok(())
    }

    #[test]
    fn control_device_validate_accepts_a_known_device_and_property() {
        assert_eq!(control_device("lamp", &["on"]).validate(&snapshot(), &preset_registry()), []);
    }

    #[test]
    fn control_device_validate_reports_an_unknown_device_once_for_all_properties() {
        let problems = control_device("unknown", &["on", "brightness"]).validate(&snapshot(), &preset_registry());

        assert_eq!(problems, [Problem::UnknownDevice { device_id: "unknown".to_string() }]);
    }

    #[test]
    fn control_device_validate_reports_only_the_unknown_property() {
        let problems = control_device("lamp", &["on", "missing"]).validate(&snapshot(), &preset_registry());

        assert_eq!(
            problems,
            [Problem::UnknownProperty { device_id: "lamp".to_string(), property_id: "missing".to_string() }]
        );
    }

    #[test]
    fn control_device_validate_reports_only_the_readonly_property() {
        let problems = control_device("lamp", &["on", "motion"]).validate(&snapshot(), &preset_registry());

        assert_eq!(
            problems,
            [Problem::ReadOnlyProperty { device_id: "lamp".to_string(), property_id: "motion".to_string() }]
        );
    }

    #[rstest]
    #[case::set_boolean("on", property_command(Value::Boolean(true)))]
    #[case::set_number("brightness", property_command(Value::Number(number(50))))]
    #[case::set_number_to_the_minimum("brightness", property_command(Value::Number(number(1))))]
    #[case::set_number_to_the_maximum("brightness", property_command(Value::Number(number(100))))]
    #[case::set_float_within_the_range("brightness", property_command(Value::Number(Number::Float(99.5))))]
    #[case::set_float_to_the_minimum("brightness", property_command(Value::Number(Number::Float(1.0))))]
    #[case::set_float_to_the_maximum("brightness", property_command(Value::Number(Number::Float(100.0))))]
    #[case::set_number_without_a_range("temperature", property_command(Value::Number(number(1_000_000))))]
    #[case::set_color("color", property_command(Value::Color(red())))]
    fn control_device_validate_accepts_a_value_that_matches_the_property(
        #[case] property_id: &str,
        #[case] command: PropertyCommand,
    ) {
        assert_eq!(control_device_with("lamp", property_id, command).validate(&snapshot(), &preset_registry()), []);
    }

    #[rstest]
    #[case::number_for_boolean(
        "on",
        property_command(Value::Number(number(50))),
        ValueKind::Boolean,
        ValueKind::Number
    )]
    #[case::color_for_boolean("on", property_command(Value::Color(red())), ValueKind::Boolean, ValueKind::Color)]
    #[case::boolean_for_number(
        "brightness",
        property_command(Value::Boolean(true)),
        ValueKind::Number,
        ValueKind::Boolean
    )]
    fn control_device_validate_reports_an_incompatible_value(
        #[case] property_id: &str,
        #[case] command: PropertyCommand,
        #[case] expected: ValueKind,
        #[case] actual: ValueKind,
    ) {
        let problems = control_device_with("lamp", property_id, command).validate(&snapshot(), &preset_registry());

        assert_eq!(
            problems,
            [Problem::IncompatibleValue {
                device_id: "lamp".to_string(),
                property_id: property_id.to_string(),
                expected,
                actual
            }]
        );
    }

    #[test]
    fn control_device_validate_reports_a_readonly_property_instead_of_an_incompatible_value() {
        let problems = control_device_with("lamp", "motion", property_command(Value::Number(number(50))))
            .validate(&snapshot(), &preset_registry());

        assert_eq!(
            problems,
            [Problem::ReadOnlyProperty { device_id: "lamp".to_string(), property_id: "motion".to_string() }]
        );
    }

    #[rstest]
    #[case::positive_int(number(0))]
    #[case::negative_int(Number::NegativeInt(-1))]
    #[case::float(Number::Float(0.5))]
    fn control_device_validate_reports_a_value_below_the_minimum(#[case] value: Number) {
        let problems = control_device_with("lamp", "brightness", property_command(Value::Number(value)))
            .validate(&snapshot(), &preset_registry());

        assert_eq!(
            problems,
            [Problem::ValueTooSmall {
                device_id: "lamp".to_string(),
                property_id: "brightness".to_string(),
                value,
                minimum: number(1)
            }]
        );
    }

    #[rstest]
    #[case::positive_int(number(101))]
    #[case::float(Number::Float(100.5))]
    fn control_device_validate_reports_a_value_above_the_maximum(#[case] value: Number) {
        let problems = control_device_with("lamp", "brightness", property_command(Value::Number(value)))
            .validate(&snapshot(), &preset_registry());

        assert_eq!(
            problems,
            [Problem::ValueTooLarge {
                device_id: "lamp".to_string(),
                property_id: "brightness".to_string(),
                value,
                maximum: number(100)
            }]
        );
    }

    #[test]
    fn control_device_validate_skips_the_range_for_a_value_that_is_only_known_at_runtime() {
        let command = PropertyCommand {
            value: Expression::PropertyValue { device_id: "lamp".to_string(), property_id: "temperature".to_string() },
            transition: None,
        };

        assert_eq!(control_device_with("lamp", "brightness", command).validate(&snapshot(), &preset_registry()), []);
    }

    #[rstest]
    #[case::number("brightness", "dim")]
    #[case::boolean("on", "enabled")]
    #[case::color("color", "warm")]
    fn control_device_validate_accepts_a_preset_that_matches_the_property_type(
        #[case] property_id: &str,
        #[case] preset_id: &str,
    ) {
        let action = control_device_with("lamp", property_id, preset_command(preset_id));

        assert_eq!(action.validate(&snapshot(), &preset_registry_with_presets()), []);
    }

    #[test]
    fn control_device_validate_reports_an_unknown_preset() {
        let action = control_device_with("lamp", "brightness", preset_command("missing"));

        let problems = action.validate(&snapshot(), &preset_registry_with_presets());

        assert_eq!(problems, [Problem::PresetNotFound { id: "missing".to_string() }]);
    }

    #[rstest]
    #[case::number_for_boolean("on", "dim", ValueKind::Boolean, ValueKind::Number)]
    #[case::boolean_for_number("brightness", "enabled", ValueKind::Number, ValueKind::Boolean)]
    #[case::color_for_number("brightness", "warm", ValueKind::Number, ValueKind::Color)]
    fn control_device_validate_reports_a_preset_of_the_wrong_kind(
        #[case] property_id: &str,
        #[case] preset_id: &str,
        #[case] expected: ValueKind,
        #[case] actual: ValueKind,
    ) {
        let action = control_device_with("lamp", property_id, preset_command(preset_id));

        let problems = action.validate(&snapshot(), &preset_registry_with_presets());

        assert_eq!(
            problems,
            [Problem::IncompatibleValue {
                device_id: "lamp".to_string(),
                property_id: property_id.to_string(),
                expected,
                actual
            }]
        );
    }

    #[test]
    fn control_device_validate_reports_a_preset_below_the_minimum() {
        let action = control_device_with("lamp", "brightness", preset_command("too_dark"));

        let problems = action.validate(&snapshot(), &preset_registry_with_presets());

        assert_eq!(
            problems,
            [Problem::ValueTooSmall {
                device_id: "lamp".to_string(),
                property_id: "brightness".to_string(),
                value: number(0),
                minimum: number(1)
            }]
        );
    }

    #[test]
    fn control_device_validate_reports_a_preset_above_the_maximum() {
        let action = control_device_with("lamp", "brightness", preset_command("too_bright"));

        let problems = action.validate(&snapshot(), &preset_registry_with_presets());

        assert_eq!(
            problems,
            [Problem::ValueTooLarge {
                device_id: "lamp".to_string(),
                property_id: "brightness".to_string(),
                value: number(101),
                maximum: number(100)
            }]
        );
    }

    #[test]
    fn control_device_validate_reports_a_range_problem_for_each_property() {
        let action = ControlDeviceAction::new(
            "lamp".to_string(),
            None,
            HashMap::from([
                ("brightness".to_string(), property_command(Value::Number(number(101)))),
                ("on".to_string(), property_command(Value::Boolean(true))),
                ("missing".to_string(), property_command(Value::Boolean(true))),
            ]),
        );

        let mut problems = action.validate(&snapshot(), &preset_registry());
        problems.sort_by_key(|problem| problem.to_string());

        assert_eq!(
            problems,
            [
                Problem::UnknownProperty { device_id: "lamp".to_string(), property_id: "missing".to_string() },
                Problem::ValueTooLarge {
                    device_id: "lamp".to_string(),
                    property_id: "brightness".to_string(),
                    value: number(101),
                    maximum: number(100)
                },
            ]
        );
    }

    async fn execute(action: &ControlDeviceAction) -> Option<CommandMap> {
        execute_with_presets(action, preset_registry()).await
    }

    async fn execute_with_presets(
        action: &ControlDeviceAction,
        preset_registry: Arc<PresetRegistry>,
    ) -> Option<CommandMap> {
        let context = Context::builder().snapshot(snapshot()).preset_registry(preset_registry).build();
        let mut scope = Scope::new();
        action.execute(&context, &mut scope).await;
        scope.take().remove("command_map").and_then(|v| v.downcast::<CommandMap>().ok()).map(|boxed| *boxed)
    }

    #[rstest]
    #[case::claim_then_none(Some(ClaimMode::Claim), None, Some(ClaimMode::Claim))]
    #[case::none_then_release(None, Some(ClaimMode::Release), Some(ClaimMode::Release))]
    #[case::claim_then_release(Some(ClaimMode::Claim), Some(ClaimMode::Release), Some(ClaimMode::Release))]
    #[case::release_then_claim(Some(ClaimMode::Release), Some(ClaimMode::Claim), Some(ClaimMode::Claim))]
    #[tokio::test]
    async fn execute_keeps_the_last_claim_of_a_device(
        #[case] first: Option<ClaimMode>,
        #[case] second: Option<ClaimMode>,
        #[case] expected: Option<ClaimMode>,
    ) {
        let context = Context::builder().snapshot(snapshot()).build();
        let mut scope = Scope::new();

        ControlDeviceAction::new("lamp".to_string(), first, HashMap::new()).execute(&context, &mut scope).await;
        ControlDeviceAction::new("lamp".to_string(), second, HashMap::new()).execute(&context, &mut scope).await;

        assert_eq!(scope.get::<CommandMap>("command_map").unwrap()["lamp"].claim, expected);
    }

    #[tokio::test]
    async fn execute_resolves_the_expression_against_the_snapshot() {
        let brightness_plus_ten = Expression::Add {
            lhs: Box::new(Expression::PropertyValue {
                device_id: "lamp".to_string(),
                property_id: "brightness".to_string(),
            }),
            rhs: Box::new(Expression::Literal { value: Value::Number(number(10)) }),
        };
        let transition = Some(Duration::from_secs(1));
        let action =
            control_device_with("lamp", "brightness", PropertyCommand { value: brightness_plus_ten, transition });

        let command_map = execute(&action).await;

        let expected = ResolvedPropertyCommand { value: Value::Number(number(60)), transition };
        assert_eq!(
            command_map,
            Some(HashMap::from([(
                "lamp".to_string(),
                DeviceCommand { claim: None, properties: HashMap::from([("brightness".to_string(), expected)]) }
            )]))
        );
    }

    #[rstest]
    #[case::too_large(Expression::Add{ lhs: brightness(), rhs: Box::new(Expression::Literal { value: Value::Number(number(60)) }) }, number(100))]
    #[case::too_small(Expression::Subtract{ lhs: brightness(), rhs: Box::new(Expression::Literal { value: Value::Number(number(60)) }) }, number(1))]
    #[tokio::test]
    async fn execute_clamps_a_calculated_value_to_the_property_range(
        #[case] value: Expression,
        #[case] expected: Number,
    ) {
        let action = control_device_with("lamp", "brightness", PropertyCommand { value, transition: None });

        let command_map = execute(&action).await;

        let expected = ResolvedPropertyCommand { value: Value::Number(expected), transition: None };
        assert_eq!(
            command_map,
            Some(HashMap::from([(
                "lamp".to_string(),
                DeviceCommand { claim: None, properties: HashMap::from([("brightness".to_string(), expected)]) }
            )]))
        );
    }

    fn lamp_command(properties: HashMap<String, ResolvedPropertyCommand>) -> Option<CommandMap> {
        Some(HashMap::from([("lamp".to_string(), DeviceCommand { claim: None, properties })]))
    }

    #[rstest]
    #[case::number("brightness", "dim", Value::Number(number(10)))]
    #[case::boolean("on", "enabled", Value::Boolean(true))]
    #[case::color("color", "warm", Value::Color(red()))]
    #[tokio::test]
    async fn execute_resolves_a_preset(#[case] property_id: &str, #[case] preset_id: &str, #[case] expected: Value) {
        let transition = Some(Duration::from_secs(1));
        let command = PropertyCommand { transition, ..preset_command(preset_id) };
        let action = control_device_with("lamp", property_id, command);

        let command_map = execute_with_presets(&action, preset_registry_with_presets()).await;

        let expected = ResolvedPropertyCommand { value: expected, transition };
        assert_eq!(command_map, lamp_command(HashMap::from([(property_id.to_string(), expected)])));
    }

    #[rstest]
    #[case::too_large("too_bright", number(100))]
    #[case::too_small("too_dark", number(1))]
    #[tokio::test]
    async fn execute_clamps_a_preset_to_the_property_range(#[case] preset_id: &str, #[case] expected: Number) {
        let action = control_device_with("lamp", "brightness", preset_command(preset_id));

        let command_map = execute_with_presets(&action, preset_registry_with_presets()).await;

        let expected = ResolvedPropertyCommand { value: Value::Number(expected), transition: None };
        assert_eq!(command_map, lamp_command(HashMap::from([("brightness".to_string(), expected)])));
    }

    #[tokio::test]
    async fn execute_skips_an_unknown_preset() {
        let action = control_device_with("lamp", "brightness", preset_command("missing"));

        let command_map = execute_with_presets(&action, preset_registry_with_presets()).await;

        assert_eq!(command_map, lamp_command(HashMap::new()));
    }

    #[rstest]
    #[case::boolean_preset_for_a_number("brightness", "enabled")]
    #[case::number_preset_for_a_boolean("on", "dim")]
    #[case::color_preset_for_a_number("brightness", "warm")]
    #[tokio::test]
    async fn execute_skips_a_preset_that_no_longer_matches_the_property_kind(
        #[case] property_id: &str,
        #[case] preset_id: &str,
    ) {
        let action = control_device_with("lamp", property_id, preset_command(preset_id));

        let command_map = execute_with_presets(&action, preset_registry_with_presets()).await;

        assert_eq!(command_map, lamp_command(HashMap::new()));
    }

    #[tokio::test]
    async fn execute_skips_only_the_properties_that_do_not_match() {
        let action = ControlDeviceAction::new(
            "lamp".to_string(),
            None,
            HashMap::from([
                ("brightness".to_string(), preset_command("enabled")),
                ("on".to_string(), preset_command("enabled")),
            ]),
        );

        let command_map = execute_with_presets(&action, preset_registry_with_presets()).await;

        let on = ResolvedPropertyCommand { value: Value::Boolean(true), transition: None };
        assert_eq!(command_map, lamp_command(HashMap::from([("on".to_string(), on)])));
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
        let action = control_device_with("lamp", "brightness", PropertyCommand { value: invalid, transition: None });

        let command_map = execute(&action).await;

        assert_eq!(
            command_map,
            Some(HashMap::from([("lamp".to_string(), DeviceCommand { claim: None, properties: HashMap::new() })]))
        );
    }

    #[tokio::test]
    async fn execute_skips_an_unknown_property() {
        let command_map = execute(&control_device("lamp", &["unknown"])).await;

        assert_eq!(
            command_map,
            Some(HashMap::from([("lamp".to_string(), DeviceCommand { claim: None, properties: HashMap::new() })]))
        );
    }

    #[tokio::test]
    async fn execute_ignores_an_unknown_device() {
        let command_map = execute(&control_device("unknown", &["on"])).await;

        assert_eq!(command_map, None);
    }

    #[test]
    fn deserialize_control_device_action_error_names_the_property() {
        let json = serde_json::json!({
            "type": "controlDevice",
            "deviceId": "42",
            "property": {
                "on": {
                    "type": "boolean",
                    "value": true
                }
            }
        });

        let error = Box::<dyn Action>::deserialize(&json).unwrap_err();
        assert_eq!(
            error.to_string(),
            "property.on.value: invalid type: boolean `true`, expected internally tagged enum Expression"
        );
    }

    #[test]
    fn control_device_expressions_returns_the_expression_of_every_property() {
        let property = HashMap::from([
            ("on".to_string(), preset_command("enabled")),
            ("color".to_string(), preset_command("warm")),
        ]);
        let action = ControlDeviceAction::new("device".to_string(), None, property);

        let mut ids: Vec<&str> = action
            .expressions()
            .into_iter()
            .map(|e| match e {
                Expression::PresetValue { id } => id.as_str(),
                other => panic!("unexpected expression {other:?}"),
            })
            .collect();
        ids.sort();

        assert_eq!(ids, ["enabled", "warm"]);
    }
}
