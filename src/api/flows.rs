use crate::api::ApiState;
use crate::flow_loader;
use crate::flow_loader::{FlowFactoryError, SerializedFlow};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::put;
use axum::{Json, Router};
use serde::Serialize;
use tracing::{debug, info};

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/api/flow/{id}", put(update_flow))
}

async fn update_flow(Path(id): Path<String>, State(state): State<ApiState>, Json(payload): Json<SerializedFlow>) -> Response {
    debug!("Received request to update flow '{id}'...");
    let flow = match flow_loader::from_json(payload) {
        Ok(flow) => flow,
        Err(flow_factory_error) => {
            let message = flow_factory_error.to_string();
            let error_response = match flow_factory_error {
                FlowFactoryError::Deserialization(_) => unreachable!("payload is already deserialized by the json extractor"),
                FlowFactoryError::MissingStartNode => ErrorResponse::new("missingStartNode", message),
                FlowFactoryError::TooManyStartNodes(_) => ErrorResponse::new("tooManyStartNodes", message),
                FlowFactoryError::MissingEndNode => ErrorResponse::new("missingEndNode", message),
                FlowFactoryError::NoConnectingNode { .. } => ErrorResponse::new("noConnectingNode", message),
                FlowFactoryError::MissingNode { .. } => ErrorResponse::new("missingNode", message),
                FlowFactoryError::TooManyParentNodes { .. } => ErrorResponse::new("tooManyParentNodes", message),
                FlowFactoryError::UnusedNodes { .. } => ErrorResponse::new("unusedNodes", message),
                FlowFactoryError::DuplicateLinkValues { .. } => ErrorResponse::new("duplicateLinkValues", message),
                FlowFactoryError::PropertyChangedInScheduledFlow => ErrorResponse::new("propertyChangedInScheduledFlow", message),
            };

            return (StatusCode::UNPROCESSABLE_ENTITY, Json(error_response)).into_response();
        }
    };

    info!("Received request to update flow '{id}'... OK");
    (StatusCode::OK, Json(UpdateFlowResponse { id })).into_response()
}

#[derive(Debug, Serialize)]
struct UpdateFlowResponse {
    pub id: String,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    pub code: String,
    pub message: Option<String>,
}

impl ErrorResponse {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        ErrorResponse { code: code.to_string(), message: Some(message.into()) }
    }
}