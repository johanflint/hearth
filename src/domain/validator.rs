use crate::domain::device::Device;
use crate::domain::property::{NumberProperty, Property, PropertyError, ValidatedValue, ValueKind};
use crate::domain::{Group, Number, Value};
use crate::flow_engine::Expression;
use crate::flow_engine::flow::{Flow, FlowNodeKind};
use crate::flow_engine::property_command::PropertyCommand;
use crate::preset_registry::PresetRegistry;
use crate::store::StoreSnapshot;
use std::fmt;
use thiserror::Error;

/// Validates a flow against the given snapshot.
///
/// Performs semantic validation, it assumes the structural validation by the
/// `flow_loader::from_json` is already done.
pub fn validate(
    flow: &Flow,
    snapshot: &StoreSnapshot,
    preset_registry: &PresetRegistry,
) -> Result<(), FlowValidationError> {
    let mut issues = vec![];

    issues.extend(
        validate_expression(snapshot, flow.trigger(), preset_registry).into_iter().map(|p| p.at(Location::Trigger)),
    );

    for node in flow.walk() {
        let location = || Location::Node { node_id: node.id().to_string() };
        match node.kind() {
            FlowNodeKind::Start | FlowNodeKind::End | FlowNodeKind::Sleep(_) => {}
            FlowNodeKind::Conditional(expression) => {
                issues.extend(
                    validate_expression(snapshot, expression, preset_registry).into_iter().map(|p| p.at(location())),
                );
            }
            FlowNodeKind::Action(action_node) => {
                issues.extend(
                    action_node.action().validate(snapshot, preset_registry).into_iter().map(|p| p.at(location())),
                );
            }
        }
    }

    FlowValidationError::from_issues(issues)
}

pub fn find_group<'a>(group_id: &str, snapshot: &'a StoreSnapshot) -> Result<&'a Group, Problem> {
    snapshot
        .groups
        .get(group_id)
        .map(|group| group.as_ref())
        .ok_or_else(|| Problem::UnknownGroup { group_id: group_id.to_string() })
}

pub fn find_device<'a>(device_id: &str, snapshot: &'a StoreSnapshot) -> Result<&'a Device, Problem> {
    snapshot
        .devices
        .get(device_id)
        .map(|device| device.as_ref())
        .ok_or_else(|| Problem::UnknownDevice { device_id: device_id.to_string() })
}

pub fn find_property<'a>(device: &'a Device, property_id: &str) -> Result<&'a dyn Property, Problem> {
    device
        .properties
        .get(property_id)
        .map(|property| property.as_ref())
        .ok_or_else(|| Problem::UnknownProperty { device_id: device.id.clone(), property_id: property_id.to_string() })
}

fn validate_expression(
    snapshot: &StoreSnapshot,
    expression: &Expression,
    preset_registry: &PresetRegistry,
) -> Vec<Problem> {
    let problems: Vec<Problem> = expression
        .walk()
        .filter_map(|expression| match expression {
            Expression::PropertyChanged { device_id, property_id }
            | Expression::PropertyValue { device_id, property_id } => {
                find_device(device_id, snapshot).and_then(|device| find_property(device, property_id)).err()
            }
            _ => None,
        })
        .collect();

    if !problems.is_empty() {
        return problems;
    }

    expression.value_kind(snapshot, preset_registry).err().into_iter().collect()
}

