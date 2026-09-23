use crate::domain::commands::Command;
use crate::domain::device::Device;
use crate::domain::{GeoLocation, controller_registry};
use crate::flow_engine;
use crate::flow_engine::flow::Flow;
use crate::flow_engine::property_value::{ConflictMergeSemantics, PropertyValue};
use crate::flow_engine::{Context, FlowEngineError, FlowExecutionReport};
use crate::scheduler::SchedulerCommand;
use crate::store::{PropertyChange, StoreSnapshot};
use futures::stream::FuturesUnordered;
use futures::stream::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;
use tracing::{instrument, warn};

type DeviceId = String;
type PropertyId = String;
type CommandMap = HashMap<DeviceId, HashMap<PropertyId, PropertyValue>>;

#[derive(Debug)]
struct ProposedWrite {
    origin: Arc<Flow>,
    value: PropertyValue,
}

type ProposalMap = HashMap<(DeviceId, PropertyId), Vec<ProposedWrite>>;

#[instrument(skip_all, fields(flow = flow.name(), node_id = node_id.as_deref().unwrap_or("<start>")))]
pub async fn execute_flow(flow: Arc<Flow>, node_id: Option<String>, snapshot: StoreSnapshot, tx: Sender<SchedulerCommand>, geo_location: GeoLocation) {
    let context = Context::builder().snapshot(snapshot.clone()).location(geo_location).build();
    let result = flow_engine::execute(&flow, node_id, &context, tx).await;

    let command_map = merge_command_maps(vec![(flow, result)]);
    dispatch_commands(&snapshot, command_map).await;
}

#[instrument(skip_all)]
pub async fn execute_flows(flows: Vec<Arc<Flow>>, snapshot: StoreSnapshot, changed: Option<PropertyChange>, tx: Sender<SchedulerCommand>, geo_location: GeoLocation) {
    let context = Context::builder().snapshot(snapshot.clone()).changed(changed).location(geo_location).build();
    let results = FuturesUnordered::from_iter(flows.into_iter().map(|flow| async {
        let result = flow_engine::execute(&flow, None, &context, tx.clone()).await;
        (flow, result)
    }))
    .collect::<Vec<_>>()
    .await;

    let command_map = merge_command_maps(results);
    dispatch_commands(&snapshot, command_map).await;
}

fn merge_command_maps(reports: Vec<(Arc<Flow>, Result<FlowExecutionReport, FlowEngineError>)>) -> CommandMap {
    let mut proposals: ProposalMap = HashMap::new();

    for (origin, report) in reports {
        let Ok(report) = report else { continue };

        if let Some(command_map) = report.take_from_scope::<CommandMap>("command_map") {
            for (device_id, properties) in command_map {
                for (property_id, value) in properties {
                    proposals
                        .entry((device_id.to_owned(), property_id))
                        .or_default()
                        .push(ProposedWrite { origin: origin.clone(), value });
                }
            }
        }
    }

    let mut merged_map: CommandMap = HashMap::new();
    for ((device_id, property_id), writes) in proposals {
        let Some(value) = merged_value(&writes) else {
            log_conflict(&device_id, &property_id, &writes);
            continue;
        };
        merged_map.entry(device_id).or_default().insert(property_id, value.clone());
    }

    merged_map
}

fn merged_value(writes: &[ProposedWrite]) -> Option<&PropertyValue> {
    let first = writes.first()?;

    if writes.len() == 1 {
        return Some(&first.value);
    }

    let all_equal = writes.iter().all(|w| w.value == first.value);
    (first.value.conflict_merge_semantics() == ConflictMergeSemantics::DeduplicateIfEqual && all_equal).then_some(&first.value)
}

fn log_conflict(device_id: &DeviceId, property_id: &PropertyId, writes: &[ProposedWrite]) {
    let mut proposals: Vec<String> = writes.iter().map(|w| format!("flow '{}' requested {:?}", w.origin.id(), w.value)).collect();
    proposals.sort_unstable();
    #[rustfmt::skip]
    warn!(device_id, property_id, proposals = ?proposals, "⚠️ Conflicting same-precedence writes; skipping this property, no command dispatched");
}

