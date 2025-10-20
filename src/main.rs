use crate::app_config::AppConfig;
use crate::domain::controller_registry;
use crate::domain::events::Event;
use crate::flow_engine::{SchedulerCommand, scheduler};
use crate::flow_registry::FlowRegistry;
use crate::store::Store;
use crate::store_listener::store_listener;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::{signal, task};
use tracing::{error, info, trace};

mod app_config;
mod domain;
mod execute_flows;
mod extensions;
mod flow_engine;
mod flow_loader;
mod flow_registry;
mod hue;
mod property_changed_reducer;
mod sse;
mod store;
mod store_listener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).init();

    info!("🪵 Starting {} v{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));

    let config = Arc::new(AppConfig::load());
    info!("✅  Loaded configuration");

    let flows = flow_loader::load_flows_from(config.flows().directory(), "json").await.unwrap_or_else(|_| Vec::new()); // Errors are already logged in the function
    let flow_registry = Arc::new(FlowRegistry::new(flows));
    info!("✅  Loaded flows");

    let (tx, rx) = mpsc::channel::<Event>(config.core().store_buffer_size());
    let mut store = Store::new(rx);

    let (scheduler_tx, scheduler_rx) = mpsc::channel::<SchedulerCommand>(32);
    let scheduler_tx_clone = scheduler_tx.clone();
    let store_rx = store.notifier();
    let registry_clone = flow_registry.clone();
    task::spawn(async move {
        scheduler(scheduler_tx_clone, scheduler_rx, store_rx, registry_clone).await;
    });
    info!("✅  Started scheduler");

    for scheduled_flow in flow_registry.scheduled_flows() {
        scheduler_tx
            .send(SchedulerCommand::Schedule {
                flow_id: scheduled_flow.id().to_string(),
            })
            .await?;
    }
    info!("✅  Scheduled flows");

    let hue_client = hue::new_client(&config)?;
    let hue_controller = hue::Controller::new(hue_client.clone(), config.clone());
    controller_registry::register(Arc::new(hue_controller));
    info!("✅  Initialized controllers");

    let store_rx = store.notifier();
    task::spawn(async move {
        store_listener(store_rx, flow_registry, scheduler_tx).await;
    });
    info!("✅  Initialized store listener");

    task::spawn(async move {
        store.listen().await;
    });
    info!("✅  Initialized store");

    let hue_devices = hue::discover(&hue_client, &config).await.expect("Could not discover Hue devices");
    trace!("Observed Hue devices: {:?}", &hue_devices);
    tx.send(Event::DiscoveredDevices(hue_devices))
        .await
        .expect("Could not send discovered devices to the store");

    info!("✅  Discovered all devices");
    info!("🔥 {} is up and running", env!("CARGO_PKG_NAME"));

    hue::observe(tx, &hue_client, &config).await?;

    match signal::ctrl_c().await {
        Ok(()) => {}
        Err(err) => {
            error!("Unable to listen for shutdown signal: {}", err);
        }
    }

    Ok(())
}
