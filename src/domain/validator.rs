use crate::domain::Number;
use crate::domain::device::Device;
use crate::domain::property::{Property, ValueKind};
use crate::flow_engine::Expression;
use crate::flow_engine::flow::{Flow, FlowNodeKind};
use crate::store::StoreSnapshot;
use std::fmt;
use std::fmt::Formatter;
use thiserror::Error;

/// Validates a flow against the given snapshot.
///
/// Performs semantic validation, it assumes the structural validation by the
/// `flow_loader::from_json` is already done.
pub fn validate(flow: &Flow, snapshot: &StoreSnapshot) -> Result<(), FlowValidationError> {
    let mut issues = vec![];

    issues.extend(validate_expression(snapshot, flow.trigger()).into_iter().map(|p| p.at(Location::Trigger)));

    for node in flow.walk() {
        let location = || Location::Node { node_id: node.id().to_string() };
        match node.kind() {
            FlowNodeKind::Start | FlowNodeKind::End | FlowNodeKind::Sleep(_) => {}
            FlowNodeKind::Conditional(expression) => {
                issues.extend(validate_expression(snapshot, expression).into_iter().map(|p| p.at(location())));
            }
            FlowNodeKind::Action(action_node) => {
                issues.extend(action_node.action().validate(snapshot).into_iter().map(|p| p.at(location())));
            }
        }
    }

    FlowValidationError::from_issues(issues)
}

pub fn find_device<'a>(device_id: &str, snapshot: &'a StoreSnapshot) -> Result<&'a Device, Problem> {
    snapshot.devices.get(device_id)
        .map(|device| device.as_ref())
        .ok_or_else(|| Problem::UnknownDevice { device_id: device_id.to_string() })
}

pub fn find_property<'a>(device: &'a Device, property_id: &str) -> Result<&'a dyn Property, Problem> {
    device.properties.get(property_id).map(|property| property.as_ref()).ok_or_else(|| Problem::UnknownProperty {
        device_id: device.id.clone(),
        property_id: property_id.to_string(),
    })
}

