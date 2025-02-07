use futures::StreamExt;
use reqwest::{Client, StatusCode};
use std::error::Error;
use std::time::Duration;
use tokio::time::timeout;
use tokio_retry::strategy::{jitter, ExponentialBackoff};
use tokio_retry::Retry;
use tracing::{error, info, instrument, warn};

#[instrument(skip(client), fields(url = %url.as_ref()))]
pub async fn listen(client: &Client, url: impl AsRef<str>) -> Result<(), Box<dyn Error>> {
    let strategy = ExponentialBackoff::from_millis(500)
        .factor(2)
        .max_delay(Duration::from_secs(30))
        .map(jitter);

    info!("Connecting to SSE stream {}...", url.as_ref());
    Retry::spawn(strategy, || async {
        match connect_sse_stream(&client, url.as_ref()).await {
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

#[instrument(skip(client, url))]
async fn connect_sse_stream(client: &Client, url: impl AsRef<str>) -> Result<(), Box<dyn Error>> {
    let response = client
        .get(url.as_ref())
        .header("Accept", "text/event-stream")
        .send()
        .await?
        .error_for_status()?;

    if response.status() == StatusCode::OK {
        info!(status = %response.status(), "Connecting to SSE stream {}... OK", url.as_ref());
    }

    let mut stream = response.bytes_stream();
    let timeout_duration = Duration::from_secs(30);

    loop {
        let event = timeout(timeout_duration, stream.next()).await;
        match event {
            Ok(Some(Ok(chunk))) => {
                if let Ok(text) = String::from_utf8(chunk.to_vec()) {
                    for line in text.lines() {
                        if line.starts_with("data:") {
                            let event_data = line.trim_start_matches("data:").trim();
                            info!(event = %event_data, "🔹 Received event: {:?}", event_data);
                        }
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
                warn!("⏳ No data for {} seconds. Reconnecting...", timeout_duration.as_secs());
                return Err("Timeout".into());
            }
        }
    }
}
