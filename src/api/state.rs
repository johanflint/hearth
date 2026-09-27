use metrics_exporter_prometheus::PrometheusHandle;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ApiState {
    pub(super) prometheus_handle: Arc<PrometheusHandle>,
}

impl ApiState {
    pub fn new(prometheus_handle: PrometheusHandle) -> Self {
        ApiState {
            prometheus_handle: Arc::new(prometheus_handle),
        }
    }
}
