use crate::metrics::{Metric, ResultOutcomeLabel};
use crate::sse::server_sent_event::ServerSentEvent;
use futures::StreamExt;
use metrics::counter;
use reqwest::{Client, StatusCode};
use serde::de::DeserializeOwned;
use std::error::Error;
use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::sync::mpsc::Sender;
use tokio::sync::mpsc::error::SendTimeoutError;
use tokio::time::timeout;
use tokio_retry::Retry;
use tokio_retry::strategy::{ExponentialBackoff, jitter};
use tracing::{debug, error, info, instrument, warn};

#[derive(Debug, Clone)]
pub struct Config {
    pub url: String,
    pub retry_ms: u64,
    pub retry_max_delay: Duration,
    pub stale_connection_timeout_ms: Duration,
    pub send_timeout_ms: Duration,
}

/// Listens to the SSE stream and reconnects with backoff when it drops.
/// `on_connection_change` is called with `true` once connected and with `false` when the connection is lost or fails.
#[instrument(skip_all)]
pub async fn listen<T>(
    tx: Sender<ServerSentEvent<T>>,
    client: &Client,
    config: &Config,
    on_connection_change: impl Fn(bool) + Send + Sync,
) -> Result<(), Box<dyn Error>>
where
    T: DeserializeOwned + Debug + 'static,
{
    let strategy =
        ExponentialBackoff::from_millis(config.retry_ms).factor(2).max_delay(config.retry_max_delay).map(jitter);

    info!("Connecting to SSE stream {}...", config.url);
    let last_event_id = Arc::new(Mutex::new(None::<String>));
    let on_connection_change = &on_connection_change;
    Retry::start(strategy, || {
        let tx = tx.clone();
        let client = client.clone();
        let last_event_id = last_event_id.clone();

        async move {
            match connect_sse_stream::<T>(tx, &client, &config, last_event_id, on_connection_change).await {
                Ok(_) => {
                    on_connection_change(false);
                    info!("✅ SSE stream ended gracefully. Restarting...");
                    Err("Stream ended") // Triggers retry
                }
                Err(e) => {
                    on_connection_change(false);
                    warn!("⚠️ SSE error: {:?}. Retrying...", e);
                    Err("SSE failed") // Triggers retry
                }
            }
        }
    })
    .await?;

    Ok(())
}

async fn connect_sse_stream<T>(
    tx: Sender<ServerSentEvent<T>>,
    client: &Client,
    config: &Config,
    last_event_id: Arc<Mutex<Option<String>>>,
    on_connection_change: &impl Fn(bool),
) -> Result<(), Box<dyn Error>>
where
    T: DeserializeOwned + Debug + 'static,
{
    let url = format!("{}/eventstream/clip/v2", config.url);
    let mut request = client
        .get(&url)
        .timeout(Duration::MAX) // Overrides the client_request_timeout_ms config
        .header("Accept", "text/event-stream");

    let current_id = { last_event_id.lock().await.clone() };
    if let Some(id) = current_id {
        request = request.header("Last-Event-ID", id);
    }
    let result = request.send().await.and_then(|r| r.error_for_status());
    counter!(Metric::SseConnectionAttempts.name(), "result" => result.metric_label()).increment(1);

    let response = result?;
    on_connection_change(true);
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

                    let event_id = event.id.clone();
                    match tx.send_timeout(event, config.send_timeout_ms).await {
                        Ok(()) => {
                            // Track last event id for resume support
                            if let Some(id) = event_id {
                                let mut guard = last_event_id.lock().await;
                                *guard = Some(id);
                            }
                        }
                        Err(SendTimeoutError::Timeout(_)) => {
                            warn!(
                                "⏳ SSE listener stalled for {}ms while forwarding event. Reconnecting...",
                                config.send_timeout_ms.as_millis()
                            );
                            return Err("SSE listener stalled".into());
                        }
                        Err(SendTimeoutError::Closed(_)) => {
                            error!("❌ SSE forwarding channel closed. Reconnecting...");
                            return Err("SSE forwarding channel closed".into());
                        }
                    }
                }
            }
            Ok(Some(Err(e))) => {
                // SSE stream error
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use tokio::sync::mpsc;
    use tokio::task::JoinHandle;

    fn config(url: String) -> Config {
        Config {
            url,
            retry_ms: 1,
            retry_max_delay: Duration::from_millis(10),
            stale_connection_timeout_ms: Duration::from_secs(5),
            send_timeout_ms: Duration::from_secs(1),
        }
    }

    /// Spawns `listen` and returns the connection changes it reports, in order. It retries forever, so abort it.
    fn spawn_listen(url: String) -> (JoinHandle<()>, mpsc::UnboundedReceiver<bool>) {
        let (event_tx, event_rx) = mpsc::channel::<ServerSentEvent<Value>>(8);
        let (change_tx, change_rx) = mpsc::unbounded_channel();
        let handle = tokio::spawn(async move {
            let _event_rx = event_rx; // Keeps the channel open
            let _ = listen(event_tx, &Client::new(), &config(url), move |connected| {
                let _ = change_tx.send(connected);
            })
            .await;
        });
        (handle, change_rx)
    }

    async fn next_change(changes: &mut mpsc::UnboundedReceiver<bool>) -> bool {
        timeout(Duration::from_secs(5), changes.recv()).await.expect("timed out").expect("listen stopped")
    }

    #[tokio::test]
    async fn listen_reports_connected_and_then_disconnected_when_the_stream_ends() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/eventstream/clip/v2")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body("id: 1\ndata: []\n\n")
            .create_async()
            .await;
        let (handle, mut changes) = spawn_listen(server.url());

        assert!(next_change(&mut changes).await);
        assert!(!next_change(&mut changes).await);
        handle.abort();
    }

    #[tokio::test]
    async fn listen_keeps_reporting_disconnected_when_the_server_responds_with_an_error() {
        let mut server = mockito::Server::new_async().await;
        server.mock("GET", "/eventstream/clip/v2").with_status(500).expect_at_least(2).create_async().await;
        let (handle, mut changes) = spawn_listen(server.url());

        assert!(!next_change(&mut changes).await);
        assert!(!next_change(&mut changes).await);
        handle.abort();
    }
}
