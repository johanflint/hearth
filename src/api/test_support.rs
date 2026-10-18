use crate::api::ApiState;
use crate::database::{Database, DatabaseError};
use crate::flow_engine::SchedulerCommand;
use crate::flow_registry::FlowRegistry;
use crate::flow_service::FlowService;
use crate::flow_store::FlowStore;
use crate::preset_registry::PresetRegistry;
use crate::preset_service::PresetService;
use crate::preset_store::PresetStore;
use crate::store::StoreSnapshot;
use axum::response::Response;
use chrono::Utc;
use http_body_util::BodyExt;
use metrics_exporter_prometheus::PrometheusBuilder;
use rusqlite::params;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{mpsc, watch};

pub(super) const VALID_FLOW_ID: &str = "01K7KK6H5R7Y72QJEJSJQCKMRQ";
pub(super) const VALID_FLOW_JSON: &str = include_str!("../../tests/resources/flows/logFlow.json");
pub(super) const INVALID_FLOW_JSON: &str = include_str!("../../tests/resources/flows/invalid/missingEndNodeFlow.json");

pub(super) fn valid_flow_document() -> serde_json::Value {
    serde_json::from_str(VALID_FLOW_JSON).expect("valid flow JSON")
}

/// The API state, plus the dependencies behind its flow service to seed and inspect them.
/// Bind `scheduler_rx` when the handler reserves a scheduler permit: dropping it, e.g. by leaving it out with `..`,
/// makes the scheduler unavailable.
pub(super) struct Fixture {
    pub(super) state: ApiState,
    pub(super) flow_store: Arc<FlowStore>,
    pub(super) flow_registry: Arc<FlowRegistry>,
    pub(super) scheduler_rx: mpsc::Receiver<SchedulerCommand>,
}

pub(super) fn create_state(flow_registry: FlowRegistry) -> Fixture {
    create_state_with_database(flow_registry).0
}

/// Also returns the database behind the stores, to seed rows the stores can't write
pub(super) fn create_state_with_database(flow_registry: FlowRegistry) -> (Fixture, Database) {
    let (scheduler_tx, scheduler_rx) = mpsc::channel(8);
    let handle = PrometheusBuilder::new().build_recorder().handle();
    let database = Database::open(Path::new(":memory:")).expect("failed to open in-memory flow store");
    let flow_store = Arc::new(FlowStore::new(database.clone()));
    let flow_registry = Arc::new(flow_registry);
    let preset_store = Arc::new(PresetStore::new(database.clone()));
    let preset_registry = Arc::new(PresetRegistry::new(vec![]));
    let (_, snapshot_rx) = watch::channel(StoreSnapshot::default());
    let flow_service = Arc::new(FlowService::new(
        Arc::clone(&flow_store),
        Arc::clone(&flow_registry),
        preset_registry.clone(),
        scheduler_tx,
        snapshot_rx,
    ));
    let preset_service = Arc::new(PresetService::new(preset_store, preset_registry));
    let state = ApiState::new(handle, flow_service, preset_service);
    (Fixture { state, flow_store, flow_registry, scheduler_rx }, database)
}

pub(super) async fn body_json(response: Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

/// Writes a preset row directly, with a chosen id
pub(super) async fn insert_preset_row(database: &Database, id: &str, name: &str, value: &str) {
    let (id, name, value) = (id.to_string(), name.to_string(), value.to_string());
    database
        .call(move |conn| {
            conn.execute(
                "INSERT INTO presets (id, revision, name, value, created_at) VALUES (?1, 0, ?2, ?3, ?4)",
                params![id, name, value, Utc::now().to_rfc3339()],
            )
            .map_err(DatabaseError::from)
        })
        .await
        .expect("insert preset row");
}

/// Writes a preset's revision directly, the store only ever increments it by one
pub(super) async fn set_preset_revision(database: &Database, id: &str, revision: i64) {
    let id = id.to_string();
    database
        .call(move |conn| {
            conn.execute("UPDATE presets SET revision = ?2 WHERE id = ?1", params![id, revision])
                .map_err(DatabaseError::from)
        })
        .await
        .expect("set preset revision");
}
