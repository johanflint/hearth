use crate::flow_engine::action_registry::{known_actions, register_action, ACTION_REGISTRY};
use action_macros::register_action;
use async_trait::async_trait;
use serde::{Deserialize, Deserializer};
use std::any::Any;
use std::fmt::Debug;
use tracing::{info, instrument};

#[async_trait]
pub trait Action: Debug + Send + Sync {
    fn kind(&self) -> &'static str;

    async fn execute(&self);

    fn as_any(&self) -> &dyn Any;
}

impl<'de> Deserialize<'de> for Box<dyn Action> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value: serde_json::Value = Deserialize::deserialize(deserializer)?;
        let kind = value
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| serde::de::Error::custom("missing field 'type'"))?;

        let registry = ACTION_REGISTRY.read().unwrap();
        if let Some(action) = registry.get(kind) {
            Ok(action(&value))
        } else {
            Err(serde::de::Error::custom(format!(
                "unknown action type '{}', known types: {}",
                kind,
                known_actions().join(",")
            )))
        }
    }
}

#[derive(Debug, Deserialize, Default, PartialEq)]
#[register_action]
pub struct LogAction {
    message: String,
}

#[cfg(test)]
impl LogAction {
    pub fn new(message: String) -> LogAction {
        LogAction { message }
    }
}

#[async_trait]
impl Action for LogAction {
    fn kind(&self) -> &'static str {
        "log"
    }

    #[instrument(skip(self))]
    async fn execute(&self) {
        info!("{}", self.message);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[tokio::test]
    async fn deserialize_log_action() -> io::Result<()> {
        let json = r#"{
            "type": "log",
            "message": "Hello"
        }"#;

        let node = serde_json::from_str::<Box<dyn Action>>(json)?;

        let expected = LogAction {
            message: "Hello".to_string(),
        };

        let action = node.as_any().downcast_ref::<LogAction>().unwrap();
        assert_eq!(&expected, action);

        Ok(())
    }

    #[tokio::test]
    async fn deserialize_returns_error_if_type_is_missing() {
        let json = "{}";

        let node = serde_json::from_str::<Box<dyn Action>>(json);
        assert!(node.is_err());
        assert_eq!(node.unwrap_err().to_string(), "missing field 'type'");
    }

    #[tokio::test]
    async fn deserialize_returns_error_for_invalid_type() {
        let json = r#"{
            "type": "UnknownAction"
        }"#;

        let node = serde_json::from_str::<Box<dyn Action>>(json);
        assert!(node.is_err());
        assert!(node
            .unwrap_err()
            .to_string()
            .starts_with("unknown action type 'UnknownAction', known types:"));
    }
}
