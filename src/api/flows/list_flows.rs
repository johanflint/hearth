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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::flows::router;
    use crate::api::flows::test_support::{VALID_FLOW_ID, body_json, create_state, valid_flow_document};
    use crate::flow_registry::FlowRegistry;
    use axum::body::Body;
    use axum::http::Request;
    use serde_json::json;
    use tower::ServiceExt;

    async fn call_list_flows(state: ApiState) -> Response {
        let request = Request::builder()
            .method("GET")
            .uri("/api/flows")
            .header("content-type", "application/json")
            .body(Body::empty())
            .unwrap();

        router().with_state(state).oneshot(request).await.unwrap()
    }

    #[tokio::test]
    async fn list_flows_returns_an_empty_list_for_an_empty_store() {
        let (state, _scheduler_rx) = create_state(FlowRegistry::new(Vec::new()));

        let response = call_list_flows(state).await;

        assert_eq!(StatusCode::OK, response.status());
        assert_eq!(body_json(response).await, json!({ "flows": [] }));
    }

    #[tokio::test]
    async fn list_flows_returns_the_stored_flows_with_camel_case_fields() {
        let (state, _scheduler_rx) = create_state(FlowRegistry::new(vec![]));
        state.flow_store.insert(VALID_FLOW_ID, valid_flow_document()).await.expect("seed store");
        state.flow_store.update(VALID_FLOW_ID, 0, valid_flow_document()).await.expect("update store");

        let response = call_list_flows(state).await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        let flows = body["flows"].as_array().expect("flows array");
        assert_eq!(flows.len(), 1);
        assert_eq!(flows[0]["id"], VALID_FLOW_ID);
        assert_eq!(flows[0]["revision"], 1);
        assert!(flows[0]["createdAt"].is_string(), "got {body}");
        assert!(flows[0]["updatedAt"].is_string(), "got {body}");
    }
}