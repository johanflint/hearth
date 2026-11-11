use crate::mqtt::{MqttMessage, MqttPublishError, MqttService, MqttSubscribeError};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;
use thiserror::Error;
use tokio::sync::{Mutex, mpsc};
use uuid::Uuid;

const CONTROL_TOPIC: &str = "$CONTROL/dynamic-security/v1";
const RESPONSE_TOPIC: &str = "$CONTROL/dynamic-security/v1/response";
const TIMEOUT: Duration = Duration::from_secs(10);

// Manages accounts through Mosquitto's dynamic security plugin
#[derive(Debug)]
pub struct MqttSecurity {
    mqtt: MqttService,
    responses: Mutex<mpsc::Receiver<MqttMessage>>,
}

impl MqttSecurity {
    pub fn new(mqtt: MqttService) -> Result<Self, MqttSubscribeError> {
        let responses = mqtt.subscribe(&[RESPONSE_TOPIC])?;
        Ok(MqttSecurity { mqtt, responses: Mutex::new(responses) })
    }

    pub async fn ensure_account(
        &self,
        role: &str,
        client_id: &str,
        username: &str,
        password: &str,
    ) -> Result<(), SecurityError> {
        let responses = self.send(account_commands(role, client_id, username, password)).await?;
        match responses.into_iter().find(|response| !response.is_ok()) {
            None => Ok(()),
            Some(Response { command, error, .. }) => {
                Err(SecurityError::Rejected { command, error: error.unwrap_or_default() })
            }
        }
    }

    async fn send(&self, commands: Vec<Value>) -> Result<Vec<Response>, SecurityError> {
        let mut responses = self.responses.lock().await;
        let correlation = Uuid::now_v7().to_string();
        let commands: Vec<Value> = commands
            .into_iter()
            .map(|mut command| {
                command["correlationData"] = json!(correlation);
                command
            })
            .collect();

        let payload = serde_json::to_vec(&json!({"commands": commands }))?;
        let round_trip = async {
            self.mqtt.publish(CONTROL_TOPIC, payload).await?;
            while let Some(message) = responses.recv().await {
                if let Some(responses) = responses_for(&message.payload, &correlation) {
                    return Ok(responses);
                }
            }
            Err(SecurityError::Closed)
        };
        tokio::time::timeout(TIMEOUT, round_trip).await.map_err(|_| SecurityError::Timeout)?
    }
}

fn account_commands(role: &str, client_id: &str, username: &str, password: &str) -> Vec<Value> {
    let topics = format!("{client_id}/#");
    vec![
        json!({"command": "createRole", "rolename": role, "acls": [
            {"acltype": "publishClientSend", "topic": topics, "allow": true},
            {"acltype": "subscribePattern", "topic": topics, "allow": true},
        ]}),
        json!({"command": "createClient", "username": username, "password": password,
               "clientid": client_id, "roles": [{"rolename": role}]}),
        json!({"command": "setClientPassword", "username": username, "password": password}),
    ]
}

// The broker answers a batch with one response per command, all carrying the correlation data of their command
fn responses_for(payload: &[u8], correlation: &str) -> Option<Vec<Response>> {
    let batch = serde_json::from_slice::<Responses>(payload).ok()?;
    let is_ours = batch.responses.first().is_some_and(|r| r.correlation_data.as_deref() == Some(correlation));
    is_ours.then_some(batch.responses)
}

