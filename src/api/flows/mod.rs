mod update_flow;
mod list_flows;
#[cfg(test)]
mod test_support;

use crate::api::ApiState;
use crate::api::flows::list_flows::list_flows;
use crate::api::flows::update_flow::update_flow;
use axum::Router;
use axum::routing::{get, put};

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/api/flows", get(list_flows))
        .route("/api/flows/{id}", put(update_flow))
}
