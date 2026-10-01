mod list_flows;
mod retrieve_flow;
#[cfg(test)]
mod test_support;
mod update_flow;

use crate::api::ApiState;
use crate::api::flows::list_flows::list_flows;
use crate::api::flows::retrieve_flow::retrieve_flow;
use crate::api::flows::update_flow::update_flow;
use axum::Router;
use axum::routing::get;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/api/flows", get(list_flows))
        .route("/api/flows/{id}", get(retrieve_flow).put(update_flow))
}
