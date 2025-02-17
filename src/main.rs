use crate::app_config::AppConfig;
use crate::domain::events::Event;
use crate::sse_listen::listen;
use crate::store::Store;
use reqwest::Client;
use tokio::sync::mpsc;
use tokio::task;
use tracing::info;

mod app_config;
mod domain;
mod hue;
mod sse_listen;
mod store;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).init();

    let config = AppConfig::load();
    info!("Loaded configuration file: {:?}", &config);

    let client = Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .expect("Failed to build client");

    let (tx, rx) = mpsc::channel::<Event>(config.core().store_buffer_size());
    task::spawn(async move {
        let mut store = Store::new(rx);
        store.listen().await;
    });

    let hue_devices = hue::observer::observe(&client, &config).await.expect("Could not observe Hue");
    info!("Observed Hue devices: {:?}", &hue_devices);
    tx.send(Event::DiscoveredDevices(hue_devices))
        .await
        .expect("Could not send discovered devices to the store");

    listen(&client, &config).await.expect("Could not listen to SSE stream");
}
