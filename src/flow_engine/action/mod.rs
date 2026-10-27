use crate::domain::Problem;
use crate::flow_engine::Expression;
use crate::flow_engine::action_registry::{ACTION_REGISTRY, known_actions};
use crate::flow_engine::context::Context;
use crate::flow_engine::scope::Scope;
use crate::preset_registry::PresetRegistry;
use crate::store::StoreSnapshot;
use async_trait::async_trait;
use serde::{Deserialize, Deserializer};
use std::any::Any;
use std::fmt::Debug;

mod control_device_action;
mod control_group_action;
mod device_command;
mod log_action;

pub use device_command::CommandMap;

#[cfg(test)]
pub use {control_device_action::ControlDeviceAction, device_command::DeviceCommand, log_action::LogAction};

#[async_trait]
pub trait Action: Debug + Send + Sync {
    fn kind(&self) -> &'static str;

    async fn execute(&self, context: &Context, scope: &mut Scope);

    fn validate(&self, _snapshot: &StoreSnapshot, _preset_registry: &PresetRegistry) -> Vec<Problem> {
        vec![]
    }

    fn expressions(&self) -> Vec<&Expression>;

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
            action(&value).map_err(serde::de::Error::custom)
        } else {
            Err(serde::de::Error::custom(format!(
                "unknown action type '{}', known types: {}",
                kind,
                known_actions().join(", ")
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

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
