use crate::domain::{ClaimMode, Problem, find_device, find_group, find_zone, validate_property_command};
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
pub struct ControlZoneAction {
    zone_id: String,
    claim: Option<ClaimMode>,
    property: HashMap<String, PropertyCommand>,
}

#[async_trait]
impl Action for ControlZoneAction {
    fn kind(&self) -> &'static str {
        "controlZone"
    }

    #[instrument(fields(action = self.kind()), skip_all)]
    async fn execute(&self, context: &Context, scope: &mut Scope) {
        let snapshot = context.snapshot();
        let Some(zone) = snapshot.zones.get(&self.zone_id) else {
            warn!(
                zone_id = self.zone_id,
                "⚠️ Unable to control unknown zone '{}', ignoring action: {:?}", self.zone_id, self.property
            );
            return;
        };

        let Some(command_map) = scope.ensure_entry_mut::<CommandMap, _>("command_map".to_string(), HashMap::new) else {
            error!("🛑 Incorrect type for the command map");
            return;
        };

        let evaluated = evaluate_commands(&self.property, context);
        let mut device_ids = zone.device_ids.clone();
        for group_id in &zone.group_ids {
            let Some(group) = snapshot.groups.get(group_id) else {
                warn!(group_id, "⚠️ Zone '{}' ({}) refers to unknown group '{}'", zone.id, zone.name, group_id);
                continue;
            };
            device_ids.extend(group.device_ids.clone());
        }

        for device_id in &device_ids {
            let Some(device) = snapshot.devices.get(device_id) else {
                warn!(
                    zone_id = self.zone_id,
                    "⚠️ Zone '{}' ({}) refers to unknown device '{}'", zone.id, zone.name, device_id
                );
                continue;
            };

            let device_command = command_map.entry(device_id.clone()).or_default();
            device_command.set_properties(device, &evaluated);
            device_command.set_claim(self.claim);
            device_command.set_zone_id(self.zone_id.clone());
        }
    }

    fn validate(&self, snapshot: &StoreSnapshot, preset_registry: &PresetRegistry) -> Vec<Problem> {
        let zone = match find_zone(&self.zone_id, snapshot) {
            Ok(zone) => zone,
            Err(problem) => return vec![problem],
        };

        let mut problems = vec![];
        let mut device_ids = zone.device_ids.clone();
        for group_id in &zone.group_ids {
            let group = match find_group(group_id, snapshot) {
                Ok(group) => group,
                Err(problem) => {
                    problems.push(problem);
                    continue;
                }
            };

            device_ids.extend(group.device_ids.clone());
        }

        for device_id in &device_ids {
            let device = match find_device(device_id, snapshot) {
                Ok(device) => device,
                Err(problem) => {
                    problems.push(problem);
                    continue;
                }
            };

            for (property_id, command) in &self.property {
                if let Err(problem) = validate_property_command(snapshot, preset_registry, device, property_id, command)
                {
                    // The range differs per device, the value is clamped for each device at runtime
                    let is_range_problem =
                        matches!(problem, Problem::ValueTooSmall { .. } | Problem::ValueTooLarge { .. });

                    // Problems that are not tied to the device, like an unknown preset, would repeat for every device
                    if !is_range_problem && !problems.contains(&problem) {
                        problems.push(problem);
                    }
                }
            }
        }
        problems
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
    use crate::domain::property::{NumberProperty, PropertyType, ValueKind};
    use crate::domain::{Device, Group, GroupOwner, Number, Value, Zone};
    use crate::flow_engine::action::device_command::DeviceCommand;
    use crate::store::{DeviceMap, GroupMap, ZoneMap};
    use crate::test_support::{DeviceBuilder, property_command, resolved_property_command};
    use pretty_assertions::assert_eq;
    use rstest::rstest;
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

    // lamp1 and lamp2 have a different brightness range, lamp3 has no brightness at all.
    // The group "room" contains lamp2 and lamp3.
    fn snapshot(zone_device_ids: &[&str], zone_group_ids: &[&str]) -> StoreSnapshot {
        let lamp3 = DeviceBuilder::new("lamp3").with_boolean_property("on", false).build();
        let devices: DeviceMap = [lamp("lamp1", 100), lamp("lamp2", 50), lamp3]
            .into_iter()
            .map(|device| (device.id.clone(), Arc::new(device)))
            .collect();

        let room = Group {
            id: "room".to_string(),
            name: "Room".to_string(),
            device_ids: ["lamp2", "lamp3"].iter().map(|id| id.to_string()).collect(),
            revision: 0,
            owner: GroupOwner::Integration,
            external_id: None,
        };
        let groups: GroupMap = HashMap::from([(room.id.clone(), Arc::new(room))]);

        let zone = Zone {
            id: "zone".to_string(),
            name: "Zone".to_string(),
            group_ids: zone_group_ids.iter().map(|id| id.to_string()).collect(),
            device_ids: zone_device_ids.iter().map(|id| id.to_string()).collect(),
            revision: 0,
            owner: GroupOwner::Hearth,
            external_id: None,
        };
        let zones: ZoneMap = HashMap::from([(zone.id.clone(), Arc::new(zone))]);

        StoreSnapshot { devices: Arc::new(devices), groups: Arc::new(groups), zones: Arc::new(zones) }
    }

    fn control_zone(zone_id: &str, claim: Option<ClaimMode>, properties: &[(&str, Value)]) -> ControlZoneAction {
        ControlZoneAction {
            zone_id: zone_id.to_string(),
            claim,
            property: properties.iter().map(|(id, value)| (id.to_string(), property_command(value.clone()))).collect(),
        }
    }

    async fn execute(action: &ControlZoneAction, snapshot: StoreSnapshot) -> CommandMap {
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
            zone_id: Some("zone".to_string()),
            claim,
            properties: properties
                .iter()
                .map(|(id, value)| (id.to_string(), resolved_property_command(value.clone())))
                .collect(),
            ..Default::default()
        }
    }

    fn sorted_keys(command_map: &CommandMap) -> Vec<&str> {
        let mut keys: Vec<&str> = command_map.keys().map(String::as_str).collect();
        keys.sort();
        keys
    }

    #[test]
    fn deserialize_control_zone_action() -> io::Result<()> {
        let json = r#"{
            "type": "controlZone",
            "zoneId": "zone",
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

        let action = node.as_any().downcast_ref::<ControlZoneAction>().unwrap();
        assert_eq!(&control_zone("zone", None, &[("on", Value::Boolean(true))]), action);

        Ok(())
    }

    #[test]
    fn deserialize_control_zone_action_with_claim() -> io::Result<()> {
        let json = r#"{
            "type": "controlZone",
            "zoneId": "zone",
            "claim": "release",
            "property": {}
        }"#;

        let node = serde_json::from_str::<Box<dyn Action>>(json)?;

        let action = node.as_any().downcast_ref::<ControlZoneAction>().unwrap();
        assert_eq!(&control_zone("zone", Some(ClaimMode::Release), &[]), action);

        Ok(())
    }

    #[tokio::test]
    async fn execute_applies_the_properties_to_every_device_of_the_zone() {
        let action = control_zone("zone", None, &[("on", Value::Boolean(true))]);

        let command_map = execute(&action, snapshot(&["lamp1", "lamp2", "lamp3"], &[])).await;

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
    async fn execute_sets_the_claim_on_every_device_of_the_zone() {
        let action = control_zone("zone", Some(ClaimMode::Claim), &[]);

        let command_map = execute(&action, snapshot(&["lamp1", "lamp2"], &[])).await;

        assert_eq!(
            command_map,
            HashMap::from([
                ("lamp1".to_string(), device_command(Some(ClaimMode::Claim), &[])),
                ("lamp2".to_string(), device_command(Some(ClaimMode::Claim), &[])),
            ])
        );
    }

    #[tokio::test]
    async fn execute_only_controls_the_devices_of_the_zone() {
        let action = control_zone("zone", None, &[("on", Value::Boolean(true))]);

        let command_map = execute(&action, snapshot(&["lamp1"], &[])).await;

        assert_eq!(sorted_keys(&command_map), ["lamp1"]);
    }

    #[tokio::test]
    async fn execute_also_controls_the_devices_of_the_groups_of_the_zone() {
        let action = control_zone("zone", None, &[("on", Value::Boolean(true))]);

        let command_map = execute(&action, snapshot(&["lamp1"], &["room"])).await;

        assert_eq!(sorted_keys(&command_map), ["lamp1", "lamp2", "lamp3"]);
    }

    #[tokio::test]
    async fn execute_controls_a_zone_that_only_contains_groups() {
        let action = control_zone("zone", None, &[("on", Value::Boolean(true))]);

        let command_map = execute(&action, snapshot(&[], &["room"])).await;

        assert_eq!(sorted_keys(&command_map), ["lamp2", "lamp3"]);
    }

    #[tokio::test]
    async fn execute_controls_a_device_in_both_the_zone_and_its_group_once() {
        let action = control_zone("zone", None, &[("brightness", number(20))]);

        let command_map = execute(&action, snapshot(&["lamp2"], &["room"])).await;

        assert_eq!(sorted_keys(&command_map), ["lamp2", "lamp3"]);
        assert_eq!(command_map["lamp2"], device_command(None, &[("brightness", number(20))]));
    }

    #[tokio::test]
    async fn execute_skips_an_unknown_group_but_controls_the_other_members() {
        let action = control_zone("zone", None, &[("on", Value::Boolean(true))]);

        let command_map = execute(&action, snapshot(&["lamp1"], &["ghost", "room"])).await;

        assert_eq!(sorted_keys(&command_map), ["lamp1", "lamp2", "lamp3"]);
    }

    #[tokio::test]
    async fn execute_sets_the_zone_id_but_not_the_group_id() {
        let action = control_zone("zone", None, &[("on", Value::Boolean(true))]);

        let command_map = execute(&action, snapshot(&["lamp1"], &["room"])).await;

        for command in command_map.values() {
            assert_eq!(command.zone_id.as_deref(), Some("zone"));
            assert_eq!(command.group_id, None);
        }
    }

    #[tokio::test]
    async fn execute_clamps_the_value_per_device() {
        let action = control_zone("zone", None, &[("brightness", number(80))]);

        let command_map = execute(&action, snapshot(&["lamp1", "lamp2"], &[])).await;

        assert_eq!(command_map["lamp1"], device_command(None, &[("brightness", number(80))]));
        assert_eq!(command_map["lamp2"], device_command(None, &[("brightness", number(50))]));
    }

    #[tokio::test]
    async fn execute_skips_a_property_only_for_the_devices_that_do_not_have_it() {
        let action = control_zone("zone", None, &[("on", Value::Boolean(true)), ("brightness", number(20))]);

        let command_map = execute(&action, snapshot(&["lamp1", "lamp3"], &[])).await;

        assert_eq!(
            command_map["lamp1"],
            device_command(None, &[("on", Value::Boolean(true)), ("brightness", number(20))])
        );
        assert_eq!(command_map["lamp3"], device_command(None, &[("on", Value::Boolean(true))]));
    }

    #[tokio::test]
    async fn execute_skips_a_device_that_is_not_in_the_store() {
        let action = control_zone("zone", None, &[("on", Value::Boolean(true))]);

        let command_map = execute(&action, snapshot(&["lamp1", "ghost"], &[])).await;

        assert_eq!(sorted_keys(&command_map), ["lamp1"]);
    }

    #[tokio::test]
    async fn execute_ignores_an_unknown_zone() {
        let action = control_zone("unknown", None, &[("on", Value::Boolean(true))]);

        let command_map = execute(&action, snapshot(&["lamp1", "lamp2"], &[])).await;

        assert!(command_map.is_empty());
    }

    #[tokio::test]
    async fn execute_ignores_an_empty_zone() {
        let action = control_zone("zone", Some(ClaimMode::Claim), &[("on", Value::Boolean(true))]);

        let command_map = execute(&action, snapshot(&[], &[])).await;

        assert!(command_map.is_empty());
    }

    #[tokio::test]
    async fn execute_adds_to_the_commands_of_earlier_actions() {
        let context = Context::builder().snapshot(snapshot(&["lamp1", "lamp2"], &[])).build();
        let mut scope = Scope::new();
        let on = control_zone("zone", None, &[("on", Value::Boolean(true))]);
        let dim = control_zone("zone", Some(ClaimMode::Claim), &[("brightness", number(20))]);

        on.execute(&context, &mut scope).await;
        dim.execute(&context, &mut scope).await;

        let command_map = *scope.take().remove("command_map").unwrap().downcast::<CommandMap>().unwrap();
        let expected =
            device_command(Some(ClaimMode::Claim), &[("on", Value::Boolean(true)), ("brightness", number(20))]);
        assert_eq!(command_map["lamp1"], expected);
    }

    fn validate_zone(action: &ControlZoneAction, snapshot: StoreSnapshot) -> Vec<Problem> {
        action.validate(&snapshot, &PresetRegistry::new(vec![]))
    }

    #[test]
    fn validate_accepts_a_zone_whose_devices_have_the_property() {
        let action = control_zone("zone", None, &[("on", Value::Boolean(true))]);

        assert_eq!(validate_zone(&action, snapshot(&["lamp1", "lamp2", "lamp3"], &[])), []);
    }

    #[test]
    fn validate_accepts_an_empty_zone() {
        let action = control_zone("zone", None, &[("on", Value::Boolean(true))]);

        assert_eq!(validate_zone(&action, snapshot(&[], &[])), []);
    }

    #[test]
    fn validate_accepts_a_zone_with_groups() {
        let action = control_zone("zone", None, &[("on", Value::Boolean(true))]);

        assert_eq!(validate_zone(&action, snapshot(&["lamp1"], &["room"])), []);
    }

    #[test]
    fn validate_reports_an_unknown_zone() {
        let action = control_zone("unknown", None, &[("on", Value::Boolean(true))]);

        let problems = validate_zone(&action, snapshot(&["lamp1"], &[]));

        assert_eq!(problems, [Problem::UnknownZone { zone_id: "unknown".to_string() }]);
    }

    #[test]
    fn validate_reports_an_unknown_group_of_the_zone() {
        let action = control_zone("zone", None, &[("on", Value::Boolean(true))]);

        let problems = validate_zone(&action, snapshot(&["lamp1"], &["ghost"]));

        assert_eq!(problems, [Problem::UnknownGroup { group_id: "ghost".to_string() }]);
    }

    #[test]
    fn validate_still_checks_the_other_members_when_a_group_is_unknown() {
        let action = control_zone("zone", None, &[("brightness", number(20))]);

        let problems = validate_zone(&action, snapshot(&["lamp3"], &["ghost"]));

        assert_eq!(
            problems,
            [
                Problem::UnknownGroup { group_id: "ghost".to_string() },
                Problem::UnknownProperty { device_id: "lamp3".to_string(), property_id: "brightness".to_string() },
            ]
        );
    }

    #[test]
    fn validate_reports_a_device_of_the_zone_that_is_not_in_the_store() {
        let action = control_zone("zone", None, &[("on", Value::Boolean(true))]);

        let problems = validate_zone(&action, snapshot(&["lamp1", "ghost"], &[]));

        assert_eq!(problems, [Problem::UnknownDevice { device_id: "ghost".to_string() }]);
    }

    #[test]
    fn validate_reports_an_unknown_property_for_the_device_that_lacks_it() {
        let action = control_zone("zone", None, &[("brightness", number(20))]);

        let problems = validate_zone(&action, snapshot(&["lamp1", "lamp3"], &[]));

        assert_eq!(
            problems,
            [Problem::UnknownProperty { device_id: "lamp3".to_string(), property_id: "brightness".to_string() }]
        );
    }

    #[test]
    fn validate_checks_the_devices_of_the_groups_of_the_zone() {
        let action = control_zone("zone", None, &[("brightness", number(20))]);

        let problems = validate_zone(&action, snapshot(&["lamp1"], &["room"]));

        assert_eq!(
            problems,
            [Problem::UnknownProperty { device_id: "lamp3".to_string(), property_id: "brightness".to_string() }]
        );
    }

    #[test]
    fn validate_reports_a_device_in_both_the_zone_and_its_group_once() {
        let action = control_zone("zone", None, &[("brightness", number(20))]);

        let problems = validate_zone(&action, snapshot(&["lamp3"], &["room"]));

        assert_eq!(
            problems,
            [Problem::UnknownProperty { device_id: "lamp3".to_string(), property_id: "brightness".to_string() }]
        );
    }

    #[test]
    fn validate_reports_an_incompatible_value_for_each_device() {
        let action = control_zone("zone", None, &[("on", number(50))]);

        let problems = validate_zone(&action, snapshot(&["lamp1", "lamp2"], &[]));

        let incompatible = |device_id: &str| Problem::IncompatibleValue {
            device_id: device_id.to_string(),
            property_id: "on".to_string(),
            expected: ValueKind::Boolean,
            actual: ValueKind::Number,
        };
        assert_eq!(problems, [incompatible("lamp1"), incompatible("lamp2")]);
    }

    #[rstest]
    #[case::too_large(150)]
    #[case::too_small(0)]
    fn validate_ignores_a_value_outside_the_range_of_a_device(#[case] value: u64) {
        let action = control_zone("zone", None, &[("brightness", number(value))]);

        assert_eq!(validate_zone(&action, snapshot(&["lamp1", "lamp2"], &[])), []);
    }

    #[test]
    fn validate_reports_an_unknown_preset_once_for_the_whole_zone() {
        let preset = PropertyCommand { value: Expression::PresetValue { id: "missing".to_string() }, transition: None };
        let action = ControlZoneAction {
            zone_id: "zone".to_string(),
            claim: None,
            property: HashMap::from([("brightness".to_string(), preset)]),
        };

        let problems = validate_zone(&action, snapshot(&["lamp1", "lamp2"], &[]));

        assert_eq!(problems, [Problem::PresetNotFound { id: "missing".to_string() }]);
    }
}
