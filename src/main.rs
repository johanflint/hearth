use crate::api::ApiState;
use crate::app_config::AppConfig;
use crate::database::Database;
use crate::domain::{Event, controller_registry};
use crate::flow_engine::{SchedulerCommand, scheduler};
use crate::flow_registry::FlowRegistry;
use crate::flow_service::FlowService;
use crate::flow_store::FlowStore;
use crate::integration::Discovery;
use crate::integration_list::{enabled_only, integrations};
use crate::metrics_layer::MetricsLayer;
use crate::preset_registry::PresetRegistry;
use crate::preset_service::PresetService;
use crate::preset_store::PresetStore;
use crate::store::Store;
use crate::store_listener::store_listener;
use metrics_exporter_prometheus::PrometheusBuilder;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::{signal, task};
use tracing::{error, info, trace};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

mod api;
mod app_config;
mod claim_registry;
mod database;
mod domain;
mod execute_flows;
mod extensions;
mod flow_engine;
mod flow_loader;
mod flow_registry;
mod flow_service;
mod flow_store;
mod geo_location_deserializer;
mod hue;
mod integration;
mod integration_list;
mod metrics;
mod metrics_layer;
mod preset_loader;
mod preset_registry;
mod preset_service;
mod preset_store;
mod property_changed_reducer;
mod server;
mod sse;
mod store;
mod store_listener;
#[cfg(test)]
mod test_support;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,hearth::flow_engine=warn"))
        .add_directive("hearth::flow_log=info".parse()?);
    tracing_subscriber::registry().with(env_filter).with(tracing_subscriber::fmt::layer()).with(MetricsLayer).init();

    let prometheus_handle = PrometheusBuilder::new().install_recorder()?;
    metrics::describe();

    info!("🪵 Starting {} v{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));

    let config = Arc::new(AppConfig::load());
    info!("✅  Loaded configuration");

    let database = Database::open(Path::new(config.db().file_name()))?;
    info!("✅  Started database");

    let flow_store = Arc::new(FlowStore::new(database.clone()));
    let flows = flow_loader::load_flows_from_store(&flow_store).await?;
    let flow_registry = Arc::new(FlowRegistry::new(flows));
    info!("✅  Loaded flows");

    let preset_store = Arc::new(PresetStore::new(database));
    let presets = preset_loader::load_presets_from_store(&preset_store).await?;
    let preset_registry = Arc::new(PresetRegistry::new(presets));
    info!("✅  Loaded presets");

    let (tx, rx) = mpsc::channel::<Event>(config.core().store_buffer_size());
    let (mut store, store_reactive_rx) = Store::new(rx);

    let (scheduler_tx, scheduler_rx) = mpsc::channel::<SchedulerCommand>(32);
    let scheduler_tx_clone = scheduler_tx.clone();
    let store_rx = store.notifier();
    let flow_registry_clone = flow_registry.clone();
    let preset_registry_clone = preset_registry.clone();
    let geo_location_clone = config.geo_location().clone();
    task::spawn(async move {
        scheduler(
            scheduler_tx_clone,
            scheduler_rx,
            store_rx,
            flow_registry_clone,
            preset_registry_clone,
            geo_location_clone,
        )
        .await;
    });
    info!("✅  Started scheduler");

    for scheduled_flow in flow_registry.scheduled_flows() {
        scheduler_tx
            .send(SchedulerCommand::Reconcile {
                flow_id: scheduled_flow.id().to_string(),
                revision: scheduled_flow.revision,
            })
            .await?;
    }
    info!("✅  Scheduled flows");

    let geo_location = config.geo_location().clone();
    let flow_registry_clone = flow_registry.clone();
    let preset_registry_clone = preset_registry.clone();
    let scheduler_tx_clone = scheduler_tx.clone();
    task::spawn(async move {
        store_listener(store_reactive_rx, flow_registry_clone, preset_registry_clone, scheduler_tx_clone, geo_location)
            .await;
    });
    info!("✅  Initialized store listener");

    let store_rx_clone = store.notifier();
    let integrations_rx_clone = store.notifier();
    task::spawn(async move {
        store.listen().await;
    });
    info!("✅  Initialized store");

    let integrations = enabled_only(integrations(config.clone(), integrations_rx_clone)?);
    for integration in &integrations {
        controller_registry::register(integration.controller());
    }
    info!("✅  Initialized controllers");

    let flow_service = Arc::new(FlowService::new(
        flow_store,
        flow_registry.clone(),
        preset_registry.clone(),
        scheduler_tx,
        store_rx_clone,
    ));
    let preset_service = Arc::new(PresetService::new(preset_store, preset_registry, flow_registry));
    let api_state = ApiState::new(prometheus_handle, flow_service, preset_service);
    server::start(config.core().port(), api_state).await?;
    info!("✅  Initialized server");

    for integration in &integrations {
        let Discovery { devices, groups } = integration.discover().await?;
        trace!("Discovered devices: {:?}", &devices);
        trace!("Discovered groups: {:?}", &groups);
        tx.send(Event::DiscoveredDevices(devices)).await?;
        tx.send(Event::DiscoveredGroups(groups)).await?;
    }
    info!("✅  Discovered all devices");

    for integration in integrations {
        trace!("Observing '{}'", integration.name());
        let tx = tx.clone();
        integration.observe(tx).await?;
    }
    info!("✅  Observing all integrations");
    info!("🔥 {} is up and running", env!("CARGO_PKG_NAME"));

    if let Err(err) = signal::ctrl_c().await {
        error!("Unable to listen for shutdown signal: {}", err);
    }

    Ok(())
}
