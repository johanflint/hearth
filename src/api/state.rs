use crate::core_tasks::CoreTasks;
use crate::database::Database;
use crate::integration::Integration;
use crate::repositories::UserRepository;
use crate::services::{AttentionService, DeviceService, FlowService, GroupService, PresetService, ZoneService};
use metrics_exporter_prometheus::PrometheusHandle;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ApiState {
    pub(super) prometheus_handle: Arc<PrometheusHandle>,
    pub(super) database: Database,
    pub(super) user_repository: Arc<UserRepository>,
    pub(super) core_tasks: Arc<CoreTasks>,
    pub(super) integrations: Arc<[Arc<dyn Integration>]>,
    pub(super) device_service: Arc<DeviceService>,
    pub(super) flow_service: Arc<FlowService>,
    pub(super) group_service: Arc<GroupService>,
    pub(super) zone_service: Arc<ZoneService>,
    pub(super) preset_service: Arc<PresetService>,
    pub(super) attention_service: Arc<AttentionService>,
}

impl ApiState {
    pub fn new(
        prometheus_handle: PrometheusHandle,
        database: Database,
        user_repository: Arc<UserRepository>,
        core_tasks: Arc<CoreTasks>,
        integrations: Vec<Arc<dyn Integration>>,
        device_service: Arc<DeviceService>,
        flow_service: Arc<FlowService>,
        group_service: Arc<GroupService>,
        zone_service: Arc<ZoneService>,
        preset_service: Arc<PresetService>,
        attention_service: Arc<AttentionService>,
    ) -> Self {
        ApiState {
            prometheus_handle: Arc::new(prometheus_handle),
            database,
            user_repository,
            core_tasks,
            integrations: integrations.into(),
            device_service,
            flow_service,
            group_service,
            zone_service,
            preset_service,
            attention_service,
        }
    }
}
