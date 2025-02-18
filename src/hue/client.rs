use crate::app_config::AppConfig;
use reqwest::header::HeaderValue;
use reqwest::{header, Client};
use thiserror::Error;

pub fn new_client(config: &AppConfig) -> Result<Client, HueClientError> {
    let mut headers = header::HeaderMap::new();
    let mut application_key_value = HeaderValue::from_str(config.hue().application_key())?;
    application_key_value.set_sensitive(true);
    headers.insert("hue-application-key", application_key_value);

    let client = Client::builder().danger_accept_invalid_certs(true).default_headers(headers).build()?;
    Ok(client)
}

#[derive(Error, Debug)]
pub enum HueClientError {
    #[error("request error: {0}")]
    RequestError(#[from] reqwest::Error),
    #[error("Hue client set an invalid header value: {0}")]
    InvalidHeaderValue(#[from] header::InvalidHeaderValue),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_config::AppConfigBuilder;

    #[tokio::test]
    async fn new_client_sets_the_hue_application_key_header() -> Result<(), HueClientError> {
        let mut server = mockito::Server::new_async().await;

        let mock = server
            .mock("GET", "/")
            .with_status(200)
            .match_header("hue-application-key", "key")
            .create_async()
            .await;

        let config = AppConfigBuilder::new().hue_url(server.url()).build();
        let client = new_client(&config)?;

        client.get(format!("{}{}", server.url(), "/")).send().await?;

        // Verify that the call came in and that the header is set
        mock.assert();

        Ok(())
    }
}
