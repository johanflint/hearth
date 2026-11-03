use crate::flow_registry::FlowSummary;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl ErrorResponse {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        ErrorResponse { code: code.to_string(), message: Some(message.into()) }
    }

    pub fn new_code_only(code: &'static str) -> Self {
        ErrorResponse { code: code.to_string(), message: None }
    }
}

#[derive(Debug, Serialize)]
pub struct InUseErrorResponse {
    code: &'static str,
    message: String,
    flows: Vec<FlowReference>,
}

impl InUseErrorResponse {
    pub fn new(code: &'static str, message: impl Into<String>, flows: Vec<FlowSummary>) -> Self {
        let flows = flows
            .into_iter()
            .map(|summary| FlowReference { id: summary.id, name: summary.name, revision: summary.revision })
            .collect();
        InUseErrorResponse { code, message: message.into(), flows }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowReference {
    id: String,
    name: String,
    revision: u64,
}
