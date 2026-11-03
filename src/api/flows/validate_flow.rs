use crate::api::ApiState;
use crate::api::flows::flow_error::validate_error_response;
use crate::api::parse_request::parse_request;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use tracing::info;

pub(super) async fn validate_flow(State(state): State<ApiState>, body: Bytes) -> Result<Response, Response> {
    let request: ValidateFlowRequest = parse_request(&body)?;
    state.flow_service.validate(&request.flow).await.map_err(validate_error_response)?;
    info!("📥 Received request to validate flow... OK");

    Ok(StatusCode::OK.into_response())
}

#[derive(Debug, Deserialize)]
struct ValidateFlowRequest {
    // Kept as generic JSON so it can be parsed into a `SerializedFlow`
    flow: serde_json::Value,
}
