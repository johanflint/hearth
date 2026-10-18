use crate::api::ApiState;
use crate::api::error::ErrorResponse;
use crate::api::presets::preset_response::PresetResponse;
use crate::preset_service::RetrievePresetError;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tracing::error;

pub(super) async fn retrieve_preset(
    Path(id): Path<String>,
    State(state): State<ApiState>,
) -> Result<Response, Response> {
    let stored_preset =
        state.preset_service.retrieve_preset(&id).await.map_err(|err| retrieve_error_response(&id, err))?;

    let response = PresetResponse::from(stored_preset);
    Ok((StatusCode::OK, Json(response)).into_response())
}

fn retrieve_error_response(id: &str, err: RetrievePresetError) -> Response {
    match err {
        RetrievePresetError::NotFound => StatusCode::NOT_FOUND.into_response(),
        RetrievePresetError::Internal(err) => {
            error!("❌ Failed to retrieve preset '{id}' from store: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse::new_code_only("storageError"))).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::presets::router;
    use crate::api::test_support::{
        Fixture, body_json, create_state_with_database, insert_preset_row, set_preset_revision,
    };
    use crate::flow_registry::FlowRegistry;
    use axum::body::Body;
    use axum::http::Request;
    use serde_json::json;
    use tower::ServiceExt;

    async fn call_retrieve_preset(state: ApiState, id: &str) -> Response {
        let request = Request::builder().method("GET").uri(format!("/api/presets/{id}")).body(Body::empty()).unwrap();

        router().with_state(state).oneshot(request).await.unwrap()
    }

    #[tokio::test]
    async fn retrieve_preset_returns_the_stored_preset_with_camel_case_fields() {
        let (Fixture { state, .. }, database) = create_state_with_database(FlowRegistry::new(vec![]));
        insert_preset_row(&database, "other", "Other", "10").await;
        insert_preset_row(&database, "warm", "Warm", r##"{"type":"color","value":"#ff0000"}"##).await;
        set_preset_revision(&database, "warm", 3).await;

        let response = call_retrieve_preset(state, "warm").await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        assert_eq!(body["id"], "warm");
        assert_eq!(body["revision"], 3);
        assert_eq!(body["name"], "Warm");
        assert_eq!(body["value"], json!({ "type": "color", "value": "#ff0000" }));
        assert!(body["createdAt"].is_string(), "got {body}");
        assert!(body["updatedAt"].is_null(), "got {body}");
    }

    #[tokio::test]
    async fn retrieve_preset_returns_not_found_for_an_unknown_id() {
        let (Fixture { state, .. }, database) = create_state_with_database(FlowRegistry::new(vec![]));
        insert_preset_row(&database, "other", "Other", "10").await;

        let response = call_retrieve_preset(state, "warm").await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn retrieve_preset_returns_a_storage_error_when_the_store_fails() {
        let (Fixture { state, .. }, database) = create_state_with_database(FlowRegistry::new(vec![]));
        insert_preset_row(&database, "corrupt", "Corrupt", "not valid json").await;

        let response = call_retrieve_preset(state, "corrupt").await;

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body_json(response).await["code"], "storageError");
    }
}
