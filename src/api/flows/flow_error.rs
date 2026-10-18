use crate::api::error::ErrorResponse;
use crate::flow_loader::FlowFactoryError;
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

pub(super) fn flow_factory_error_response(err: FlowFactoryError) -> Response {
    let message = err.to_string();
    let error_response = match err {
        FlowFactoryError::Deserialization(_) => unreachable!("payload is already deserialized into a SerializedFlow"),
        FlowFactoryError::MissingStartNode => ErrorResponse::new("missingStartNode", message),
        FlowFactoryError::TooManyStartNodes(_) => ErrorResponse::new("tooManyStartNodes", message),
        FlowFactoryError::MissingEndNode => ErrorResponse::new("missingEndNode", message),
        FlowFactoryError::NoConnectingNode { .. } => ErrorResponse::new("noConnectingNode", message),
        FlowFactoryError::MissingNode { .. } => ErrorResponse::new("missingNode", message),
        FlowFactoryError::TooManyParentNodes { .. } => ErrorResponse::new("tooManyParentNodes", message),
        FlowFactoryError::UnusedNodes { .. } => ErrorResponse::new("unusedNodes", message),
        FlowFactoryError::DuplicateLinkValues { .. } => ErrorResponse::new("duplicateLinkValues", message),
        FlowFactoryError::PropertyChangedInScheduledFlow => {
            ErrorResponse::new("propertyChangedInScheduledFlow", message)
        }
    };

    (StatusCode::UNPROCESSABLE_ENTITY, Json(error_response)).into_response()
}
