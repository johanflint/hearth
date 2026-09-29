use crate::api::ApiState;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/metrics", get(metrics_handler))
}

async fn metrics_handler(State(state): State<ApiState>) -> (StatusCode, String) {
    (StatusCode::OK, state.prometheus_handle.render())
}
