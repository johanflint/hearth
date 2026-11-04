use crate::api::ApiState;
use crate::api::groups::group_view::GroupView;
use crate::services::RetrieveGroupError;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

pub(super) async fn retrieve_group(
    Path(id): Path<String>,
    State(state): State<ApiState>,
) -> Result<Response, Response> {
    let group = state.group_service.retrieve_group(&id).map_err(retrieve_error_response)?;
    let response = GroupView::from(group);
    Ok((StatusCode::OK, Json(response)).into_response())
}

fn retrieve_error_response(err: RetrieveGroupError) -> Response {
    match err {
        RetrieveGroupError::NotFound => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::groups::router;
    use crate::api::groups::test_support::{group, state_with_groups};
    use crate::api::test_support::body_json;
    use crate::domain::{Group, Owner};
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use rstest::rstest;
    use serde_json::json;
    use std::collections::BTreeSet;
    use tower::ServiceExt;

    async fn call_retrieve_group(state: ApiState, id: &str) -> Response {
        let request = Request::builder().method("GET").uri(format!("/api/groups/{id}")).body(Body::empty()).unwrap();

        router().with_state(state).oneshot(request).await.unwrap()
    }

    #[tokio::test]
    async fn retrieve_group_returns_the_group_with_camel_case_fields() {
        let state = state_with_groups(vec![group("living-room", "Living room", 3, Owner::Integration)]);

        let response = call_retrieve_group(state, "living-room").await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            body_json(response).await,
            json!({
                "id": "living-room",
                "name": "Living room",
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
    async fn retrieve_group_returns_the_owner_of_the_group(#[case] owner: Owner, #[case] expected: &str) {
        let state = state_with_groups(vec![group("group", "Group", 0, owner)]);

        let response = call_retrieve_group(state, "group").await;

        assert_eq!(body_json(response).await["owner"], expected);
    }

    #[rstest]
    #[case::none(&[], json!([]))]
    #[case::sorted(&["b", "c", "a"], json!(["a", "b", "c"]))]
    #[tokio::test]
    async fn retrieve_group_returns_the_sorted_device_ids_of_the_group(
        #[case] device_ids: &[&str],
        #[case] expected: serde_json::Value,
    ) {
        let device_ids = device_ids.iter().map(|id| id.to_string()).collect::<BTreeSet<_>>();
        let state = state_with_groups(vec![Group { device_ids, ..group("group", "Group", 0, Owner::Hearth) }]);

        let response = call_retrieve_group(state, "group").await;

        assert_eq!(body_json(response).await["deviceIds"], expected);
    }

    #[tokio::test]
    async fn retrieve_group_finds_an_id_with_url_encoded_characters() {
        let state = state_with_groups(vec![group("a b", "Spaced", 0, Owner::Hearth)]);

        let response = call_retrieve_group(state, "a%20b").await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body_json(response).await["name"], "Spaced");
    }

    #[tokio::test]
    async fn retrieve_group_returns_only_the_requested_group() {
        let state =
            state_with_groups(vec![group("a", "A", 0, Owner::Integration), group("b", "B", 0, Owner::Integration)]);

        let response = call_retrieve_group(state, "b").await;

        assert_eq!(body_json(response).await["name"], "B");
    }

    #[tokio::test]
    async fn retrieve_group_returns_404_for_an_unknown_group() {
        let state = state_with_groups(vec![group("a", "A", 0, Owner::Integration)]);

        let response = call_retrieve_group(state, "unknown").await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert!(body.is_empty(), "got {body:?}");
    }
}
