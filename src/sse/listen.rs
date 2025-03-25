use crate::sse::server_sent_event::ServerSentEvent;
use futures::StreamExt;
use reqwest::{Client, StatusCode};
use serde::de::DeserializeOwned;
use std::error::Error;
use std::fmt::Debug;
use std::time::Duration;
use tokio::sync::mpsc::Sender;
use tokio::time::timeout;
use tokio_retry::Retry;
use tokio_retry::strategy::{ExponentialBackoff, jitter};
use tracing::{debug, error, info, instrument, warn};

#[derive(Debug)]
pub struct Config {
    pub url: String,
    pub retry_ms: u64,
    pub retry_max_delay: Duration,
    pub stale_connection_timeout_ms: Duration,
}

#[instrument(skip_all)]
pub async fn listen<T>(tx: Sender<ServerSentEvent<T>>, client: &Client, config: &Config) -> Result<(), Box<dyn Error>>
where
    T: DeserializeOwned + Debug + 'static,
{
    let strategy = ExponentialBackoff::from_millis(config.retry_ms)
        .factor(2)
        .max_delay(config.retry_max_delay)
        .map(jitter);

    info!("Connecting to SSE stream {}...", config.url);
    Retry::spawn(strategy, || async {
        match connect_sse_stream::<T>(tx.clone(), &client, config).await {
            Ok(_) => {
                info!("✅ SSE stream ended gracefully. Restarting...");
                Err("Stream ended") // Triggers retry
            }
            Err(e) => {
                warn!("⚠️ SSE error: {}. Retrying...", e);
                Err("SSE failed") // Triggers retry
            }
        }
    })
    .await?;

    Ok(())
}

async fn connect_sse_stream<T>(tx: Sender<ServerSentEvent<T>>, client: &Client, config: &Config) -> Result<(), Box<dyn Error>>
where
    T: DeserializeOwned + Debug + 'static,
{
    let url = format!("{}/eventstream/clip/v2", config.url);
    let response = client.get(&url).header("Accept", "text/event-stream").send().await?.error_for_status()?;

    if response.status() == StatusCode::OK {
        info!(status = %response.status(), "Connecting to SSE stream {}... OK", config.url);
    }

    let mut stream = response.bytes_stream();
    loop {
        let event = timeout(config.stale_connection_timeout_ms, stream.next()).await;
        match event {
            Ok(Some(Ok(chunk))) => {
                if let Ok(text) = String::from_utf8(chunk.to_vec()) {
                    let event = ServerSentEvent::<T>::from_str(&text)?;
                    debug!(event = text.trim(), "🔸 Received event: {:?}", event);
                    tx.send(event).await?;
                }
            }
            Ok(Some(Err(e))) => {
                error!("❌ SSE stream error: {}", e);
                return Err(Box::new(e));
            }
            Ok(None) => {
                warn!("🔴 SSE stream ended");
                return Err("Stream closed".into());
            }
            Err(_) => {
                warn!("⏳ No data for {} seconds. Reconnecting...", config.stale_connection_timeout_ms.as_secs());
                return Err("Timeout".into());
            }
        }
    }
}
