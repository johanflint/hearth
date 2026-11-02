mod group_view;
mod list_groups;
mod retrieve_group;
#[cfg(test)]
mod test_support;

use crate::api::ApiState;
use crate::api::groups::list_groups::list_groups;
use crate::api::groups::retrieve_group::retrieve_group;
use axum::Router;
use axum::routing::get;

pub fn router() -> Router<ApiState> {
    Router::new().route("/api/groups", get(list_groups)).route("/api/groups/{id}", get(retrieve_group))
}
