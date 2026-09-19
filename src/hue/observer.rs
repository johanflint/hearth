use crate::app_config::AppConfig;
use crate::domain::events::Event;
use crate::hue::domain::{ChangedProperty, ServerSentEventPayload, UnknownProperty};
use crate::hue::map_light_changed::map_light_changed_property;
use crate::hue::map_motion_sensors_changed::map_motion_sensors_changed;
use crate::sse;
use crate::sse::{Config, ServerSentEvent};
use reqwest::Client;
use std::error::Error;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::task;
use tokio::task::JoinError;
use tracing::{debug, error, info, instrument, trace, warn};

type HueEvent = ServerSentEvent<Vec<ServerSentEventPayload>>;

#[instrument(skip_all)]
pub async fn observe(tx: Sender<Event>, client: &Client, config: &AppConfig) -> Result<(), Box<dyn Error>> {
    let sse_config = Config {
        url: config.hue().url().to_owned(),
        retry_ms: config.hue().retry_ms(),
        retry_max_delay: config.hue().retry_max_delay_ms(),
        stale_connection_timeout_ms: config.hue().stale_connection_timeout_ms(),
        send_timeout_ms: config.hue().send_timeout_ms(),
    };
    let buffer_size = config.core().store_buffer_size();
    let client = client.clone();
    task::spawn(supervise_hue_pipeline(tx, client, sse_config, buffer_size));

    Ok(())
}

/// Owns the Hue SSE listener and event processor pair for the app's lifetime.
/// If either task ends - gracefully, with an error, or by panicking - the other
/// task is aborted and a fresh chanel and task pair is rebuilt.
async fn supervise_hue_pipeline(tx: Sender<Event>, client: Client, sse_config: Config, buffer_size: usize) {
    loop {
        let (sse_tx, sse_rx) = mpsc::channel::<HueEvent>(buffer_size);

        let mut processor = task::spawn(run_processor(sse_rx, tx.clone()));
        let mut listener = {
            let cloned_client = client.clone();
            let cloned_sse_config = sse_config.clone();
            task::spawn(async move {
                if let Err(e) = sse::listen::<Vec<ServerSentEventPayload>>(sse_tx, &cloned_client, &cloned_sse_config).await {
                    warn!("⚠️ SSE listener exited with error: {}", e);
                }
            })
        };

        tokio::select! {
            result = &mut processor => {
                warn!("⚠️ Hue event processor {}. Rebuilding Hue pipeline...", describe(result));
                listener.abort();
            }
            result = &mut listener => {
                warn!("⚠️ Hue SSE listener {}. Rebuilding Hue pipeline...", describe(result));
                processor.abort();
            }
        }

        tokio::time::sleep(Duration::from_secs(1)).await; // Avoid a tight crash loop
    }
}

async fn run_processor(mut sse_rx: Receiver<HueEvent>, tx: Sender<Event>) {
    while let Some(hue_event) = sse_rx.recv().await {
        if let Some(comment) = &hue_event.comment {
            info!("🔹 {}", comment);
        }
        if let Some(data) = hue_event.data {
            for payload in data {
                for property in payload.data {
                    handle_changed_property(tx.clone(), property).await;
                }
            }
        }
    }
}

async fn handle_changed_property(tx: Sender<Event>, property: ChangedProperty) {
    match property {
        ChangedProperty::Light(property) => {
            for event in map_light_changed_property(property) {
                tx.send(event).await.unwrap_or_else(|e| {
                    warn!("⚠️ Unable to send changed light event: {}", e);
                });
            }
        }
        ChangedProperty::Motion(property) => {
            for event in map_motion_sensors_changed(property) {
                tx.send(event).await.unwrap_or_else(|e| {
                    warn!("⚠️ Unable to send changed motion sensor event: {}", e);
                });
            }
        }
        ChangedProperty::Unknown(UnknownProperty { property_type, value }) => {
            debug!("⚠️ Unknown changed property type '{}'", property_type);
            trace!("   Payload: {}", value);
        }
    }
}

fn describe(result: Result<(), JoinError>) -> &'static str {
    match result {
        Ok(()) => "ended gracefully",
        Err(e) if e.is_panic() => "panicked",
        Err(_) => "was cancelled",
    }
}