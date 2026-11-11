use crate::shelly::device_address::DeviceAddress;
use reqwest::{Client, RequestBuilder, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

pub async fn fetch<T>(
    client: &Client,
    address: &DeviceAddress,
    method: &str,
    params: &[(&str, &str)],
) -> Result<T, FetchError>
where
    T: DeserializeOwned,
{
    send(client.get(url(address, method)).query(params)).await
}

pub async fn call<T: DeserializeOwned>(
    client: &Client,
    address: &DeviceAddress,
    method: &str,
    body: &impl Serialize,
) -> Result<T, FetchError> {
    send(client.post(url(address, method)).json(body)).await
}

async fn send<T: DeserializeOwned>(request: RequestBuilder) -> Result<T, FetchError> {
    let response = request.send().await.map_err(FetchError::Request)?;

    let status = response.status();
    if !status.is_success() {
        return Err(FetchError::UnexpectedResponse(status, response.url().to_string()));
    }

    response.json::<T>().await.map_err(FetchError::InvalidBody)
}

fn url(address: &DeviceAddress, method: &str) -> String {
    format!("http://{address}/rpc/{method}")
}

#[derive(Error, Debug)]
pub enum FetchError {
    #[error("request failed: {0}")]
    Request(#[source] reqwest::Error),
    #[error("unexpected status code {0} when calling {1}")]
    UnexpectedResponse(StatusCode, String),
    #[error("unexpected response body: {0}")]
    InvalidBody(#[source] reqwest::Error),
}

impl FetchError {
    pub fn is_permanent(&self) -> bool {
        match self {
            FetchError::Request(_) => false,
            FetchError::UnexpectedResponse(status, _) => status.is_client_error(),
            FetchError::InvalidBody(_) => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shelly::test_support::device_address;
    use mockito::{Matcher, Server};
    use rstest::rstest;
    use serde_json::{Value, json};
    use std::net::TcpListener;

    // An address nothing listens on, so connecting fails
    fn unreachable_address() -> DeviceAddress {
        device_address(TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap())
    }

    #[tokio::test]
    async fn fetch_sends_a_get_with_the_query_and_decodes_the_body() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/rpc/Shelly.GetComponents")
            .match_query(Matcher::UrlEncoded("include".to_string(), "config".to_string()))
            .with_body(r#"{"a":1}"#)
            .create_async()
            .await;

        let result: Value = fetch(
            &Client::new(),
            &device_address(server.socket_address()),
            "Shelly.GetComponents",
            &[("include", "config")],
        )
        .await
        .unwrap();

        assert_eq!(result, json!({"a": 1}));
        mock.assert();
    }

    #[tokio::test]
    async fn fetch_fails_on_an_unexpected_status() {
        let mut server = Server::new_async().await;
        server.mock("GET", "/rpc/Missing").with_status(404).create_async().await;

        let error =
            fetch::<Value>(&Client::new(), &device_address(server.socket_address()), "Missing", &[]).await.unwrap_err();

        assert!(
            matches!(&error, FetchError::UnexpectedResponse(StatusCode::NOT_FOUND, url) if url.ends_with("/rpc/Missing")),
            "got {error:?}"
        );
    }

    #[tokio::test]
    async fn fetch_fails_on_an_invalid_body() {
        let mut server = Server::new_async().await;
        server.mock("GET", "/rpc/Broken").with_body("not json").create_async().await;

        let error =
            fetch::<Value>(&Client::new(), &device_address(server.socket_address()), "Broken", &[]).await.unwrap_err();

        assert!(matches!(error, FetchError::InvalidBody(_)), "got {error:?}");
    }

    #[tokio::test]
    async fn fetch_fails_when_the_device_is_unreachable() {
        let error =
            fetch::<Value>(&Client::new(), &unreachable_address(), "Shelly.GetDeviceInfo", &[]).await.unwrap_err();

        assert!(matches!(error, FetchError::Request(_)), "got {error:?}");
    }

    #[tokio::test]
    async fn call_posts_the_body_as_json_and_decodes_the_result() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/rpc/MQTT.SetConfig")
            .match_body(Matcher::Json(json!({"config": {"enable": true}})))
            .with_body(r#"{"restart_required":true}"#)
            .create_async()
            .await;

        let result: Value = call(
            &Client::new(),
            &device_address(server.socket_address()),
            "MQTT.SetConfig",
            &json!({"config": {"enable": true}}),
        )
        .await
        .unwrap();

        assert_eq!(result, json!({"restart_required": true}));
        mock.assert();
    }

    // Shelly.Reboot answers with `null`
    #[tokio::test]
    async fn call_decodes_a_null_result_as_unit() {
        let mut server = Server::new_async().await;
        server.mock("POST", "/rpc/Shelly.Reboot").with_body("null").create_async().await;

        call::<()>(&Client::new(), &device_address(server.socket_address()), "Shelly.Reboot", &json!({}))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn call_fails_on_an_unexpected_status() {
        let mut server = Server::new_async().await;
        server.mock("POST", "/rpc/MQTT.SetConfig").with_status(401).create_async().await;

        let error =
            call::<Value>(&Client::new(), &device_address(server.socket_address()), "MQTT.SetConfig", &json!({}))
                .await
                .unwrap_err();

        assert!(matches!(error, FetchError::UnexpectedResponse(StatusCode::UNAUTHORIZED, _)), "got {error:?}");
    }

    #[rstest]
    #[case::unauthorized(StatusCode::UNAUTHORIZED, true)]
    #[case::not_found(StatusCode::NOT_FOUND, true)]
    #[case::server_error(StatusCode::INTERNAL_SERVER_ERROR, false)]
    #[case::service_unavailable(StatusCode::SERVICE_UNAVAILABLE, false)]
    fn unexpected_response_is_permanent_for_client_errors_only(#[case] status: StatusCode, #[case] expected: bool) {
        assert_eq!(FetchError::UnexpectedResponse(status, "url".to_string()).is_permanent(), expected);
    }

    #[tokio::test]
    async fn a_request_error_is_transient() {
        let error =
            fetch::<Value>(&Client::new(), &unreachable_address(), "Shelly.GetDeviceInfo", &[]).await.unwrap_err();

        assert!(!error.is_permanent());
    }

    #[tokio::test]
    async fn an_invalid_body_is_permanent() {
        let mut server = Server::new_async().await;
        server.mock("GET", "/rpc/Broken").with_body("not json").create_async().await;

        let error =
            fetch::<Value>(&Client::new(), &device_address(server.socket_address()), "Broken", &[]).await.unwrap_err();

        assert!(error.is_permanent());
    }
}
