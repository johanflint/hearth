use crate::sse::server_sent_event::ServerSentEvent;
use futures::StreamExt;
use reqwest::{Client, StatusCode};
use serde::de::DeserializeOwned;
use std::error::Error;
use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
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
    pub send_timeout_ms: Duration,
}

#[instrument(skip_all)]
pub async fn listen<T>(tx: Sender<ServerSentEvent<T>>, client: &Client, config: &Config) -> Result<(), Box<dyn Error>>
where
    T: DeserializeOwned + Debug + 'static,
{
    let strategy = ExponentialBackoff::from_millis(config.retry_ms).factor(2).max_delay(config.retry_max_delay).map(jitter);

    info!("Connecting to SSE stream {}...", config.url);
    let last_event_id = Arc::new(Mutex::new(None::<String>));
    Retry::start(strategy, || {
        let tx = tx.clone();
        let client = client.clone();
        let last_event_id = last_event_id.clone();

        async move {
            match connect_sse_stream::<T>(tx, &client, &config, last_event_id).await {
                Ok(_) => {
                    info!("✅ SSE stream ended gracefully. Restarting...");
                    Err("Stream ended") // Triggers retry
                }
                Err(e) => {
                    warn!("⚠️ SSE error: {}. Retrying...", e);
                    Err("SSE failed") // Triggers retry
                }
            }
        }
    })
    .await?;

    Ok(())
}

async fn connect_sse_stream<T>(tx: Sender<ServerSentEvent<T>>, client: &Client, config: &Config, last_event_id: Arc<Mutex<Option<String>>>) -> Result<(), Box<dyn Error>>
where
    T: DeserializeOwned + Debug + 'static,
{
    let url = format!("{}/eventstream/clip/v2", config.url);
    let mut request = client.get(&url).header("Accept", "text/event-stream");

    let current_id = { last_event_id.lock().await.clone() };
    if let Some(id) = current_id {
        request = request.header("Last-Event-ID", id);
    }
    let response = request.send().await?.error_for_status()?;

    if response.status() == StatusCode::OK {
        info!(status = %response.status(), "Connecting to SSE stream {}... OK", config.url);
    }

    let mut stream = response.bytes_stream();
    let mut buffer: Vec<u8> = Vec::new();

    loop {
        let event = timeout(config.stale_connection_timeout_ms, stream.next()).await;
        match event {
            Ok(Some(Ok(chunk))) => {
                buffer.extend_from_slice(&chunk);

                while let Some((idx, separator_length)) = find_separator(&buffer) {
                    // Drain including separator, then strip it
                    let mut event = buffer.drain(..idx + separator_length).collect::<Vec<u8>>();
                    event.truncate(event.len() - separator_length);

                    let raw = match String::from_utf8(event) {
                        Ok(string) => string,
                        Err(e) => {
                            error!("❌ SSE UTF-8 decode error: {}", e);
                            continue;
                        }
                    };

                    debug!("🔸 Received event: {:?}", raw.trim());
                    let event = match ServerSentEvent::<T>::from_str(&raw) {
                        Ok(event) => event,
                        Err(e) => {
                            warn!("⚠️ Failed to parse SSE event, skipping it: {}", e);
                            continue;
                        }
                    };


                    // Track last event id for resume support
                    if let Some(id) = &event.id {
                        let mut guard = last_event_id.lock().await;
                        *guard = Some(id.clone());
                    }

                    if timeout(config.send_timeout_ms, tx.send(event)).await.is_err() {
                        warn!("⏳ SSE listener stalled for {}ms while forwarding event. Reconnecting...", config.send_timeout_ms.as_millis());
                        return Err("SSE listener stalled".into());
                    }
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

fn find_separator(buf: &[u8]) -> Option<(usize, usize)> {
    if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
        return Some((i, 4));
    }

    if let Some(i) = buf.windows(2).position(|w| w == b"\n\n") {
        return Some((i, 2));
    }

    None
}