// Validates the command for one property of a device, so a flow can't write a value the device would reject
pub fn validate_property_command(
    snapshot: &StoreSnapshot,
    preset_registry: &PresetRegistry,
    device: &Device,
    property_id: &str,
    command: &PropertyCommand,
) -> Result<(), Problem> {
    let property = find_property(device, property_id)?;
    let device_id = device.id.clone();
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

#[derive(Debug, Error, PartialEq)]
#[error("flow has {} validation {}", .0.len(), if .0.len() == 1 { "issue" } else { "issues" })]
pub struct FlowValidationError(Vec<ValidationIssue>);

impl FlowValidationError {
    pub fn from_issues(issues: Vec<ValidationIssue>) -> Result<(), Self> {
        if issues.is_empty() { Ok(()) } else { Err(Self(issues)) }
    }

    pub fn issues(&self) -> &[ValidationIssue] {
        &self.0
    }
}

#[derive(Debug, Error, PartialEq)]
#[error("{location}: {problem}")]
pub struct ValidationIssue {
    pub location: Location,
    pub problem: Problem,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Location {
    Trigger,
    Node { node_id: String },
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Location::Trigger => write!(f, "trigger"),
            Location::Node { node_id } => write!(f, "node '{}'", node_id),
        }
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum Problem {
    #[error("unknown group '{group_id}'")]
    UnknownGroup { group_id: String },
    #[error("unknown device '{device_id}'")]
    UnknownDevice { device_id: String },
    #[error("unknown property '{property_id}' for device '{device_id}'")]
    UnknownProperty { device_id: String, property_id: String },
    #[error("readonly property '{property_id}' for device '{device_id}'")]
    ReadOnlyProperty { device_id: String, property_id: String },
    #[error(
        "incompatible value for property '{property_id}' for device '{device_id}': expected {expected}, got {actual}"
    )]
    IncompatibleValue { device_id: String, property_id: String, expected: ValueKind, actual: ValueKind },
    #[error("incompatible operands for '{operator}': expected {expected}, got {}", .actual.iter().map(ToString::to_string).collect::<Vec<_>>().join(" and "))]
    IncompatibleOperands { operator: &'static str, expected: &'static str, actual: Vec<ValueKind> },
    #[error("value too small for property '{property_id}' for device '{device_id}': minimum {minimum}, got {value}")]
    ValueTooSmall { device_id: String, property_id: String, value: Number, minimum: Number },
    #[error("value too large for property '{property_id}' for device '{device_id}': maximum {maximum}, got {value}")]
    ValueTooLarge { device_id: String, property_id: String, value: Number, maximum: Number },
    #[error("unknown preset '{id}'")]
    PresetNotFound { id: String },
}

