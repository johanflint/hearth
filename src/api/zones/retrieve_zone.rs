use crate::api::ApiState;
use crate::api::zones::zone_view::ZoneView;
use crate::zone_service::RetrieveZoneError;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

pub(super) async fn retrieve_zone(Path(id): Path<String>, State(state): State<ApiState>) -> Result<Response, Response> {
    let zone = state.zone_service.retrieve_zone(&id).map_err(retrieve_error_response)?;
    let response = ZoneView::from(zone);
    Ok((StatusCode::OK, Json(response)).into_response())
}

fn retrieve_error_response(err: RetrieveZoneError) -> Response {
    match err {
        RetrieveZoneError::NotFound => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::body_json;
    use crate::api::zones::router;
    use crate::api::zones::test_support::{ids, state_with_zones, zone};
    use crate::domain::{Owner, Zone};
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use rstest::rstest;
    use serde_json::json;
    use tower::ServiceExt;

    async fn call_retrieve_zone(state: ApiState, id: &str) -> Response {
        let request = Request::builder().method("GET").uri(format!("/api/zones/{id}")).body(Body::empty()).unwrap();

        router().with_state(state).oneshot(request).await.unwrap()
    }

    #[tokio::test]
    async fn retrieve_zone_returns_the_zone_with_camel_case_fields() {
        let state = state_with_zones(vec![zone("downstairs", "Downstairs", 3, Owner::Integration)]);

        let response = call_retrieve_zone(state, "downstairs").await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            body_json(response).await,
            json!({
                "id": "downstairs",
                "name": "Downstairs",
                "groupIds": ["group"],
                "deviceIds": ["device"],
                "revision": 3,
                "owner": "integration",
            })
        );
    }

    #[rstest]
    #[case::hearth(Owner::Hearth, "hearth")]
    #[case::integration(Owner::Integration, "integration")]
    #[tokio::test]
    async fn retrieve_zone_returns_the_owner_of_the_zone(#[case] owner: Owner, #[case] expected: &str) {
        let state = state_with_zones(vec![zone("zone", "Zone", 0, owner)]);

        let response = call_retrieve_zone(state, "zone").await;

        assert_eq!(body_json(response).await["owner"], expected);
    }

    #[rstest]
    #[case::none(&[], json!([]))]
    #[case::sorted(&["b", "c", "a"], json!(["a", "b", "c"]))]
    #[tokio::test]
    async fn retrieve_zone_returns_the_sorted_group_ids_of_the_zone(
        #[case] group_ids: &[&str],
        #[case] expected: serde_json::Value,
    ) {
        let state =
            state_with_zones(vec![Zone { group_ids: ids(group_ids), ..zone("zone", "Zone", 0, Owner::Hearth) }]);

        let response = call_retrieve_zone(state, "zone").await;

        assert_eq!(body_json(response).await["groupIds"], expected);
    }

    #[rstest]
    #[case::none(&[], json!([]))]
    #[case::sorted(&["b", "c", "a"], json!(["a", "b", "c"]))]
    #[tokio::test]
    async fn retrieve_zone_returns_the_sorted_device_ids_of_the_zone(
        #[case] device_ids: &[&str],
        #[case] expected: serde_json::Value,
    ) {
        let state =
            state_with_zones(vec![Zone { device_ids: ids(device_ids), ..zone("zone", "Zone", 0, Owner::Hearth) }]);

        let response = call_retrieve_zone(state, "zone").await;

        assert_eq!(body_json(response).await["deviceIds"], expected);
    }

    #[tokio::test]
    async fn retrieve_zone_finds_an_id_with_url_encoded_characters() {
        let state = state_with_zones(vec![zone("a b", "Spaced", 0, Owner::Hearth)]);

        let response = call_retrieve_zone(state, "a%20b").await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body_json(response).await["name"], "Spaced");
    }

    #[tokio::test]
    async fn retrieve_zone_returns_only_the_requested_zone() {
        let state =
            state_with_zones(vec![zone("a", "A", 0, Owner::Integration), zone("b", "B", 0, Owner::Integration)]);

        let response = call_retrieve_zone(state, "b").await;

        assert_eq!(body_json(response).await["name"], "B");
    }

    #[tokio::test]
    async fn retrieve_zone_returns_404_for_an_unknown_zone() {
        let state = state_with_zones(vec![zone("a", "A", 0, Owner::Integration)]);

        let response = call_retrieve_zone(state, "unknown").await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert!(body.is_empty(), "got {body:?}");
    }
}
