use crate::api::ApiState;
use axum::Router;
use axum::routing::get;

mod list_zones;
mod retrieve_zone;
#[cfg(test)]
mod test_support;
mod zone_view;

use crate::api::zones::list_zones::list_zones;
use crate::api::zones::retrieve_zone::retrieve_zone;

pub fn router() -> Router<ApiState> {
    Router::new().route("/api/zones", get(list_zones)).route("/api/zones/{id}", get(retrieve_zone))
}