impl Problem {
    pub fn at(self, location: Location) -> ValidationIssue {
        ValidationIssue { location, problem: self }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::property::{BooleanProperty, CartesianCoordinate, NumberProperty, PropertyType};
    use crate::domain::{Color, Number, Preset, Value, VersionedPreset};
    use crate::flow_engine::Expression::*;
    use crate::flow_engine::action::{ControlDeviceAction, LogAction};
    use crate::flow_engine::flow::{ActionFlowNode, FlowLink, FlowNode, FlowNodeKind};
    use crate::flow_engine::property_command::PropertyCommand;
    use crate::store::DeviceMap;
    use crate::test_support::{DeviceBuilder, property_command};
    use rstest::rstest;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;

    const KNOWN_DEVICE_ID: &str = "known";
    const UNKNOWN_PROPERTY_ID: &str = "missing";
    const NODE_ID: &str = "node";
    const READONLY_PROPERTY_ID: &str = "motion";

    fn flow_with_trigger(trigger: Expression) -> Flow {
        let end_node = FlowNode::new("end_node".to_string(), vec![], FlowNodeKind::End);
        let start_node = FlowNode::new(
            "start_node".to_string(),
            vec![FlowLink::new(Arc::new(end_node), Value::None)],
            FlowNodeKind::Start,
        );

        Flow::new("flow".to_string(), "flow".to_string(), None, Some(trigger), Arc::new(start_node), HashMap::new())
            .unwrap()
    }

    /// Builds `start -> nodes[0] -> nodes[1] -> ... -> end`, the nodes are given as `(id, kind)`.
    fn flow_with_nodes(trigger: Expression, nodes: Vec<(&str, FlowNodeKind)>) -> Flow {
        let end_node = FlowNode::new("end_node".to_string(), vec![], FlowNodeKind::End);
        let first_node = nodes.into_iter().rev().fold(end_node, |next, (id, kind)| {
            FlowNode::new(id.to_string(), vec![FlowLink::new(Arc::new(next), Value::None)], kind)
        });
        let start_node = FlowNode::new(
            "start_node".to_string(),
            vec![FlowLink::new(Arc::new(first_node), Value::None)],
            FlowNodeKind::Start,
        );

        Flow::new("flow".to_string(), "flow".to_string(), None, Some(trigger), Arc::new(start_node), HashMap::new())
            .unwrap()
    }

    fn flow_with_node(kind: FlowNodeKind) -> Flow {
        flow_with_nodes(Literal { value: Value::Boolean(true) }, vec![(NODE_ID, kind)])
    }

    fn control_device(device_id: &str, property_id: &str) -> FlowNodeKind {
        control_device_with(device_id, property_id, property_command(Value::Boolean(true)))
    }

    fn control_device_with(device_id: &str, property_id: &str, value: PropertyCommand) -> FlowNodeKind {
        let property = HashMap::from([(property_id.to_string(), value)]);
        FlowNodeKind::Action(ActionFlowNode::new(Box::new(ControlDeviceAction::new(
            device_id.to_string(),
            None,
            property,
        ))))
    }

    fn log() -> FlowNodeKind {
        FlowNodeKind::Action(ActionFlowNode::new(Box::new(LogAction::new("message".to_string()))))
    }

    fn node(node_id: &str) -> Location {
        Location::Node { node_id: node_id.to_string() }
    }

    fn snapshot() -> StoreSnapshot {
        let device = DeviceBuilder::new(KNOWN_DEVICE_ID)
            .with_boolean_property("on", true)
            .with_properties(vec![
                Box::new(BooleanProperty::new(
                    READONLY_PROPERTY_ID.to_string(),
                    PropertyType::Motion,
                    true,
                    None,
                    false,
                )),
                Box::new(
                    NumberProperty::builder("brightness".to_string(), PropertyType::Brightness, false)
                        .positive_int(Some(50), Some(1), Some(100))
                        .build(),
                ),
                Box::new(
                    NumberProperty::builder("temperature".to_string(), PropertyType::ColorTemperature, false).build(),
                ),
            ])
            .with_color_property("color", CartesianCoordinate::new(0.3, 0.3), None)
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
            preset("enabled", Value::Boolean(true)),
            preset("dim", Value::Number(Number::PositiveInt(10))),
            preset("too_dark", Value::Number(Number::PositiveInt(0))),
            preset("too_bright", Value::Number(Number::PositiveInt(101))),
            preset("warm", Value::Color(red())),
        ]))
    }

    fn number(value: u64) -> Number {
        Number::PositiveInt(value)
    }

    fn red() -> Color {
        Color::Hex("#ff0000".to_string())
    }

    fn validate_command(property_id: &str, command: PropertyCommand) -> Result<(), Problem> {
        let snapshot = snapshot();
        let device = find_device(KNOWN_DEVICE_ID, &snapshot).unwrap();
        validate_property_command(&snapshot, &preset_registry_with_presets(), device, property_id, &command)
    }

    fn incompatible_value(property_id: &str, expected: ValueKind, actual: ValueKind) -> Problem {
        Problem::IncompatibleValue {
            device_id: KNOWN_DEVICE_ID.to_string(),
            property_id: property_id.to_string(),
            expected,
            actual,
        }
    }

    fn value_too_small(value: Number, minimum: Number) -> Problem {
        Problem::ValueTooSmall {
            device_id: KNOWN_DEVICE_ID.to_string(),
            property_id: "brightness".to_string(),
            value,
            minimum,
        }
    }

    fn value_too_large(value: Number, maximum: Number) -> Problem {
        Problem::ValueTooLarge {
            device_id: KNOWN_DEVICE_ID.to_string(),
            property_id: "brightness".to_string(),
            value,
            maximum,
        }
    }

    fn preset_value(id: &str) -> Expression {
        PresetValue { id: id.to_string() }
    }

    fn preset_command(id: &str) -> PropertyCommand {
        PropertyCommand { value: preset_value(id), transition: None }
    }

    fn preset_not_found_at(id: &str, location: Location) -> ValidationIssue {
        Problem::PresetNotFound { id: id.to_string() }.at(location)
    }

    fn property_changed(device_id: &str) -> Expression {
        property_changed_of(device_id, "on")
    }

    fn property_changed_of(device_id: &str, property_id: &str) -> Expression {
        PropertyChanged { device_id: device_id.to_string(), property_id: property_id.to_string() }
    }

    fn property_value(device_id: &str) -> Expression {
        property_value_of(device_id, "on")
    }

    fn property_value_of(device_id: &str, property_id: &str) -> Expression {
        PropertyValue { device_id: device_id.to_string(), property_id: property_id.to_string() }
    }

    fn unknown_device(device_id: &str) -> ValidationIssue {
        unknown_device_at(device_id, Location::Trigger)
    }

    fn unknown_device_at(device_id: &str, location: Location) -> ValidationIssue {
        Problem::UnknownDevice { device_id: device_id.to_string() }.at(location)
    }

    fn unknown_property(device_id: &str, property_id: &str) -> ValidationIssue {
        unknown_property_at(device_id, property_id, Location::Trigger)
    }

    fn unknown_property_at(device_id: &str, property_id: &str, location: Location) -> ValidationIssue {
        Problem::UnknownProperty { device_id: device_id.to_string(), property_id: property_id.to_string() }.at(location)
    }

    fn readonly_property_at(device_id: &str, property_id: &str, location: Location) -> ValidationIssue {
        Problem::ReadOnlyProperty { device_id: device_id.to_string(), property_id: property_id.to_string() }
            .at(location)
    }

    fn value_too_small_at(
        device_id: &str,
        property_id: &str,
        value: Number,
        minimum: Number,
        location: Location,
    ) -> ValidationIssue {
        Problem::ValueTooSmall {
            device_id: device_id.to_string(),
            property_id: property_id.to_string(),
            value,
            minimum,
        }
        .at(location)
    }

    fn value_too_large_at(
        device_id: &str,
        property_id: &str,
        value: Number,
        maximum: Number,
        location: Location,
    ) -> ValidationIssue {
        Problem::ValueTooLarge {
            device_id: device_id.to_string(),
            property_id: property_id.to_string(),
            value,
            maximum,
        }
        .at(location)
    }

    fn incompatible_value_at(
        device_id: &str,
        property_id: &str,
        expected: ValueKind,
        actual: ValueKind,
        location: Location,
    ) -> ValidationIssue {
        Problem::IncompatibleValue {
            device_id: device_id.to_string(),
            property_id: property_id.to_string(),
            expected,
            actual,
        }
        .at(location)
    }

    fn incompatible_operands_at(
        operator: &'static str,
        expected: &'static str,
        actual: Vec<ValueKind>,
        location: Location,
    ) -> ValidationIssue {
        Problem::IncompatibleOperands { operator, expected, actual }.at(location)
    }

    #[rstest]
    #[case::literal(Literal{ value: Value::Boolean(true) })]
    #[case::property_changed(property_changed(KNOWN_DEVICE_ID))]
    #[case::property_value(property_value(KNOWN_DEVICE_ID))]
    #[case::nested(And{ lhs: Box::new(property_changed(KNOWN_DEVICE_ID)), rhs: Box::new(Not { expression: Box::new(property_value(KNOWN_DEVICE_ID)) }) })]
    fn validate_accepts_a_trigger_that_only_references_known_devices_and_properties(#[case] trigger: Expression) {
        assert_eq!(validate(&flow_with_trigger(trigger), &snapshot(), &preset_registry()), Ok(()));
    }

    #[rstest]
    #[case::property_changed(property_changed("unknown"))]
    #[case::property_value(property_value("unknown"))]
    #[case::nested_in_not(Not{ expression: Box::new(property_value("unknown")) })]
    #[case::next_to_a_known_device(Or{ lhs: Box::new(property_changed(KNOWN_DEVICE_ID)), rhs: Box::new(property_value("unknown")) })]
    fn validate_reports_an_unknown_device_in_the_trigger(#[case] trigger: Expression) {
        let result = validate(&flow_with_trigger(trigger), &snapshot(), &preset_registry());

        assert_eq!(result.unwrap_err().issues(), [unknown_device("unknown")]);
    }

    #[rstest]
    #[case::property_changed(property_changed_of(KNOWN_DEVICE_ID, UNKNOWN_PROPERTY_ID))]
    #[case::property_value(property_value_of(KNOWN_DEVICE_ID, UNKNOWN_PROPERTY_ID))]
    #[case::nested_in_not(Not{ expression: Box::new(property_value_of(KNOWN_DEVICE_ID, UNKNOWN_PROPERTY_ID)) })]
    #[case::next_to_a_known_property(And{ lhs: Box::new(property_changed(KNOWN_DEVICE_ID)), rhs: Box::new(property_value_of(KNOWN_DEVICE_ID, UNKNOWN_PROPERTY_ID)) })]
    fn validate_reports_an_unknown_property_in_the_trigger(#[case] trigger: Expression) {
        let result = validate(&flow_with_trigger(trigger), &snapshot(), &preset_registry());

        assert_eq!(result.unwrap_err().issues(), [unknown_property(KNOWN_DEVICE_ID, UNKNOWN_PROPERTY_ID)]);
    }

    #[test]
    fn validate_only_reports_the_unknown_device_when_its_property_is_unknown_too() {
        let result = validate(
            &flow_with_trigger(property_changed_of("unknown", UNKNOWN_PROPERTY_ID)),
            &snapshot(),
            &preset_registry(),
        );

        assert_eq!(result.unwrap_err().issues(), [unknown_device("unknown")]);
    }

    #[test]
    fn validate_reports_unknown_devices_and_properties_in_the_trigger_in_order() {
        let trigger = Or {
            lhs: Box::new(property_value_of(KNOWN_DEVICE_ID, UNKNOWN_PROPERTY_ID)),
            rhs: Box::new(property_changed("unknown")),
        };

        let result = validate(&flow_with_trigger(trigger), &snapshot(), &preset_registry());

        assert_eq!(
            result.unwrap_err().issues(),
            [unknown_property(KNOWN_DEVICE_ID, UNKNOWN_PROPERTY_ID), unknown_device("unknown")]
        );
    }

    #[test]
    fn validate_reports_all_unknown_devices_in_the_trigger_in_order() {
        let trigger = And { lhs: Box::new(property_changed("first")), rhs: Box::new(property_value("second")) };

        let result = validate(&flow_with_trigger(trigger), &snapshot(), &preset_registry());

        assert_eq!(result.unwrap_err().issues(), [unknown_device("first"), unknown_device("second")]);
    }

    #[rstest]
    #[case::direct(preset_value("enabled"))]
    #[case::nested_in_not(Not{ expression: Box::new(preset_value("enabled")) })]
    #[case::next_to_a_known_device(And{ lhs: Box::new(property_changed(KNOWN_DEVICE_ID)), rhs: Box::new(preset_value("enabled")) })]
    fn validate_accepts_a_known_preset_in_the_trigger(#[case] trigger: Expression) {
        assert_eq!(validate(&flow_with_trigger(trigger), &snapshot(), &preset_registry_with_presets()), Ok(()));
    }

    #[rstest]
    #[case::direct(preset_value("unknown"))]
    #[case::nested_in_not(Not{ expression: Box::new(preset_value("unknown")) })]
    #[case::next_to_a_known_device(And{ lhs: Box::new(property_changed(KNOWN_DEVICE_ID)), rhs: Box::new(preset_value("unknown")) })]
    fn validate_reports_an_unknown_preset_in_the_trigger(#[case] trigger: Expression) {
        let result = validate(&flow_with_trigger(trigger), &snapshot(), &preset_registry_with_presets());

        assert_eq!(result.unwrap_err().issues(), [preset_not_found_at("unknown", Location::Trigger)]);
    }

    #[test]
    fn validate_reports_an_unknown_preset_in_a_conditional_node() {
        let flow = flow_with_node(FlowNodeKind::Conditional(preset_value("unknown")));

        let result = validate(&flow, &snapshot(), &preset_registry_with_presets());

        assert_eq!(result.unwrap_err().issues(), [preset_not_found_at("unknown", node(NODE_ID))]);
    }

    #[test]
    fn validate_reports_an_unknown_preset_in_a_control_device_action() {
        let flow = flow_with_node(control_device_with(KNOWN_DEVICE_ID, "brightness", preset_command("unknown")));

        let result = validate(&flow, &snapshot(), &preset_registry_with_presets());

        assert_eq!(result.unwrap_err().issues(), [preset_not_found_at("unknown", node(NODE_ID))]);
    }

    #[test]
    fn validate_reports_every_device_as_unknown_for_an_empty_snapshot() {
        let result = validate(
            &flow_with_trigger(property_changed(KNOWN_DEVICE_ID)),
            &StoreSnapshot::default(),
            &preset_registry(),
        );

        assert_eq!(result.unwrap_err().issues(), [unknown_device(KNOWN_DEVICE_ID)]);
    }

    #[test]
    fn validate_reports_incompatible_operands_in_the_trigger() {
        let trigger = EqualTo {
            lhs: Box::new(Literal { value: Value::Boolean(true) }),
            rhs: Box::new(Literal { value: Value::String("short_release".to_string()) }),
        };

        let result = validate(&flow_with_trigger(trigger), &snapshot(), &preset_registry());

        assert_eq!(
            result.unwrap_err().issues(),
            [incompatible_operands_at(
                "EqualTo",
                "operands of the same kind",
                vec![ValueKind::Boolean, ValueKind::Enum],
                Location::Trigger
            )]
        );
    }

    #[test]
    fn validate_reports_incompatible_operands_in_a_conditional_node() {
        let condition = And {
            lhs: Box::new(property_value(KNOWN_DEVICE_ID)),
            rhs: Box::new(property_value_of(KNOWN_DEVICE_ID, "brightness")),
        };

        let result = validate(&flow_with_node(FlowNodeKind::Conditional(condition)), &snapshot(), &preset_registry());

        assert_eq!(
            result.unwrap_err().issues(),
            [incompatible_operands_at("And", "Boolean", vec![ValueKind::Boolean, ValueKind::Number], node(NODE_ID))]
        );
    }

    #[test]
    fn validate_only_reports_the_unknown_device_when_the_operands_are_incompatible_too() {
        let trigger = And {
            lhs: Box::new(property_value("unknown")),
            rhs: Box::new(Literal { value: Value::Number(Number::PositiveInt(1)) }),
        };

        let result = validate(&flow_with_trigger(trigger), &snapshot(), &preset_registry());

        assert_eq!(result.unwrap_err().issues(), [unknown_device("unknown")]);
    }

    #[rstest]
    #[case::conditional(FlowNodeKind::Conditional(property_value(KNOWN_DEVICE_ID)))]
    #[case::control_device(control_device(KNOWN_DEVICE_ID, "on"))]
    #[case::action_without_validation(log())]
    #[case::sleep(FlowNodeKind::Sleep(Duration::from_secs(1)))]
    fn validate_accepts_a_node_that_only_references_known_devices_and_properties(#[case] kind: FlowNodeKind) {
        assert_eq!(validate(&flow_with_node(kind), &snapshot(), &preset_registry()), Ok(()));
    }

    #[rstest]
    #[case::conditional(FlowNodeKind::Conditional(property_value("unknown")))]
    #[case::conditional_nested(FlowNodeKind::Conditional(Not{ expression: Box::new(property_changed("unknown")) }))]
    #[case::control_device(control_device("unknown", "on"))]
    fn validate_reports_an_unknown_device_in_a_node(#[case] kind: FlowNodeKind) {
        let result = validate(&flow_with_node(kind), &snapshot(), &preset_registry());

        assert_eq!(result.unwrap_err().issues(), [unknown_device_at("unknown", node(NODE_ID))]);
    }

    #[rstest]
    #[case::conditional(FlowNodeKind::Conditional(property_value_of(KNOWN_DEVICE_ID, UNKNOWN_PROPERTY_ID)))]
    #[case::control_device(control_device(KNOWN_DEVICE_ID, UNKNOWN_PROPERTY_ID))]
    fn validate_reports_an_unknown_property_in_a_node(#[case] kind: FlowNodeKind) {
        let result = validate(&flow_with_node(kind), &snapshot(), &preset_registry());

        assert_eq!(
            result.unwrap_err().issues(),
            [unknown_property_at(KNOWN_DEVICE_ID, UNKNOWN_PROPERTY_ID, node(NODE_ID))]
        );
    }

    #[rstest]
    #[case::trigger(flow_with_trigger(property_changed_of(KNOWN_DEVICE_ID, READONLY_PROPERTY_ID)))]
    #[case::conditional(flow_with_node(FlowNodeKind::Conditional(property_value_of(
        KNOWN_DEVICE_ID,
        READONLY_PROPERTY_ID
    ))))]
    fn validate_accepts_reading_a_readonly_property(#[case] flow: Flow) {
        assert_eq!(validate(&flow, &snapshot(), &preset_registry()), Ok(()));
    }

    #[test]
    fn validate_reports_a_control_device_action_that_sets_a_readonly_property() {
        let result = validate(
            &flow_with_node(control_device(KNOWN_DEVICE_ID, READONLY_PROPERTY_ID)),
            &snapshot(),
            &preset_registry(),
        );

        assert_eq!(
            result.unwrap_err().issues(),
            [readonly_property_at(KNOWN_DEVICE_ID, READONLY_PROPERTY_ID, node(NODE_ID))]
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
    fn validate_property_command_accepts_a_value_that_matches_the_property(
        #[case] property_id: &str,
        #[case] command: PropertyCommand,
    ) {
        assert_eq!(validate_command(property_id, command), Ok(()));
    }

    #[test]
    fn validate_property_command_reports_an_unknown_property() {
        let result = validate_command(UNKNOWN_PROPERTY_ID, property_command(Value::Boolean(true)));

        assert_eq!(
            result,
            Err(Problem::UnknownProperty {
                device_id: KNOWN_DEVICE_ID.to_string(),
                property_id: UNKNOWN_PROPERTY_ID.to_string()
            })
        );
    }

    #[test]
    fn validate_property_command_reports_a_readonly_property() {
        let result = validate_command(READONLY_PROPERTY_ID, property_command(Value::Boolean(true)));

        assert_eq!(
            result,
            Err(Problem::ReadOnlyProperty {
                device_id: KNOWN_DEVICE_ID.to_string(),
                property_id: READONLY_PROPERTY_ID.to_string()
            })
        );
    }

    #[test]
    fn validate_property_command_reports_a_readonly_property_instead_of_an_incompatible_value() {
        let result = validate_command(READONLY_PROPERTY_ID, property_command(Value::Number(number(50))));

        assert_eq!(
            result,
            Err(Problem::ReadOnlyProperty {
                device_id: KNOWN_DEVICE_ID.to_string(),
                property_id: READONLY_PROPERTY_ID.to_string()
            })
        );
    }

    #[rstest]
    #[case::number_for_boolean("on", Value::Number(number(50)), ValueKind::Boolean, ValueKind::Number)]
    #[case::color_for_boolean("on", Value::Color(red()), ValueKind::Boolean, ValueKind::Color)]
    #[case::boolean_for_number("brightness", Value::Boolean(true), ValueKind::Number, ValueKind::Boolean)]
    fn validate_property_command_reports_an_incompatible_value(
        #[case] property_id: &str,
        #[case] value: Value,
        #[case] expected: ValueKind,
        #[case] actual: ValueKind,
    ) {
        let result = validate_command(property_id, property_command(value));

        assert_eq!(result, Err(incompatible_value(property_id, expected, actual)));
    }

    #[rstest]
    #[case::positive_int(number(0))]
    #[case::negative_int(Number::NegativeInt(-1))]
    #[case::float(Number::Float(0.5))]
    fn validate_property_command_reports_a_value_below_the_minimum(#[case] value: Number) {
        let result = validate_command("brightness", property_command(Value::Number(value)));

        assert_eq!(result, Err(value_too_small(value, number(1))));
    }

    #[rstest]
    #[case::positive_int(number(101))]
    #[case::float(Number::Float(100.5))]
    fn validate_property_command_reports_a_value_above_the_maximum(#[case] value: Number) {
        let result = validate_command("brightness", property_command(Value::Number(value)));

        assert_eq!(result, Err(value_too_large(value, number(100))));
    }

    #[test]
    fn validate_property_command_skips_the_range_for_a_value_that_is_only_known_at_runtime() {
        let command = PropertyCommand { value: property_value_of(KNOWN_DEVICE_ID, "temperature"), transition: None };

        assert_eq!(validate_command("brightness", command), Ok(()));
    }

    #[rstest]
    #[case::number("brightness", "dim")]
    #[case::boolean("on", "enabled")]
    #[case::color("color", "warm")]
    fn validate_property_command_accepts_a_preset_that_matches_the_property(
        #[case] property_id: &str,
        #[case] preset_id: &str,
    ) {
        assert_eq!(validate_command(property_id, preset_command(preset_id)), Ok(()));
    }

    #[test]
    fn validate_property_command_reports_an_unknown_preset() {
        let result = validate_command("brightness", preset_command("missing"));

        assert_eq!(result, Err(Problem::PresetNotFound { id: "missing".to_string() }));
    }

    #[rstest]
    #[case::number_for_boolean("on", "dim", ValueKind::Boolean, ValueKind::Number)]
    #[case::boolean_for_number("brightness", "enabled", ValueKind::Number, ValueKind::Boolean)]
    #[case::color_for_number("brightness", "warm", ValueKind::Number, ValueKind::Color)]
    fn validate_property_command_reports_a_preset_of_the_wrong_kind(
        #[case] property_id: &str,
        #[case] preset_id: &str,
        #[case] expected: ValueKind,
        #[case] actual: ValueKind,
    ) {
        let result = validate_command(property_id, preset_command(preset_id));

        assert_eq!(result, Err(incompatible_value(property_id, expected, actual)));
    }

    #[test]
    fn validate_property_command_reports_a_preset_below_the_minimum() {
        let result = validate_command("brightness", preset_command("too_dark"));

        assert_eq!(result, Err(value_too_small(number(0), number(1))));
    }

    #[test]
    fn validate_property_command_reports_a_preset_above_the_maximum() {
        let result = validate_command("brightness", preset_command("too_bright"));

        assert_eq!(result, Err(value_too_large(number(101), number(100))));
    }

    #[test]
    fn validate_reports_issues_in_the_trigger_first_then_in_the_nodes_in_order() {
        let nodes = vec![
            ("first", FlowNodeKind::Conditional(property_value("conditional_device"))),
            ("second", control_device("action_device", "on")),
        ];
        let flow = flow_with_nodes(property_changed("trigger_device"), nodes);

        let result = validate(&flow, &snapshot(), &preset_registry());

        assert_eq!(
            result.unwrap_err().issues(),
            [
                unknown_device("trigger_device"),
                unknown_device_at("conditional_device", node("first")),
                unknown_device_at("action_device", node("second"))
            ]
        );
    }

    #[test]
    fn validate_reports_the_same_unknown_device_once_per_reference() {
        let flow = flow_with_nodes(property_changed("unknown"), vec![(NODE_ID, control_device("unknown", "on"))]);

        let result = validate(&flow, &snapshot(), &preset_registry());

        assert_eq!(
            result.unwrap_err().issues(),
            [unknown_device("unknown"), unknown_device_at("unknown", node(NODE_ID))]
        );
    }
    #[test]
    fn from_issues_returns_ok_without_issues() {
        assert_eq!(FlowValidationError::from_issues(vec![]), Ok(()));
    }

    #[test]
    fn from_issues_returns_an_error_with_the_issues() {
        let result = FlowValidationError::from_issues(vec![unknown_device("first"), unknown_device("second")]);

        assert_eq!(result.unwrap_err().issues(), [unknown_device("first"), unknown_device("second")]);
    }

    #[test]
    fn flow_validation_error_displays_a_single_issue() {
        let error = FlowValidationError::from_issues(vec![unknown_device("first")]).unwrap_err();

        assert_eq!(error.to_string(), "flow has 1 validation issue");
    }

    #[test]
    fn flow_validation_error_displays_the_number_of_issues() {
        let error =
            FlowValidationError::from_issues(vec![unknown_device("first"), unknown_device("second")]).unwrap_err();

        assert_eq!(error.to_string(), "flow has 2 validation issues");
    }

    #[rstest]
    #[case::unknown_device(unknown_device("lamp"), "trigger: unknown device 'lamp'")]
    #[case::unknown_property(unknown_property("lamp", "on"), "trigger: unknown property 'on' for device 'lamp'")]
    #[case::node(unknown_device_at("lamp", node("turn_on")), "node 'turn_on': unknown device 'lamp'")]
    #[case::readonly_property(
        readonly_property_at("sensor", "motion", node("turn_on")),
        "node 'turn_on': readonly property 'motion' for device 'sensor'"
    )]
    #[case::incompatible_value(
        incompatible_value_at("lamp", "on", ValueKind::Boolean, ValueKind::Number, node("turn_on")),
        "node 'turn_on': incompatible value for property 'on' for device 'lamp': expected boolean, got number"
    )]
    #[case::value_too_small(
        value_too_small_at("lamp", "brightness", Number::PositiveInt(0), Number::PositiveInt(1), node("dim")),
        "node 'dim': value too small for property 'brightness' for device 'lamp': minimum 1, got 0"
    )]
    #[case::value_too_large(
        value_too_large_at("lamp", "brightness", Number::Float(100.5), Number::PositiveInt(100), node("dim")),
        "node 'dim': value too large for property 'brightness' for device 'lamp': maximum 100, got 100.5"
    )]
    fn validation_issue_displays_the_location_and_the_problem(#[case] issue: ValidationIssue, #[case] expected: &str) {
        assert_eq!(issue.to_string(), expected);
    }
}
