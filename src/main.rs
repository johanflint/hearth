use crate::app_config::AppConfig;
use crate::domain::events::Event;
use crate::sse_listen::listen;
use crate::store::Store;
use crate::store_listener::store_listener;
use tokio::sync::mpsc;
use tokio::task;
use tracing::{info, trace};

mod app_config;
mod domain;
mod flow_engine;
mod flow_loader;
mod hue;
mod sse_listen;
mod store;
mod store_listener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).init();

    info!("🪵 Starting {} v{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));

    let config = AppConfig::load();
    info!("✅  Loaded configuration");

    let hue_client = hue::client::new_client(&config)?;

    let (tx, rx) = mpsc::channel::<Event>(config.core().store_buffer_size());
    let mut store = Store::new(rx);
    let notifier_rx = store.notifier();

    task::spawn(async move {
        store_listener(notifier_rx).await;
    });
    info!("✅  Initialized store listener");

    task::spawn(async move {
        store.listen().await;
    });
    info!("✅  Initialized store");

    let hue_devices = hue::observer::observe(&hue_client, &config).await.expect("Could not observe Hue");
    trace!("Observed Hue devices: {:?}", &hue_devices);
    tx.send(Event::DiscoveredDevices(hue_devices))
        .await
        .expect("Could not send discovered devices to the store");

    info!("✅  Discovered all devices");
    info!("🔥 {} is up and running", env!("CARGO_PKG_NAME"));

    listen(&hue_client, &config).await.expect("Could not listen to SSE stream");

    Ok(())
}
