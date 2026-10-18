use crate::api::error::ErrorResponse;
use axum::Json;
use axum::body::Bytes;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;

pub(super) fn parse_request<T: DeserializeOwned>(body: &Bytes) -> Result<T, Response> {
    let body = str::from_utf8(body)
        .map_err(|_| (StatusCode::BAD_REQUEST, Json(ErrorResponse::new_code_only("invalidUtf8"))).into_response())?;
    serde_json::from_str(body).map_err(invalid_json)
}

pub(super) fn invalid_json(err: serde_json::Error) -> Response {
    (StatusCode::BAD_REQUEST, Json(ErrorResponse::new("invalidJson", err.to_string()))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::body_json;
    use serde_json::json;

    async fn assert_error(response: Response, status: StatusCode, code: &str) {
        assert_eq!(response.status(), status);
        assert_eq!(body_json(response).await["code"], code);
    }

    #[test]
    fn parse_request_parses_a_json_body() {
        let body = Bytes::from(r#"{"flow":{"id":"flow"}}"#);

        let request: serde_json::Value = parse_request(&body).expect("parsed");

        assert_eq!(request, json!({"flow": {"id": "flow"}}));
    }

    #[tokio::test]
    async fn parse_request_returns_400_invalid_utf8_for_a_body_that_is_not_utf8() {
        let body = Bytes::from_static(&[0xff, 0xfe]);

        let response = parse_request::<serde_json::Value>(&body).expect_err("must fail");

        assert_error(response, StatusCode::BAD_REQUEST, "invalidUtf8").await;
    }

    #[tokio::test]
    async fn parse_request_returns_400_invalid_json_for_a_body_that_is_not_json() {
        let body = Bytes::from("not json");

        let response = parse_request::<serde_json::Value>(&body).expect_err("must fail");

        assert_error(response, StatusCode::BAD_REQUEST, "invalidJson").await;
    }
}
