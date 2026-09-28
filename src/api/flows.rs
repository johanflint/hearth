use crate::api::ApiState;
use crate::flow_engine::SchedulerCommand;
use crate::flow_loader;
use crate::flow_loader::{FlowFactoryError, SerializedFlow};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::put;
use axum::{Json, Router};
use serde::Serialize;
use tracing::{debug, error, info};

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/api/flow/{id}", put(update_flow))
}

async fn update_flow(Path(id): Path<String>, State(state): State<ApiState>, Json(payload): Json<SerializedFlow>) -> Response {
    debug!("Received request to update flow '{id}'...");
    if id != payload.id {
        return (StatusCode::UNPROCESSABLE_ENTITY, Json(ErrorResponse::new("flowIdMismatch", format!("body id '{}' does not match path id '{}'", payload.id, id)))).into_response();
    }

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

    // Reserve channel capacity before mutating the registry. This way, a cancelled
    // request or a full channel can never leave the registry updated without a
    // matching `Reconcile` guaranteed to follow.
    let permit = match state.scheduler_tx.clone().reserve_owned().await {
        Ok(permit) => permit,
        Err(err) => {
            error!("❌ Scheduler is unavailable: {err}");
            return (StatusCode::SERVICE_UNAVAILABLE, Json(ErrorResponse::new_code_only("schedulerUnavailable"))).into_response();
        }
    };

    let Some(updated_revision) = state.flow_registry.replace_existing(flow) else {
        return (StatusCode::NOT_FOUND, Json(ErrorResponse::new_code_only("flowNotFound"))).into_response();
    };

    // Always reconcile: the scheduler derives the desired schedule state itself
    // Send on a reserved permit is synchronous and infallible
    permit.send(SchedulerCommand::Reconcile { flow_id: id.clone(), revision: updated_revision });

    info!("Received request to update flow '{id}'... OK, revision {updated_revision}");
    (StatusCode::OK, Json(UpdateFlowResponse { id, revision: updated_revision })).into_response()
}

#[derive(Debug, Serialize)]
struct UpdateFlowResponse {
    pub id: String,
    pub revision: u64,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl ErrorResponse {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        ErrorResponse { code: code.to_string(), message: Some(message.into()) }
    }

    pub fn new_code_only(code: &'static str) -> Self {
        ErrorResponse { code: code.to_string(), message: None }
    }
}