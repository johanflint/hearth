use crate::domain::property::{Property, PropertyError};
use crate::store::DeviceMap;
use std::any::type_name;
use std::sync::Arc;
use thiserror::Error;
use tracing::{info, warn};

#[inline(always)]
pub(crate) fn reduce_property_changed_event<F, T>(devices: &mut DeviceMap, device_id: &str, property_id: &str, set_value: F) -> Result<bool, ReducerError>
where
    F: FnOnce(&mut T) -> Result<(), PropertyError>,
    T: Property + 'static,
{
    let mut new_device = if let Some(device) = devices.get(device_id) {
        device.as_ref().clone()
    } else {
        warn!(device_id, "⚠️ Received property changed event for unknown device '{}'", device_id);
        return Err(ReducerError::UnknownDevice { device_id: device_id.to_string() });
    };

    let Some(property) = new_device.properties.get_mut(property_id) else {
        warn!(device_id = new_device.id, "⚠️ Unknown property '{}' for device '{}'", property_id, new_device.name);
        return Err(ReducerError::UnknownProperty {
            device_id: device_id.to_string(),
            property_id: property_id.to_string(),
        });
    };

    let previous_value = property.value_string();
    let previous_property = property.clone_box();
    let Some(downcast_property) = property.as_any_mut().downcast_mut::<T>() else {
        warn!(device_id, "⚠️ Expected '{}' property for property '{}'", type_name::<T>(), &property_id);
        return Err(ReducerError::IncorrectPropertyType {
            device_id: device_id.to_string(),
            property_id: property_id.to_string(),
            expected_type: type_name::<T>().to_owned(),
        });
    };

    if let Err(err) = set_value(downcast_property) {
        warn!(device_id = new_device.id, "⚠️ Could not set value for property '{}': {}", property_id, err);
        return Err(ReducerError::PropertyChangedError(err));
    }

    let changed = !previous_property.eq_dyn(downcast_property);
    if changed {
        info!(
            device_id,
            "🟢 Updated device '{}', set '{}' to '{}', was '{}'",
            &new_device.name,
            property.name(),
            property.value_string(),
            previous_value
        );
    }

    devices.insert(device_id.to_string(), Arc::new(new_device));
    Ok(changed)
}

#[derive(Error, PartialEq, Debug)]
pub enum ReducerError {
    #[error("unknown device '{device_id}'")]
    UnknownDevice { device_id: String },
    #[error("unknown property '{property_id}' for device '{device_id}'")]
    UnknownProperty { device_id: String, property_id: String },
    #[error("expected device '{device_id}' to have property '{property_id}' of type '{expected_type}'")]
    IncorrectPropertyType { device_id: String, property_id: String, expected_type: String },
    #[error(transparent)]
    PropertyChangedError(PropertyError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::property::{BooleanProperty, NumberProperty};
    use crate::test_support::DeviceBuilder;
    use std::collections::HashMap;
    use std::sync::Arc;
    use test_log::test;

    const DEVICE_ID: &str = "079e0321-7e18-46bc-bc16-fcbc3dd09e30";

    fn create_devices() -> DeviceMap {
        let device = DeviceBuilder::new(DEVICE_ID)
            .with_boolean_property("on", false)
            .build();

        HashMap::from([(DEVICE_ID.to_string(), Arc::new(device))])
    }

    #[test]
    fn reduce_returns_error_if_the_device_is_unknown() {
        let mut devices = create_devices();
        let result = reduce_property_changed_event(&mut devices, "unknown", "on", |_: &mut BooleanProperty| Ok(()));

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ReducerError::UnknownDevice { device_id: "unknown".to_string() });
    }

    #[test]
    fn reduce_returns_error_if_the_property_is_unknown() {
        let mut devices = create_devices();
        let result = reduce_property_changed_event(&mut devices, DEVICE_ID, "unknown", |_: &mut BooleanProperty| Ok(()));

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            ReducerError::UnknownProperty {
                device_id: DEVICE_ID.to_string(),
                property_id: "unknown".to_string(),
            }
        );
    }

    #[test]
    fn reduce_returns_error_if_the_property_type_is_incorrect() {
        let mut devices = create_devices();
        let result = reduce_property_changed_event(&mut devices, DEVICE_ID, "on", |_: &mut NumberProperty| Ok(()));

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            ReducerError::IncorrectPropertyType {
                device_id: DEVICE_ID.to_string(),
                property_id: "on".to_string(),
                expected_type: "hearth::domain::property::number_property::NumberProperty".to_string()
            }
        );
    }

    #[test]
    fn reduce_returns_error_if_the_lambda_returns_an_error() {
        let mut devices = create_devices();
        let result = reduce_property_changed_event(&mut devices, DEVICE_ID, "on", |_: &mut BooleanProperty| Err(PropertyError::ReadOnly));

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ReducerError::PropertyChangedError(PropertyError::ReadOnly));
    }

    #[test]
    fn reduce_returns_false_for_a_duplicate_value() {
        let mut devices = create_devices(); // property "on" starts at false

        let first = reduce_property_changed_event(&mut devices, DEVICE_ID, "on", |p: &mut BooleanProperty| p.set_value(true));
        assert_eq!(first, Ok(true), "the first update to a new value must report a change");

        let duplicate = reduce_property_changed_event(&mut devices, DEVICE_ID, "on", |p: &mut BooleanProperty| p.set_value(true));
        assert_eq!(duplicate, Ok(false), "re-applying the same value must not report a change");
    }
}
