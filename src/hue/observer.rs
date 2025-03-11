use crate::app_config::AppConfig;
use crate::hue::domain::ServerSentEventPayload;
use crate::sse;
use crate::sse::{Config, ServerSentEvent};
use reqwest::Client;
use std::error::Error;
use tokio::sync::mpsc;
use tokio::task;
use tracing::{info, instrument};

type HueEvent = ServerSentEvent<Vec<ServerSentEventPayload>>;

#[instrument(skip_all)]
pub async fn observe(client: &Client, config: &AppConfig) -> Result<(), Box<dyn Error>> {
    let (tx, mut rx) = mpsc::channel::<HueEvent>(config.core().store_buffer_size());

    let sse_config = Config {
        url: config.hue().url().to_owned(),
        retry_ms: config.hue().retry_ms(),
        retry_max_delay: config.hue().retry_max_delay_ms(),
        stale_connection_timeout_ms: config.hue().stale_connection_timeout_ms(),
    };

    task::spawn(async move {
        while let Some(hue_event) = rx.recv().await {
            info!("🔹 {:?}", hue_event);
        }
    });

    sse::listen::<Vec<ServerSentEventPayload>>(tx, &client, &sse_config)
        .await
        .expect("Could not listen to SSE stream");

    Ok(())
}
