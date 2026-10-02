use crate::flow_engine::SchedulerCommand;
use crate::flow_registry::FlowRegistry;
use crate::flow_service::FlowService;
use crate::flow_store::FlowStore;
use metrics_exporter_prometheus::PrometheusHandle;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;

#[derive(Debug, Clone)]
pub struct ApiState {
    pub(super) prometheus_handle: Arc<PrometheusHandle>,
    pub(super) flow_service: Arc<FlowService>,
    pub(super) flow_registry: Arc<FlowRegistry>,
    pub(super) flow_store: Arc<FlowStore>,
    pub(super) scheduler_tx: Sender<SchedulerCommand>,
}

impl ApiState {
    pub fn new(
        prometheus_handle: PrometheusHandle,
        flow_service: Arc<FlowService>,
        flow_registry: Arc<FlowRegistry>,
        flow_store: Arc<FlowStore>,
        scheduler_tx: Sender<SchedulerCommand>,
    ) -> Self {
        ApiState {
            prometheus_handle: Arc::new(prometheus_handle),
            flow_service,
            flow_registry,
            flow_store,
            scheduler_tx,
        }
    }
}
