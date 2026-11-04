use crate::api::ApiState;
use crate::api::error::{ErrorResponse, InUseErrorResponse};
use crate::services::DeleteZoneError;
use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use tracing::{error, info};

pub(super) async fn delete_zone(
    Path(id): Path<String>,
    query: Result<Query<DeleteZoneQuery>, QueryRejection>,
    State(state): State<ApiState>,
) -> Result<Response, Response> {
    let Query(DeleteZoneQuery { base_revision }) = query.map_err(invalid_query)?;
    state.zone_service.delete_zone(&id, base_revision).await.map_err(delete_error_response)?;
    info!("📥 Received request to delete zone '{id}'... OK, revision {base_revision}");

    Ok(StatusCode::NO_CONTENT.into_response())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteZoneQuery {
    base_revision: u64,
}

fn invalid_query(err: QueryRejection) -> Response {
    (StatusCode::BAD_REQUEST, Json(ErrorResponse::new("invalidQuery", err.body_text()))).into_response()
}

fn delete_error_response(err: DeleteZoneError) -> Response {
    let error_message = err.to_string();
    match err {
        DeleteZoneError::NotFound => {
            (StatusCode::NOT_FOUND, Json(ErrorResponse::new_code_only("zoneNotFound"))).into_response()
        }
        DeleteZoneError::NotOwned => {
            (StatusCode::UNPROCESSABLE_ENTITY, Json(ErrorResponse::new_code_only("zoneNotOwned"))).into_response()
        }
        DeleteZoneError::RevisionConflict { .. } => {
            (StatusCode::CONFLICT, Json(ErrorResponse::new("revisionConflict", err.to_string()))).into_response()
        }
        DeleteZoneError::InUse(flows) => {
            (StatusCode::CONFLICT, Json(InUseErrorResponse::new("zoneInUse", error_message, flows))).into_response()
        }
        DeleteZoneError::StoreUnavailable => {
            error!("❌ Store is unavailable");
            (StatusCode::SERVICE_UNAVAILABLE, Json(ErrorResponse::new_code_only("storeUnavailable"))).into_response()
        }
        DeleteZoneError::Internal(_) => {
            error!("❌ Failed to persist zone to store: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse::new_code_only("storageError"))).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::{body_json, create_state_with_flows};
    use crate::api::zones::router;
    use crate::database::{Database, DatabaseError};
    use crate::domain::{Event, Owner, Zone};
    use crate::flow_engine::VersionedFlow;
    use crate::flow_registry::{FlowRegistry, FlowSummary};
    use crate::store::StoreSnapshot;
    use crate::test_support::flow_using_zones;
    use crate::zone_store::{StoredZone, ZoneStore};
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

    fn zone(id: &str, name: &str, owner: Owner) -> Zone {
        Zone {
            id: id.to_string(),
            name: name.to_string(),
            group_ids: BTreeSet::new(),
            device_ids: BTreeSet::new(),
            revision: 0,
            owner,
            external_id: None,
        }
    }

    /// Creates a state with the given zones in the snapshot, and the Hearth zones stored at revision 0.
    /// Bind `store_rx`: dropping it, e.g. by leaving it out with `..`, makes the store unavailable.
    async fn create_state(zones: Vec<Zone>) -> Fixture {
        create_state_with_flows_using(zones, vec![]).await
    }

    async fn create_state_with_flows_using(zones: Vec<Zone>, flows: Vec<VersionedFlow>) -> Fixture {
        let zones: Vec<_> = zones.into_iter().map(Arc::new).collect();
        let snapshot_zones: HashMap<_, _> = zones.iter().map(|zone| (zone.id.clone(), Arc::clone(zone))).collect();
        let snapshot = StoreSnapshot { zones: Arc::new(snapshot_zones), ..Default::default() };
        let (store_tx, store_rx) = mpsc::channel(8);
        let (state, database) = create_state_with_flows(FlowRegistry::new(flows), snapshot, store_tx);

        let zone_store = ZoneStore::new(database.clone());
        for zone in zones.iter().filter(|zone| zone.owner == Owner::Hearth) {
            zone_store.insert(&zone.id, &zone.name, &zone.group_ids, &zone.device_ids).await.expect("insert succeeded");
        }
        Fixture { state, database, store_rx }
    }

    async fn call_delete_zone(state: ApiState, uri: &str) -> Response {
        let request = Request::builder().method("DELETE").uri(uri).body(Body::empty()).unwrap();

        router().with_state(state).oneshot(request).await.unwrap()
    }

    fn delete_uri(id: &str, base_revision: u64) -> String {
        format!("/api/zones/{id}?baseRevision={base_revision}")
    }

    async fn stored_zones(database: Database) -> Vec<StoredZone> {
        ZoneStore::new(database).list().await.expect("list succeeded")
    }

    #[tokio::test]
    async fn delete_zone_returns_204_no_content_and_deletes_the_zone() {
        let Fixture { state, database, store_rx: _store_rx } =
            create_state(vec![zone("downstairs", "Downstairs", Owner::Hearth)]).await;

        let response = call_delete_zone(state, &delete_uri("downstairs", 0)).await;

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let stored = stored_zones(database).await;
        assert!(stored.is_empty(), "zone must be deleted, got {stored:?}");
    }

    #[tokio::test]
    async fn delete_zone_sends_a_zone_removed_event_to_the_store() {
        let Fixture { state, mut store_rx, .. } =
            create_state(vec![zone("downstairs", "Downstairs", Owner::Hearth)]).await;

        call_delete_zone(state, &delete_uri("downstairs", 0)).await;

        assert_eq!(
            store_rx.try_recv().expect("an event was sent"),
            Event::ZoneRemoved { zone_id: "downstairs".to_string() }
        );
        assert!(store_rx.try_recv().is_err(), "only one event must be sent");
    }

    #[tokio::test]
    async fn delete_zone_only_deletes_the_given_zone() {
        let Fixture { state, database, store_rx: _store_rx } = create_state(vec![
            zone("downstairs", "Downstairs", Owner::Hearth),
            zone("upstairs", "Upstairs", Owner::Hearth),
        ])
        .await;

        let response = call_delete_zone(state, &delete_uri("downstairs", 0)).await;

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let stored = stored_zones(database).await;
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].id, "upstairs");
    }

    #[tokio::test]
    async fn delete_zone_returns_404_not_found_for_an_unknown_zone() {
        let Fixture { state, database, mut store_rx } =
            create_state(vec![zone("downstairs", "Downstairs", Owner::Hearth)]).await;

        let response = call_delete_zone(state, &delete_uri("unknown", 0)).await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(body_json(response).await["code"], "zoneNotFound");
        assert_eq!(stored_zones(database).await.len(), 1, "nothing must be deleted");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn delete_zone_returns_422_unprocessable_entity_for_a_zone_owned_by_an_integration() {
        let Fixture { state, mut store_rx, .. } = create_state(vec![zone("hue", "Hue zone", Owner::Integration)]).await;

        let response = call_delete_zone(state, &delete_uri("hue", 0)).await;

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body_json(response).await["code"], "zoneNotOwned");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[rstest]
    #[case::behind(1)]
    #[case::ahead(5)]
    #[tokio::test]
    async fn delete_zone_returns_409_conflict_for_a_mismatching_base_revision(#[case] base_revision: u64) {
        let Fixture { state, database, mut store_rx } =
            create_state(vec![zone("downstairs", "Downstairs", Owner::Hearth)]).await;

        let response = call_delete_zone(state, &delete_uri("downstairs", base_revision)).await;

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = body_json(response).await;
        assert_eq!(body["code"], "revisionConflict");
        assert_eq!(
            body["message"],
            format!("revision conflict: delete is based on revision {base_revision}, but the current revision is 0")
        );
        assert_eq!(stored_zones(database).await.len(), 1, "nothing must be deleted");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[rstest]
    #[case::missing_base_revision("/api/zones/downstairs")]
    #[case::base_revision_is_not_a_number("/api/zones/downstairs?baseRevision=latest")]
    #[case::negative_base_revision("/api/zones/downstairs?baseRevision=-1")]
    #[tokio::test]
    async fn delete_zone_returns_400_bad_request_for_an_invalid_base_revision(#[case] uri: &str) {
        let Fixture { state, database, mut store_rx } =
            create_state(vec![zone("downstairs", "Downstairs", Owner::Hearth)]).await;

        let response = call_delete_zone(state, uri).await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(body_json(response).await["code"], "invalidQuery");
        assert_eq!(stored_zones(database).await.len(), 1, "nothing must be deleted");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn delete_zone_returns_503_service_unavailable_when_the_store_is_gone() {
        let Fixture { state, database, store_rx } =
            create_state(vec![zone("downstairs", "Downstairs", Owner::Hearth)]).await;
        drop(store_rx);

        let response = call_delete_zone(state, &delete_uri("downstairs", 0)).await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body_json(response).await["code"], "storeUnavailable");
        assert_eq!(stored_zones(database).await.len(), 1, "nothing must be deleted");
    }

    #[tokio::test]
    async fn delete_zone_returns_500_internal_server_error_when_the_database_fails() {
        let Fixture { state, database, mut store_rx } =
            create_state(vec![zone("downstairs", "Downstairs", Owner::Hearth)]).await;
        database
            .call(|conn| conn.execute("DROP TABLE zones", []).map_err(DatabaseError::from))
            .await
            .expect("drop the table");

        let response = call_delete_zone(state, &delete_uri("downstairs", 0)).await;

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body_json(response).await["code"], "storageError");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn delete_zone_returns_409_conflict_when_flows_use_the_zone() {
        let flows = vec![
            flow_using_zones("b", "Flow B", 2, &["downstairs"]),
            flow_using_zones("a", "Flow A", 0, &["downstairs"]),
            flow_using_zones("c", "Flow C", 1, &["upstairs"]),
        ];
        let Fixture { state, database, mut store_rx } =
            create_state_with_flows_using(vec![zone("downstairs", "Downstairs", Owner::Hearth)], flows).await;

        let response = call_delete_zone(state, &delete_uri("downstairs", 0)).await;

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = body_json(response).await;
        assert_eq!(body["code"], "zoneInUse");
        assert_eq!(body["message"], "zone in use by 2 flow(s)");
        assert_eq!(
            body["flows"],
            serde_json::json!([
                {"id": "a", "name": "Flow A", "revision": 0},
                {"id": "b", "name": "Flow B", "revision": 2},
            ])
        );
        assert_eq!(stored_zones(database).await.len(), 1, "nothing must be deleted");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn delete_zone_ignores_flows_using_other_zones() {
        let flows = vec![flow_using_zones("a", "Flow A", 0, &["upstairs"])];
        let Fixture { state, database, store_rx: _store_rx } =
            create_state_with_flows_using(vec![zone("downstairs", "Downstairs", Owner::Hearth)], flows).await;

        let response = call_delete_zone(state, &delete_uri("downstairs", 0)).await;

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert!(stored_zones(database).await.is_empty());
    }

    #[tokio::test]
    async fn delete_error_response_maps_in_use_to_409_conflict_with_the_flows() {
        let flows = vec![
            FlowSummary { id: "a".to_string(), name: "Flow A".to_string(), revision: 0 },
            FlowSummary { id: "b".to_string(), name: "Flow B".to_string(), revision: 2 },
        ];

        let response = delete_error_response(DeleteZoneError::InUse(flows));

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = body_json(response).await;
        assert_eq!(body["code"], "zoneInUse");
        assert_eq!(body["message"], "zone in use by 2 flow(s)");
        assert_eq!(
            body["flows"],
            serde_json::json!([
                {"id": "a", "name": "Flow A", "revision": 0},
                {"id": "b", "name": "Flow B", "revision": 2},
            ])
        );
    }
}
