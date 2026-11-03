mod create_group;
mod delete_group;
mod group_view;
mod list_groups;
mod retrieve_group;
#[cfg(test)]
mod test_support;
mod update_group;

use crate::api::ApiState;
use crate::api::groups::create_group::create_group;
use crate::api::groups::delete_group::delete_group;
use crate::api::groups::list_groups::list_groups;
use crate::api::groups::retrieve_group::retrieve_group;
use crate::api::groups::update_group::update_group;
use axum::Router;
use axum::routing::get;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/api/groups", get(list_groups).post(create_group))
        .route("/api/groups/{id}", get(retrieve_group).put(update_group).delete(delete_group))
}
