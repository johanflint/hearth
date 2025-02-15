use crate::app_config::AppConfig;
use crate::sse_listen::listen;
use reqwest::Client;
use tracing::info;

mod app_config;
mod sse_listen;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).init();

    let config = AppConfig::load();
    info!("Loaded configuration file: {:?}", &config);

    let client = Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .expect("Failed to build client");

    listen(&client, &config).await.expect("Could not listen to SSE stream");
}
