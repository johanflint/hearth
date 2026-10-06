use crate::flow_engine::Expression;
use crate::flow_engine::flow::Flow;
use crate::store::StoreSnapshot;
use std::fmt;
use std::fmt::Formatter;
use thiserror::Error;

/// Validates a flow against the given snapshot.
///
/// Performs semantic validation, it assumes the structural validation by the
/// `flow_loader::from_json` is already done.
pub fn validate(flow: Flow, snapshot: StoreSnapshot) -> Result<(), FlowValidationError> {
    let mut issues = vec![];

    for expression in flow.trigger().walk() {
        match expression {
            Expression::PropertyChanged { device_id, .. } |
            Expression::PropertyValue { device_id, .. } => {
                if !snapshot.devices.contains_key(device_id) {
                    issues.push(Problem::UnknownDevice { device_id: device_id.to_string() }.at(Location::Trigger));
                }
            }
            _ => {}
        }
    }

    FlowValidationError::from_issues(issues)
}

#[derive(Debug, Error, PartialEq)]
#[error("flow has {} validation issue(s)", .0.len())]
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
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Location::Trigger => write!(f, "trigger"),
        }
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum Problem {
    #[error("unknown device '{device_id}'")]
    UnknownDevice { device_id: String },
}

impl Problem {
    pub fn at(self, location: Location) -> ValidationIssue {
        ValidationIssue { location, problem: self }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow_engine::Expression::*;
    use crate::flow_engine::Value;
    use crate::flow_engine::flow::{FlowLink, FlowNode, FlowNodeKind};
    use crate::store::DeviceMap;
    use crate::test_support::DeviceBuilder;
    use rstest::rstest;
    use std::collections::HashMap;
    use std::sync::Arc;

    const KNOWN_DEVICE_ID: &str = "known";

    fn flow_with_trigger(trigger: Expression) -> Flow {
        let end_node = FlowNode::new("end_node".to_string(), vec![], FlowNodeKind::End);
        let start_node = FlowNode::new("start_node".to_string(), vec![FlowLink::new(Arc::new(end_node), Value::None)], FlowNodeKind::Start);

        Flow::new("flow".to_string(), "flow".to_string(), None, Some(trigger), Arc::new(start_node), HashMap::new()).unwrap()
    }

    fn snapshot() -> StoreSnapshot {
        let device = DeviceBuilder::new(KNOWN_DEVICE_ID).with_boolean_property("on", true).build();
        let devices: DeviceMap = HashMap::from([(device.id.clone(), Arc::new(device))]);
        StoreSnapshot { devices: Arc::new(devices) }
    }

    fn property_changed(device_id: &str) -> Expression {
        PropertyChanged { device_id: device_id.to_string(), property_id: "on".to_string() }
    }

    fn property_value(device_id: &str) -> Expression {
        PropertyValue { device_id: device_id.to_string(), property_id: "on".to_string() }
    }

    fn unknown_device(device_id: &str) -> ValidationIssue {
        Problem::UnknownDevice { device_id: device_id.to_string() }.at(Location::Trigger)
    }

    #[rstest]
    #[case::literal(Literal{ value: Value::Boolean(true) })]
    #[case::property_changed(property_changed(KNOWN_DEVICE_ID))]
    #[case::property_value(property_value(KNOWN_DEVICE_ID))]
    #[case::nested(And{ lhs: Box::new(property_changed(KNOWN_DEVICE_ID)), rhs: Box::new(Not { expression: Box::new(property_value(KNOWN_DEVICE_ID)) }) })]
    fn validate_accepts_a_trigger_that_only_references_known_devices(#[case] trigger: Expression) {
        assert_eq!(validate(flow_with_trigger(trigger), snapshot()), Ok(()));
    }

    #[rstest]
    #[case::property_changed(property_changed("unknown"))]
    #[case::property_value(property_value("unknown"))]
    #[case::nested_in_not(Not{ expression: Box::new(property_value("unknown")) })]
    #[case::next_to_a_known_device(Or{ lhs: Box::new(property_changed(KNOWN_DEVICE_ID)), rhs: Box::new(property_value("unknown")) })]
    fn validate_reports_an_unknown_device_in_the_trigger(#[case] trigger: Expression) {
        let result = validate(flow_with_trigger(trigger), snapshot());

        assert_eq!(result.unwrap_err().issues(), [unknown_device("unknown")]);
    }

    #[test]
    fn validate_reports_all_unknown_devices_in_the_trigger_in_order() {
        let trigger = And { lhs: Box::new(property_changed("first")), rhs: Box::new(property_value("second")) };

        let result = validate(flow_with_trigger(trigger), snapshot());

        assert_eq!(result.unwrap_err().issues(), [unknown_device("first"), unknown_device("second")]);
    }

    #[test]
    fn validate_reports_every_device_as_unknown_for_an_empty_snapshot() {
        let result = validate(flow_with_trigger(property_changed(KNOWN_DEVICE_ID)), StoreSnapshot::default());

        assert_eq!(result.unwrap_err().issues(), [unknown_device(KNOWN_DEVICE_ID)]);
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
    fn flow_validation_error_displays_the_number_of_issues() {
        let error = FlowValidationError::from_issues(vec![unknown_device("first"), unknown_device("second")]).unwrap_err();

        assert_eq!(error.to_string(), "flow has 2 validation issue(s)");
    }

    #[test]
    fn validation_issue_displays_the_location_and_the_problem() {
        assert_eq!(unknown_device("lamp").to_string(), "trigger: unknown device 'lamp'");
    }
}