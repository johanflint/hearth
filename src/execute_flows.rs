use crate::claim_registry::{AuthorizeError, Claim, ClaimOrigin};
use crate::domain::{ClaimMode, Command, Controller, Device, GeoLocation, Group, controller_registry};
use crate::flow_engine::flow::Flow;
use crate::flow_engine::property_command::ResolvedPropertyCommand;
use crate::flow_engine::{CommandMap, Context, FlowEngineError, FlowExecutionReport};
use crate::preset_registry::PresetRegistry;
use crate::scheduler::SchedulerCommand;
use crate::store::{PropertyChange, StoreSnapshot};
use crate::{claim_registry, flow_engine};
use futures::stream::FuturesUnordered;
use futures::stream::StreamExt;
use std::collections::{BTreeSet, HashMap};
use std::fmt::Debug;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;
use tracing::{info, instrument, warn};

type GroupId = String;
type DeviceId = String;
type PropertyId = String;

#[derive(Debug)]
struct Proposal<T> {
    origin: Arc<Flow>,
    value: T,
}

type ProposalMap = HashMap<(DeviceId, PropertyId), Vec<Proposal<ResolvedPropertyCommand>>>;
type ClaimProposalMap = HashMap<DeviceId, Vec<Proposal<ClaimMode>>>;
type GroupProposalMap = HashMap<DeviceId, Vec<Proposal<GroupId>>>;

// The commands for one device after merging the command maps of all flows
#[derive(Default, PartialEq, Debug)]
pub struct DispatchCommand {
    pub group_id: Option<GroupId>,
    pub claim: Option<Claim>,
    pub properties: HashMap<PropertyId, ResolvedPropertyCommand>,
}

pub type DispatchMap = HashMap<DeviceId, DispatchCommand>;

#[instrument(skip_all, fields(flow = flow.name(), node_id = node_id.as_deref().unwrap_or("<start>")))]
pub async fn execute_flow(
    flow: Arc<Flow>,
    node_id: Option<String>,
    snapshot: StoreSnapshot,
    preset_registry: Arc<PresetRegistry>,
    tx: Sender<SchedulerCommand>,
    geo_location: GeoLocation,
) {
    let context = Context::builder()
        .snapshot(snapshot.clone())
        .preset_registry(preset_registry) // Passing the Arc to avoid expensive copies
        .location(geo_location)
        .build();
    let result = flow_engine::execute(flow.clone(), node_id, &context, tx).await;

    let dispatch_map = merge_command_maps(vec![(flow, result)]);
    dispatch_commands(&snapshot, dispatch_map).await;
}

#[instrument(skip_all)]
pub async fn execute_flows(
    flows: Vec<Arc<Flow>>,
    snapshot: StoreSnapshot,
    preset_registry: Arc<PresetRegistry>,
    changed: Option<PropertyChange>,
    tx: Sender<SchedulerCommand>,
    geo_location: GeoLocation,
) {
    let context = Context::builder()
        .snapshot(snapshot.clone())
        .preset_registry(preset_registry)
        .changed(changed)
        .location(geo_location)
        .build();
    let results = FuturesUnordered::from_iter(flows.into_iter().map(|flow| async {
        let result = flow_engine::execute(flow.clone(), None, &context, tx.clone()).await;
        (flow, result)
    }))
    .collect::<Vec<_>>()
    .await;

    let dispatch_map = merge_command_maps(results);
    dispatch_commands(&snapshot, dispatch_map).await;
}

