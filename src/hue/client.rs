use crate::app_config::AppConfig;
use reqwest::header::HeaderValue;
use reqwest::{Client, header};
use thiserror::Error;

pub fn new_client(config: &AppConfig) -> Result<Client, HueClientError> {
    let mut headers = header::HeaderMap::new();
    let mut application_key_value = HeaderValue::from_str(config.hue().application_key())?;
    application_key_value.set_sensitive(true);
    headers.insert("hue-application-key", application_key_value);

    let client = Client::builder()
        .danger_accept_invalid_certs(true)
        .default_headers(headers)
        .connect_timeout(config.core().client_connection_timeout_ms())
        .timeout(config.core().client_request_timeout_ms())
        .build()?;
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
    use std::time::{Duration, Instant};
    use tokio::net::TcpListener;

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

    #[tokio::test]
    async fn request_times_out_instead_of_hanging_forever() -> Result<(), HueClientError> {
        let url = start_hanging_server().await;

        let config = AppConfigBuilder::new().hue_url(url.clone()).build();
        let client = new_client(&config)?;

        let start = Instant::now();
        let result = client.get(&url).send().await;

        assert!(result.is_err(), "expected the request to time out, got {:?}", result);
        assert!(result.unwrap_err().is_timeout(), "expected a timeout error");
        assert!(start.elapsed() < Duration::from_secs(1), "expected request to fail fast, not hang; took {:?}", start.elapsed());

        Ok(())
    }

    async fn start_hanging_server() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                if let Ok((socket, _)) = listener.accept().await {
                    std::mem::forget(socket); // Hold the connection open, never respond
                }
            }
        });
        format!("http://{}", address)
    }
}
