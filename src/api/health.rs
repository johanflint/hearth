use crate::api::ApiState;
use crate::integration::IntegrationStatus;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use std::collections::BTreeMap;
use tokio::task::JoinHandle;
use tracing::warn;

pub fn router() -> Router<ApiState> {
    Router::new().route("/health", get(health))
}

async fn health(State(state): State<ApiState>) -> impl IntoResponse {
    let components = Components {
        database: ComponentStatus { status: database_status(&state).await },
        store: task_status("store", &state.core_tasks.store),
        scheduler: task_status("scheduler", &state.core_tasks.scheduler),
        store_listener: task_status("storeListener", &state.core_tasks.store_listener),
        integrations: state
            .integrations
            .iter()
            .map(|integration| (integration.name(), ComponentStatus { status: integration.status().into() }))
            .collect(),
    };
    let response = HealthStatusResponse::new(components);
    (response.status.http_status(), Json(response))
}

async fn database_status(state: &ApiState) -> HealthStatus {
    match state.database.ping().await {
        Ok(_) => HealthStatus::Up,
        Err(e) => {
            warn!("🏥 Health check: database ping failed: {e}");
            HealthStatus::Down
        }
    }
}

fn task_status(name: &str, handle: &JoinHandle<()>) -> ComponentStatus {
    if handle.is_finished() {
        warn!("🏥 Health check: task '{name}' has stopped");
        return ComponentStatus { status: HealthStatus::Down };
    }
    ComponentStatus { status: HealthStatus::Up }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
enum HealthStatus {
    Up,
    Degraded,
    Down,
}

impl HealthStatus {
    fn http_status(&self) -> StatusCode {
        match self {
            HealthStatus::Up | HealthStatus::Degraded => StatusCode::OK,
            HealthStatus::Down => StatusCode::SERVICE_UNAVAILABLE,
        }
    }
}

impl From<IntegrationStatus> for HealthStatus {
    fn from(status: IntegrationStatus) -> Self {
        match status {
            IntegrationStatus::Up => HealthStatus::Up,
            IntegrationStatus::Degraded => HealthStatus::Degraded,
            IntegrationStatus::Down => HealthStatus::Down,
        }
    }
}

#[derive(Debug, Serialize)]
struct HealthStatusResponse {
    status: HealthStatus,
    components: Components,
}

impl HealthStatusResponse {
    fn new(components: Components) -> Self {
        // Critical components only: all of them must be up
        let critical = [
            components.database.status,
            components.store.status,
            components.scheduler.status,
            components.store_listener.status,
        ];

        let status = if critical.contains(&HealthStatus::Down) { HealthStatus::Down } else { HealthStatus::Up };
        Self { status, components }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Components {
    database: ComponentStatus,
    store: ComponentStatus,
    scheduler: ComponentStatus,
    store_listener: ComponentStatus,
    integrations: BTreeMap<&'static str, ComponentStatus>,
}

#[derive(Debug, Serialize)]
struct ComponentStatus {
    status: HealthStatus,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::{Fixture, body_json, create_state_with_database};
    use crate::database::DatabaseError;
    use crate::domain::{Command, Controller, Event};
    use crate::flow_registry::FlowRegistry;
    use crate::integration::{Discovery, Integration, IntegrationError};
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::Request;
    use axum::response::Response;
    use rstest::rstest;
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::mpsc::Sender;
    use tower::ServiceExt;

    #[derive(Debug)]
    struct StubIntegration {
        name: &'static str,
        status: IntegrationStatus,
    }

    #[derive(Debug)]
    struct StubController;

    #[async_trait]
    impl Controller for StubController {
        fn id(&self) -> &'static str {
            "stub"
        }

        async fn execute(&self, _command: Command) {}
    }

    #[async_trait]
    impl Integration for StubIntegration {
        fn name(&self) -> &'static str {
            self.name
        }

        fn enabled(&self) -> bool {
            true
        }

        async fn discover(&self) -> Result<Discovery, IntegrationError> {
            unimplemented!()
        }

        async fn observe(&self, _tx: Sender<Event>) -> Result<(), IntegrationError> {
            unimplemented!()
        }

        fn controller(&self) -> Arc<dyn Controller> {
            Arc::new(StubController)
        }

        fn status(&self) -> IntegrationStatus {
            self.status
        }
    }

    fn with_integrations(mut state: ApiState, integrations: Vec<(&'static str, IntegrationStatus)>) -> ApiState {
        state.integrations = integrations
            .into_iter()
            .map(|(name, status)| Arc::new(StubIntegration { name, status }) as Arc<dyn Integration>)
            .collect();
        state
    }

    async fn call_health(state: ApiState) -> Response {
        let request = Request::builder().method("GET").uri("/health").body(Body::empty()).unwrap();

        router().with_state(state).oneshot(request).await.unwrap()
    }

    fn up() -> ComponentStatus {
        ComponentStatus { status: HealthStatus::Up }
    }

    fn down() -> ComponentStatus {
        ComponentStatus { status: HealthStatus::Down }
    }

    fn components(database: ComponentStatus, store: ComponentStatus) -> Components {
        Components { database, store, scheduler: up(), store_listener: up(), integrations: BTreeMap::new() }
    }

    async fn stop(handle: &JoinHandle<()>) {
        handle.abort();
        while !handle.is_finished() {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn health_returns_200_and_up_when_all_components_are_up() {
        let (Fixture { state, .. }, _database) = create_state_with_database(FlowRegistry::new(vec![]));

        let response = call_health(state).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            body_json(response).await,
            json!({
                "status": "UP",
                "components": {
                    "database": {"status": "UP"},
                    "store": {"status": "UP"},
                    "scheduler": {"status": "UP"},
                    "storeListener": {"status": "UP"},
                    "integrations": {},
                }
            })
        );
    }

    #[tokio::test]
    async fn health_lists_each_integration_under_its_name() {
        let (Fixture { state, .. }, _database) = create_state_with_database(FlowRegistry::new(vec![]));
        let state =
            with_integrations(state, vec![("Hue", IntegrationStatus::Up), ("Zigbee", IntegrationStatus::Degraded)]);

        let response = call_health(state).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            body_json(response).await["components"]["integrations"],
            json!({"Hue": {"status": "UP"}, "Zigbee": {"status": "DEGRADED"}})
        );
    }

    #[tokio::test]
    async fn health_stays_200_and_up_when_an_integration_is_down() {
        let (Fixture { state, .. }, _database) = create_state_with_database(FlowRegistry::new(vec![]));
        let state = with_integrations(state, vec![("Hue", IntegrationStatus::Down)]);

        let response = call_health(state).await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        assert_eq!(body["status"], "UP");
        assert_eq!(body["components"]["integrations"]["Hue"]["status"], "DOWN");
    }

    #[tokio::test]
    async fn health_returns_503_and_down_when_the_database_is_unreachable() {
        let (Fixture { state, .. }, database) = create_state_with_database(FlowRegistry::new(vec![]));
        // A panic while holding the connection poisons it, so every later ping fails
        let _ = database.call(|_| -> Result<(), DatabaseError> { panic!("closure panicked") }).await;

        let response = call_health(state).await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = body_json(response).await;
        assert_eq!(body["status"], "DOWN");
        assert_eq!(body["components"]["database"]["status"], "DOWN");
        assert_eq!(body["components"]["store"]["status"], "UP");
    }

    #[rstest]
    #[case::store("store")]
    #[case::scheduler("scheduler")]
    #[case::store_listener("storeListener")]
    #[tokio::test]
    async fn health_returns_503_and_down_when_a_core_task_has_stopped(#[case] component: &str) {
        let (Fixture { state, .. }, _database) = create_state_with_database(FlowRegistry::new(vec![]));
        let handle = match component {
            "store" => &state.core_tasks.store,
            "scheduler" => &state.core_tasks.scheduler,
            _ => &state.core_tasks.store_listener,
        };
        stop(handle).await;

        let response = call_health(state).await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = body_json(response).await;
        assert_eq!(body["status"], "DOWN");
        assert_eq!(body["components"][component]["status"], "DOWN");
        assert_eq!(body["components"]["database"]["status"], "UP");
    }

    #[tokio::test]
    async fn health_only_reports_the_stopped_task_as_down() {
        let (Fixture { state, .. }, _database) = create_state_with_database(FlowRegistry::new(vec![]));
        stop(&state.core_tasks.scheduler).await;

        let response = call_health(state).await;

        let body = body_json(response).await;
        let statuses: Vec<_> = ["database", "store", "scheduler", "storeListener"]
            .iter()
            .map(|name| body["components"][name]["status"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(statuses, vec!["UP", "UP", "DOWN", "UP"]);
    }

    #[tokio::test]
    async fn task_status_is_up_for_a_running_task_and_down_for_a_finished_one() {
        let running = tokio::spawn(std::future::pending());
        let finished = tokio::spawn(async {});
        while !finished.is_finished() {
            tokio::task::yield_now().await;
        }

        assert_eq!(task_status("running", &running).status, HealthStatus::Up);
        assert_eq!(task_status("finished", &finished).status, HealthStatus::Down);
        running.abort();
    }

    #[test]
    fn the_response_is_up_when_all_critical_components_are_up() {
        let response = HealthStatusResponse::new(components(up(), up()));

        assert_eq!(response.status, HealthStatus::Up);
    }

    #[rstest]
    #[case::database_down(down(), up())]
    #[case::store_down(up(), down())]
    #[case::all_down(down(), down())]
    fn the_response_is_down_when_a_critical_component_is_down(
        #[case] database: ComponentStatus,
        #[case] store: ComponentStatus,
    ) {
        let response = HealthStatusResponse::new(components(database, store));

        assert_eq!(response.status, HealthStatus::Down);
    }

    #[rstest]
    #[case::up(HealthStatus::Up, StatusCode::OK)]
    #[case::degraded(HealthStatus::Degraded, StatusCode::OK)]
    #[case::down(HealthStatus::Down, StatusCode::SERVICE_UNAVAILABLE)]
    fn http_status_follows_the_health_status(#[case] status: HealthStatus, #[case] expected: StatusCode) {
        assert_eq!(status.http_status(), expected);
    }

    #[rstest]
    #[case::up(IntegrationStatus::Up, HealthStatus::Up)]
    #[case::degraded(IntegrationStatus::Degraded, HealthStatus::Degraded)]
    #[case::down(IntegrationStatus::Down, HealthStatus::Down)]
    fn an_integration_status_maps_to_the_same_health_status(
        #[case] status: IntegrationStatus,
        #[case] expected: HealthStatus,
    ) {
        assert_eq!(HealthStatus::from(status), expected);
    }

    #[test]
    fn the_response_ignores_integrations_that_are_down() {
        let mut components = components(up(), up());
        components.integrations.insert("Hue", down());

        let response = HealthStatusResponse::new(components);

        assert_eq!(response.status, HealthStatus::Up);
    }
}
