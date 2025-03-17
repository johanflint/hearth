use crate::app_config::AppConfig;
use crate::domain::events::Event;
use crate::hue::domain::{ChangedProperty, ServerSentEventPayload, UnknownProperty};
use crate::hue::map_light_changed::map_light_changed_property;
use crate::sse;
use crate::sse::{Config, ServerSentEvent};
use reqwest::Client;
use std::error::Error;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;
use tokio::task;
use tracing::{debug, info, instrument, trace, warn};

type HueEvent = ServerSentEvent<Vec<ServerSentEventPayload>>;

#[instrument(skip_all)]
pub async fn observe(tx: Sender<Event>, client: &Client, config: &AppConfig) -> Result<(), Box<dyn Error>> {
    let (sse_tx, mut sse_rx) = mpsc::channel::<HueEvent>(config.core().store_buffer_size());

    let sse_config = Config {
        url: config.hue().url().to_owned(),
        retry_ms: config.hue().retry_ms(),
        retry_max_delay: config.hue().retry_max_delay_ms(),
        stale_connection_timeout_ms: config.hue().stale_connection_timeout_ms(),
    };

    task::spawn(async move {
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
    });

    let cloned_client = client.clone();
    task::spawn(async move {
        sse::listen::<Vec<ServerSentEventPayload>>(sse_tx, &cloned_client, &sse_config)
            .await
            .expect("Could not listen to SSE stream");
    });

    Ok(())
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
        ChangedProperty::Unknown(UnknownProperty { property_type, value }) => {
            debug!("⚠️ Unknown changed property type '{}'", property_type);
            trace!("   Payload: {}", value);
        }
    }
}
