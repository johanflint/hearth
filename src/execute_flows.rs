use crate::domain::commands::Command;
use crate::domain::device::Device;
use crate::domain::{ClaimMode, GeoLocation, controller_registry};
use crate::flow_engine::flow::Flow;
use crate::flow_engine::property_command::ResolvedPropertyCommand;
use crate::flow_engine::{CommandMap, Context, FlowEngineError, FlowExecutionReport};
use crate::scheduler::SchedulerCommand;
use crate::store::{PropertyChange, StoreSnapshot};
use crate::{claim_registry, flow_engine};
use futures::stream::FuturesUnordered;
use futures::stream::StreamExt;
use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;
use tracing::{info, instrument, warn};

type DeviceId = String;
type PropertyId = String;

#[derive(Debug)]
struct Proposal<T> {
    origin: Arc<Flow>,
    value: T,
}

type ProposalMap = HashMap<(DeviceId, PropertyId), Vec<Proposal<ResolvedPropertyCommand>>>;
type ClaimProposalMap = HashMap<DeviceId, Vec<Proposal<ClaimMode>>>;

#[instrument(skip_all, fields(flow = flow.name(), node_id = node_id.as_deref().unwrap_or("<start>")))]
pub async fn execute_flow(flow: Arc<Flow>, node_id: Option<String>, snapshot: StoreSnapshot, tx: Sender<SchedulerCommand>, geo_location: GeoLocation) {
    let context = Context::builder().snapshot(snapshot.clone()).location(geo_location).build();
    let result = flow_engine::execute(flow.clone(), node_id, &context, tx).await;

    let command_map = merge_command_maps(vec![(flow, result)]);
    dispatch_commands(&snapshot, command_map).await;
}

#[instrument(skip_all)]
pub async fn execute_flows(flows: Vec<Arc<Flow>>, snapshot: StoreSnapshot, changed: Option<PropertyChange>, tx: Sender<SchedulerCommand>, geo_location: GeoLocation) {
    let context = Context::builder().snapshot(snapshot.clone()).changed(changed).location(geo_location).build();
    let results = FuturesUnordered::from_iter(flows.into_iter().map(|flow| async {
        let result = flow_engine::execute(flow.clone(), None, &context, tx.clone()).await;
        (flow, result)
    }))
    .collect::<Vec<_>>()
    .await;

    let command_map = merge_command_maps(results);
    dispatch_commands(&snapshot, command_map).await;
}

fn merge_command_maps(reports: Vec<(Arc<Flow>, Result<FlowExecutionReport, FlowEngineError>)>) -> CommandMap {
    let mut proposals: ProposalMap = HashMap::new();
    let mut claim_proposals: ClaimProposalMap = HashMap::new();

    for (origin, report) in reports {
        let Ok(report) = report else { continue };

        if let Some(command_map) = report.take_from_scope::<CommandMap>("command_map") {
            for (device_id, device_command) in command_map {
                if let Some(claim) = device_command.claim {
                    claim_proposals.entry(device_id.clone()).or_default().push(Proposal { origin: origin.clone(), value: claim })
                }

                for (property_id, command) in device_command.properties {
                    proposals
                        .entry((device_id.clone(), property_id))
                        .or_default()
                        .push(Proposal { origin: origin.clone(), value: command });
                }
            }
        }
    }

    let mut merged_map: CommandMap = HashMap::new();
    for ((device_id, property_id), writes) in proposals {
        let Some(command) = agreed_value(&writes) else {
            log_conflict(&device_id, &property_id, &writes);
            continue;
        };
        merged_map.entry(device_id).or_default().properties.insert(property_id, command.clone());
    }

    for (device_id, claims) in claim_proposals {
        let Some(claim) = agreed_value(&claims) else {
            log_claim_conflict(&device_id, &claims);
            continue;
        };
        merged_map.entry(device_id).or_default().claim = Some(*claim);
    }

    merged_map
}

