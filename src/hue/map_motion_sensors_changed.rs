use crate::domain::Number;
use crate::domain::events::Event;
use crate::hue::domain::{MotionChanged, SensitivityChanged, SensitivityStatus};

pub fn map_motion_sensors_changed(property: MotionChanged) -> Vec<Event> {
    let mut events = Vec::<Event>::with_capacity(3);
    if let Some(enabled) = property.enabled {
        events.push(Event::BooleanPropertyChanged {
            device_id: property.owner.rid.to_string(),
            property_id: "enabled".to_string(),
            value: enabled,
        });
    }

    if let Some(report) = property.motion.and_then(|m| m.motion_report) {
        events.push(Event::BooleanPropertyChanged {
            device_id: property.owner.rid.to_string(),
            property_id: "motion".to_string(),
            value: report.motion,
        });
        events.push(Event::DateTimePropertyChanged {
            device_id: property.owner.rid.to_string(),
            property_id: "motionLastChanged".to_string(),
            value: Some(report.changed),
        })
    }

    if let Some(SensitivityChanged { status, sensitivity: Some(sensitivity) }) = property.sensitivity {
        // The bridge re-announces the target value with status "changing" while it propagates to the
        // (battery-powered) physical sensor; only apply it once the sensor has confirmed the change.
        if matches!(status, Some(SensitivityStatus::Set)) {
            events.push(Event::NumberPropertyChanged {
                device_id: property.owner.rid.to_string(),
                property_id: "sensitivity".to_string(),
                value: Some(Number::PositiveInt(sensitivity)),
            });
        }
    }

    events
}