#[derive(Deserialize)]
struct Responses {
    responses: Vec<Response>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Response {
    command: String,
    error: Option<String>,
    correlation_data: Option<String>,
}

impl Response {
    // Creating fails for an existing role or client, setClientPassword then updates the existing client
    fn is_ok(&self) -> bool {
        matches!(
            (self.command.as_str(), self.error.as_deref()),
            (_, None) | ("createRole", Some("Role already exists")) | ("createClient", Some("Client already exists"))
        )
    }
}

#[derive(Debug, Error)]
pub enum SecurityError {
    #[error(transparent)]
    Publish(#[from] MqttPublishError),
    #[error(transparent)]
    Serialize(#[from] serde_json::Error),
    #[error("response channel closed")]
    Closed,
    #[error("no response from the broker within {TIMEOUT:?}")]
    Timeout,
    #[error("broker rejected '{command}': {error}")]
    Rejected { command: String, error: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    fn response(command: &str, error: Option<&str>) -> Response {
        Response { command: command.to_string(), error: error.map(str::to_string), correlation_data: None }
    }

    #[rstest]
    #[case::create_role("createRole", None, true)]
    #[case::existing_role("createRole", Some("Role already exists"), true)]
    #[case::existing_client("createClient", Some("Client already exists"), true)]
    #[case::other_role_error("createRole", Some("Invalid ACL"), false)]
    #[case::other_client_error("createClient", Some("Role not found"), false)]
    #[case::password_error("setClientPassword", Some("Client not found"), false)]
    #[case::error_of_another_command("setClientPassword", Some("Client already exists"), false)]
    fn response_is_ok(#[case] command: &str, #[case] error: Option<&str>, #[case] expected: bool) {
        assert_eq!(response(command, error).is_ok(), expected);
    }

    #[test]
    fn account_commands_create_a_role_and_a_client_with_that_role_and_set_its_password() {
        let commands = account_commands("shelly-dev1", "dev1", "user1", "secret");

        let expected = vec![
            json!({"command": "createRole", "rolename": "shelly-dev1", "acls": [
                {"acltype": "publishClientSend", "topic": "dev1/#", "allow": true},
                {"acltype": "subscribePattern", "topic": "dev1/#", "allow": true},
            ]}),
            json!({"command": "createClient", "username": "user1", "password": "secret",
                   "clientid": "dev1", "roles": [{"rolename": "shelly-dev1"}]}),
            json!({"command": "setClientPassword", "username": "user1", "password": "secret"}),
        ];
        assert_eq!(commands, expected);
    }

    #[test]
    fn responses_for_returns_the_responses_of_the_batch_with_the_correlation() {
        let payload = br#"{"responses":[
            {"command":"createRole","correlationData":"abc"},
            {"command":"createClient","error":"Client already exists","correlationData":"abc"}]}"#;

        let responses = responses_for(payload, "abc").expect("batch should match");

        let commands: Vec<_> = responses.iter().map(|r| r.command.as_str()).collect();
        assert_eq!(commands, vec!["createRole", "createClient"]);
        assert_eq!(responses[1].error.as_deref(), Some("Client already exists"));
    }

    #[rstest]
    #[case::other_correlation(br#"{"responses":[{"command":"createRole","correlationData":"other"}]}"#.as_slice())]
    #[case::no_correlation(br#"{"responses":[{"command":"createRole"}]}"#.as_slice())]
    #[case::no_responses(br#"{"responses":[]}"#.as_slice())]
    #[case::not_a_response(br#"{"commands":[]}"#.as_slice())]
    #[case::not_json(b"garbage".as_slice())]
    fn responses_for_ignores_other_payloads(#[case] payload: &[u8]) {
        assert!(responses_for(payload, "abc").is_none());
    }

    #[test]
    fn check_accepts_responses_that_are_ok_or_exist_already() {
        let responses = vec![
            response("createRole", Some("Role already exists")),
            response("createClient", None),
            response("setClientPassword", None),
        ];

        assert!(
            (match responses.into_iter().find(|response| !response.is_ok()) {
                None => Ok(()),
                Some(Response { command, error, .. }) => {
                    Err(SecurityError::Rejected { command, error: error.unwrap_or_default() })
                }
            })
            .is_ok()
        );
    }

    #[test]
    fn check_rejects_with_the_first_failing_command() {
        let responses = vec![
            response("createRole", None),
            response("createClient", Some("Role not found")),
            response("setClientPassword", Some("Client not found")),
        ];

        let result = match responses.into_iter().find(|response| !response.is_ok()) {
            None => Ok(()),
            Some(Response { command, error, .. }) => {
                Err(SecurityError::Rejected { command, error: error.unwrap_or_default() })
            }
        };

        assert!(
            matches!(&result, Err(SecurityError::Rejected { command, error })
                if command == "createClient" && error == "Role not found"),
            "got {result:?}"
        );
    }
}
