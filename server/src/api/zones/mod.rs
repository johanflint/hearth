use crate::api::ApiState;
use crate::api::zones::create_zone::create_zone;
use crate::api::zones::delete_zone::delete_zone;
use crate::api::zones::update_zone::update_zone;
use axum::Router;
use axum::routing::get;

mod create_zone;
mod delete_zone;
mod list_zones;
mod retrieve_zone;
#[cfg(test)]
mod test_support;
mod update_zone;
mod zone_view;

use crate::api::zones::list_zones::list_zones;
use crate::api::zones::retrieve_zone::retrieve_zone;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/api/zones", get(list_zones).post(create_zone))
        .route("/api/zones/{id}", get(retrieve_zone).put(update_zone).delete(delete_zone))
}
