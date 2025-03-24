use crate::domain::property::{Property, PropertyError};
use crate::store::DeviceMap;
use std::any::type_name;
use tracing::{info, warn};

#[inline(always)]
pub(crate) async fn reduce_property_changed_event<F, T>(devices: &mut DeviceMap, device_id: &str, property_id: &str, set_value: F)
where
    F: FnOnce(&mut T) -> Result<(), PropertyError>,
    T: Property + 'static,
{
    let mut write_guard = devices.write().await;

    let Some(device) = write_guard.get_mut(device_id) else {
        #[rustfmt::skip]
        warn!(device_id, "⚠️ Received property changed event for unknown device '{}'", device_id);
        return;
    };

    let Some(property) = device.properties.get_mut(property_id) else {
        #[rustfmt::skip]
        warn!(device_id = device.id,"⚠️ Unknown property '{}' for device '{}'", property_id, device.name);
        return;
    };

    let previous_value = property.value_string();
    let Some(downcast_property) = property.as_any_mut().downcast_mut::<T>() else {
        #[rustfmt::skip]
        warn!(device_id, "⚠️ Expected {} property for property '{}'", type_name::<T>(), &property_id);
        return;
    };

    if let Err(err) = set_value(downcast_property) {
        #[rustfmt::skip]
        warn!(device_id = device.id, "⚠️ Could not set value for property '{}': {}", property_id, err);
        return;
    }

    info!(
        device_id = device.id,
        "🟢 Updated device '{}', set '{}' to '{}', was '{}'",
        device.name,
        property.name(),
        property.value_string(),
        previous_value
    );
}
