use crate::api::ApiState;
use crate::flow_engine::SchedulerCommand;
use crate::flow_registry::FlowRegistry;
use crate::flow_store::FlowStore;
use axum::response::Response;
use http_body_util::BodyExt;
use metrics_exporter_prometheus::PrometheusBuilder;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;

pub(super) const VALID_FLOW_ID: &str = "01K7KK6H5R7Y72QJEJSJQCKMRQ";
pub(super) const VALID_FLOW_JSON: &str = include_str!("../../../tests/resources/flows/logFlow.json");
pub(super) const INVALID_FLOW_JSON: &str = include_str!("../../../tests/resources/flows/invalid/missingEndNodeFlow.json");

pub(super) fn valid_flow_document() -> serde_json::Value {
    serde_json::from_str(VALID_FLOW_JSON).expect("valid flow JSON")
}

pub(super) fn create_state(flow_registry: FlowRegistry) -> (ApiState, mpsc::Receiver<SchedulerCommand>) {
    let (scheduler_tx, scheduler_rx) = mpsc::channel(8);
    let handle = PrometheusBuilder::new().build_recorder().handle();
    let flow_store = FlowStore::open(Path::new(":memory:")).expect("failed to open in-memory flow store");
    (ApiState::new(handle, Arc::new(flow_registry), Arc::new(flow_store), scheduler_tx), scheduler_rx)
}

pub(super) async fn body_json(response: Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}
