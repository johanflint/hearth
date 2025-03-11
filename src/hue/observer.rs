use crate::app_config::AppConfig;
use crate::hue::domain::ServerSentEventPayload;
use crate::sse;
use reqwest::Client;
use std::error::Error;
use tracing::instrument;

#[instrument(skip_all)]
pub async fn observe(client: &Client, config: &AppConfig) -> Result<(), Box<dyn Error>> {
    sse::listen::<Vec<ServerSentEventPayload>>(&client, &config)
        .await
        .expect("Could not listen to SSE stream");

    Ok(())
}
