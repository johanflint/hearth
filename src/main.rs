use crate::api::ApiState;
use crate::app_config::AppConfig;
use crate::core_tasks::CoreTasks;
use crate::database::Database;
use crate::domain::{Event, Group, Zone, controller_registry};
use crate::flow_engine::{SchedulerCommand, scheduler};
use crate::flow_registry::FlowRegistry;
use crate::integration::Discovery;
use crate::integration_list::{enabled_only, integrations};
use crate::mdns_browser::MdnsBrowser;
use crate::metrics_layer::MetricsLayer;
use crate::mqtt::MqttService;
use crate::preset_registry::PresetRegistry;
use crate::repositories::{FlowRepository, GroupRepository, PresetRepository, ZoneRepository};
use crate::services::{FlowService, GroupService, PresetService, ZoneService};
use crate::store::Store;
use crate::store_listener::store_listener;
use metrics_exporter_prometheus::PrometheusBuilder;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::{signal, task};
use tracing::{error, info, trace, warn};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

mod api;
mod app_config;
mod claim_registry;
mod core_tasks;
mod database;
mod domain;
mod execute_flows;
mod extensions;
mod flow_engine;
mod flow_loader;
mod flow_registry;
mod geo_location_deserializer;
mod hue;
mod integration;
mod integration_list;
mod mdns_browser;
mod metrics;
mod metrics_layer;
mod mqtt;
mod preset_loader;
mod preset_registry;
mod property_changed_reducer;
mod repositories;
mod server;
mod services;
mod shelly;
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

    let mdns_browser = match MdnsBrowser::spawn(&["_shelly._tcp.local."]) {
        Ok(browser) => {
            info!("✅  Started mdns browser");
            Some(browser)
        }
        Err(err) => {
            warn!("⚠️ mDNS unavailable: {err}");
            None
        }
    };

    let mqtt_service = if config.mqtt().enabled() {
        Some(MqttService::spawn(&config))
    } else {
        info!("⏭️  Skipping MQTT connection, disabled in config");
        None
    };

    let database = Database::open(Path::new(config.db().file_name()))?;
    info!("✅  Started database");

    let flow_repository = Arc::new(FlowRepository::new(database.clone()));
    let flows = flow_loader::load_flows_from_repository(&flow_repository).await?;
    let flow_registry = Arc::new(FlowRegistry::new(flows));
    info!("✅  Loaded flows");

    let preset_repository = Arc::new(PresetRepository::new(database.clone()));
    let presets = preset_loader::load_presets_from_repository(&preset_repository).await?;
    let preset_registry = Arc::new(PresetRegistry::new(presets));
    info!("✅  Loaded presets");

    let (tx, rx) = mpsc::channel::<Event>(config.core().store_buffer_size());
    let group_repository = Arc::new(GroupRepository::new(database.clone()));
    let groups = group_repository.list().await?.into_iter().map(Group::from).collect();
    let zone_repository = Arc::new(ZoneRepository::new(database.clone()));
    let zones = zone_repository.list().await?.into_iter().map(Zone::from).collect();
    let (mut store, store_reactive_rx) = Store::new(rx, groups, zones);

    let (scheduler_tx, scheduler_rx) = mpsc::channel::<SchedulerCommand>(32);
    let scheduler_tx_clone = scheduler_tx.clone();
    let store_rx = store.notifier();
    let flow_registry_clone = flow_registry.clone();
    let preset_registry_clone = preset_registry.clone();
    let geo_location_clone = config.geo_location().clone();
    let scheduler_handle = task::spawn(async move {
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
    let store_listener_handle = task::spawn(async move {
        store_listener(store_reactive_rx, flow_registry_clone, preset_registry_clone, scheduler_tx_clone, geo_location)
            .await;
    });
    info!("✅  Initialized store listener");

    let store_rx_clone = store.notifier();
    let integrations_rx_clone = store.notifier();
    let group_service_rx_clone = store.notifier();
    let zone_service_rx_clone = store.notifier();
    let store_handle = task::spawn(async move {
        store.listen().await;
    });
    info!("✅  Initialized store");

    let mdns_rx = mdns_browser.as_ref().map(MdnsBrowser::notifier);
    let integrations = enabled_only(integrations(config.clone(), integrations_rx_clone, mdns_rx, mqtt_service)?);
    for integration in &integrations {
        controller_registry::register(integration.controller());
    }
    info!("✅  Initialized controllers");

    let flow_service = Arc::new(FlowService::new(
        flow_repository,
        flow_registry.clone(),
        preset_registry.clone(),
        scheduler_tx,
        store_rx_clone,
    ));
    let preset_service = Arc::new(PresetService::new(preset_repository, preset_registry, flow_registry.clone()));
    let core_tasks =
        Arc::new(CoreTasks { store: store_handle, scheduler: scheduler_handle, store_listener: store_listener_handle });
    let group_service =
        Arc::new(GroupService::new(group_service_rx_clone, group_repository, flow_registry.clone(), tx.clone()));
    let zone_service = Arc::new(ZoneService::new(zone_service_rx_clone, zone_repository, flow_registry, tx.clone()));
    let api_state = ApiState::new(
        prometheus_handle,
        database,
        core_tasks,
        integrations.clone(),
        flow_service,
        group_service,
        zone_service,
        preset_service,
    );
    server::start(config.core().port(), api_state).await?;
    info!("✅  Initialized server");

    for integration in &integrations {
        let Discovery { devices, groups, zones } = integration.discover().await?;
        trace!("Discovered devices: {:?}", &devices);
        trace!("Discovered groups: {:?}", &groups);
        trace!("Discovered zones: {:?}", &zones);
        tx.send(Event::DiscoveredDevices(devices)).await?;
        tx.send(Event::DiscoveredGroups(groups)).await?;
        tx.send(Event::DiscoveredZones(zones)).await?;
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
