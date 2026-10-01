use crate::api::ApiState;
use crate::api::error::ErrorResponse;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::Serialize;
use tracing::error;

pub(super) async fn list_flows(State(state): State<ApiState>) -> Response {
    let stored_flows = match state.flow_store.list().await {
        Ok(stored_flows) => stored_flows,
        Err(err) => {
            error!("❌ Failed to list flows from store: {err}");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse::new_code_only("storageError"))).into_response();
        }
    };

    let flows: Vec<FlowSummary> = stored_flows.into_iter()
        .map(|flow| FlowSummary { id: flow.id, revision: flow.revision, created_at: flow.created_at, updated_at: flow.updated_at })
        .collect();

    (StatusCode::OK, Json(ListFlowsResponse { flows })).into_response()
}

#[derive(Debug, Serialize)]
struct ListFlowsResponse {
    flows: Vec<FlowSummary>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FlowSummary {
    id: String,
    revision: u64,
    created_at: DateTime<Utc>,
    updated_at: Option<DateTime<Utc>>,
}
