use crate::domain::{ClaimMode, Problem};
use crate::flow_engine::action::Action;
use crate::flow_engine::action::device_command::evaluate_commands;
use crate::flow_engine::property_command::PropertyCommand;
use crate::flow_engine::scope::Scope;
use crate::flow_engine::{CommandMap, Context, Expression};
use crate::preset_registry::PresetRegistry;
use crate::store::StoreSnapshot;
use action_macros::register_action;
use async_trait::async_trait;
use serde::Deserialize;
use std::any::Any;
use std::collections::HashMap;
use tracing::{error, instrument, warn};

#[derive(Debug, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
#[register_action]
pub struct ControlGroupAction {
    group_id: String,
    claim: Option<ClaimMode>,
    property: HashMap<String, PropertyCommand>,
}

#[async_trait]
impl Action for ControlGroupAction {
    fn kind(&self) -> &'static str {
        "controlGroup"
    }

    #[instrument(fields(action = self.kind()), skip_all)]
    async fn execute(&self, context: &Context, scope: &mut Scope) {
        let snapshot = context.snapshot();
        let Some(group) = snapshot.groups.get(&self.group_id) else {
            warn!(
                group_id = self.group_id,
                "⚠️ Unable to control unknown group '{}', ignoring action: {:?}", self.group_id, self.property
            );
            return;
        };

        let Some(command_map) = scope.ensure_entry_mut::<CommandMap, _>("command_map".to_string(), HashMap::new) else {
            error!("🛑 Incorrect type for the command map");
            return;
        };

        let evaluated = evaluate_commands(&self.property, context);
        for device_id in &group.device_ids {
            let Some(device) = snapshot.devices.get(device_id) else {
                warn!(
                    group_id = self.group_id,
                    "⚠️ Group '{}' ({}) refers to unknown device '{}'", group.id, group.name, device_id
                );
                continue;
            };

            let device_command = command_map.entry(device_id.clone()).or_default();
            device_command.set_properties(device, &evaluated);
            device_command.set_claim(self.claim);
        }
    }

    fn validate(&self, _snapshot: &StoreSnapshot, _preset_registry: &PresetRegistry) -> Vec<Problem> {
        todo!()
    }

    fn expressions(&self) -> Vec<&Expression> {
        self.property.values().map(|property| &property.value).collect()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::property::{NumberProperty, PropertyType};
    use crate::domain::{Device, Group, GroupOwner, Number, Value};
    use crate::flow_engine::action::device_command::DeviceCommand;
    use crate::store::{DeviceMap, GroupMap};
    use crate::test_support::{DeviceBuilder, property_command, resolved_property_command};
    use pretty_assertions::assert_eq;
    use std::io;
    use std::sync::Arc;

    fn number(value: u64) -> Value {
        Value::Number(Number::PositiveInt(value))
    }

    fn lamp(id: &str, maximum_brightness: u64) -> Device {
        DeviceBuilder::new(id)
            .with_boolean_property("on", false)
            .with_properties(vec![Box::new(
                NumberProperty::builder("brightness".to_string(), PropertyType::Brightness, false)
                    .positive_int(Some(50), Some(1), Some(maximum_brightness))
                    .build(),
            )])
            .build()
    }

    // lamp1 and lamp2 have a different brightness range, lamp3 has no brightness at all
    fn snapshot(device_ids: &[&str]) -> StoreSnapshot {
        let lamp3 = DeviceBuilder::new("lamp3").with_boolean_property("on", false).build();
        let devices: DeviceMap = [lamp("lamp1", 100), lamp("lamp2", 50), lamp3]
            .into_iter()
            .map(|device| (device.id.clone(), Arc::new(device)))
            .collect();

        let group = Group {
            id: "room".to_string(),
            name: "Room".to_string(),
            device_ids: device_ids.iter().map(|id| id.to_string()).collect(),
            revision: 0,
            owner: GroupOwner::Integration,
        };
        let groups: GroupMap = HashMap::from([(group.id.clone(), Arc::new(group))]);

        StoreSnapshot { devices: Arc::new(devices), groups: Arc::new(groups) }
    }

    fn control_group(group_id: &str, claim: Option<ClaimMode>, properties: &[(&str, Value)]) -> ControlGroupAction {
        ControlGroupAction {
            group_id: group_id.to_string(),
            claim,
            property: properties.iter().map(|(id, value)| (id.to_string(), property_command(value.clone()))).collect(),
        }
    }

    async fn execute(action: &ControlGroupAction, snapshot: StoreSnapshot) -> CommandMap {
        let context = Context::builder().snapshot(snapshot).build();
        let mut scope = Scope::new();
        action.execute(&context, &mut scope).await;
        scope
            .take()
            .remove("command_map")
            .and_then(|v| v.downcast::<CommandMap>().ok())
            .map(|boxed| *boxed)
            .unwrap_or_default()
    }

    fn device_command(claim: Option<ClaimMode>, properties: &[(&str, Value)]) -> DeviceCommand {
        DeviceCommand {
            claim,
            properties: properties
                .iter()
                .map(|(id, value)| (id.to_string(), resolved_property_command(value.clone())))
                .collect(),
        }
    }

    #[test]
    fn deserialize_control_group_action() -> io::Result<()> {
        let json = r#"{
            "type": "controlGroup",
            "groupId": "room",
            "property": {
                "on": {
                    "value": {
                      "type": "literal",
                      "value": true
                    }
                }
            }
        }"#;

        let node = serde_json::from_str::<Box<dyn Action>>(json)?;

        let action = node.as_any().downcast_ref::<ControlGroupAction>().unwrap();
        assert_eq!(&control_group("room", None, &[("on", Value::Boolean(true))]), action);

        Ok(())
    }

    #[test]
    fn deserialize_control_group_action_with_claim() -> io::Result<()> {
        let json = r#"{
            "type": "controlGroup",
            "groupId": "room",
            "claim": "release",
            "property": {}
        }"#;

        let node = serde_json::from_str::<Box<dyn Action>>(json)?;

        let action = node.as_any().downcast_ref::<ControlGroupAction>().unwrap();
        assert_eq!(&control_group("room", Some(ClaimMode::Release), &[]), action);

        Ok(())
    }

    #[tokio::test]
    async fn execute_applies_the_properties_to_every_device_of_the_group() {
        let action = control_group("room", None, &[("on", Value::Boolean(true))]);

        let command_map = execute(&action, snapshot(&["lamp1", "lamp2", "lamp3"])).await;

        assert_eq!(
            command_map,
            HashMap::from([
                ("lamp1".to_string(), device_command(None, &[("on", Value::Boolean(true))])),
                ("lamp2".to_string(), device_command(None, &[("on", Value::Boolean(true))])),
                ("lamp3".to_string(), device_command(None, &[("on", Value::Boolean(true))])),
            ])
        );
    }

    #[tokio::test]
    async fn execute_sets_the_claim_on_every_device_of_the_group() {
        let action = control_group("room", Some(ClaimMode::Claim), &[]);

        let command_map = execute(&action, snapshot(&["lamp1", "lamp2"])).await;

        assert_eq!(
            command_map,
            HashMap::from([
                ("lamp1".to_string(), device_command(Some(ClaimMode::Claim), &[])),
                ("lamp2".to_string(), device_command(Some(ClaimMode::Claim), &[])),
            ])
        );
    }

    #[tokio::test]
    async fn execute_only_controls_the_devices_of_the_group() {
        let action = control_group("room", None, &[("on", Value::Boolean(true))]);

        let command_map = execute(&action, snapshot(&["lamp1"])).await;

        assert_eq!(command_map.keys().collect::<Vec<_>>(), vec!["lamp1"]);
    }

    #[tokio::test]
    async fn execute_clamps_the_value_per_device() {
        let action = control_group("room", None, &[("brightness", number(80))]);

        let command_map = execute(&action, snapshot(&["lamp1", "lamp2"])).await;

        assert_eq!(command_map["lamp1"], device_command(None, &[("brightness", number(80))]));
        assert_eq!(command_map["lamp2"], device_command(None, &[("brightness", number(50))]));
    }

    #[tokio::test]
    async fn execute_skips_a_property_only_for_the_devices_that_do_not_have_it() {
        let action = control_group("room", None, &[("on", Value::Boolean(true)), ("brightness", number(20))]);

        let command_map = execute(&action, snapshot(&["lamp1", "lamp3"])).await;

        assert_eq!(
            command_map["lamp1"],
            device_command(None, &[("on", Value::Boolean(true)), ("brightness", number(20))])
        );
        assert_eq!(command_map["lamp3"], device_command(None, &[("on", Value::Boolean(true))]));
    }

    #[tokio::test]
    async fn execute_skips_a_device_that_is_not_in_the_store() {
        let action = control_group("room", None, &[("on", Value::Boolean(true))]);

        let command_map = execute(&action, snapshot(&["lamp1", "ghost"])).await;

        assert_eq!(command_map.keys().collect::<Vec<_>>(), vec!["lamp1"]);
    }

    #[tokio::test]
    async fn execute_ignores_an_unknown_group() {
        let action = control_group("unknown", None, &[("on", Value::Boolean(true))]);

        let command_map = execute(&action, snapshot(&["lamp1", "lamp2"])).await;

        assert!(command_map.is_empty());
    }

    #[tokio::test]
    async fn execute_ignores_an_empty_group() {
        let action = control_group("room", Some(ClaimMode::Claim), &[("on", Value::Boolean(true))]);

        let command_map = execute(&action, snapshot(&[])).await;

        assert!(command_map.is_empty());
    }

    #[tokio::test]
    async fn execute_adds_to_the_commands_of_earlier_actions() {
        let context = Context::builder().snapshot(snapshot(&["lamp1", "lamp2"])).build();
        let mut scope = Scope::new();
        let on = control_group("room", None, &[("on", Value::Boolean(true))]);
        let dim = control_group("room", Some(ClaimMode::Claim), &[("brightness", number(20))]);

        on.execute(&context, &mut scope).await;
        dim.execute(&context, &mut scope).await;

        let command_map = *scope.take().remove("command_map").unwrap().downcast::<CommandMap>().unwrap();
        let expected =
            device_command(Some(ClaimMode::Claim), &[("on", Value::Boolean(true)), ("brightness", number(20))]);
        assert_eq!(command_map["lamp1"], expected);
    }
}
