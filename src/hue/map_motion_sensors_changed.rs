use crate::domain::Number;
use crate::domain::events::Event;
use crate::hue::domain::MotionChanged;

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

    if let Some(sensitivity) = property.sensitivity.and_then(|s| s.sensitivity) {
        events.push(Event::NumberPropertyChanged {
            device_id: property.owner.rid.to_string(),
            property_id: "sensitivity".to_string(),
            value: Some(Number::PositiveInt(sensitivity)),
        });
    }

    events
}
