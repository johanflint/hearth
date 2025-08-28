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
}

#[instrument(skip_all)]
pub async fn listen<T>(tx: Sender<ServerSentEvent<T>>, client: &Client, config: &Config) -> Result<(), Box<dyn Error>>
where
    T: DeserializeOwned + Debug + 'static,
{
    let strategy = ExponentialBackoff::from_millis(config.retry_ms).factor(2).max_delay(config.retry_max_delay).map(jitter);

    info!("Connecting to SSE stream {}...", config.url);
    let last_event_id = Arc::new(Mutex::new(None::<String>));
    Retry::spawn(strategy, || {
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
    let mut buffer = String::new();

    loop {
        let event = timeout(config.stale_connection_timeout_ms, stream.next()).await;
        match event {
            Ok(Some(Ok(chunk))) => {
                match String::from_utf8(chunk.to_vec()) {
                    Ok(string) => buffer.push_str(&string),
                    Err(e) => {
                        error!("❌ SSE UTF-8 decode error: {}", e);
                        return Err(Box::new(e));
                    }
                }

                while let Some(mut raw) = extract_next_event(&mut buffer) {
                    if raw.ends_with("\r\n\r\n") {
                        raw.truncate(raw.len() - 4);
                    } else if raw.ends_with("\n\n") {
                        raw.truncate(raw.len() - 2);
                    }

                    let event = ServerSentEvent::<T>::from_str(&raw)?;
                    debug!(event = raw.trim(), "🔸 Received event: {:?}", event);

                    // Track last event id for resume support
                    if let Some(id) = &event.id {
                        let mut guard = last_event_id.lock().await;
                        *guard = Some(id.clone());
                    }

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

// Function to extract next framed SSE event (supports CRLF and LF)
fn extract_next_event(buffer: &mut String) -> Option<String> {
    if let Some(idx) = buffer.find("\r\n\r\n") {
        let event = buffer.drain(..idx + 4).collect::<String>();
        return Some(event);
    }

    if let Some(idx) = buffer.find("\n\n") {
        let event = buffer.drain(..idx + 2).collect::<String>();
        return Some(event);
    }

    None
}
