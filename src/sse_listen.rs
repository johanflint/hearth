use crate::app_config::AppConfig;
use futures::StreamExt;
use reqwest::{Client, StatusCode};
use std::error::Error;
use tokio::time::timeout;
use tokio_retry::strategy::{jitter, ExponentialBackoff};
use tokio_retry::Retry;
use tracing::{error, info, instrument, warn};

#[instrument(skip(client, config))]
pub async fn listen(client: &Client, config: &AppConfig) -> Result<(), Box<dyn Error>> {
    let strategy = ExponentialBackoff::from_millis(config.hue().retry_ms())
        .factor(2)
        .max_delay(config.hue().max_delay_ms())
        .map(jitter);

    info!("Connecting to SSE stream {}...", config.hue().url());
    Retry::spawn(strategy, || async {
        match connect_sse_stream(&client, config).await {
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

#[instrument(skip(client, config))]
async fn connect_sse_stream(client: &Client, config: &AppConfig) -> Result<(), Box<dyn Error>> {
    let url = format!("{}/eventstream/clip/v2", config.hue().url());
    let response = client.get(&url).header("Accept", "text/event-stream").send().await?.error_for_status()?;

    if response.status() == StatusCode::OK {
        info!(status = %response.status(), "Connecting to SSE stream {}... OK", config.hue().url());
    }

    let mut stream = response.bytes_stream();
    loop {
        let event = timeout(config.hue().max_delay_ms(), stream.next()).await;
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
                warn!("⏳ No data for {} seconds. Reconnecting...", config.hue().max_delay_ms().as_secs());
                return Err("Timeout".into());
            }
        }
    }
}
