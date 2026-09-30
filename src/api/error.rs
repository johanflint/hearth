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
