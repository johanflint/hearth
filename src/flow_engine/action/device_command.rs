use crate::domain::property::{NumberProperty, Property, ValidatedValue};
use crate::domain::{ClaimMode, Device, Value};
use crate::flow_engine::Context;
use crate::flow_engine::expression::evaluate;
use crate::flow_engine::property_command::{PropertyCommand, ResolvedPropertyCommand};
use std::collections::HashMap;
use tracing::{info, warn};

#[derive(PartialEq, Default, Debug)]
pub struct DeviceCommand {
    pub claim: Option<ClaimMode>,
    pub properties: HashMap<String, ResolvedPropertyCommand>,
}

pub type CommandMap = HashMap<String, DeviceCommand>;

impl DeviceCommand {
    // A later action without a claim doesn't undo the claim of an earlier action
    pub fn set_claim(&mut self, claim: Option<ClaimMode>) {
        self.claim = claim.or(self.claim);
    }

    pub fn set_properties(&mut self, device: &Device, evaluated: &HashMap<String, ResolvedPropertyCommand>) {
        for (property_id, command) in evaluated {
            let Some(command) = fit_to_property(device, property_id, command) else {
                continue;
            };

            if let Some(previous) = self.properties.insert(property_id.clone(), command) {
                warn!(
                    device_id = device.id,
                    "⚠️ Overriding property '{}' for device '{}', it was set by another node to '{:?}'",
                    property_id,
                    device.name,
                    previous.value
                );
            }
        }
    }
}

// Evaluates the expressions once, so the result can be applied to multiple devices. Skips the ones that fail.
pub fn evaluate_commands(
    commands: &HashMap<String, PropertyCommand>,
    context: &Context,
) -> HashMap<String, ResolvedPropertyCommand> {
    commands
        .iter()
        .filter_map(|(property_id, command)| match evaluate(&command.value, context) {
            Ok(value) => {
                info!(property_id, "Evaluated '{}' → '{}'", command.value, value);
                Some((property_id.clone(), ResolvedPropertyCommand { value, transition: command.transition }))
            }
            Err(err) => {
                warn!(property_id, expression = %command.value, "⚠️ Evaluating expression... failed: {}", err);
                None
            }
        })
        .collect()
}

// Checks the evaluated command against the property of the device, returns None (after logging) if it can't be applied
fn fit_to_property(
    device: &Device,
    property_id: &str,
    command: &ResolvedPropertyCommand,
) -> Option<ResolvedPropertyCommand> {
    let Some(property) = device.properties.get(property_id) else {
        warn!(
            device_id = device.id,
            property_id, "⚠️ Unable to control device '{}'... unknown property '{}'", device.id, property_id
        );
        return None;
    };

    // Validation can't catch a preset that changed kind after the flow was stored
    let (expected, actual) = (property.value_kind(), command.value.kind());
    if expected != actual {
        warn!(
            device_id = device.id,
            property_id, "⚠️ Expected a {expected} for property '{property_id}', got a {actual}, ignoring"
        );
        return None;
    }

    let value = clamp_to_range(&device.id, property_id, property.as_ref(), command.value.clone());
    Some(ResolvedPropertyCommand { value, transition: command.transition })
}

