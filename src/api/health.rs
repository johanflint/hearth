use crate::api::ApiState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use tracing::warn;

pub fn router() -> Router<ApiState> {
    Router::new().route("/health", get(health))
}

async fn health(State(state): State<ApiState>) -> impl IntoResponse {
    let components = Components { database: ComponentStatus { status: database_status(&state).await } };
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

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
enum HealthStatus {
    Up,
    Down,
}

impl HealthStatus {
    fn http_status(&self) -> StatusCode {
        match self {
            HealthStatus::Up => StatusCode::OK,
            HealthStatus::Down => StatusCode::SERVICE_UNAVAILABLE,
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
        let status = components.database.status;
        Self { status, components }
    }
}

#[derive(Debug, Serialize)]
struct Components {
    database: ComponentStatus,
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
    use crate::flow_registry::FlowRegistry;
    use axum::body::Body;
    use axum::http::Request;
    use axum::response::Response;
    use serde_json::json;
    use tower::ServiceExt;

    async fn call_health(state: ApiState) -> Response {
        let request = Request::builder().method("GET").uri("/health").body(Body::empty()).unwrap();

        router().with_state(state).oneshot(request).await.unwrap()
    }

    #[tokio::test]
    async fn health_returns_200_and_up_when_the_database_is_reachable() {
        let (Fixture { state, .. }, _database) = create_state_with_database(FlowRegistry::new(vec![]));

        let response = call_health(state).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body_json(response).await, json!({"status": "UP", "components": {"database": {"status": "UP"}}}));
    }

    #[tokio::test]
    async fn health_returns_503_and_down_when_the_database_is_unreachable() {
        let (Fixture { state, .. }, database) = create_state_with_database(FlowRegistry::new(vec![]));
        // A panic while holding the connection poisons it, so every later ping fails
        let _ = database.call(|_| -> Result<(), DatabaseError> { panic!("closure panicked") }).await;

        let response = call_health(state).await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            body_json(response).await,
            json!({"status": "DOWN", "components": {"database": {"status": "DOWN"}}})
        );
    }
}
