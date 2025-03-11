use crate::app_config::AppConfig;
use crate::hue::domain::ServerSentEventPayload;
use crate::sse;
use crate::sse::Config;
use reqwest::Client;
use std::error::Error;
use tracing::instrument;

#[instrument(skip_all)]
pub async fn observe(client: &Client, config: &AppConfig) -> Result<(), Box<dyn Error>> {
    let sse_config = Config {
        url: config.hue().url().to_owned(),
        retry_ms: config.hue().retry_ms(),
        retry_max_delay: config.hue().retry_max_delay_ms(),
        stale_connection_timeout_ms: config.hue().stale_connection_timeout_ms(),
    };

    sse::listen::<Vec<ServerSentEventPayload>>(&client, &sse_config)
        .await
        .expect("Could not listen to SSE stream");

    Ok(())
}
