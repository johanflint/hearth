use crate::api::ApiState;
use crate::api::attention::list_items::attention_response;
use axum::extract::State;
use axum::response::Sse;
use axum::response::sse::{Event, KeepAlive};
use futures::Stream;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::WatchStream;

const SNAPSHOT_EVENT_NAME: &str = "snapshot";

pub(super) async fn attention_events(
    State(state): State<ApiState>,
) -> Sse<impl Stream<Item = Result<Event, axum::Error>>> {
    let updates = WatchStream::new(state.attention_service.subscribe()).map(move |list| {
        let response = attention_response(&list, &state.device_service, &state.flow_service);
        Event::default().event(SNAPSHOT_EVENT_NAME).json_data(response)
    });

    Sse::new(updates).keep_alive(KeepAlive::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::attention::router;
    use crate::api::test_support::create_state_with_snapshot;
    use crate::attention::{Attention, AttentionList, Issue, Subject};
    use crate::domain::{BatteryState, Connectivity, Device};
    use crate::integration::IntegrationStatus;
    use crate::services::AttentionService;
    use crate::store::StoreSnapshot;
    use crate::test_support::DeviceBuilder;
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use axum::response::Response;
    use chrono::{TimeZone, Utc};
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::watch;
    use tokio::time::{Instant, timeout};
    use tower::ServiceExt;

    struct Fixture {
        state: ApiState,
        attention_tx: watch::Sender<AttentionList>,
    }

    /// The API state with the given devices in the store and attention list, and control over the list its clients receive
    fn fixture(devices: Vec<Device>, list: AttentionList) -> Fixture {
        let devices: HashMap<_, _> = devices.into_iter().map(|device| (device.id.clone(), Arc::new(device))).collect();
        let snapshot = StoreSnapshot { devices: Arc::new(devices), ..Default::default() };
        let (attention_tx, attention_rx) = watch::channel(list);
        let mut state = create_state_with_snapshot(snapshot);
        state.attention_service = Arc::new(AttentionService::new(attention_rx));
        Fixture { state, attention_tx }
    }

    fn device_attention(device_id: &str, issue: Issue) -> Attention {
        Attention {
            subject: Subject::Device(device_id.into()),
            issue,
            since: Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap(),
        }
    }

    fn devices(items: Vec<Attention>) -> AttentionList {
        AttentionList { devices: items.into(), ..Default::default() }
    }

    async fn call_attention_events(state: ApiState) -> Response {
        let request = Request::builder().method("GET").uri("/api/attention/events").body(Body::empty()).unwrap();

        router().with_state(state).oneshot(request).await.unwrap()
    }

    async fn next_frame(body: &mut Body) -> Option<String> {
        let frame = timeout(Duration::from_secs(1), body.frame()).await.expect("expected a frame within a second")?;
        Some(String::from_utf8(frame.unwrap().into_data().unwrap().to_vec()).unwrap())
    }

    /// The name and data of the next event
    async fn next_event(body: &mut Body) -> (String, Value) {
        let text = next_frame(body).await.expect("expected the stream to be open");
        let name = text.lines().find_map(|line| line.strip_prefix("event: ")).expect("expected an event name");
        let data = text.lines().find_map(|line| line.strip_prefix("data: ")).expect("expected event data");
        (name.to_string(), serde_json::from_str(data).unwrap())
    }

    #[tokio::test]
    async fn attention_events_responds_with_an_event_stream() {
        let response = call_attention_events(fixture(vec![], AttentionList::default()).state).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "text/event-stream");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-cache");
    }

    #[tokio::test]
    async fn attention_events_starts_with_an_empty_snapshot_when_nothing_needs_attention() {
        let mut body = call_attention_events(fixture(vec![], AttentionList::default()).state).await.into_body();

        assert_eq!(next_event(&mut body).await, ("snapshot".to_string(), json!({ "items": [] })));
    }

    #[tokio::test]
    async fn attention_events_starts_with_a_snapshot_like_the_attention_list() {
        let lamp = Device { name: "Kitchen lamp".into(), ..DeviceBuilder::new("1").build() };
        let list = devices(vec![device_attention("1", Issue::Connectivity(Connectivity::Disconnected))]);
        let mut body = call_attention_events(fixture(vec![lamp], list).state).await.into_body();

        assert_eq!(
            next_event(&mut body).await,
            (
                "snapshot".to_string(),
                json!({
                    "items": [{
                        "subject": { "type": "device", "id": "1", "name": "Kitchen lamp" },
                        "issue": { "type": "connectivity", "state": "disconnected" },
                        "since": "2026-01-01T12:00:00Z",
                    }]
                })
            )
        );
    }

    #[tokio::test]
    async fn attention_events_sends_a_snapshot_when_the_list_changes() {
        let Fixture { state, attention_tx } = fixture(vec![], AttentionList::default());
        let mut body = call_attention_events(state).await.into_body();
        next_event(&mut body).await;

        attention_tx.send_replace(AttentionList {
            integrations: vec![Attention {
                subject: Subject::Integration("hue".into()),
                issue: Issue::Integration(IntegrationStatus::Down),
                since: Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap(),
            }]
            .into(),
            ..Default::default()
        });

        let (name, data) = next_event(&mut body).await;
        assert_eq!(name, "snapshot");
        assert_eq!(data["items"][0]["subject"], json!({ "type": "integration", "name": "hue" }));
    }

    #[tokio::test]
    async fn attention_events_sends_an_empty_snapshot_when_everything_is_resolved() {
        let list = devices(vec![device_attention("1", Issue::Battery(BatteryState::Low))]);
        let Fixture { state, attention_tx } = fixture(vec![], list);
        let mut body = call_attention_events(state).await.into_body();
        next_event(&mut body).await;

        attention_tx.send_replace(AttentionList::default());

        assert_eq!(next_event(&mut body).await, ("snapshot".to_string(), json!({ "items": [] })));
    }

    #[tokio::test]
    async fn attention_events_sends_only_the_latest_list_when_the_client_missed_updates() {
        let Fixture { state, attention_tx } = fixture(vec![], AttentionList::default());
        let mut body = call_attention_events(state).await.into_body();
        next_event(&mut body).await;

        attention_tx.send_replace(devices(vec![device_attention("1", Issue::Battery(BatteryState::Low))]));
        attention_tx.send_replace(devices(vec![device_attention("1", Issue::Battery(BatteryState::Critical))]));

        let (_, data) = next_event(&mut body).await;
        assert_eq!(data["items"][0]["issue"], json!({ "type": "battery", "state": "critical" }));
    }

    #[tokio::test]
    async fn attention_events_ends_when_the_tracker_stops() {
        let Fixture { state, attention_tx } = fixture(vec![], AttentionList::default());
        let mut body = call_attention_events(state).await.into_body();
        next_event(&mut body).await;

        drop(attention_tx);

        assert_eq!(next_frame(&mut body).await, None);
    }

    #[tokio::test(start_paused = true)]
    async fn attention_events_sends_a_keep_alive_while_idle() {
        let fixture = fixture(vec![], AttentionList::default());
        let mut body = call_attention_events(fixture.state).await.into_body();
        body.frame().await.unwrap().unwrap();
        let start = Instant::now();

        // No timeout, the paused clock skips ahead to the keep-alive
        let frame = body.frame().await.unwrap().unwrap().into_data().unwrap();

        assert_eq!(frame, ":\n\n");
        assert_eq!(start.elapsed(), Duration::from_secs(15));
    }
}
