use crate::app_config::AppConfig;
use reqwest::Client;
use thiserror::Error;

pub fn new_client(config: &AppConfig) -> Result<Client, ShellyClientError> {
    let client = Client::builder()
        .connect_timeout(config.shelly().client_connection_timeout_ms())
        .timeout(config.shelly().client_request_timeout_ms())
        .build()?;
    Ok(client)
}

#[derive(Error, Debug)]
pub enum ShellyClientError {
    #[error("request error: {0}")]
    RequestError(#[from] reqwest::Error),
}
