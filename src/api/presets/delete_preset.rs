use crate::api::ApiState;
use crate::api::error::ErrorResponse;
use crate::preset_service::DeletePresetError;
use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tracing::{error, info};

pub(super) async fn delete_preset(
    Path(id): Path<String>,
    query: Result<Query<DeletePresetQuery>, QueryRejection>,
    State(state): State<ApiState>,
) -> Result<Response, Response> {
    let Query(DeletePresetQuery { base_revision }) = query.map_err(invalid_query)?;
    state.preset_service.delete_preset(&id, base_revision).await.map_err(delete_error_response)?;
    info!("📥 Received request to delete preset '{id}'... OK, revision {base_revision}");

    Ok(StatusCode::NO_CONTENT.into_response())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletePresetQuery {
    base_revision: u64,
}

fn invalid_query(err: QueryRejection) -> Response {
    (StatusCode::BAD_REQUEST, Json(ErrorResponse::new("invalidQuery", err.body_text()))).into_response()
}

fn delete_error_response(err: DeletePresetError) -> Response {
    let error_message = err.to_string();
    match err {
        DeletePresetError::InUse(flows) => {
            let flows = flows
                .into_iter()
                .map(|summary| FlowReference { id: summary.id, name: summary.name, revision: summary.revision })
                .collect();
            let body = InUseErrorResponse { code: "presetInUse", message: error_message, flows };
            (StatusCode::CONFLICT, Json(body)).into_response()
        }
        DeletePresetError::NotFound => {
            (StatusCode::NOT_FOUND, Json(ErrorResponse::new_code_only("presetNotFound"))).into_response()
        }
        DeletePresetError::RevisionConflict { .. } => {
            (StatusCode::CONFLICT, Json(ErrorResponse::new("revisionConflict", err.to_string()))).into_response()
        }
        DeletePresetError::Internal(err) => {
            error!("❌ Failed to delete preset from store: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse::new_code_only("storageError"))).into_response()
        }
    }
}

#[derive(Debug, Serialize)]
struct InUseErrorResponse {
    pub code: &'static str,
    pub message: String,
    pub flows: Vec<FlowReference>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FlowReference {
    id: String,
    name: String,
    revision: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::presets::router;
    use crate::api::test_support::{
        Fixture, body_json, create_state_with_database, insert_preset_row, set_preset_revision,
    };
    use crate::database::DatabaseError;
    use crate::flow_registry::FlowRegistry;
    use crate::test_support::flow_using_preset;
    use axum::body::Body;
    use axum::http::Request;
    use rstest::rstest;
    use tower::ServiceExt;

    async fn call_delete_preset(state: ApiState, uri: &str) -> Response {
        let request = Request::builder().method("DELETE").uri(uri).body(Body::empty()).unwrap();

        router().with_state(state).oneshot(request).await.unwrap()
    }

    fn delete_uri(id: &str, base_revision: u64) -> String {
        format!("/api/presets/{id}?baseRevision={base_revision}")
    }

    #[tokio::test]
    async fn delete_preset_returns_204_no_content_when_the_base_revision_matches() {
        let (Fixture { state, .. }, database) = create_state_with_database(FlowRegistry::new(vec![]));
        insert_preset_row(&database, "warm", "Warm", "true").await;
        let preset_service = state.preset_service.clone();

        let response = call_delete_preset(state, &delete_uri("warm", 0)).await;

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let presets = preset_service.list().await.expect("list succeeded");
        assert!(presets.is_empty(), "preset must be deleted, got {presets:?}");
    }

    #[tokio::test]
    async fn delete_preset_only_deletes_the_given_preset() {
        let (Fixture { state, .. }, database) = create_state_with_database(FlowRegistry::new(vec![]));
        insert_preset_row(&database, "warm", "Warm", "true").await;
        insert_preset_row(&database, "other", "Other", "10").await;
        let preset_service = state.preset_service.clone();

        let response = call_delete_preset(state, &delete_uri("warm", 0)).await;

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let presets = preset_service.list().await.expect("list succeeded");
        assert_eq!(presets.len(), 1);
        assert_eq!(presets[0].id, "other");
    }

    #[tokio::test]
    async fn delete_preset_returns_404_not_found_for_an_unknown_preset() {
        let (Fixture { state, .. }, _) = create_state_with_database(FlowRegistry::new(vec![]));

        let response = call_delete_preset(state, &delete_uri("unknown", 0)).await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(body_json(response).await["code"], "presetNotFound");
    }

    #[rstest]
    #[case::behind(0, 2)]
    #[case::ahead(5, 2)]
    #[tokio::test]
    async fn delete_preset_returns_409_conflict_for_a_mismatching_base_revision(
        #[case] base_revision: u64,
        #[case] current_revision: u64,
    ) {
        let (Fixture { state, .. }, database) = create_state_with_database(FlowRegistry::new(vec![]));
        insert_preset_row(&database, "warm", "Warm", "true").await;
        set_preset_revision(&database, "warm", current_revision as i64).await;
        let preset_service = state.preset_service.clone();

        let response = call_delete_preset(state, &delete_uri("warm", base_revision)).await;

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = body_json(response).await;
        assert_eq!(body["code"], "revisionConflict");
        assert_eq!(
            body["message"],
            format!(
                "revision conflict: delete is based on revision {base_revision}, but the current revision is {current_revision}"
            )
        );
        let warm = preset_service.retrieve_preset("warm").await.expect("preset must not be deleted");
        assert_eq!(warm.revision, current_revision);
    }

    #[rstest]
    #[case::missing_base_revision("/api/presets/warm")]
    #[case::base_revision_is_not_a_number("/api/presets/warm?baseRevision=latest")]
    #[case::negative_base_revision("/api/presets/warm?baseRevision=-1")]
    #[tokio::test]
    async fn delete_preset_returns_400_bad_request_for_an_invalid_base_revision(#[case] uri: &str) {
        let (Fixture { state, .. }, database) = create_state_with_database(FlowRegistry::new(vec![]));
        insert_preset_row(&database, "warm", "Warm", "true").await;
        let preset_service = state.preset_service.clone();

        let response = call_delete_preset(state, uri).await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(body_json(response).await["code"], "invalidQuery");
        preset_service.retrieve_preset("warm").await.expect("preset must not be deleted");
    }

    #[tokio::test]
    async fn delete_preset_returns_500_internal_server_error_when_the_store_fails() {
        let (Fixture { state, .. }, database) = create_state_with_database(FlowRegistry::new(vec![]));
        database
            .call(|conn| conn.execute("DROP TABLE presets", []).map_err(DatabaseError::from))
            .await
            .expect("drop the table");

        let response = call_delete_preset(state, &delete_uri("warm", 0)).await;

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body_json(response).await["code"], "storageError");
    }

    #[tokio::test]
    async fn delete_preset_returns_409_conflict_with_the_flows_using_the_preset() {
        let flow_registry = FlowRegistry::new(vec![
            flow_using_preset("b", "Flow B", 2, "warm"),
            flow_using_preset("a", "Flow A", 0, "warm"),
            flow_using_preset("c", "Flow C", 1, "cold"),
        ]);
        let (Fixture { state, .. }, database) = create_state_with_database(flow_registry);
        insert_preset_row(&database, "warm", "Warm", "true").await;
        let preset_service = state.preset_service.clone();

        let response = call_delete_preset(state, &delete_uri("warm", 0)).await;

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = body_json(response).await;
        assert_eq!(body["code"], "presetInUse");
        assert_eq!(body["message"], "preset in use by 2 flow(s)");
        assert_eq!(
            body["flows"],
            serde_json::json!([
                {"id": "a", "name": "Flow A", "revision": 0},
                {"id": "b", "name": "Flow B", "revision": 2},
            ])
        );
        preset_service.retrieve_preset("warm").await.expect("preset must not be deleted");
    }
}
