mod update_flow;

use crate::api::ApiState;
use crate::api::flows::update_flow::update_flow;
use axum::Router;
use axum::routing::put;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/api/flows/{id}", put(update_flow))
}
