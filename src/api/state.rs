use crate::core_tasks::CoreTasks;
use crate::database::Database;
use crate::flow_service::FlowService;
use crate::preset_service::PresetService;
use metrics_exporter_prometheus::PrometheusHandle;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ApiState {
    pub(super) prometheus_handle: Arc<PrometheusHandle>,
    pub(super) database: Database,
    pub(super) core_tasks: Arc<CoreTasks>,
    pub(super) flow_service: Arc<FlowService>,
    pub(super) preset_service: Arc<PresetService>,
}

impl ApiState {
    pub fn new(
        prometheus_handle: PrometheusHandle,
        database: Database,
        core_tasks: Arc<CoreTasks>,
        flow_service: Arc<FlowService>,
        preset_service: Arc<PresetService>,
    ) -> Self {
        ApiState { prometheus_handle: Arc::new(prometheus_handle), database, core_tasks, flow_service, preset_service }
    }
}
