use crate::flow_registry::FlowRegistry;
use metrics_exporter_prometheus::PrometheusHandle;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ApiState {
    pub(super) prometheus_handle: Arc<PrometheusHandle>,
    pub(super) flow_registry: Arc<FlowRegistry>,
}

impl ApiState {
    pub fn new(prometheus_handle: PrometheusHandle, flow_registry: Arc<FlowRegistry>) -> Self {
        ApiState {
            prometheus_handle: Arc::new(prometheus_handle),
            flow_registry,
        }
    }
}
