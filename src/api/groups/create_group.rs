use crate::api::ApiState;
use crate::api::error::ErrorResponse;
use crate::api::groups::group_view::GroupView;
use crate::api::parse_request::parse_request;
use crate::group_service::CreateGroupError;
use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use tracing::{error, info};

pub(super) async fn create_group(State(state): State<ApiState>, body: Bytes) -> Result<Response, Response> {
    let request: CreateGroupRequest = parse_request(&body)?;
    let stored_group = state
        .group_service
        .create_group(&request.id, &request.name, request.device_ids)
        .await
        .map_err(create_error_response)?;
    info!("📥 Received request to create group '{}'... OK, revision {}", stored_group.id, stored_group.revision);

    let location = format!("/api/groups/{}", stored_group.id);
    Ok((StatusCode::CREATED, [(header::LOCATION, location)], Json(GroupView::from(stored_group))).into_response())
}

fn create_error_response(err: CreateGroupError) -> Response {
    match err {
        CreateGroupError::IdAlreadyExists => {
            (StatusCode::CONFLICT, Json(ErrorResponse::new_code_only("groupIdAlreadyExists"))).into_response()
        }
        CreateGroupError::NameAlreadyExists => {
            (StatusCode::CONFLICT, Json(ErrorResponse::new_code_only("groupNameAlreadyExists"))).into_response()
        }
        CreateGroupError::UnknownDevices { .. } => {
            (StatusCode::UNPROCESSABLE_ENTITY, Json(ErrorResponse::new("unknownDevices", err.to_string())))
                .into_response()
        }
        CreateGroupError::Internal(_) => {
            error!("❌ Failed to persist group to store: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse::new_code_only("storageError"))).into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateGroupRequest {
    id: String,
    name: String,
    device_ids: Vec<String>,
}
