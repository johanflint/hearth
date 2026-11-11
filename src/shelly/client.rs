use crate::app_config::AppConfig;
use crate::mdns_resolver::MdnsResolver;
use reqwest::Client;
use thiserror::Error;

pub fn new_client(config: &AppConfig, resolver: MdnsResolver) -> Result<Client, ShellyClientError> {
    let client = Client::builder()
        .dns_resolver(resolver)
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
