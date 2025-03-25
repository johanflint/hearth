use crate::flow_engine::action_registry::{ACTION_REGISTRY, known_actions, register_action};
use crate::flow_engine::property_value::PropertyValue;
use action_macros::register_action;
use async_trait::async_trait;
use serde::{Deserialize, Deserializer};
use std::any::Any;
use std::collections::HashMap;
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
                known_actions().join(", ")
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

    #[instrument(fields(action = self.kind()), skip(self))]
    async fn execute(&self) {
        info!("{}", self.message);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
#[register_action]
pub struct ControlDeviceAction {
    device_id: String,
    property: HashMap<String, PropertyValue>,
}

#[cfg(test)]
impl ControlDeviceAction {
    pub fn new(device_id: String, property: HashMap<String, PropertyValue>) -> ControlDeviceAction {
        ControlDeviceAction { device_id, property }
    }
}

#[async_trait]
impl Action for ControlDeviceAction {
    fn kind(&self) -> &'static str {
        "controlDevice"
    }

    #[instrument(fields(action = self.kind()), skip(self))]
    async fn execute(&self) {
        info!(device_id = self.device_id, "Control device");
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow_engine::property_value::PropertyValue::SetBooleanValue;
    use pretty_assertions::assert_eq;
    use std::io;

    #[test]
    fn deserialize_log_action() -> io::Result<()> {
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

    #[test]
    fn deserialize_control_device_action() -> io::Result<()> {
        let json = r#"{
            "type": "controlDevice",
            "deviceId": "42",
            "property": {
                "fan": {
                    "type": "boolean",
                    "value": true
                }
            }
        }"#;

        let node = serde_json::from_str::<Box<dyn Action>>(json)?;

        let expected = ControlDeviceAction {
            device_id: "42".to_string(),
            property: HashMap::from([("fan".to_string(), SetBooleanValue(true))]),
        };

        let action = node.as_any().downcast_ref::<ControlDeviceAction>().unwrap();
        assert_eq!(&expected, action);

        Ok(())
    }

    #[test]
    fn deserialize_returns_error_if_type_is_missing() {
        let json = "{}";

        let node = serde_json::from_str::<Box<dyn Action>>(json);
        assert!(node.is_err());
        assert_eq!(node.unwrap_err().to_string(), "missing field 'type'");
    }

    #[test]
    fn deserialize_returns_error_for_invalid_type() {
        let json = r#"{
            "type": "UnknownAction"
        }"#;

        let node = serde_json::from_str::<Box<dyn Action>>(json);
        assert!(node.is_err());
        assert!(node.unwrap_err().to_string().starts_with("unknown action type 'UnknownAction', known types:"));
    }
}
