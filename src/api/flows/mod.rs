mod commit;
mod create_flow;
mod delete_flow;
mod list_flows;
mod retrieve_flow;
#[cfg(test)]
mod test_support;
mod update_flow;
mod validate_flow;

use crate::api::ApiState;
use crate::api::flows::create_flow::create_flow;
use crate::api::flows::delete_flow::delete_flow;
use crate::api::flows::list_flows::list_flows;
use crate::api::flows::retrieve_flow::retrieve_flow;
use crate::api::flows::update_flow::update_flow;
use crate::api::flows::validate_flow::validate_flow;
use axum::Router;
use axum::routing::{get, post};

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/api/flows", get(list_flows).post(create_flow))
        .route("/api/flows/{id}", get(retrieve_flow).put(update_flow).delete(delete_flow))
        .route("/api/flows/validate", post(validate_flow))
}