fn merge_command_maps(reports: Vec<(Arc<Flow>, Result<FlowExecutionReport, FlowEngineError>)>) -> DispatchMap {
    let mut proposals: ProposalMap = HashMap::new();
    let mut claim_proposals: ClaimProposalMap = HashMap::new();
    let mut group_proposals: GroupProposalMap = HashMap::new();

    for (origin, report) in reports {
        let Ok(report) = report else { continue };

        if let Some(command_map) = report.take_from_scope::<CommandMap>("command_map") {
            for (device_id, device_command) in command_map {
                if let Some(group_id) = device_command.group_id {
                    group_proposals
                        .entry(device_id.clone())
                        .or_default()
                        .push(Proposal { origin: origin.clone(), value: group_id })
                }

                if let Some(claim) = device_command.claim {
                    claim_proposals
                        .entry(device_id.clone())
                        .or_default()
                        .push(Proposal { origin: origin.clone(), value: claim })
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

    let mut merged_map: DispatchMap = HashMap::new();
    // Disagreeing flows only lose the group optimization, the properties are still sent per device
    for (device_id, group_ids) in group_proposals {
        if let Some(group_id) = agreed_value(&group_ids) {
            merged_map.entry(device_id).or_default().group_id = Some(group_id.clone());
        }
    }

    for ((device_id, property_id), writes) in proposals {
        let Some(command) = agreed_value(&writes) else {
            log_conflict(&device_id, &property_id, &writes);
            continue;
        };
        merged_map.entry(device_id).or_default().properties.insert(property_id, command.clone());
    }

    for (device_id, claims) in claim_proposals {
        let Some(mode) = agreed_value(&claims) else {
            log_claim_conflict(&device_id, &claims);
            continue;
        };
        let origin = ClaimOrigin::Flows(flow_ids(&claims));
        merged_map.entry(device_id).or_default().claim = Some(Claim { mode: *mode, origin });
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
    let mut descriptions: Vec<String> =
        proposals.iter().map(|p| format!("flow '{}' requested {:?}", p.origin.id(), p.value)).collect();
    descriptions.sort_unstable();
    descriptions
}

fn flow_ids<T>(proposals: &[Proposal<T>]) -> Vec<String> {
    let mut flow_ids: Vec<String> = proposals.iter().map(|p| p.origin.id().to_string()).collect();
    flow_ids.sort_unstable();
    flow_ids
}

struct DeviceDispatch {
    group_id: Option<GroupId>,
    device: Arc<Device>,
    controller: Arc<dyn Controller>,
    properties: HashMap<PropertyId, ResolvedPropertyCommand>,
}

async fn dispatch_commands(snapshot: &StoreSnapshot, dispatch_map: DispatchMap) {
    let dispatches = authorize_dispatches(snapshot, dispatch_map);
    for (controller, command) in controller_commands(snapshot, dispatches) {
        controller.execute(command).await;
    }
}

fn authorize_dispatches(snapshot: &StoreSnapshot, dispatch_map: DispatchMap) -> HashMap<DeviceId, DeviceDispatch> {
    let mut dispatches: HashMap<DeviceId, DeviceDispatch> = HashMap::new();
    for (device_id, device_command) in dispatch_map {
        let Some(device) = snapshot.devices.get(&device_id) else { continue };
        match claim_registry::authorize(device, device_command.claim) {
            Ok(()) => {}
            Err(AuthorizeError::Claimed(origin)) => {
                info!(device_id, claimed_by = %origin, "Device '{}' is claimed by {}, skipping writes without a claim", device.name, origin);
                continue;
            }
            Err(AuthorizeError::NotClaimable(_)) => {
                warn!(device_id, "⚠️ Device '{}' is not claimable, skipping writes", device.name);
                continue;
            }
        }

        let properties = filter_flow_editable_properties(device, device_command.properties);
        if properties.is_empty() {
            continue;
        }

        let Some(controller) = device.controller_id.and_then(controller_registry::get) else {
            warn!(device_id, "⚠️ Device '{}' is not tied to a controller", device.name);
            continue;
        };
        dispatches.insert(
            device_id,
            DeviceDispatch { group_id: device_command.group_id, device: device.clone(), controller, properties },
        );
    }
    dispatches
}

// Drops properties that a flow is not allowed to write (unknown and marked as readonly).
fn filter_flow_editable_properties(
    device: &Device,
    properties: HashMap<PropertyId, ResolvedPropertyCommand>,
) -> HashMap<PropertyId, ResolvedPropertyCommand> {
    properties
        .into_iter()
        .filter(|(property_id, _)| match device.properties.get(property_id) {
            Some(property) if property.readonly() => {
                warn!(
                    device_id = device.id,
                    property_id,
                    "⚠️ Ignoring a command to set readonly property '{}' for device '{}'",
                    property_id,
                    device.name
                );
                false
            }
            Some(_) => true,
            None => {
                warn!(
                    device_id = device.id,
                    property_id,
                    "⚠️ Ignoring a command to set unknown property '{}' for device '{}'",
                    property_id,
                    device.name
                );
                false
            }
        })
        .collect()
}

// Determines which command to send to which controller. A group command is only used for a group that a flow
// targeted, and only when every member is dispatched with the same properties, otherwise it would change more
// than the flows stated.
fn controller_commands(
    snapshot: &StoreSnapshot,
    mut dispatches: HashMap<DeviceId, DeviceDispatch>,
) -> Vec<(Arc<dyn Controller>, Command)> {
    let mut commands = Vec::new();
    let group_ids: BTreeSet<GroupId> = dispatches.values().filter_map(|d| d.group_id.clone()).collect();
    for group_id in group_ids {
        let Some(group) = snapshot.groups.get(&group_id) else { continue };
        if group.external_id.is_none() || group.device_ids.len() < 2 || !is_exact_match(group, &dispatches) {
            continue;
        }

        let mut members = group.device_ids.iter().filter_map(|id| dispatches.remove(id));
        let Some(first) = members.next() else { continue };

        let controller = first.controller;
        let property = Arc::new(first.properties);
        let devices = std::iter::once(first.device).chain(members.map(|member| member.device)).collect();
        commands.push((controller, Command::ControlGroup { group: group.clone(), devices, property }));
    }

    let device_commands = dispatches.into_values().map(|dispatch| {
        let command = Command::ControlDevice { device: dispatch.device, property: Arc::new(dispatch.properties) };
        (dispatch.controller, command)
    });

    commands.extend(device_commands);
    commands
}

fn is_exact_match(group: &Group, dispatches: &HashMap<DeviceId, DeviceDispatch>) -> bool {
    let Some(first) = group.device_ids.first().and_then(|id| dispatches.get(id)) else { return false };

    for device_id in &group.device_ids {
        let Some(member) = dispatches.get(device_id) else { return false };
        if member.group_id.as_ref() != Some(&group.id) {
            return false;
        }
        if member.device.controller_id != first.device.controller_id {
            return false;
        }
        if member.properties != first.properties {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::property::{BooleanProperty, PropertyType};
    use crate::domain::{Color, GroupOwner};
    use crate::domain::{Number, Value};
    use crate::flow_engine::action::DeviceCommand;
    use crate::flow_engine::flow::{FlowLink, FlowNode, FlowNodeKind};
    use crate::test_support::{DeviceBuilder, resolved_property_command};
    use async_trait::async_trait;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use std::any::Any;
    use std::time::Duration;

    const DEVICE_ID: &str = "device_id";
    const DEVICE2_ID: &str = "device2_id";

    fn create_report(device_id: &str, properties: HashMap<String, ResolvedPropertyCommand>) -> FlowExecutionReport {
        create_report_with_claim(device_id, None, properties)
    }

    fn create_report_with_claim(
        device_id: &str,
        claim: Option<ClaimMode>,
        properties: HashMap<String, ResolvedPropertyCommand>,
    ) -> FlowExecutionReport {
        create_report_with_command(device_id, DeviceCommand { group_id: None, claim, properties, ..Default::default() })
    }

    fn create_report_with_group(
        device_id: &str,
        group_id: &str,
        properties: HashMap<String, ResolvedPropertyCommand>,
    ) -> FlowExecutionReport {
        let device_command =
            DeviceCommand { group_id: Some(group_id.to_string()), claim: None, properties, ..Default::default() };
        create_report_with_command(device_id, device_command)
    }

    fn create_report_with_command(device_id: &str, device_command: DeviceCommand) -> FlowExecutionReport {
        let command_map: CommandMap = HashMap::from([(device_id.to_string(), device_command)]);

        let mut scope: HashMap<String, Box<dyn Any + Send + Sync>> = HashMap::new();
        scope.insert("command_map".to_string(), Box::new(command_map));

        FlowExecutionReport::new(scope, Duration::from_millis(1))
    }

    fn create_flow(id: &str) -> Arc<Flow> {
        let end_node = FlowNode::new("end_node".to_string(), vec![], FlowNodeKind::End);
        let start_node = FlowNode::new(
            "start_node".to_string(),
            vec![FlowLink::new(Arc::new(end_node), Value::None)],
            FlowNodeKind::Start,
        );

        Arc::new(Flow::new(id.to_string(), id.to_string(), None, None, Arc::new(start_node), HashMap::new()).unwrap())
    }

    fn device_command(properties: HashMap<String, ResolvedPropertyCommand>) -> DispatchCommand {
        DispatchCommand { group_id: None, claim: None, properties }
    }

    fn flow_claim(mode: ClaimMode, flow_ids: &[&str]) -> Option<Claim> {
        let flow_ids = flow_ids.iter().map(|id| id.to_string()).collect();
        Some(Claim { mode, origin: ClaimOrigin::Flows(flow_ids) })
    }

    fn on_command() -> HashMap<String, ResolvedPropertyCommand> {
        HashMap::from([("on".to_string(), resolved_property_command(Value::Boolean(true)))])
    }

    #[test]
    fn merge_command_maps_single_proposal_returns_property() {
        let report = create_report(
            DEVICE_ID,
            HashMap::from([("property_id".to_string(), resolved_property_command(Value::Boolean(true)))]),
        );
        let flow = create_flow("flow");
        let result = merge_command_maps(vec![(flow, Ok(report))]);

        assert_eq!(
            result,
            HashMap::from([(
                DEVICE_ID.to_string(),
                device_command(HashMap::from([(
                    "property_id".to_string(),
                    resolved_property_command(Value::Boolean(true))
                )]))
            )])
        );
    }

    #[rstest]
    #[case::boolean(Value::Boolean(true))]
    #[case::number(Value::Number(Number::PositiveInt(1)))]
    #[case::color(Value::Color(Color::Hex("#000000".to_string())))]
    fn merge_command_maps_equal_absolute_proposals_deduplicate(#[case] value: Value) {
        let flow = create_flow("flow");
        let flow2 = create_flow("other_flow");
        let report = create_report(
            DEVICE_ID,
            HashMap::from([("property_id".to_string(), resolved_property_command(value.clone()))]),
        );
        let report2 = create_report(
            DEVICE_ID,
            HashMap::from([("property_id".to_string(), resolved_property_command(value.clone()))]),
        );

        let result = merge_command_maps(vec![(flow, Ok(report)), (flow2, Ok(report2))]);
        assert_eq!(
            result,
            HashMap::from([(
                DEVICE_ID.to_string(),
                device_command(HashMap::from([("property_id".to_string(), resolved_property_command(value))]))
            )])
        );
    }

    #[test]
    fn merge_command_maps_three_equal_absolute_proposals_deduplicate() {
        let flow = create_flow("flow");
        let flow2 = create_flow("other_flow");
        let flow3 = create_flow("third_flow");
        let report = create_report(
            DEVICE_ID,
            HashMap::from([("property_id".to_string(), resolved_property_command(Value::Boolean(true)))]),
        );
        let report2 = create_report(
            DEVICE_ID,
            HashMap::from([("property_id".to_string(), resolved_property_command(Value::Boolean(true)))]),
        );
        let report3 = create_report(
            DEVICE_ID,
            HashMap::from([("property_id".to_string(), resolved_property_command(Value::Boolean(true)))]),
        );

        let result = merge_command_maps(vec![(flow, Ok(report)), (flow2, Ok(report2)), (flow3, Ok(report3))]);
        assert_eq!(
            result,
            HashMap::from([(
                DEVICE_ID.to_string(),
                device_command(HashMap::from([(
                    "property_id".to_string(),
                    resolved_property_command(Value::Boolean(true))
                )]))
            )])
        );
    }

    #[test]
    fn merge_command_maps_different_absolute_proposals_skip_property() {
        let flow = create_flow("flow");
        let flow2 = create_flow("other_flow");
        let report = create_report(
            DEVICE_ID,
            HashMap::from([("property_id".to_string(), resolved_property_command(Value::Boolean(true)))]),
        );
        let report2 = create_report(
            DEVICE_ID,
            HashMap::from([("property_id".to_string(), resolved_property_command(Value::Boolean(false)))]),
        );

        let result = merge_command_maps(vec![(flow, Ok(report)), (flow2, Ok(report2))]);
        assert_eq!(result, HashMap::new());
    }

    #[test]
    fn merge_command_maps_equal_proposals_with_equal_transitions_deduplicate() {
        let flow = create_flow("flow");
        let flow2 = create_flow("other_flow");
        let command =
            ResolvedPropertyCommand { value: Value::Boolean(false), transition: Some(Duration::from_secs(600)) };
        let report = create_report(DEVICE_ID, HashMap::from([("property_id".to_string(), command.clone())]));
        let report2 = create_report(DEVICE_ID, HashMap::from([("property_id".to_string(), command.clone())]));

        let result = merge_command_maps(vec![(flow, Ok(report)), (flow2, Ok(report2))]);
        assert_eq!(
            result,
            HashMap::from([(
                DEVICE_ID.to_string(),
                device_command(HashMap::from([("property_id".to_string(), command)]))
            )])
        );
    }

    #[rstest]
    #[case::different_transitions(Some(Duration::from_secs(600)), Some(Duration::from_secs(60)))]
    #[case::transition_and_no_transition(Some(Duration::from_secs(600)), None)]
    fn merge_command_maps_equal_values_with_different_transitions_skip_property(
        #[case] transition: Option<Duration>,
        #[case] transition2: Option<Duration>,
    ) {
        let flow = create_flow("flow");
        let flow2 = create_flow("other_flow");
        let report = create_report(
            DEVICE_ID,
            HashMap::from([(
                "property_id".to_string(),
                ResolvedPropertyCommand { value: Value::Boolean(false), transition },
            )]),
        );
        let report2 = create_report(
            DEVICE_ID,
            HashMap::from([(
                "property_id".to_string(),
                ResolvedPropertyCommand { value: Value::Boolean(false), transition: transition2 },
            )]),
        );

        let result = merge_command_maps(vec![(flow, Ok(report)), (flow2, Ok(report2))]);
        assert_eq!(result, HashMap::new());
    }

    #[test]
    fn merge_command_maps_conflicting_property_preserves_independent_property() {
        let flow = create_flow("flow");
        let flow2 = create_flow("other_flow");
        let report = create_report(
            DEVICE_ID,
            HashMap::from([("property_id".to_string(), resolved_property_command(Value::Boolean(true)))]),
        );
        let report2 = create_report(
            DEVICE_ID,
            HashMap::from([
                ("property_id".to_string(), resolved_property_command(Value::Boolean(false))),
                ("property2_id".to_string(), resolved_property_command(Value::Boolean(false))),
            ]),
        );

        let result = merge_command_maps(vec![(flow, Ok(report)), (flow2, Ok(report2))]);
        assert_eq!(
            result,
            HashMap::from([(
                DEVICE_ID.to_string(),
                device_command(HashMap::from([(
                    "property2_id".to_string(),
                    resolved_property_command(Value::Boolean(false))
                )]))
            )])
        );
    }

    #[test]
    fn merge_command_maps_different_devices_separately() {
        let flow = create_flow("flow");
        let flow2 = create_flow("other_flow");
        let report = create_report(
            DEVICE_ID,
            HashMap::from([(
                "property_id".to_string(),
                resolved_property_command(Value::Number(Number::PositiveInt(42))),
            )]),
        );
        let report2 = create_report(
            DEVICE2_ID,
            HashMap::from([(
                "property_id".to_string(),
                resolved_property_command(Value::Number(Number::PositiveInt(42))),
            )]),
        );

        let result = merge_command_maps(vec![(flow, Ok(report)), (flow2, Ok(report2))]);
        assert_eq!(
            result,
            HashMap::from([
                (
                    DEVICE_ID.to_string(),
                    device_command(HashMap::from([(
                        "property_id".to_string(),
                        resolved_property_command(Value::Number(Number::PositiveInt(42)))
                    )]))
                ),
                (
                    DEVICE2_ID.to_string(),
                    device_command(HashMap::from([(
                        "property_id".to_string(),
                        resolved_property_command(Value::Number(Number::PositiveInt(42)))
                    )]))
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

        assert_eq!(
            result,
            HashMap::from([(
                DEVICE_ID.to_string(),
                DispatchCommand { group_id: None, claim: flow_claim(claim, &["flow"]), properties: on_command() }
            )])
        );
    }

    #[test]
    fn merge_command_maps_claim_without_properties_is_kept() {
        let report = create_report_with_claim(DEVICE_ID, Some(ClaimMode::Release), HashMap::new());

        let result = merge_command_maps(vec![(create_flow("flow"), Ok(report))]);

        assert_eq!(
            result,
            HashMap::from([(
                DEVICE_ID.to_string(),
                DispatchCommand {
                    group_id: None,
                    claim: flow_claim(ClaimMode::Release, &["flow"]),
                    properties: HashMap::new()
                }
            )])
        );
    }

    #[test]
    fn merge_command_maps_equal_claims_deduplicate_and_keep_the_sorted_origins() {
        let report = create_report_with_claim(DEVICE_ID, Some(ClaimMode::Claim), on_command());
        let report2 = create_report_with_claim(DEVICE_ID, Some(ClaimMode::Claim), on_command());

        let result =
            merge_command_maps(vec![(create_flow("other_flow"), Ok(report)), (create_flow("flow"), Ok(report2))]);

        assert_eq!(
            result,
            HashMap::from([(
                DEVICE_ID.to_string(),
                DispatchCommand {
                    group_id: None,
                    claim: flow_claim(ClaimMode::Claim, &["flow", "other_flow"]),
                    properties: on_command()
                }
            )])
        );
    }

    #[test]
    fn merge_command_maps_a_flow_without_claim_does_not_conflict_with_a_claim() {
        let report = create_report_with_claim(DEVICE_ID, Some(ClaimMode::Claim), on_command());
        let report2 = create_report(DEVICE_ID, on_command());

        let result =
            merge_command_maps(vec![(create_flow("flow"), Ok(report)), (create_flow("other_flow"), Ok(report2))]);

        assert_eq!(
            result,
            HashMap::from([(
                DEVICE_ID.to_string(),
                DispatchCommand {
                    group_id: None,
                    claim: flow_claim(ClaimMode::Claim, &["flow"]),
                    properties: on_command()
                }
            )])
        );
    }

    #[test]
    fn merge_command_maps_conflicting_claims_reset_the_claim_and_keep_the_properties() {
        let report = create_report_with_claim(DEVICE_ID, Some(ClaimMode::Claim), on_command());
        let report2 = create_report_with_claim(DEVICE_ID, Some(ClaimMode::Release), on_command());

        let result =
            merge_command_maps(vec![(create_flow("flow"), Ok(report)), (create_flow("other_flow"), Ok(report2))]);

        assert_eq!(result, HashMap::from([(DEVICE_ID.to_string(), device_command(on_command()))]));
    }

    #[test]
    fn merge_command_maps_conflicting_claims_without_properties_skip_the_device() {
        let report = create_report_with_claim(DEVICE_ID, Some(ClaimMode::Claim), HashMap::new());
        let report2 = create_report_with_claim(DEVICE_ID, Some(ClaimMode::Release), HashMap::new());

        let result =
            merge_command_maps(vec![(create_flow("flow"), Ok(report)), (create_flow("other_flow"), Ok(report2))]);

        assert_eq!(result, HashMap::new());
    }

    #[test]
    fn merge_command_maps_claims_of_different_devices_are_merged_separately() {
        let report = create_report_with_claim(DEVICE_ID, Some(ClaimMode::Claim), HashMap::new());
        let report2 = create_report_with_claim(DEVICE2_ID, Some(ClaimMode::Release), HashMap::new());

        let result =
            merge_command_maps(vec![(create_flow("flow"), Ok(report)), (create_flow("other_flow"), Ok(report2))]);

        assert_eq!(
            result,
            HashMap::from([
                (
                    DEVICE_ID.to_string(),
                    DispatchCommand {
                        group_id: None,
                        claim: flow_claim(ClaimMode::Claim, &["flow"]),
                        properties: HashMap::new()
                    }
                ),
                (
                    DEVICE2_ID.to_string(),
                    DispatchCommand {
                        group_id: None,
                        claim: flow_claim(ClaimMode::Release, &["other_flow"]),
                        properties: HashMap::new()
                    }
                ),
            ])
        );
    }

    fn group_command(group_id: &str, properties: HashMap<String, ResolvedPropertyCommand>) -> DispatchCommand {
        DispatchCommand { group_id: Some(group_id.to_string()), claim: None, properties }
    }

    #[test]
    fn merge_command_maps_keeps_the_group() {
        let report = create_report_with_group(DEVICE_ID, "room", on_command());

        let result = merge_command_maps(vec![(create_flow("flow"), Ok(report))]);

        assert_eq!(result, HashMap::from([(DEVICE_ID.to_string(), group_command("room", on_command()))]));
    }

    #[test]
    fn merge_command_maps_equal_groups_deduplicate() {
        let report = create_report_with_group(DEVICE_ID, "room", on_command());
        let report2 = create_report_with_group(DEVICE_ID, "room", on_command());

        let result =
            merge_command_maps(vec![(create_flow("flow"), Ok(report)), (create_flow("other_flow"), Ok(report2))]);

        assert_eq!(result, HashMap::from([(DEVICE_ID.to_string(), group_command("room", on_command()))]));
    }

    #[test]
    fn merge_command_maps_a_flow_without_group_does_not_conflict_with_a_group() {
        let report = create_report_with_group(DEVICE_ID, "room", on_command());
        let report2 = create_report(DEVICE_ID, on_command());

        let result =
            merge_command_maps(vec![(create_flow("flow"), Ok(report)), (create_flow("other_flow"), Ok(report2))]);

        assert_eq!(result, HashMap::from([(DEVICE_ID.to_string(), group_command("room", on_command()))]));
    }

    #[test]
    fn merge_command_maps_conflicting_groups_reset_the_group_and_keep_the_properties() {
        let report = create_report_with_group(DEVICE_ID, "room", on_command());
        let report2 = create_report_with_group(DEVICE_ID, "zone", on_command());

        let result =
            merge_command_maps(vec![(create_flow("flow"), Ok(report)), (create_flow("other_flow"), Ok(report2))]);

        assert_eq!(result, HashMap::from([(DEVICE_ID.to_string(), device_command(on_command()))]));
    }

    #[test]
    fn merge_command_maps_empty_map() {
        let result = merge_command_maps(vec![]);
        assert_eq!(result, HashMap::new());
    }

    #[test]
    fn filter_flow_editable_properties_keeps_a_writable_property() {
        let device = DeviceBuilder::new(DEVICE_ID)
            .with_properties(vec![Box::new(BooleanProperty::new(
                "on".to_string(),
                PropertyType::On,
                false,
                None,
                false,
            ))])
            .build();

        let properties = HashMap::from([("on".to_string(), resolved_property_command(Value::Boolean(true)))]);

        let result = filter_flow_editable_properties(&device, properties);

        assert_eq!(result, HashMap::from([("on".to_string(), resolved_property_command(Value::Boolean(true)))]));
    }

    #[test]
    fn filter_flow_editable_properties_drops_a_readonly_property() {
        let device = DeviceBuilder::new(DEVICE_ID)
            .with_properties(vec![Box::new(BooleanProperty::new(
                "on".to_string(),
                PropertyType::On,
                true,
                None,
                false,
            ))])
            .build();

        let properties = HashMap::from([("on".to_string(), resolved_property_command(Value::Boolean(true)))]);

        let result = filter_flow_editable_properties(&device, properties);

        assert_eq!(result, HashMap::new());
    }

    #[test]
    fn filter_flow_editable_properties_drops_an_unknown_property() {
        let device = DeviceBuilder::new(DEVICE_ID)
            .with_properties(vec![Box::new(BooleanProperty::new(
                "on".to_string(),
                PropertyType::On,
                true,
                None,
                false,
            ))])
            .build();

        let properties =
            HashMap::from([("does_not_exist".to_string(), resolved_property_command(Value::Boolean(true)))]);

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
            ("brightness".to_string(), resolved_property_command(Value::Boolean(true))),
        ]);

        let result = filter_flow_editable_properties(&device, properties);

        assert_eq!(result, HashMap::from([("on".to_string(), resolved_property_command(Value::Boolean(true)))]));
    }

    // The controller and claim registries are global, so these tests use ids that no other test uses
    const STUB_CONTROLLER_ID: &str = "authorize_dispatches_stub";

    fn controlled_device(device_id: &str, controller_id: Option<&'static str>) -> Device {
        controller_registry::register(Arc::new(StubController(STUB_CONTROLLER_ID)));

        let on = BooleanProperty::new("on".to_string(), PropertyType::On, false, None, false);
        let mut device = DeviceBuilder::new(device_id).with_properties(vec![Box::new(on)]).build();
        device.controller_id = controller_id;
        device
    }

    fn snapshot_with_device(device: Device) -> StoreSnapshot {
        let devices = HashMap::from([(device.id.clone(), Arc::new(device))]);
        StoreSnapshot { devices: Arc::new(devices), ..Default::default() }
    }

    #[test]
    fn authorize_dispatches_keeps_the_group_device_controller_and_properties() {
        let device_id = "authorize_device";
        let snapshot = snapshot_with_device(controlled_device(device_id, Some(STUB_CONTROLLER_ID)));
        let dispatch_map = HashMap::from([(device_id.to_string(), group_command("room", on_command()))]);

        let dispatches = authorize_dispatches(&snapshot, dispatch_map);

        assert_eq!(dispatches.len(), 1);
        let dispatch = &dispatches[device_id];
        assert_eq!(dispatch.group_id.as_deref(), Some("room"));
        assert_eq!(dispatch.device.id, device_id);
        assert_eq!(dispatch.controller.id(), STUB_CONTROLLER_ID);
        assert_eq!(dispatch.properties, on_command());
    }

    #[test]
    fn authorize_dispatches_skips_an_unknown_device() {
        let dispatch_map = HashMap::from([("authorize_unknown_device".to_string(), device_command(on_command()))]);

        let dispatches = authorize_dispatches(&StoreSnapshot::default(), dispatch_map);

        assert!(dispatches.is_empty());
    }

    #[test]
    fn authorize_dispatches_skips_a_claimed_device_without_a_claim() {
        let device = controlled_device("authorize_claimed_device", Some(STUB_CONTROLLER_ID));
        claim_registry::claim(&device, ClaimOrigin::External).unwrap();
        let dispatch_map = HashMap::from([(device.id.clone(), device_command(on_command()))]);

        let dispatches = authorize_dispatches(&snapshot_with_device(device), dispatch_map);

        assert!(dispatches.is_empty());
    }

    #[test]
    fn authorize_dispatches_dispatches_a_claimed_device_when_the_flow_claims_it() {
        let device = controlled_device("authorize_reclaimed_device", Some(STUB_CONTROLLER_ID));
        claim_registry::claim(&device, ClaimOrigin::External).unwrap();
        let dispatch_command = DispatchCommand {
            group_id: None,
            claim: flow_claim(ClaimMode::Claim, &["flow"]),
            properties: on_command(),
        };
        let dispatch_map = HashMap::from([(device.id.clone(), dispatch_command)]);

        let dispatches = authorize_dispatches(&snapshot_with_device(device), dispatch_map);

        assert!(dispatches.contains_key("authorize_reclaimed_device"));
    }

    #[test]
    fn authorize_dispatches_skips_an_unclaimable_device_with_a_claim() {
        let mut device = controlled_device("authorize_unclaimable_device", Some(STUB_CONTROLLER_ID));
        device.claimable = false;
        let dispatch_command = DispatchCommand {
            group_id: None,
            claim: flow_claim(ClaimMode::Claim, &["flow"]),
            properties: on_command(),
        };
        let dispatch_map = HashMap::from([(device.id.clone(), dispatch_command)]);

        let dispatches = authorize_dispatches(&snapshot_with_device(device), dispatch_map);

        assert!(dispatches.is_empty());
    }

    #[test]
    fn authorize_dispatches_skips_a_device_without_writable_properties() {
        let device = controlled_device("authorize_readonly_device", Some(STUB_CONTROLLER_ID));
        let properties = HashMap::from([("unknown".to_string(), resolved_property_command(Value::Boolean(true)))]);
        let dispatch_map = HashMap::from([(device.id.clone(), device_command(properties))]);

        let dispatches = authorize_dispatches(&snapshot_with_device(device), dispatch_map);

        assert!(dispatches.is_empty());
    }

    #[rstest]
    #[case::no_controller(None)]
    #[case::unregistered_controller(Some("unregistered"))]
    fn authorize_dispatches_skips_a_device_without_a_registered_controller(
        #[case] controller_id: Option<&'static str>,
    ) {
        let device = controlled_device("authorize_uncontrolled_device", controller_id);
        let dispatch_map = HashMap::from([(device.id.clone(), device_command(on_command()))]);

        let dispatches = authorize_dispatches(&snapshot_with_device(device), dispatch_map);

        assert!(dispatches.is_empty());
    }

    #[derive(Debug)]
    struct StubController(&'static str);

    #[async_trait]
    impl Controller for StubController {
        fn id(&self) -> &'static str {
            self.0
        }

        async fn execute(&self, _command: Command) {}
    }

    fn dispatch_for(
        device_id: &str,
        group_id: Option<&str>,
        controller_id: &'static str,
        properties: HashMap<String, ResolvedPropertyCommand>,
    ) -> (DeviceId, DeviceDispatch) {
        let mut device = DeviceBuilder::new(device_id).build();
        device.controller_id = Some(controller_id);

        let dispatch = DeviceDispatch {
            group_id: group_id.map(str::to_string),
            device: Arc::new(device),
            controller: Arc::new(StubController(controller_id)),
            properties,
        };
        (device_id.to_string(), dispatch)
    }

    // A device that a flow controlled through the group 'room'
    fn room_dispatch(device_id: &str) -> (DeviceId, DeviceDispatch) {
        dispatch_for(device_id, Some("room"), "hue", on_command())
    }

    // A device that a flow controlled directly
    fn device_dispatch(device_id: &str) -> (DeviceId, DeviceDispatch) {
        dispatch_for(device_id, None, "hue", on_command())
    }

    fn group(id: &str, device_ids: &[&str], external_id: Option<&str>) -> Group {
        Group {
            id: id.to_string(),
            name: id.to_string(),
            device_ids: device_ids.iter().map(|id| id.to_string()).collect(),
            revision: 0,
            owner: GroupOwner::Integration,
            external_id: external_id.map(str::to_string),
        }
    }

    fn snapshot_with_groups(groups: Vec<Group>) -> StoreSnapshot {
        let groups = groups.into_iter().map(|group| (group.id.clone(), Arc::new(group))).collect();
        StoreSnapshot { groups: Arc::new(groups), ..Default::default() }
    }

    fn snapshot_with_group(device_ids: &[&str]) -> StoreSnapshot {
        snapshot_with_groups(vec![group("room", device_ids, Some("grouped-light"))])
    }

    // The order of the commands is not defined, so they are described and sorted
    fn describe_commands(commands: &[(Arc<dyn Controller>, Command)]) -> Vec<String> {
        let mut descriptions: Vec<String> = commands
            .iter()
            .map(|(_, command)| match command {
                Command::ControlDevice { device, .. } => format!("device {}", device.id),
                Command::ControlGroup { group, devices, .. } => {
                    let device_ids: Vec<&str> = devices.iter().map(|device| device.id.as_str()).collect();
                    format!("group {}: {}", group.id, device_ids.join(", "))
                }
            })
            .collect();
        descriptions.sort();
        descriptions
    }

    #[test]
    fn controller_commands_sends_a_group_command_when_every_member_has_the_same_properties() {
        let snapshot = snapshot_with_group(&[DEVICE_ID, DEVICE2_ID]);
        let dispatches = HashMap::from([room_dispatch(DEVICE_ID), room_dispatch(DEVICE2_ID)]);

        let commands = controller_commands(&snapshot, dispatches);

        assert_eq!(describe_commands(&commands), ["group room: device2_id, device_id"]);
        let (controller, Command::ControlGroup { group, property, .. }) = &commands[0] else {
            panic!("expected a group command");
        };
        assert_eq!(controller.id(), "hue");
        assert_eq!(group.external_id.as_deref(), Some("grouped-light"));
        assert_eq!(**property, on_command());
    }

    #[test]
    fn controller_commands_sends_device_commands_for_an_exact_match_that_no_flow_controlled_as_a_group() {
        let dispatches = HashMap::from([device_dispatch(DEVICE_ID), device_dispatch(DEVICE2_ID)]);

        let commands = controller_commands(&snapshot_with_group(&[DEVICE_ID, DEVICE2_ID]), dispatches);

        assert_eq!(describe_commands(&commands), ["device device2_id", "device device_id"]);
    }

    #[test]
    fn controller_commands_sends_nothing_without_dispatches() {
        let commands = controller_commands(&snapshot_with_group(&[DEVICE_ID, DEVICE2_ID]), HashMap::new());

        assert!(commands.is_empty());
    }

    #[test]
    fn controller_commands_sends_device_commands_when_a_member_is_not_dispatched() {
        // A member that is claimed, or has nothing left to write after filtering, is not dispatched
        let snapshot = snapshot_with_group(&[DEVICE_ID, DEVICE2_ID]);
        let dispatches = HashMap::from([room_dispatch(DEVICE_ID)]);

        let commands = controller_commands(&snapshot, dispatches);

        assert_eq!(describe_commands(&commands), ["device device_id"]);
    }

    #[rstest]
    #[case::different_value(HashMap::from([("on".to_string(), resolved_property_command(Value::Boolean(false)))]))]
    #[case::different_property(HashMap::from([("brightness".to_string(), resolved_property_command(Value::Number(Number::PositiveInt(50))))]))]
    #[case::additional_property(HashMap::from([
        ("on".to_string(), resolved_property_command(Value::Boolean(true))),
        ("brightness".to_string(), resolved_property_command(Value::Number(Number::PositiveInt(50)))),
    ]))]
    #[case::different_transition(HashMap::from([(
        "on".to_string(),
        ResolvedPropertyCommand { value: Value::Boolean(true), transition: Some(Duration::from_secs(10)) }
    )]))]
    fn controller_commands_sends_device_commands_when_the_members_have_different_properties(
        #[case] other_properties: HashMap<String, ResolvedPropertyCommand>,
    ) {
        let snapshot = snapshot_with_group(&[DEVICE_ID, DEVICE2_ID]);
        let dispatches =
            HashMap::from([room_dispatch(DEVICE_ID), dispatch_for(DEVICE2_ID, Some("room"), "hue", other_properties)]);

        let commands = controller_commands(&snapshot, dispatches);

        assert_eq!(describe_commands(&commands), ["device device2_id", "device device_id"]);
    }

    #[test]
    fn controller_commands_sends_device_commands_when_the_members_have_different_controllers() {
        let snapshot = snapshot_with_group(&[DEVICE_ID, DEVICE2_ID]);
        let dispatches =
            HashMap::from([room_dispatch(DEVICE_ID), dispatch_for(DEVICE2_ID, Some("room"), "other", on_command())]);

        let commands = controller_commands(&snapshot, dispatches);

        assert_eq!(describe_commands(&commands), ["device device2_id", "device device_id"]);
    }

    #[test]
    fn controller_commands_sends_device_commands_for_a_group_without_an_external_id() {
        let snapshot = snapshot_with_groups(vec![group("room", &[DEVICE_ID, DEVICE2_ID], None)]);
        let dispatches = HashMap::from([room_dispatch(DEVICE_ID), room_dispatch(DEVICE2_ID)]);

        let commands = controller_commands(&snapshot, dispatches);

        assert_eq!(describe_commands(&commands), ["device device2_id", "device device_id"]);
    }

    #[test]
    fn controller_commands_sends_a_device_command_for_a_group_with_a_single_device() {
        let snapshot = snapshot_with_group(&[DEVICE_ID]);
        let dispatches = HashMap::from([room_dispatch(DEVICE_ID)]);

        let commands = controller_commands(&snapshot, dispatches);

        assert_eq!(describe_commands(&commands), ["device device_id"]);
    }

    #[test]
    fn controller_commands_sends_a_device_command_for_a_device_outside_the_group() {
        let snapshot = snapshot_with_group(&[DEVICE_ID, DEVICE2_ID]);
        let dispatches =
            HashMap::from([room_dispatch(DEVICE_ID), room_dispatch(DEVICE2_ID), device_dispatch("device3_id")]);

        let commands = controller_commands(&snapshot, dispatches);

        assert_eq!(describe_commands(&commands), ["device device3_id", "group room: device2_id, device_id"]);
    }

    #[test]
    fn controller_commands_sends_a_device_command_for_a_group_that_refers_to_an_unknown_device() {
        let snapshot = snapshot_with_group(&[DEVICE_ID, "ghost"]);
        let dispatches = HashMap::from([room_dispatch(DEVICE_ID)]);

        let commands = controller_commands(&snapshot, dispatches);

        assert_eq!(describe_commands(&commands), ["device device_id"]);
    }

    // DEVICE2_ID is the first member of the group, so this also covers the first member
    #[rstest]
    #[case::untagged(None)]
    #[case::tagged_with_another_group(Some("zone"))]
    fn controller_commands_sends_device_commands_when_a_member_is_not_controlled_through_the_group(
        #[case] group_id: Option<&str>,
    ) {
        let snapshot = snapshot_with_group(&[DEVICE_ID, DEVICE2_ID]);
        let dispatches =
            HashMap::from([room_dispatch(DEVICE_ID), dispatch_for(DEVICE2_ID, group_id, "hue", on_command())]);

        let commands = controller_commands(&snapshot, dispatches);

        assert_eq!(describe_commands(&commands), ["device device2_id", "device device_id"]);
    }

    #[test]
    fn controller_commands_sends_device_commands_for_an_unknown_group() {
        let dispatches = HashMap::from([room_dispatch(DEVICE_ID), room_dispatch(DEVICE2_ID)]);

        let commands = controller_commands(&StoreSnapshot::default(), dispatches);

        assert_eq!(describe_commands(&commands), ["device device2_id", "device device_id"]);
    }

    #[test]
    fn controller_commands_sends_a_group_command_per_controlled_group() {
        let snapshot = snapshot_with_groups(vec![
            group("room", &[DEVICE_ID, DEVICE2_ID], Some("grouped-light")),
            group("zone", &["device3_id", "device4_id"], Some("grouped-light-2")),
        ]);
        let zone_dispatch = |device_id| dispatch_for(device_id, Some("zone"), "hue", on_command());
        let dispatches = HashMap::from([
            room_dispatch(DEVICE_ID),
            room_dispatch(DEVICE2_ID),
            zone_dispatch("device3_id"),
            zone_dispatch("device4_id"),
        ]);

        let commands = controller_commands(&snapshot, dispatches);

        assert_eq!(
            describe_commands(&commands),
            ["group room: device2_id, device_id", "group zone: device3_id, device4_id"]
        );
    }

    #[test]
    fn controller_commands_uses_the_controlled_group_of_overlapping_groups() {
        // Both groups match exactly, but the flow only controlled the zone
        let snapshot = snapshot_with_groups(vec![
            group("room", &[DEVICE_ID, DEVICE2_ID], Some("grouped-light")),
            group("zone", &[DEVICE_ID, DEVICE2_ID], Some("grouped-light-2")),
        ]);
        let zone_dispatch = |device_id| dispatch_for(device_id, Some("zone"), "hue", on_command());
        let dispatches = HashMap::from([zone_dispatch(DEVICE_ID), zone_dispatch(DEVICE2_ID)]);

        let commands = controller_commands(&snapshot, dispatches);

        assert_eq!(describe_commands(&commands), ["group zone: device2_id, device_id"]);
    }
}
