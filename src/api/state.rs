use crate::flow_service::FlowService;
use metrics_exporter_prometheus::PrometheusHandle;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ApiState {
    pub(super) prometheus_handle: Arc<PrometheusHandle>,
    pub(super) flow_service: Arc<FlowService>,
}

impl ApiState {
    pub fn new(
        prometheus_handle: PrometheusHandle,
        flow_service: Arc<FlowService>,
    ) -> Self {
        ApiState {
            prometheus_handle: Arc::new(prometheus_handle),
            flow_service,
        }
    }
}
