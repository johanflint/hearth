use crate::api::ApiState;
use crate::api::error::ErrorResponse;
use crate::flow_service::RetrieveFlowError;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::Serialize;
use tracing::error;

pub(super) async fn retrieve_flow(Path(id): Path<String>, State(state): State<ApiState>) -> Result<Response, Response> {
    let stored_flow = state.flow_service.retrieve_flow(&id).await.map_err(|err| retrieve_error_response(&id, err))?;

    let response = RetrieveFlowResponse {
        id: stored_flow.id,
        revision: stored_flow.revision,
        flow: stored_flow.document,
        created_at: stored_flow.created_at,
        updated_at: stored_flow.updated_at,
    };
    Ok((StatusCode::OK, Json(response)).into_response())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RetrieveFlowResponse {
    pub id: String,
    pub revision: u64,
    pub flow: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
}

fn retrieve_error_response(id: &str, err: RetrieveFlowError) -> Response {
    match err {
        RetrieveFlowError::NotFound => StatusCode::NOT_FOUND.into_response(),
        RetrieveFlowError::Internal(err) => {
            error!("❌ Failed to retrieve flow '{id}' from store: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse::new_code_only("storageError"))).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::flows::router;
    use crate::api::flows::test_support::{VALID_FLOW_ID, Fixture, body_json, create_state, valid_flow_document};
    use crate::flow_registry::FlowRegistry;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn call_retrieve_flow(state: ApiState, id: &str) -> Response {
        let request = Request::builder().method("GET").uri(format!("/api/flows/{id}")).body(Body::empty()).unwrap();
        router().with_state(state).oneshot(request).await.unwrap()
    }

    #[tokio::test]
    async fn retrieve_flow_returns_the_stored_flow_as_a_json_object() {
        let Fixture { state, flow_store, .. } = create_state(FlowRegistry::new(vec![]));
        flow_store.insert(VALID_FLOW_ID, valid_flow_document()).await.expect("seed store");

        let response = call_retrieve_flow(state, VALID_FLOW_ID).await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        assert_eq!(body["id"], VALID_FLOW_ID);
        assert_eq!(body["revision"], 0);
        assert_eq!(body["flow"], valid_flow_document(), "flow must be embedded as an object, not a string");
        assert!(body["createdAt"].is_string(), "got {body}");
        assert!(body["updatedAt"].is_string(), "got {body}");
    }

    #[tokio::test]
    async fn retrieve_flow_returns_404_not_found_without_a_body_for_an_unknown_flow() {
        let Fixture { state, .. } = create_state(FlowRegistry::new(vec![]));

        let response = call_retrieve_flow(state, VALID_FLOW_ID).await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert!(body.is_empty(), "got {body:?}");
    }
}