async fn dispatch_commands(snapshot: &StoreSnapshot, command_map: CommandMap) {
    for (device_id, properties) in command_map {
        if let Some(device) = snapshot.devices.get(&device_id) {
            let properties = filter_flow_editable_properties(device, properties);
            if properties.is_empty() {
                continue;
            }

            if let Some(controller) = device.controller_id.and_then(|controller_id| controller_registry::get(controller_id)) {
                let command = Command::ControlDevice {
                    device: device.clone(),
                    property: Arc::new(properties),
                };
                controller.execute(command).await;
            } else {
                warn!(device_id, "⚠️ Device '{}' is not tied to a controller", device.name);
            }
        }
    }
}

// Drops properties that a flow is not allowed to write (unknown and marked as readonly).
fn filter_flow_editable_properties(device: &Device, properties: HashMap<PropertyId, PropertyValue>) -> HashMap<PropertyId, PropertyValue> {
    properties
        .into_iter()
        .filter(|(property_id, _)| match device.properties.get(property_id) {
            Some(property) if property.readonly() => {
                warn!(device_id = device.id, property_id, "⚠️ Ignoring a command to set readonly property '{}' for device '{}'", property_id, device.name);
                false
            }
            Some(_) => true,
            None => {
                warn!(device_id = device.id, property_id, "⚠️ Ignoring a command to set unknown property '{}' for device '{}'", property_id, device.name);
                false
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Number;
    use crate::domain::color::Color;
    use crate::domain::property::{BooleanProperty, PropertyType};
    use crate::flow_engine::Value;
    use crate::flow_engine::flow::{FlowLink, FlowNode, FlowNodeKind};
    use crate::flow_engine::property_value::PropertyValue::*;
    use crate::test_support::DeviceBuilder;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use std::any::Any;
    use std::time::Duration;

    const DEVICE_ID: &str = "device_id";
    const DEVICE2_ID: &str = "device2_id";

    fn create_report(device_id: &str, properties: HashMap<String, PropertyValue>) -> FlowExecutionReport {
        let mut command_map = HashMap::new();
        command_map.insert(device_id.to_string(), properties);

        let mut scope: HashMap<String, Box<dyn Any + Send + Sync>> = HashMap::new();
        scope.insert("command_map".to_string(), Box::new(command_map));

        FlowExecutionReport::new(scope, Duration::from_millis(1))
    }

    fn create_flow(id: &str) -> Arc<Flow> {
        let end_node = FlowNode::new("end_node".to_string(), vec![], FlowNodeKind::End);
        let start_node = FlowNode::new("start_node".to_string(), vec![FlowLink::new(Arc::new(end_node), Value::None)], FlowNodeKind::Start);

        Arc::new(Flow::new(id.to_string(), id.to_string(), None, None, Arc::new(start_node), HashMap::new()).unwrap())
    }

    #[test]
    fn merge_command_maps_single_proposal_returns_property() {
        let report = create_report(DEVICE_ID, HashMap::from([("property_id".to_string(), SetBooleanValue(true))]));
        let flow = create_flow("flow");
        let result = merge_command_maps(vec![(flow, Ok(report))]);

        assert_eq!(
            result,
            HashMap::from([(DEVICE_ID.to_string(), HashMap::from([("property_id".to_string(), SetBooleanValue(true))]))])
        );
    }

    #[rstest]
    #[case::set_boolean_value(SetBooleanValue(true))]
    #[case::set_number_value(SetNumberValue(Number::PositiveInt(1)))]
    #[case::set_color(SetColor(Color::Hex("#000000".to_string())))]
    fn merge_command_maps_equal_absolute_proposals_deduplicate(#[case] property_value: PropertyValue) {
        let flow = create_flow("flow");
        let flow2 = create_flow("other_flow");
        let report = create_report(DEVICE_ID, HashMap::from([("property_id".to_string(), property_value.clone())]));
        let report2 = create_report(DEVICE_ID, HashMap::from([("property_id".to_string(), property_value.clone())]));

        let result = merge_command_maps(vec![(flow, Ok(report)), (flow2, Ok(report2))]);
        assert_eq!(
            result,
            HashMap::from([(DEVICE_ID.to_string(), HashMap::from([("property_id".to_string(), property_value)]))])
        );
    }

    #[test]
    fn merge_command_maps_three_equal_absolute_proposals_deduplicate() {
        let flow = create_flow("flow");
        let flow2 = create_flow("other_flow");
        let flow3 = create_flow("third_flow");
        let report = create_report(DEVICE_ID, HashMap::from([("property_id".to_string(), SetBooleanValue(true))]));
        let report2 = create_report(DEVICE_ID, HashMap::from([("property_id".to_string(), SetBooleanValue(true))]));
        let report3 = create_report(DEVICE_ID, HashMap::from([("property_id".to_string(), SetBooleanValue(true))]));

        let result = merge_command_maps(vec![(flow, Ok(report)), (flow2, Ok(report2)), (flow3, Ok(report3))]);
        assert_eq!(result, HashMap::from([(DEVICE_ID.to_string(), HashMap::from([("property_id".to_string(), SetBooleanValue(true))]))]));
    }

    #[test]
    fn merge_command_maps_different_absolute_proposals_skip_property() {
        let flow = create_flow("flow");
        let flow2 = create_flow("other_flow");
        let report = create_report(DEVICE_ID, HashMap::from([("property_id".to_string(), SetBooleanValue(true))]));
        let report2 = create_report(DEVICE_ID, HashMap::from([("property_id".to_string(), SetBooleanValue(false))]));

        let result = merge_command_maps(vec![(flow, Ok(report)), (flow2, Ok(report2))]);
        assert_eq!(result, HashMap::new());
    }

    #[test]
    fn merge_command_maps_different_relative_proposals_skip_property() {
        let flow = create_flow("flow");
        let flow2 = create_flow("other_flow");
        let report = create_report(DEVICE_ID, HashMap::from([("property_id".to_string(), IncrementNumberValue(Number::PositiveInt(42)))]));
        let report2 = create_report(DEVICE_ID, HashMap::from([("property_id".to_string(), IncrementNumberValue(Number::PositiveInt(42)))]));

        let result = merge_command_maps(vec![(flow, Ok(report)), (flow2, Ok(report2))]);
        assert_eq!(result, HashMap::new());
    }

    #[test]
    fn merge_command_maps_relevative_and_absolute_proposals_skip_property() {
        let flow = create_flow("flow");
        let flow2 = create_flow("other_flow");
        let report = create_report(DEVICE_ID, HashMap::from([("property_id".to_string(), SetNumberValue(Number::PositiveInt(42)))]));
        let report2 = create_report(DEVICE_ID, HashMap::from([("property_id".to_string(), IncrementNumberValue(Number::PositiveInt(42)))]));

        let result = merge_command_maps(vec![(flow, Ok(report)), (flow2, Ok(report2))]);
        assert_eq!(result, HashMap::new());
    }

    #[test]
    fn merge_command_maps_conflicting_property_preserves_independent_property() {
        let flow = create_flow("flow");
        let flow2 = create_flow("other_flow");
        let report = create_report(DEVICE_ID, HashMap::from([("property_id".to_string(), SetBooleanValue(true))]));
        let report2 = create_report(
            DEVICE_ID,
            HashMap::from([("property_id".to_string(), SetBooleanValue(false)), ("property2_id".to_string(), SetBooleanValue(false))]),
        );

        let result = merge_command_maps(vec![(flow, Ok(report)), (flow2, Ok(report2))]);
        assert_eq!(
            result,
            HashMap::from([(DEVICE_ID.to_string(), HashMap::from([("property2_id".to_string(), SetBooleanValue(false))]))])
        );
    }

    #[test]
    fn merge_command_maps_single_relative_proposal_returns_property() {
        let report = create_report(DEVICE_ID, HashMap::from([("property_id".to_string(), IncrementNumberValue(Number::PositiveInt(42)))]));
        let flow = create_flow("flow");
        let result = merge_command_maps(vec![(flow, Ok(report))]);

        assert_eq!(
            result,
            HashMap::from([(
                DEVICE_ID.to_string(),
                HashMap::from([("property_id".to_string(), IncrementNumberValue(Number::PositiveInt(42)))])
            )])
        );
    }

    #[test]
    fn merge_command_maps_different_devices_separately() {
        let flow = create_flow("flow");
        let flow2 = create_flow("other_flow");
        let report = create_report(DEVICE_ID, HashMap::from([("property_id".to_string(), IncrementNumberValue(Number::PositiveInt(42)))]));
        let report2 = create_report(DEVICE2_ID, HashMap::from([("property_id".to_string(), IncrementNumberValue(Number::PositiveInt(42)))]));

        let result = merge_command_maps(vec![(flow, Ok(report)), (flow2, Ok(report2))]);
        assert_eq!(
            result,
            HashMap::from([
                (
                    DEVICE_ID.to_string(),
                    HashMap::from([("property_id".to_string(), IncrementNumberValue(Number::PositiveInt(42)))])
                ),
                (
                    DEVICE2_ID.to_string(),
                    HashMap::from([("property_id".to_string(), IncrementNumberValue(Number::PositiveInt(42)))])
                ),
            ])
        );
    }

    #[test]
    fn merge_command_maps_empty_map() {
        let result = merge_command_maps(vec![]);
        assert_eq!(result, HashMap::new());
    }

    #[test]
    fn filter_flow_editable_properties_keeps_a_writable_property() {
        let device = DeviceBuilder::new(DEVICE_ID)
            .with_properties(vec![Box::new(BooleanProperty::new("on".to_string(), PropertyType::On, false, None, false))])
            .build();

        let properties = HashMap::from([("on".to_string(), SetBooleanValue(true))]);

        let result = filter_flow_editable_properties(&device, properties);

        assert_eq!(result, HashMap::from([("on".to_string(), SetBooleanValue(true))]));
    }

    #[test]
    fn filter_flow_editable_properties_drops_a_readonly_property() {
        let device = DeviceBuilder::new(DEVICE_ID)
            .with_properties(vec![Box::new(BooleanProperty::new("on".to_string(), PropertyType::On, true, None, false))])
            .build();

        let properties = HashMap::from([("on".to_string(), SetBooleanValue(true))]);

        let result = filter_flow_editable_properties(&device, properties);

        assert_eq!(result, HashMap::new());
    }

    #[test]
    fn filter_flow_editable_properties_drops_an_unknown_property() {
        let device = DeviceBuilder::new(DEVICE_ID)
            .with_properties(vec![Box::new(BooleanProperty::new("on".to_string(), PropertyType::On, true, None, false))])
            .build();

        let properties = HashMap::from([("does_not_exist".to_string(), SetBooleanValue(true))]);

        let result = filter_flow_editable_properties(&device, properties);

        assert_eq!(result, HashMap::new());
    }

    #[test]
    fn filter_flow_editable_properties_keeps_only_the_writable_properties_in_a_mixed_map() {
        let device = DeviceBuilder::new(DEVICE_ID)
            .with_properties(vec![
                Box::new(BooleanProperty::new("on".to_string(), PropertyType::On, false, None, false)),
                Box::new(BooleanProperty::new("brightness".to_string(), PropertyType::Brightness, true, None, false)),
            ])
            .build();
        let properties = HashMap::from([("on".to_string(), SetBooleanValue(true)), ("brightness".to_string(), SetBooleanValue(true))]);

        let result = filter_flow_editable_properties(&device, properties);

        assert_eq!(result, HashMap::from([("on".to_string(), SetBooleanValue(true))]));
    }
}
