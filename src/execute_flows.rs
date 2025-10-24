use crate::domain::commands::Command;
use crate::domain::{GeoLocation, controller_registry};
use crate::flow_engine;
use crate::flow_engine::flow::Flow;
use crate::flow_engine::property_value::PropertyValue;
use crate::flow_engine::{Context, FlowEngineError, FlowExecutionReport};
use crate::scheduler::SchedulerCommand;
use crate::store::StoreSnapshot;
use futures::stream::FuturesUnordered;
use futures::stream::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;
use tracing::{instrument, warn};

type CommandMap = HashMap<String, HashMap<String, PropertyValue>>;

#[instrument(skip_all, fields(flow = flow.name(), node_id = node_id.as_deref().unwrap_or("<start>")))]
pub async fn execute_flow(flow: Arc<Flow>, node_id: Option<String>, snapshot: StoreSnapshot, tx: Sender<SchedulerCommand>, geo_location: GeoLocation) {
    let context = Context::builder().snapshot(snapshot.clone()).location(geo_location).build();
    let result = flow_engine::execute(&flow, node_id, &context, tx).await;

    let command_map = merge_command_maps(vec![result]);
    dispatch_commands(&snapshot, command_map).await;
}

#[instrument(skip_all)]
pub async fn execute_flows(flows: Vec<Arc<Flow>>, snapshot: StoreSnapshot, tx: Sender<SchedulerCommand>, geo_location: GeoLocation) {
    let context = Context::builder().snapshot(snapshot.clone()).location(geo_location).build();
    let results = FuturesUnordered::from_iter(flows.iter().map(|flow| async { flow_engine::execute(flow, None, &context, tx.clone()).await }))
        .collect::<Vec<_>>()
        .await;

    let command_map = merge_command_maps(results);
    dispatch_commands(&snapshot, command_map).await;
}

fn merge_command_maps(reports: Vec<Result<FlowExecutionReport, FlowEngineError>>) -> CommandMap {
    let mut merged_map = HashMap::new();

    for report in reports.into_iter().filter_map(Result::ok) {
        if let Some(command_map) = report.take_from_scope::<CommandMap>("command_map") {
            for (device_id, properties) in command_map {
                let device_properties = merged_map.entry(device_id).or_insert_with(HashMap::new);
                device_properties.extend(properties);
            }
        }
    }

    merged_map
}

async fn dispatch_commands(snapshot: &StoreSnapshot, command_map: CommandMap) {
    for (device_id, properties) in command_map {
        if let Some(device) = snapshot.devices.get(&device_id) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow_engine::property_value::PropertyValue::SetBooleanValue;
    use std::any::Any;
    use std::time::Duration;
    use test_log::test;

    const DEVICE_ID: &str = "device_id";

    fn create_report(properties: HashMap<String, PropertyValue>) -> FlowExecutionReport {
        let mut command_map = HashMap::new();
        command_map.insert(DEVICE_ID.to_string(), properties);

        let mut scope: HashMap<String, Box<dyn Any + Send + Sync>> = HashMap::new();
        scope.insert("command_map".to_string(), Box::new(command_map));

        FlowExecutionReport::new(scope, Duration::from_millis(1))
    }

    #[test]
    fn merge_a_single_command_map() {
        let report = create_report(HashMap::from([("property_id".to_string(), SetBooleanValue(true))]));
        let result = merge_command_maps(vec![Ok(report)]);

        assert_eq!(result[DEVICE_ID]["property_id"], SetBooleanValue(true));
    }

    #[test]
    fn merge_two_maps_with_overlapping_properties() {
        let report = create_report(HashMap::from([("property_id".to_string(), SetBooleanValue(true))]));
        let report2 = create_report(HashMap::from([("property_id".to_string(), SetBooleanValue(false))]));

        let result = merge_command_maps(vec![Ok(report), Ok(report2)]);

        assert_eq!(result[DEVICE_ID]["property_id"], SetBooleanValue(false));
    }
}