fn validate_expression(snapshot: &StoreSnapshot, expression: &Expression) -> Vec<Problem> {
    expression.walk()
        .filter_map(|expression| match expression {
            Expression::PropertyChanged { device_id, property_id } |
            Expression::PropertyValue { device_id, property_id } => {
                find_device(device_id, snapshot).and_then(|device| find_property(device, property_id)).err()
            }
            _ => None
        })
        .collect()
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

#[derive(Debug, PartialEq, Eq,PartialOrd, Ord)]
pub enum Location {
    Trigger,
    Node { node_id: String },
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Location::Trigger => write!(f, "trigger"),
            Location::Node { node_id } => write!(f, "node '{}'", node_id),
        }
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum Problem {
    #[error("unknown device '{device_id}'")]
    UnknownDevice { device_id: String },
    #[error("unknown property '{property_id}' for device '{device_id}'")]
    UnknownProperty { device_id: String, property_id: String },
    #[error("readonly property '{property_id}' for device '{device_id}'")]
    ReadOnlyProperty { device_id: String, property_id: String },
    #[error("incompatible value for property '{property_id}' for device '{device_id}': expected {expected}, got {actual}")]
    IncompatibleValue { device_id: String, property_id: String, expected: ValueKind, actual: ValueKind },
    #[error("incompatible operands for '{operator}': expected {expected}, got {}", .actual.iter().map(ToString::to_string).collect::<Vec<_>>().join(" and "))]
    IncompatibleOperands { operator: &'static str, expected: &'static str, actual: Vec<ValueKind> },
    #[error("value too small for property '{property_id}' for device '{device_id}': minimum {minimum}, got {value}")]
    ValueTooSmall { device_id: String, property_id: String, value: Number, minimum: Number },
    #[error("value too large for property '{property_id}' for device '{device_id}': maximum {maximum}, got {value}")]
    ValueTooLarge { device_id: String, property_id: String, value: Number, maximum: Number },
}

impl Problem {
    pub fn at(self, location: Location) -> ValidationIssue {
        ValidationIssue { location, problem: self }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Number;
    use crate::domain::property::{BooleanProperty, NumberProperty, PropertyType};
    use crate::flow_engine::Expression::*;
    use crate::flow_engine::Value;
    use crate::flow_engine::action::{ControlDeviceAction, LogAction};
    use crate::flow_engine::flow::{ActionFlowNode, FlowLink, FlowNode, FlowNodeKind};
    use crate::flow_engine::property_command::{Operation, PropertyCommand};
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
        let start_node = FlowNode::new("start_node".to_string(), vec![FlowLink::new(Arc::new(end_node), Value::None)], FlowNodeKind::Start);

        Flow::new("flow".to_string(), "flow".to_string(), None, Some(trigger), Arc::new(start_node), HashMap::new()).unwrap()
    }

    /// Builds `start -> nodes[0] -> nodes[1] -> ... -> end`, the nodes are given as `(id, kind)`.
    fn flow_with_nodes(trigger: Expression, nodes: Vec<(&str, FlowNodeKind)>) -> Flow {
        let end_node = FlowNode::new("end_node".to_string(), vec![], FlowNodeKind::End);
        let first_node = nodes.into_iter().rev().fold(end_node, |next, (id, kind)| {
            FlowNode::new(id.to_string(), vec![FlowLink::new(Arc::new(next), Value::None)], kind)
        });
        let start_node = FlowNode::new("start_node".to_string(), vec![FlowLink::new(Arc::new(first_node), Value::None)], FlowNodeKind::Start);

        Flow::new("flow".to_string(), "flow".to_string(), None, Some(trigger), Arc::new(start_node), HashMap::new()).unwrap()
    }

    fn flow_with_node(kind: FlowNodeKind) -> Flow {
        flow_with_nodes(Literal { value: Value::Boolean(true) }, vec![(NODE_ID, kind)])
    }

    fn control_device(device_id: &str, property_id: &str) -> FlowNodeKind {
        control_device_with(device_id, property_id, property_command(Operation::Set, Value::Boolean(true)))
    }

    fn control_device_with(device_id: &str, property_id: &str, value: PropertyCommand) -> FlowNodeKind {
        let property = HashMap::from([(property_id.to_string(), value)]);
        FlowNodeKind::Action(ActionFlowNode::new(Box::new(ControlDeviceAction::new(device_id.to_string(), property))))
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
                Box::new(BooleanProperty::new(READONLY_PROPERTY_ID.to_string(), PropertyType::Motion, true, None, false)),
                Box::new(NumberProperty::builder("brightness".to_string(), PropertyType::Brightness, false).positive_int(Some(50), Some(1), Some(100)).build()),
            ])
            .build();

        let devices: DeviceMap = HashMap::from([(device.id.clone(), Arc::new(device))]);
        StoreSnapshot { devices: Arc::new(devices) }
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
        Problem::ReadOnlyProperty { device_id: device_id.to_string(), property_id: property_id.to_string() }.at(location)
    }

    fn value_too_small_at(device_id: &str, property_id: &str, value: Number, minimum: Number, location: Location) -> ValidationIssue {
        Problem::ValueTooSmall { device_id: device_id.to_string(), property_id: property_id.to_string(), value, minimum }.at(location)
    }

    fn value_too_large_at(device_id: &str, property_id: &str, value: Number, maximum: Number, location: Location) -> ValidationIssue {
        Problem::ValueTooLarge { device_id: device_id.to_string(), property_id: property_id.to_string(), value, maximum }.at(location)
    }

    fn incompatible_value_at(device_id: &str, property_id: &str, expected: ValueKind, actual: ValueKind, location: Location) -> ValidationIssue {
        Problem::IncompatibleValue { device_id: device_id.to_string(), property_id: property_id.to_string(), expected, actual }.at(location)
    }

    #[rstest]
    #[case::literal(Literal{ value: Value::Boolean(true) })]
    #[case::property_changed(property_changed(KNOWN_DEVICE_ID))]
    #[case::property_value(property_value(KNOWN_DEVICE_ID))]
    #[case::nested(And{ lhs: Box::new(property_changed(KNOWN_DEVICE_ID)), rhs: Box::new(Not { expression: Box::new(property_value(KNOWN_DEVICE_ID)) }) })]
    fn validate_accepts_a_trigger_that_only_references_known_devices_and_properties(#[case] trigger: Expression) {
        assert_eq!(validate(&flow_with_trigger(trigger), &snapshot()), Ok(()));
    }

    #[rstest]
    #[case::property_changed(property_changed("unknown"))]
    #[case::property_value(property_value("unknown"))]
    #[case::nested_in_not(Not{ expression: Box::new(property_value("unknown")) })]
    #[case::next_to_a_known_device(Or{ lhs: Box::new(property_changed(KNOWN_DEVICE_ID)), rhs: Box::new(property_value("unknown")) })]
    fn validate_reports_an_unknown_device_in_the_trigger(#[case] trigger: Expression) {
        let result = validate(&flow_with_trigger(trigger), &snapshot());

        assert_eq!(result.unwrap_err().issues(), [unknown_device("unknown")]);
    }

    #[rstest]
    #[case::property_changed(property_changed_of(KNOWN_DEVICE_ID, UNKNOWN_PROPERTY_ID))]
    #[case::property_value(property_value_of(KNOWN_DEVICE_ID, UNKNOWN_PROPERTY_ID))]
    #[case::nested_in_not(Not{ expression: Box::new(property_value_of(KNOWN_DEVICE_ID, UNKNOWN_PROPERTY_ID)) })]
    #[case::next_to_a_known_property(And{ lhs: Box::new(property_changed(KNOWN_DEVICE_ID)), rhs: Box::new(property_value_of(KNOWN_DEVICE_ID, UNKNOWN_PROPERTY_ID)) })]
    fn validate_reports_an_unknown_property_in_the_trigger(#[case] trigger: Expression) {
        let result = validate(&flow_with_trigger(trigger), &snapshot());

        assert_eq!(result.unwrap_err().issues(), [unknown_property(KNOWN_DEVICE_ID, UNKNOWN_PROPERTY_ID)]);
    }

    #[test]
    fn validate_only_reports_the_unknown_device_when_its_property_is_unknown_too() {
        let result = validate(&flow_with_trigger(property_changed_of("unknown", UNKNOWN_PROPERTY_ID)), &snapshot());

        assert_eq!(result.unwrap_err().issues(), [unknown_device("unknown")]);
    }

    #[test]
    fn validate_reports_unknown_devices_and_properties_in_the_trigger_in_order() {
        let trigger = Or { lhs: Box::new(property_value_of(KNOWN_DEVICE_ID, UNKNOWN_PROPERTY_ID)), rhs: Box::new(property_changed("unknown")) };

        let result = validate(&flow_with_trigger(trigger), &snapshot());

        assert_eq!(result.unwrap_err().issues(), [unknown_property(KNOWN_DEVICE_ID, UNKNOWN_PROPERTY_ID), unknown_device("unknown")]);
    }
    
    #[test]
    fn validate_reports_all_unknown_devices_in_the_trigger_in_order() {
        let trigger = And { lhs: Box::new(property_changed("first")), rhs: Box::new(property_value("second")) };

        let result = validate(&flow_with_trigger(trigger), &snapshot());

        assert_eq!(result.unwrap_err().issues(), [unknown_device("first"), unknown_device("second")]);
    }

    #[test]
    fn validate_reports_every_device_as_unknown_for_an_empty_snapshot() {
        let result = validate(&flow_with_trigger(property_changed(KNOWN_DEVICE_ID)), &StoreSnapshot::default());

        assert_eq!(result.unwrap_err().issues(), [unknown_device(KNOWN_DEVICE_ID)]);
    }

    #[rstest]
    #[case::conditional(FlowNodeKind::Conditional(property_value(KNOWN_DEVICE_ID)))]
    #[case::control_device(control_device(KNOWN_DEVICE_ID, "on"))]
    #[case::action_without_validation(log())]
    #[case::sleep(FlowNodeKind::Sleep(Duration::from_secs(1)))]
    fn validate_accepts_a_node_that_only_references_known_devices_and_properties(#[case] kind: FlowNodeKind) {
        assert_eq!(validate(&flow_with_node(kind), &snapshot()), Ok(()));
    }

    #[rstest]
    #[case::conditional(FlowNodeKind::Conditional(property_value("unknown")))]
    #[case::conditional_nested(FlowNodeKind::Conditional(Not{ expression: Box::new(property_changed("unknown")) }))]
    #[case::control_device(control_device("unknown", "on"))]
    fn validate_reports_an_unknown_device_in_a_node(#[case] kind: FlowNodeKind) {
        let result = validate(&flow_with_node(kind), &snapshot());

        assert_eq!(result.unwrap_err().issues(), [unknown_device_at("unknown", node(NODE_ID))]);
    }

    #[rstest]
    #[case::conditional(FlowNodeKind::Conditional(property_value_of(KNOWN_DEVICE_ID, UNKNOWN_PROPERTY_ID)))]
    #[case::control_device(control_device(KNOWN_DEVICE_ID, UNKNOWN_PROPERTY_ID))]
    fn validate_reports_an_unknown_property_in_a_node(#[case] kind: FlowNodeKind) {
        let result = validate(&flow_with_node(kind), &snapshot());

        assert_eq!(result.unwrap_err().issues(), [unknown_property_at(KNOWN_DEVICE_ID, UNKNOWN_PROPERTY_ID, node(NODE_ID))]);
    }

    #[rstest]
    #[case::trigger(flow_with_trigger(property_changed_of(KNOWN_DEVICE_ID, READONLY_PROPERTY_ID)))]
    #[case::conditional(flow_with_node(FlowNodeKind::Conditional(property_value_of(KNOWN_DEVICE_ID, READONLY_PROPERTY_ID))))]
    fn validate_accepts_reading_a_readonly_property(#[case] flow: Flow) {
        assert_eq!(validate(&flow, &snapshot()), Ok(()));
    }

    #[test]
    fn validate_reports_a_control_device_action_that_sets_a_readonly_property() {
        let result = validate(&flow_with_node(control_device(KNOWN_DEVICE_ID, READONLY_PROPERTY_ID)), &snapshot());

        assert_eq!(result.unwrap_err().issues(), [readonly_property_at(KNOWN_DEVICE_ID, READONLY_PROPERTY_ID, node(NODE_ID))]);
    }

    #[test]
    fn validate_reports_a_control_device_action_that_sets_an_incompatible_value() {
        let result = validate(&flow_with_node(control_device_with(KNOWN_DEVICE_ID, "on", property_command(Operation::Set, Value::Number(Number::PositiveInt(50))))), &snapshot());

        assert_eq!(result.unwrap_err().issues(), [incompatible_value_at(KNOWN_DEVICE_ID, "on", ValueKind::Boolean, ValueKind::Number, node(NODE_ID))]);
    }

    #[rstest]
    #[case::too_small(Number::PositiveInt(0), value_too_small_at(KNOWN_DEVICE_ID, "brightness", Number::PositiveInt(0), Number::PositiveInt(1), node(NODE_ID)))]
    #[case::too_large(Number::PositiveInt(101), value_too_large_at(KNOWN_DEVICE_ID, "brightness", Number::PositiveInt(101), Number::PositiveInt(100), node(NODE_ID)))]
    fn validate_reports_a_control_device_action_that_sets_a_value_out_of_range(#[case] value: Number, #[case] expected: ValidationIssue) {
        let result = validate(&flow_with_node(control_device_with(KNOWN_DEVICE_ID, "brightness", property_command(Operation::Set, Value::Number(value)))), &snapshot());

        assert_eq!(result.unwrap_err().issues(), [expected]);
    }
    
    #[test]
    fn validate_reports_issues_in_the_trigger_first_then_in_the_nodes_in_order() {
        let nodes = vec![
            ("first", FlowNodeKind::Conditional(property_value("conditional_device"))),
            ("second", control_device("action_device", "on")),
        ];
        let flow = flow_with_nodes(property_changed("trigger_device"), nodes);

        let result = validate(&flow, &snapshot());

        assert_eq!(
            result.unwrap_err().issues(),
            [unknown_device("trigger_device"), unknown_device_at("conditional_device", node("first")), unknown_device_at("action_device", node("second"))]
        );
    }

    #[test]
    fn validate_reports_the_same_unknown_device_once_per_reference() {
        let flow = flow_with_nodes(property_changed("unknown"), vec![(NODE_ID, control_device("unknown", "on"))]);

        let result = validate(&flow, &snapshot());

        assert_eq!(result.unwrap_err().issues(), [unknown_device("unknown"), unknown_device_at("unknown", node(NODE_ID))]);
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
        let error = FlowValidationError::from_issues(vec![unknown_device("first"), unknown_device("second")]).unwrap_err();

        assert_eq!(error.to_string(), "flow has 2 validation issues");
    }

    #[rstest]
    #[case::unknown_device(unknown_device("lamp"), "trigger: unknown device 'lamp'")]
    #[case::unknown_property(unknown_property("lamp", "on"), "trigger: unknown property 'on' for device 'lamp'")]
    #[case::node(unknown_device_at("lamp", node("turn_on")), "node 'turn_on': unknown device 'lamp'")]
    #[case::readonly_property(readonly_property_at("sensor", "motion", node("turn_on")), "node 'turn_on': readonly property 'motion' for device 'sensor'")]
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