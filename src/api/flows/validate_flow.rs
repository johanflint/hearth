use crate::api::ApiState;
use crate::api::flows::commit::{flow_factory_error_response, invalid_json, parse_request};
use crate::domain::{FlowValidationError, Location, Problem, ValidationIssue};
use crate::flow_service::ValidateError;
use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tracing::info;

pub(super) async fn validate_flow(State(state): State<ApiState>, body: Bytes) -> Result<Response, Response> {
    let request: ValidateFlowRequest = parse_request(&body)?;
    state.flow_service.validate(request.flow).await.map_err(validate_error_response)?;
    info!("📥 Received request to validate flow... OK");

    Ok(StatusCode::OK.into_response())
}

#[derive(Debug, Deserialize)]
struct ValidateFlowRequest {
    // Kept as generic JSON so it can be parsed into a `SerializedFlow`
    flow: serde_json::Value,
}

fn validate_error_response(err: ValidateError) -> Response {
    match err {
        ValidateError::InvalidDocument(err) => invalid_json(err),
        ValidateError::InvalidFlow(err) => flow_factory_error_response(err),
        ValidateError::ValidationFailed(err) => validation_error_response(err),
    }
}

fn validation_error_response(err: FlowValidationError) -> Response {
    let response = ValidationErrorResponse {
        code: "validationFailed",
        message: err.to_string(),
        issues: err.issues().iter().map(IssueResponse::from).collect(),
    };

    (StatusCode::UNPROCESSABLE_ENTITY, Json(response)).into_response()
}

#[derive(Debug, Serialize)]
struct ValidationErrorResponse {
    pub code: &'static str,
    pub message: String,
    pub issues: Vec<IssueResponse>,
}

#[derive(Debug, Serialize)]
struct IssueResponse {
    location: LocationResponse,
    code: &'static str,
    message: String,
}

impl From<&ValidationIssue> for IssueResponse {
    fn from(issue: &ValidationIssue) -> Self {
        IssueResponse {
            location: (&issue.location).into(),
            code: problem_code(&issue.problem),
            message: issue.problem.to_string(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum LocationResponse {
    Trigger,
}

impl From<&Location> for LocationResponse {
    fn from(location: &Location) -> Self {
        match location {
            Location::Trigger => LocationResponse::Trigger,
        }
    }
}

fn problem_code(problem: &Problem) -> &'static str {
    match problem {
        Problem::UnknownDevice { .. } => "unknownDevice",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::flows::test_support::body_json;
    use serde_json::json;

    #[tokio::test]
    async fn validation_error_response_lists_all_issues() {
        let issues = vec![
            Problem::UnknownDevice { device_id: "lamp".to_string() }.at(Location::Trigger),
            Problem::UnknownDevice { device_id: "sensor".to_string() }.at(Location::Trigger),
        ];
        let err = FlowValidationError::from_issues(issues).unwrap_err();

        let response = validation_error_response(err);

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            body_json(response).await,
            json!({
                "code": "validationFailed",
                "message": "flow has 2 validation issues",
                "issues": [
                    { "location": { "type": "trigger" }, "code": "unknownDevice", "message": "unknown device 'lamp'" },
                    { "location": { "type": "trigger" }, "code": "unknownDevice", "message": "unknown device 'sensor'" }
                ]
            })
        );
    }
}
