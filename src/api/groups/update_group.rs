use crate::api::ApiState;
use crate::api::error::ErrorResponse;
use crate::api::groups::group_view::GroupView;
use crate::api::parse_request::parse_request;
use crate::group_service::UpdateGroupError;
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use tracing::{error, info};

pub(super) async fn update_group(
    Path(id): Path<String>,
    State(state): State<ApiState>,
    body: Bytes,
) -> Result<Response, Response> {
    let request: UpdateGroupRequest = parse_request(&body)?;

    let stored_group = state
        .group_service
        .update_group(&id, request.base_revision, &request.name, request.device_ids)
        .await
        .map_err(update_error_response)?;
    info!("📥 Received request to update group '{}'... OK, revision {}", stored_group.id, stored_group.revision);

    Ok((StatusCode::OK, Json(GroupView::from(stored_group))).into_response())
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct UpdateGroupRequest {
    base_revision: u64,
    name: String,
    device_ids: Vec<String>,
}

fn update_error_response(err: UpdateGroupError) -> Response {
    match err {
        UpdateGroupError::GroupEmpty => {
            (StatusCode::UNPROCESSABLE_ENTITY, Json(ErrorResponse::new("groupEmpty", err.to_string()))).into_response()
        }
        UpdateGroupError::UnknownDevices { .. } => {
            (StatusCode::UNPROCESSABLE_ENTITY, Json(ErrorResponse::new("unknownDevices", err.to_string())))
                .into_response()
        }
        UpdateGroupError::NotFound => {
            (StatusCode::NOT_FOUND, Json(ErrorResponse::new_code_only("groupNotFound"))).into_response()
        }
        UpdateGroupError::RevisionConflict { .. } => {
            (StatusCode::CONFLICT, Json(ErrorResponse::new("revisionConflict", err.to_string()))).into_response()
        }
        UpdateGroupError::NameAlreadyExists => {
            (StatusCode::CONFLICT, Json(ErrorResponse::new_code_only("groupNameAlreadyExists"))).into_response()
        }
        UpdateGroupError::StoreUnavailable => {
            error!("❌ Store is unavailable");
            (StatusCode::SERVICE_UNAVAILABLE, Json(ErrorResponse::new_code_only("storeUnavailable"))).into_response()
        }
        UpdateGroupError::Internal(_) => {
            error!("❌ Failed to persist group to store: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse::new_code_only("storageError"))).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::groups::router;
    use crate::api::test_support::{body_json, create_state_with_store_tx};
    use crate::database::Database;
    use crate::domain::Event;
    use crate::group_store::{GroupStore, StoredGroup};
    use crate::store::StoreSnapshot;
    use crate::test_support::DeviceBuilder;
    use axum::body::Body;
    use axum::http::Request;
    use serde_json::json;
    use std::collections::{BTreeSet, HashMap};
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use tower::ServiceExt;

    struct Fixture {
        state: ApiState,
        database: Database,
        store_rx: mpsc::Receiver<Event>,
    }

    /// Creates a state with a stored group "kitchen" named "Kitchen" with device "a".
    /// Bind `store_rx`: dropping it, e.g. by leaving it out with `..`, makes the store unavailable.
    async fn create_state(device_ids: &[&str]) -> Fixture {
        let devices: HashMap<_, _> =
            device_ids.iter().map(|id| (id.to_string(), Arc::new(DeviceBuilder::new(id).build()))).collect();
        let snapshot = StoreSnapshot { devices: Arc::new(devices), ..Default::default() };
        let (store_tx, store_rx) = mpsc::channel(8);
        let (state, database) = create_state_with_store_tx(snapshot, store_tx);
        GroupStore::new(database.clone())
            .insert("kitchen", "Kitchen", &BTreeSet::from(["a".to_string()]))
            .await
            .expect("insert succeeded");
        Fixture { state, database, store_rx }
    }

    async fn call_update_group(state: ApiState, id: &str, body: &str) -> Response {
        let request = Request::builder()
            .method("PUT")
            .uri(format!("/api/groups/{id}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();

        router().with_state(state).oneshot(request).await.unwrap()
    }

    async fn assert_error(response: Response, status: StatusCode, code: &str) {
        assert_eq!(response.status(), status);
        assert_eq!(body_json(response).await["code"], code);
    }

    async fn stored_groups(database: Database) -> Vec<StoredGroup> {
        GroupStore::new(database).list().await.expect("list succeeded")
    }

    async fn assert_unchanged(database: Database) {
        let stored = stored_groups(database).await;
        assert_eq!(stored.len(), 1);
        assert_eq!((stored[0].name.as_str(), stored[0].revision), ("Kitchen", 0));
        assert_eq!(stored[0].device_ids, BTreeSet::from(["a".to_string()]));
    }

    #[tokio::test]
    async fn update_group_returns_the_updated_group_with_camel_case_fields() {
        let Fixture { state, store_rx: _store_rx, .. } = create_state(&["a", "b"]).await;

        let response =
            call_update_group(state, "kitchen", r#"{"baseRevision":0,"name":"Hall","deviceIds":["b","a","b"]}"#).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            body_json(response).await,
            json!({
                "id": "kitchen",
                "name": "Hall",
                "deviceIds": ["a", "b"],
                "revision": 1,
                "owner": "hearth",
            })
        );
    }

    #[tokio::test]
    async fn update_group_persists_the_group_and_sends_the_change_to_the_store() {
        let Fixture { state, database, mut store_rx } = create_state(&["a", "b"]).await;

        call_update_group(state, "kitchen", r#"{"baseRevision":0,"name":"Hall","deviceIds":["b"]}"#).await;

        let stored = stored_groups(database).await;
        assert_eq!((stored[0].name.as_str(), stored[0].revision), ("Hall", 1));
        let Event::GroupChanged(changed) = store_rx.try_recv().expect("an event was sent") else {
            panic!("expected a GroupChanged event");
        };
        assert_eq!(changed.id, "kitchen");
        assert_eq!(changed.name.as_deref(), Some("Hall"));
        assert_eq!(changed.device_ids, Some(BTreeSet::from(["b".to_string()])));
        assert_eq!(changed.revision, Some(1));
    }

    #[rstest::rstest]
    #[case::missing_field(r#"{"baseRevision":0,"name":"Hall"}"#)]
    #[case::missing_revision(r#"{"name":"Hall","deviceIds":[]}"#)]
    #[case::negative_revision(r#"{"baseRevision":-1,"name":"Hall","deviceIds":[]}"#)]
    #[case::not_json("nope")]
    #[tokio::test]
    async fn update_group_returns_400_bad_request_for_an_invalid_body(#[case] body: &str) {
        let Fixture { state, database, mut store_rx } = create_state(&["a"]).await;

        let response = call_update_group(state, "kitchen", body).await;

        assert_error(response, StatusCode::BAD_REQUEST, "invalidJson").await;
        assert_unchanged(database).await;
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn update_group_returns_404_not_found_for_an_unknown_group() {
        let Fixture { state, database, mut store_rx } = create_state(&["a"]).await;

        let response =
            call_update_group(state, "unknown", r#"{"baseRevision":0,"name":"Hall","deviceIds":["a"]}"#).await;

        assert_error(response, StatusCode::NOT_FOUND, "groupNotFound").await;
        assert_unchanged(database).await;
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn update_group_returns_409_conflict_for_a_stale_revision() {
        let Fixture { state, database, mut store_rx } = create_state(&["a"]).await;

        let response =
            call_update_group(state, "kitchen", r#"{"baseRevision":3,"name":"Hall","deviceIds":["a"]}"#).await;

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = body_json(response).await;
        assert_eq!(body["code"], "revisionConflict");
        assert!(body["message"].as_str().is_some_and(|message| !message.is_empty()));
        assert_unchanged(database).await;
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn update_group_returns_409_conflict_for_a_name_in_use_ignoring_case() {
        let Fixture { state, database, mut store_rx } = create_state(&["a"]).await;
        GroupStore::new(database.clone()).insert("hall", "Hall", &BTreeSet::new()).await.unwrap();

        let response =
            call_update_group(state, "kitchen", r#"{"baseRevision":0,"name":"hALL","deviceIds":["a"]}"#).await;

        assert_error(response, StatusCode::CONFLICT, "groupNameAlreadyExists").await;
        let stored = stored_groups(database).await;
        assert!(stored.iter().any(|group| group.id == "kitchen" && group.name == "Kitchen" && group.revision == 0));
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn update_group_returns_422_unprocessable_entity_for_a_group_without_devices() {
        let Fixture { state, database, mut store_rx } = create_state(&["a"]).await;

        let response = call_update_group(state, "kitchen", r#"{"baseRevision":0,"name":"Hall","deviceIds":[]}"#).await;

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = body_json(response).await;
        assert_eq!(body["code"], "groupEmpty");
        assert_eq!(body["message"], "a group must contain at least one device");
        assert_unchanged(database).await;
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn update_group_returns_422_unprocessable_entity_for_unknown_devices() {
        let Fixture { state, database, mut store_rx } = create_state(&["a"]).await;

        let response =
            call_update_group(state, "kitchen", r#"{"baseRevision":0,"name":"Hall","deviceIds":["z","a","b"]}"#).await;

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = body_json(response).await;
        assert_eq!(body["code"], "unknownDevices");
        assert_eq!(body["message"], "unknown device ids: 'b, z'");
        assert_unchanged(database).await;
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn update_group_returns_503_service_unavailable_when_the_store_is_gone() {
        let Fixture { state, database, store_rx } = create_state(&["a"]).await;
        drop(store_rx);

        let response =
            call_update_group(state, "kitchen", r#"{"baseRevision":0,"name":"Hall","deviceIds":["a"]}"#).await;

        assert_error(response, StatusCode::SERVICE_UNAVAILABLE, "storeUnavailable").await;
        assert_unchanged(database).await;
    }
}
