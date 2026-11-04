use crate::api::ApiState;
use crate::api::error::{ErrorResponse, InUseErrorResponse};
use crate::group_service::DeleteGroupError;
use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use tracing::{error, info};

pub(super) async fn delete_group(
    Path(id): Path<String>,
    query: Result<Query<DeleteGroupQuery>, QueryRejection>,
    State(state): State<ApiState>,
) -> Result<Response, Response> {
    let Query(DeleteGroupQuery { base_revision }) = query.map_err(invalid_query)?;
    state.group_service.delete_group(&id, base_revision).await.map_err(delete_error_response)?;
    info!("📥 Received request to delete group '{id}'... OK, revision {base_revision}");

    Ok(StatusCode::NO_CONTENT.into_response())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteGroupQuery {
    base_revision: u64,
}

fn invalid_query(err: QueryRejection) -> Response {
    (StatusCode::BAD_REQUEST, Json(ErrorResponse::new("invalidQuery", err.body_text()))).into_response()
}

fn delete_error_response(err: DeleteGroupError) -> Response {
    let error_message = err.to_string();
    match err {
        DeleteGroupError::InUse(flows) => {
            (StatusCode::CONFLICT, Json(InUseErrorResponse::new("groupInUse", error_message, flows))).into_response()
        }
        DeleteGroupError::NotFound => {
            (StatusCode::NOT_FOUND, Json(ErrorResponse::new_code_only("groupNotFound"))).into_response()
        }
        DeleteGroupError::NotOwned => {
            (StatusCode::UNPROCESSABLE_ENTITY, Json(ErrorResponse::new_code_only("groupNotOwned"))).into_response()
        }
        DeleteGroupError::RevisionConflict { .. } => {
            (StatusCode::CONFLICT, Json(ErrorResponse::new("revisionConflict", err.to_string()))).into_response()
        }
        DeleteGroupError::StoreUnavailable => {
            error!("❌ Store is unavailable");
            (StatusCode::SERVICE_UNAVAILABLE, Json(ErrorResponse::new_code_only("storeUnavailable"))).into_response()
        }
        DeleteGroupError::Internal(_) => {
            error!("❌ Failed to persist group to store: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse::new_code_only("storageError"))).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::groups::router;
    use crate::api::test_support::{body_json, create_state_with_flows};
    use crate::database::{Database, DatabaseError};
    use crate::domain::{Event, Group, Owner};
    use crate::flow_engine::VersionedFlow;
    use crate::flow_registry::{FlowRegistry, FlowSummary};
    use crate::group_store::{GroupStore, StoredGroup};
    use crate::store::StoreSnapshot;
    use crate::test_support::flow_using_groups;
    use axum::body::Body;
    use axum::http::Request;
    use rstest::rstest;
    use std::collections::{BTreeSet, HashMap};
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use tower::ServiceExt;

    struct Fixture {
        state: ApiState,
        database: Database,
        store_rx: mpsc::Receiver<Event>,
    }

    fn group(id: &str, name: &str, owner: Owner) -> Group {
        Group {
            id: id.to_string(),
            name: name.to_string(),
            device_ids: BTreeSet::new(),
            revision: 0,
            owner,
            external_id: None,
        }
    }

    /// Creates a state with the given groups in the snapshot, and the Hearth groups stored at revision 0.
    /// Bind `store_rx`: dropping it, e.g. by leaving it out with `..`, makes the store unavailable.
    async fn create_state(groups: Vec<Group>) -> Fixture {
        create_state_with_flows_using(groups, vec![]).await
    }

    async fn create_state_with_flows_using(groups: Vec<Group>, flows: Vec<VersionedFlow>) -> Fixture {
        let groups: Vec<_> = groups.into_iter().map(Arc::new).collect();
        let snapshot_groups: HashMap<_, _> = groups.iter().map(|group| (group.id.clone(), Arc::clone(group))).collect();
        let snapshot = StoreSnapshot { groups: Arc::new(snapshot_groups), ..Default::default() };
        let (store_tx, store_rx) = mpsc::channel(8);
        let (state, database) = create_state_with_flows(FlowRegistry::new(flows), snapshot, store_tx);

        let group_store = GroupStore::new(database.clone());
        for group in groups.iter().filter(|group| group.owner == Owner::Hearth) {
            group_store.insert(&group.id, &group.name, &group.device_ids).await.expect("insert succeeded");
        }
        Fixture { state, database, store_rx }
    }

    async fn call_delete_group(state: ApiState, uri: &str) -> Response {
        let request = Request::builder().method("DELETE").uri(uri).body(Body::empty()).unwrap();

        router().with_state(state).oneshot(request).await.unwrap()
    }

    fn delete_uri(id: &str, base_revision: u64) -> String {
        format!("/api/groups/{id}?baseRevision={base_revision}")
    }

    async fn stored_groups(database: Database) -> Vec<StoredGroup> {
        GroupStore::new(database).list().await.expect("list succeeded")
    }

    #[tokio::test]
    async fn delete_group_returns_204_no_content_and_deletes_the_group() {
        let Fixture { state, database, store_rx: _store_rx } =
            create_state(vec![group("kitchen", "Kitchen", Owner::Hearth)]).await;

        let response = call_delete_group(state, &delete_uri("kitchen", 0)).await;

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let stored = stored_groups(database).await;
        assert!(stored.is_empty(), "group must be deleted, got {stored:?}");
    }

    #[tokio::test]
    async fn delete_group_sends_a_group_removed_event_to_the_store() {
        let Fixture { state, mut store_rx, .. } = create_state(vec![group("kitchen", "Kitchen", Owner::Hearth)]).await;

        call_delete_group(state, &delete_uri("kitchen", 0)).await;

        assert_eq!(
            store_rx.try_recv().expect("an event was sent"),
            Event::GroupRemoved { group_id: "kitchen".to_string() }
        );
        assert!(store_rx.try_recv().is_err(), "only one event must be sent");
    }

    #[tokio::test]
    async fn delete_group_only_deletes_the_given_group() {
        let Fixture { state, database, store_rx: _store_rx } =
            create_state(vec![group("kitchen", "Kitchen", Owner::Hearth), group("hall", "Hall", Owner::Hearth)]).await;

        let response = call_delete_group(state, &delete_uri("kitchen", 0)).await;

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let stored = stored_groups(database).await;
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].id, "hall");
    }

    #[tokio::test]
    async fn delete_group_returns_404_not_found_for_an_unknown_group() {
        let Fixture { state, database, mut store_rx } =
            create_state(vec![group("kitchen", "Kitchen", Owner::Hearth)]).await;

        let response = call_delete_group(state, &delete_uri("unknown", 0)).await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(body_json(response).await["code"], "groupNotFound");
        assert_eq!(stored_groups(database).await.len(), 1, "nothing must be deleted");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn delete_group_returns_422_unprocessable_entity_for_a_group_owned_by_an_integration() {
        let Fixture { state, mut store_rx, .. } =
            create_state(vec![group("hue", "Hue room", Owner::Integration)]).await;

        let response = call_delete_group(state, &delete_uri("hue", 0)).await;

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body_json(response).await["code"], "groupNotOwned");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[rstest]
    #[case::behind(1)]
    #[case::ahead(5)]
    #[tokio::test]
    async fn delete_group_returns_409_conflict_for_a_mismatching_base_revision(#[case] base_revision: u64) {
        let Fixture { state, database, mut store_rx } =
            create_state(vec![group("kitchen", "Kitchen", Owner::Hearth)]).await;

        let response = call_delete_group(state, &delete_uri("kitchen", base_revision)).await;

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = body_json(response).await;
        assert_eq!(body["code"], "revisionConflict");
        assert_eq!(
            body["message"],
            format!("revision conflict: delete is based on revision {base_revision}, but the current revision is 0")
        );
        assert_eq!(stored_groups(database).await.len(), 1, "nothing must be deleted");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[rstest]
    #[case::missing_base_revision("/api/groups/kitchen")]
    #[case::base_revision_is_not_a_number("/api/groups/kitchen?baseRevision=latest")]
    #[case::negative_base_revision("/api/groups/kitchen?baseRevision=-1")]
    #[tokio::test]
    async fn delete_group_returns_400_bad_request_for_an_invalid_base_revision(#[case] uri: &str) {
        let Fixture { state, database, mut store_rx } =
            create_state(vec![group("kitchen", "Kitchen", Owner::Hearth)]).await;

        let response = call_delete_group(state, uri).await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(body_json(response).await["code"], "invalidQuery");
        assert_eq!(stored_groups(database).await.len(), 1, "nothing must be deleted");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn delete_group_returns_503_service_unavailable_when_the_store_is_gone() {
        let Fixture { state, database, store_rx } =
            create_state(vec![group("kitchen", "Kitchen", Owner::Hearth)]).await;
        drop(store_rx);

        let response = call_delete_group(state, &delete_uri("kitchen", 0)).await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body_json(response).await["code"], "storeUnavailable");
        assert_eq!(stored_groups(database).await.len(), 1, "nothing must be deleted");
    }

    #[tokio::test]
    async fn delete_group_returns_500_internal_server_error_when_the_database_fails() {
        let Fixture { state, database, mut store_rx } =
            create_state(vec![group("kitchen", "Kitchen", Owner::Hearth)]).await;
        database
            .call(|conn| conn.execute("DROP TABLE groups", []).map_err(DatabaseError::from))
            .await
            .expect("drop the table");

        let response = call_delete_group(state, &delete_uri("kitchen", 0)).await;

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body_json(response).await["code"], "storageError");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn delete_group_returns_409_conflict_when_flows_use_the_group() {
        let flows = vec![
            flow_using_groups("b", "Flow B", 2, &["kitchen"]),
            flow_using_groups("a", "Flow A", 0, &["kitchen"]),
            flow_using_groups("c", "Flow C", 1, &["hall"]),
        ];
        let Fixture { state, database, mut store_rx } =
            create_state_with_flows_using(vec![group("kitchen", "Kitchen", Owner::Hearth)], flows).await;

        let response = call_delete_group(state, &delete_uri("kitchen", 0)).await;

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = body_json(response).await;
        assert_eq!(body["code"], "groupInUse");
        assert_eq!(body["message"], "group in use by 2 flow(s)");
        assert_eq!(
            body["flows"],
            serde_json::json!([
                {"id": "a", "name": "Flow A", "revision": 0},
                {"id": "b", "name": "Flow B", "revision": 2},
            ])
        );
        assert_eq!(stored_groups(database).await.len(), 1, "nothing must be deleted");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn delete_group_ignores_flows_using_other_groups() {
        let flows = vec![flow_using_groups("a", "Flow A", 0, &["hall"])];
        let Fixture { state, database, store_rx: _store_rx } =
            create_state_with_flows_using(vec![group("kitchen", "Kitchen", Owner::Hearth)], flows).await;

        let response = call_delete_group(state, &delete_uri("kitchen", 0)).await;

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert!(stored_groups(database).await.is_empty());
    }

    #[tokio::test]
    async fn delete_error_response_maps_in_use_to_409_conflict_with_the_flows() {
        let flows = vec![
            FlowSummary { id: "a".to_string(), name: "Flow A".to_string(), revision: 0 },
            FlowSummary { id: "b".to_string(), name: "Flow B".to_string(), revision: 2 },
        ];

        let response = delete_error_response(DeleteGroupError::InUse(flows));

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = body_json(response).await;
        assert_eq!(body["code"], "groupInUse");
        assert_eq!(body["message"], "group in use by 2 flow(s)");
        assert_eq!(
            body["flows"],
            serde_json::json!([
                {"id": "a", "name": "Flow A", "revision": 0},
                {"id": "b", "name": "Flow B", "revision": 2},
            ])
        );
    }
}