fn agreed_value<T: PartialEq>(proposals: &[Proposal<T>]) -> Option<&T> {
    let first = proposals.first()?;
    proposals.iter().all(|p| p.value == first.value).then_some(&first.value)
}

fn log_conflict(device_id: &DeviceId, property_id: &PropertyId, writes: &[Proposal<ResolvedPropertyCommand>]) {
    #[rustfmt::skip]
    warn!(device_id, property_id, proposals = ?describe(writes), "⚠️ Conflicting same-precedence writes; skipping this property, no command dispatched");
}

fn log_claim_conflict(device_id: &DeviceId, claims: &[Proposal<ClaimMode>]) {
    warn!(device_id, proposals = ?describe(claims), "⚠️ Conflicting claims; leaving the claim of this device unchanged");
}

fn describe<T: Debug>(proposals: &[Proposal<T>]) -> Vec<String> {
    let mut descriptions: Vec<String> = proposals.iter().map(|p| format!("flow '{}' requested {:?}", p.origin.id(), p.value)).collect();
    descriptions.sort_unstable();
    descriptions
}

async fn dispatch_commands(snapshot: &StoreSnapshot, command_map: CommandMap) {
    for (device_id, device_command) in command_map {
        if let Some(device) = snapshot.devices.get(&device_id) {
            if !claim_registry::authorize(&device_id, device_command.claim) {
                info!(device_id, "Device '{}' is claimed, skipping writes without a claim", device.name);
                continue;
            }

            let properties = filter_flow_editable_properties(device, device_command.properties);
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
fn filter_flow_editable_properties(device: &Device, properties: HashMap<PropertyId, ResolvedPropertyCommand>) -> HashMap<PropertyId, ResolvedPropertyCommand> {
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
    use crate::flow_engine::action::DeviceCommand;
    use crate::flow_engine::flow::{FlowLink, FlowNode, FlowNodeKind};
    use crate::test_support::{DeviceBuilder, resolved_property_command};
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use std::any::Any;
    use std::time::Duration;

    const DEVICE_ID: &str = "device_id";
    const DEVICE2_ID: &str = "device2_id";

    fn create_report(device_id: &str, properties: HashMap<String, ResolvedPropertyCommand>) -> FlowExecutionReport {
        create_report_with_claim(device_id, None, properties)
    }

    fn create_report_with_claim(device_id: &str, claim: Option<ClaimMode>, properties: HashMap<String, ResolvedPropertyCommand>) -> FlowExecutionReport {
        let command_map: CommandMap = HashMap::from([(device_id.to_string(), DeviceCommand { claim, properties })]);

        let mut scope: HashMap<String, Box<dyn Any + Send + Sync>> = HashMap::new();
        scope.insert("command_map".to_string(), Box::new(command_map));

        FlowExecutionReport::new(scope, Duration::from_millis(1))
    }

    fn create_flow(id: &str) -> Arc<Flow> {
        let end_node = FlowNode::new("end_node".to_string(), vec![], FlowNodeKind::End);
        let start_node = FlowNode::new("start_node".to_string(), vec![FlowLink::new(Arc::new(end_node), Value::None)], FlowNodeKind::Start);

        Arc::new(Flow::new(id.to_string(), id.to_string(), None, None, Arc::new(start_node), HashMap::new()).unwrap())
    }

    fn device_command(properties: HashMap<String, ResolvedPropertyCommand>) -> DeviceCommand {
        DeviceCommand { claim: None, properties }
    }

    fn on_command() -> HashMap<String, ResolvedPropertyCommand> {
        HashMap::from([("on".to_string(), resolved_property_command(Value::Boolean(true)))])
    }

    #[test]
    fn merge_command_maps_single_proposal_returns_property() {
        let report = create_report(DEVICE_ID, HashMap::from([("property_id".to_string(), resolved_property_command(Value::Boolean(true)))]));
        let flow = create_flow("flow");
        let result = merge_command_maps(vec![(flow, Ok(report))]);

        assert_eq!(
            result,
            HashMap::from([(DEVICE_ID.to_string(), device_command(HashMap::from([("property_id".to_string(), resolved_property_command(Value::Boolean(true)))])))])
        );
    }

    #[rstest]
    #[case::boolean(Value::Boolean(true))]
    #[case::number(Value::Number(Number::PositiveInt(1)))]
    #[case::color(Value::Color(Color::Hex("#000000".to_string())))]
    fn merge_command_maps_equal_absolute_proposals_deduplicate(#[case] value: Value) {
        let flow = create_flow("flow");
        let flow2 = create_flow("other_flow");
        let report = create_report(DEVICE_ID, HashMap::from([("property_id".to_string(), resolved_property_command(value.clone()))]));
        let report2 = create_report(DEVICE_ID, HashMap::from([("property_id".to_string(), resolved_property_command(value.clone()))]));

        let result = merge_command_maps(vec![(flow, Ok(report)), (flow2, Ok(report2))]);
        assert_eq!(
            result,
            HashMap::from([(DEVICE_ID.to_string(), device_command(HashMap::from([("property_id".to_string(), resolved_property_command(value))])))])
        );
    }

    #[test]
    fn merge_command_maps_three_equal_absolute_proposals_deduplicate() {
        let flow = create_flow("flow");
        let flow2 = create_flow("other_flow");
        let flow3 = create_flow("third_flow");
        let report = create_report(DEVICE_ID, HashMap::from([("property_id".to_string(), resolved_property_command(Value::Boolean(true)))]));
        let report2 = create_report(DEVICE_ID, HashMap::from([("property_id".to_string(), resolved_property_command(Value::Boolean(true)))]));
        let report3 = create_report(DEVICE_ID, HashMap::from([("property_id".to_string(), resolved_property_command(Value::Boolean(true)))]));

        let result = merge_command_maps(vec![(flow, Ok(report)), (flow2, Ok(report2)), (flow3, Ok(report3))]);
        assert_eq!(result, HashMap::from([(DEVICE_ID.to_string(), device_command(HashMap::from([("property_id".to_string(), resolved_property_command(Value::Boolean(true)))])))]));
    }

    #[test]
    fn merge_command_maps_different_absolute_proposals_skip_property() {
        let flow = create_flow("flow");
        let flow2 = create_flow("other_flow");
        let report = create_report(DEVICE_ID, HashMap::from([("property_id".to_string(), resolved_property_command(Value::Boolean(true)))]));
        let report2 = create_report(DEVICE_ID, HashMap::from([("property_id".to_string(), resolved_property_command(Value::Boolean(false)))]));

        let result = merge_command_maps(vec![(flow, Ok(report)), (flow2, Ok(report2))]);
        assert_eq!(result, HashMap::new());
    }

    #[test]
    fn merge_command_maps_equal_proposals_with_equal_transitions_deduplicate() {
        let flow = create_flow("flow");
        let flow2 = create_flow("other_flow");
        let command = ResolvedPropertyCommand { value: Value::Boolean(false), transition: Some(Duration::from_secs(600)) };
        let report = create_report(DEVICE_ID, HashMap::from([("property_id".to_string(), command.clone())]));
        let report2 = create_report(DEVICE_ID, HashMap::from([("property_id".to_string(), command.clone())]));

        let result = merge_command_maps(vec![(flow, Ok(report)), (flow2, Ok(report2))]);
        assert_eq!(result, HashMap::from([(DEVICE_ID.to_string(), device_command(HashMap::from([("property_id".to_string(), command)])))]));
    }

    #[rstest]
    #[case::different_transitions(Some(Duration::from_secs(600)), Some(Duration::from_secs(60)))]
    #[case::transition_and_no_transition(Some(Duration::from_secs(600)), None)]
    fn merge_command_maps_equal_values_with_different_transitions_skip_property(#[case] transition: Option<Duration>, #[case] transition2: Option<Duration>) {
        let flow = create_flow("flow");
        let flow2 = create_flow("other_flow");
        let report = create_report(DEVICE_ID, HashMap::from([("property_id".to_string(), ResolvedPropertyCommand { value: Value::Boolean(false), transition })]));
        let report2 = create_report(DEVICE_ID, HashMap::from([("property_id".to_string(), ResolvedPropertyCommand { value: Value::Boolean(false), transition: transition2 })]));

        let result = merge_command_maps(vec![(flow, Ok(report)), (flow2, Ok(report2))]);
        assert_eq!(result, HashMap::new());
    }

    #[test]
    fn merge_command_maps_conflicting_property_preserves_independent_property() {
        let flow = create_flow("flow");
        let flow2 = create_flow("other_flow");
        let report = create_report(DEVICE_ID, HashMap::from([("property_id".to_string(), resolved_property_command(Value::Boolean(true)))]));
        let report2 = create_report(DEVICE_ID, HashMap::from([
            ("property_id".to_string(), resolved_property_command(Value::Boolean(false))),
            ("property2_id".to_string(), resolved_property_command(Value::Boolean(false)))]),
        );

        let result = merge_command_maps(vec![(flow, Ok(report)), (flow2, Ok(report2))]);
        assert_eq!(
            result,
            HashMap::from([(DEVICE_ID.to_string(), device_command(HashMap::from([("property2_id".to_string(), resolved_property_command(Value::Boolean(false)))])))])
        );
    }

    #[test]
    fn merge_command_maps_different_devices_separately() {
        let flow = create_flow("flow");
        let flow2 = create_flow("other_flow");
        let report = create_report(DEVICE_ID, HashMap::from([("property_id".to_string(), resolved_property_command(Value::Number(Number::PositiveInt(42))))]));
        let report2 = create_report(DEVICE2_ID, HashMap::from([("property_id".to_string(), resolved_property_command(Value::Number(Number::PositiveInt(42))))]));

        let result = merge_command_maps(vec![(flow, Ok(report)), (flow2, Ok(report2))]);
        assert_eq!(
            result,
            HashMap::from([
                (
                    DEVICE_ID.to_string(),
                    device_command(HashMap::from([("property_id".to_string(), resolved_property_command(Value::Number(Number::PositiveInt(42))))]))
                ),
                (
                    DEVICE2_ID.to_string(),
                    device_command(HashMap::from([("property_id".to_string(), resolved_property_command(Value::Number(Number::PositiveInt(42))))]))
                ),
            ])
        );
    }

    #[rstest]
    #[case::claim(ClaimMode::Claim)]
    #[case::release(ClaimMode::Release)]
    fn merge_command_maps_single_claim_is_kept(#[case] claim: ClaimMode) {
        let report = create_report_with_claim(DEVICE_ID, Some(claim), on_command());

        let result = merge_command_maps(vec![(create_flow("flow"), Ok(report))]);

        assert_eq!(result, HashMap::from([(DEVICE_ID.to_string(), DeviceCommand { claim: Some(claim), properties: on_command() })]));
    }

    #[test]
    fn merge_command_maps_claim_without_properties_is_kept() {
        let report = create_report_with_claim(DEVICE_ID, Some(ClaimMode::Release), HashMap::new());

        let result = merge_command_maps(vec![(create_flow("flow"), Ok(report))]);

        assert_eq!(result, HashMap::from([(DEVICE_ID.to_string(), DeviceCommand { claim: Some(ClaimMode::Release), properties: HashMap::new() })]));
    }

    #[test]
    fn merge_command_maps_equal_claims_deduplicate() {
        let report = create_report_with_claim(DEVICE_ID, Some(ClaimMode::Claim), on_command());
        let report2 = create_report_with_claim(DEVICE_ID, Some(ClaimMode::Claim), on_command());

        let result = merge_command_maps(vec![(create_flow("flow"), Ok(report)), (create_flow("other_flow"), Ok(report2))]);

        assert_eq!(result, HashMap::from([(DEVICE_ID.to_string(), DeviceCommand { claim: Some(ClaimMode::Claim), properties: on_command() })]));
    }

    #[test]
    fn merge_command_maps_a_flow_without_claim_does_not_conflict_with_a_claim() {
        let report = create_report_with_claim(DEVICE_ID, Some(ClaimMode::Claim), on_command());
        let report2 = create_report(DEVICE_ID, on_command());

        let result = merge_command_maps(vec![(create_flow("flow"), Ok(report)), (create_flow("other_flow"), Ok(report2))]);

        assert_eq!(result, HashMap::from([(DEVICE_ID.to_string(), DeviceCommand { claim: Some(ClaimMode::Claim), properties: on_command() })]));
    }

    #[test]
    fn merge_command_maps_conflicting_claims_reset_the_claim_and_keep_the_properties() {
        let report = create_report_with_claim(DEVICE_ID, Some(ClaimMode::Claim), on_command());
        let report2 = create_report_with_claim(DEVICE_ID, Some(ClaimMode::Release), on_command());

        let result = merge_command_maps(vec![(create_flow("flow"), Ok(report)), (create_flow("other_flow"), Ok(report2))]);

        assert_eq!(result, HashMap::from([(DEVICE_ID.to_string(), device_command(on_command()))]));
    }

    #[test]
    fn merge_command_maps_conflicting_claims_without_properties_skip_the_device() {
        let report = create_report_with_claim(DEVICE_ID, Some(ClaimMode::Claim), HashMap::new());
        let report2 = create_report_with_claim(DEVICE_ID, Some(ClaimMode::Release), HashMap::new());

        let result = merge_command_maps(vec![(create_flow("flow"), Ok(report)), (create_flow("other_flow"), Ok(report2))]);

        assert_eq!(result, HashMap::new());
    }

    #[test]
    fn merge_command_maps_claims_of_different_devices_are_merged_separately() {
        let report = create_report_with_claim(DEVICE_ID, Some(ClaimMode::Claim), HashMap::new());
        let report2 = create_report_with_claim(DEVICE2_ID, Some(ClaimMode::Release), HashMap::new());

        let result = merge_command_maps(vec![(create_flow("flow"), Ok(report)), (create_flow("other_flow"), Ok(report2))]);

        assert_eq!(
            result,
            HashMap::from([
                (DEVICE_ID.to_string(), DeviceCommand { claim: Some(ClaimMode::Claim), properties: HashMap::new() }),
                (DEVICE2_ID.to_string(), DeviceCommand { claim: Some(ClaimMode::Release), properties: HashMap::new() }),
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

        let properties = HashMap::from([("on".to_string(), resolved_property_command(Value::Boolean(true)))]);

        let result = filter_flow_editable_properties(&device, properties);

        assert_eq!(result, HashMap::from([("on".to_string(), resolved_property_command(Value::Boolean(true)))]));
    }

    #[test]
    fn filter_flow_editable_properties_drops_a_readonly_property() {
        let device = DeviceBuilder::new(DEVICE_ID)
            .with_properties(vec![Box::new(BooleanProperty::new("on".to_string(), PropertyType::On, true, None, false))])
            .build();

        let properties = HashMap::from([("on".to_string(), resolved_property_command(Value::Boolean(true)))]);

        let result = filter_flow_editable_properties(&device, properties);

        assert_eq!(result, HashMap::new());
    }

    #[test]
    fn filter_flow_editable_properties_drops_an_unknown_property() {
        let device = DeviceBuilder::new(DEVICE_ID)
            .with_properties(vec![Box::new(BooleanProperty::new("on".to_string(), PropertyType::On, true, None, false))])
            .build();

        let properties = HashMap::from([("does_not_exist".to_string(), resolved_property_command(Value::Boolean(true)))]);

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
        let properties = HashMap::from([
            ("on".to_string(), resolved_property_command(Value::Boolean(true))),
            ("brightness".to_string(), resolved_property_command(Value::Boolean(true)))]);

        let result = filter_flow_editable_properties(&device, properties);

        assert_eq!(result, HashMap::from([("on".to_string(), resolved_property_command(Value::Boolean(true)))]));
    }
}
