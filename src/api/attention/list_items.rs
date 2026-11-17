use crate::api::ApiState;
use crate::attention::{Attention, Issue, Subject};
use crate::services::DeviceService;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::Serialize;

pub(super) async fn list_items(State(state): State<ApiState>) -> Result<Response, Response> {
    let items: Vec<_> = state
        .attention_service
        .list()
        .iter()
        .map(|attention| AttentionView::new(attention, &state.device_service))
        .collect();

    Ok((StatusCode::OK, Json(AttentionResponse { items })).into_response())
}

#[derive(Debug, Serialize)]
struct AttentionResponse {
    items: Vec<AttentionView>,
}

#[derive(Debug, Serialize)]
struct AttentionView {
    subject: SubjectView,
    issue: IssueView,
    since: DateTime<Utc>,
}

impl AttentionView {
    fn new(attention: &Attention, device_service: &DeviceService) -> AttentionView {
        AttentionView {
            subject: SubjectView::new(&attention.subject, device_service),
            issue: (&attention.issue).into(),
            since: attention.since,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum SubjectView {
    Device { id: String, name: Option<String> },
}

impl SubjectView {
    fn new(subject: &Subject, device_service: &DeviceService) -> SubjectView {
        match subject {
            Subject::Device(id) => SubjectView::Device {
                id: id.clone(),
                // The device can disappear between the attention snapshot and this lookup
                name: device_service.retrieve_device(id).ok().map(|device| device.name.clone()),
            },
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum IssueView {
    Connectivity { state: String },
    Battery { state: String },
}

impl From<&Issue> for IssueView {
    fn from(issue: &Issue) -> Self {
        match issue {
            Issue::Connectivity(connectivity) => IssueView::Connectivity { state: connectivity.to_string() },
            Issue::Battery(battery_state) => IssueView::Battery { state: battery_state.to_string() },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::attention::router;
    use crate::api::test_support::{body_json, create_state_with_attention};
    use crate::domain::{BatteryState, Connectivity, Device};
    use crate::store::StoreSnapshot;
    use crate::test_support::DeviceBuilder;
    use axum::body::Body;
    use axum::http::Request;
    use chrono::TimeZone;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tower::ServiceExt;

    fn snapshot_with_devices(devices: Vec<Device>) -> StoreSnapshot {
        let devices: HashMap<_, _> = devices.into_iter().map(|device| (device.id.clone(), Arc::new(device))).collect();
        StoreSnapshot { devices: Arc::new(devices), ..Default::default() }
    }

    fn attention(device_id: &str, issue: Issue) -> Attention {
        Attention {
            subject: Subject::Device(device_id.into()),
            issue,
            since: Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap(),
        }
    }

    async fn call_list_items(state: ApiState) -> Response {
        let request = Request::builder().method("GET").uri("/api/attention").body(Body::empty()).unwrap();

        router().with_state(state).oneshot(request).await.unwrap()
    }

    #[tokio::test]
    async fn list_items_returns_no_items_when_nothing_needs_attention() {
        let state = create_state_with_attention(StoreSnapshot::default(), vec![]);

        let response = call_list_items(state).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body_json(response).await, json!({ "items": [] }));
    }

    #[tokio::test]
    async fn list_items_returns_the_items_with_the_name_of_the_device() {
        let snapshot = snapshot_with_devices(vec![
            Device { name: "Kitchen lamp".into(), ..DeviceBuilder::new("1").build() },
            Device { name: "Door sensor".into(), ..DeviceBuilder::new("2").build() },
        ]);
        let items = vec![
            attention("1", Issue::Connectivity(Connectivity::Disconnected)),
            attention("2", Issue::Battery(BatteryState::Low)),
        ];

        let response = call_list_items(create_state_with_attention(snapshot, items)).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            body_json(response).await,
            json!({
                "items": [
                    {
                        "subject": { "type": "device", "id": "1", "name": "Kitchen lamp" },
                        "issue": { "type": "connectivity", "state": "disconnected" },
                        "since": "2026-01-01T12:00:00Z",
                    },
                    {
                        "subject": { "type": "device", "id": "2", "name": "Door sensor" },
                        "issue": { "type": "battery", "state": "low" },
                        "since": "2026-01-01T12:00:00Z",
                    },
                ]
            })
        );
    }

    #[tokio::test]
    async fn list_items_returns_a_null_name_for_a_device_that_is_gone() {
        let items = vec![attention("gone", Issue::Battery(BatteryState::Critical))];

        let response = call_list_items(create_state_with_attention(StoreSnapshot::default(), items)).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            body_json(response).await["items"][0]["subject"],
            json!({ "type": "device", "id": "gone", "name": null })
        );
    }
}