fn clamp_to_range(device_id: &str, property_id: &str, property: &dyn Property, value: Value) -> Value {
    let (Value::Number(number), Some(number_property)) = (&value, property.as_any().downcast_ref::<NumberProperty>())
    else {
        return value;
    };

    match number_property.validate_value(*number) {
        ValidatedValue::Clamped(clamped, error) => {
            #[rustfmt::skip]
            info!(device_id, property_id, "⚠️ Invalid value '{}' for property '{}' ({}), clamped to '{}'", number, property_id, error, clamped);
            Value::Number(clamped)
        }
        ValidatedValue::Valid(_) | ValidatedValue::Invalid(_) => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Number;
    use crate::domain::property::{NumberProperty, PropertyType};
    use crate::flow_engine::Expression;
    use crate::store::StoreSnapshot;
    use crate::test_support::{DeviceBuilder, property_command, resolved_property_command};
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use std::time::Duration;

    fn device() -> Device {
        DeviceBuilder::new("lamp")
            .with_boolean_property("on", false)
            .with_properties(vec![Box::new(
                NumberProperty::builder("brightness".to_string(), PropertyType::Brightness, false)
                    .positive_int(Some(50), Some(1), Some(100))
                    .build(),
            )])
            .build()
    }

    fn context() -> Context {
        Context::builder().snapshot(StoreSnapshot::default()).build()
    }

    fn number(value: u64) -> Value {
        Value::Number(Number::PositiveInt(value))
    }

    fn commands(property_id: &str, command: PropertyCommand) -> HashMap<String, PropertyCommand> {
        HashMap::from([(property_id.to_string(), command)])
    }

    fn set_properties(commands: &HashMap<String, PropertyCommand>) -> DeviceCommand {
        let mut device_command = DeviceCommand::default();
        device_command.set_properties(&device(), &evaluate_commands(commands, &context()));
        device_command
    }

    #[test]
    fn set_properties_resolves_a_literal() {
        let device_command = set_properties(&commands("on", property_command(Value::Boolean(true))));

        assert_eq!(
            device_command.properties,
            HashMap::from([("on".to_string(), resolved_property_command(Value::Boolean(true)))])
        );
    }

    #[test]
    fn set_properties_keeps_the_transition() {
        let transition = Some(Duration::from_secs(2));
        let command = PropertyCommand { transition, ..property_command(number(10)) };

        let device_command = set_properties(&commands("brightness", command));

        assert_eq!(device_command.properties["brightness"].transition, transition);
    }

    #[test]
    fn set_properties_resolves_multiple_properties() {
        let commands = HashMap::from([
            ("on".to_string(), property_command(Value::Boolean(true))),
            ("brightness".to_string(), property_command(number(10))),
        ]);

        let device_command = set_properties(&commands);

        assert_eq!(device_command.properties.len(), 2);
    }

    #[test]
    fn set_properties_skips_an_unknown_property() {
        let device_command = set_properties(&commands("unknown", property_command(Value::Boolean(true))));

        assert_eq!(device_command, DeviceCommand::default());
    }

    #[test]
    fn set_properties_skips_a_value_of_the_wrong_kind() {
        let device_command = set_properties(&commands("brightness", property_command(Value::Boolean(true))));

        assert_eq!(device_command, DeviceCommand::default());
    }

    #[test]
    fn set_properties_skips_an_expression_that_fails_to_evaluate() {
        let invalid = Expression::Add {
            lhs: Box::new(Expression::Literal { value: Value::Boolean(true) }),
            rhs: Box::new(Expression::Literal { value: number(10) }),
        };

        let device_command =
            set_properties(&commands("brightness", PropertyCommand { value: invalid, transition: None }));

        assert_eq!(device_command, DeviceCommand::default());
    }

    #[test]
    fn set_properties_only_skips_the_failing_properties() {
        let commands = HashMap::from([
            ("on".to_string(), property_command(Value::Boolean(true))),
            ("unknown".to_string(), property_command(Value::Boolean(true))),
            ("brightness".to_string(), property_command(Value::Boolean(true))),
        ]);

        let device_command = set_properties(&commands);

        assert_eq!(
            device_command.properties,
            HashMap::from([("on".to_string(), resolved_property_command(Value::Boolean(true)))])
        );
    }

    #[rstest]
    #[case::too_large(150, 100)]
    #[case::too_small(0, 1)]
    #[case::in_range(42, 42)]
    fn set_properties_clamps_a_number_to_the_property_range(#[case] value: u64, #[case] expected: u64) {
        let device_command = set_properties(&commands("brightness", property_command(number(value))));

        assert_eq!(device_command.properties["brightness"], resolved_property_command(number(expected)));
    }

    #[test]
    fn set_properties_overrides_a_property_set_earlier() {
        let mut device_command = DeviceCommand::default();
        device_command.set_properties(
            &device(),
            &evaluate_commands(&commands("brightness", property_command(number(10))), &context()),
        );

        device_command.set_properties(
            &device(),
            &evaluate_commands(&commands("brightness", property_command(number(20))), &context()),
        );

        assert_eq!(device_command.properties["brightness"], resolved_property_command(number(20)));
    }

    #[test]
    fn set_properties_keeps_properties_set_earlier() {
        let mut device_command = DeviceCommand::default();
        device_command.set_properties(
            &device(),
            &evaluate_commands(&commands("on", property_command(Value::Boolean(true))), &context()),
        );

        device_command.set_properties(
            &device(),
            &evaluate_commands(&commands("brightness", property_command(number(20))), &context()),
        );

        assert_eq!(device_command.properties.len(), 2);
    }

    #[rstest]
    #[case::none_stays_none(None, None, None)]
    #[case::claim_then_none(Some(ClaimMode::Claim), None, Some(ClaimMode::Claim))]
    #[case::none_then_release(None, Some(ClaimMode::Release), Some(ClaimMode::Release))]
    #[case::claim_then_release(Some(ClaimMode::Claim), Some(ClaimMode::Release), Some(ClaimMode::Release))]
    #[case::release_then_claim(Some(ClaimMode::Release), Some(ClaimMode::Claim), Some(ClaimMode::Claim))]
    fn set_claim_keeps_the_last_claim(
        #[case] first: Option<ClaimMode>,
        #[case] second: Option<ClaimMode>,
        #[case] expected: Option<ClaimMode>,
    ) {
        let mut device_command = DeviceCommand::default();

        device_command.set_claim(first);
        device_command.set_claim(second);

        assert_eq!(device_command.claim, expected);
    }
}
