use crate::core_tasks::CoreTasks;
use crate::database::Database;
use crate::integration::Integration;
use crate::services::{FlowService, GroupService, PresetService, ZoneService};
use metrics_exporter_prometheus::PrometheusHandle;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ApiState {
    pub(super) prometheus_handle: Arc<PrometheusHandle>,
    pub(super) database: Database,
    pub(super) core_tasks: Arc<CoreTasks>,
    pub(super) integrations: Arc<[Arc<dyn Integration>]>,
    pub(super) flow_service: Arc<FlowService>,
    pub(super) group_service: Arc<GroupService>,
    pub(super) zone_service: Arc<ZoneService>,
    pub(super) preset_service: Arc<PresetService>,
}

impl ApiState {
    pub fn new(
        prometheus_handle: PrometheusHandle,
        database: Database,
        core_tasks: Arc<CoreTasks>,
        integrations: Vec<Arc<dyn Integration>>,
        flow_service: Arc<FlowService>,
        group_service: Arc<GroupService>,
        zone_service: Arc<ZoneService>,
        preset_service: Arc<PresetService>,
    ) -> Self {
        ApiState {
            prometheus_handle: Arc::new(prometheus_handle),
            database,
            core_tasks,
            integrations: integrations.into(),
            flow_service,
            group_service,
            zone_service,
            preset_service,
        }
    }
}
