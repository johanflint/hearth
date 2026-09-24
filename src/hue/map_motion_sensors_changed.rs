use crate::domain::Number;
use crate::domain::events::Event;
use crate::domain::property::PropertyLocator;
use crate::hue::domain::{MotionChanged, SensitivityChanged, SensitivityStatus};

pub fn map_motion_sensors_changed(property: MotionChanged) -> Vec<Event> {
    let mut events = Vec::<Event>::with_capacity(3);
    if let Some(enabled) = property.enabled {
        events.push(Event::BooleanPropertyChanged {
            device_id: property.owner.rid.to_string(),
            property_id: PropertyLocator::Name("enabled".to_string()),
            value: enabled,
        });
    }

    if let Some(report) = property.motion.and_then(|m| m.motion_report) {
        events.push(Event::BooleanPropertyChanged {
            device_id: property.owner.rid.to_string(),
            property_id: PropertyLocator::Name("motion".to_string()),
            value: report.motion,
        });
        events.push(Event::DateTimePropertyChanged {
            device_id: property.owner.rid.to_string(),
            property_id: PropertyLocator::Name("motionLastChanged".to_string()),
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

#[cfg(test)]
mod tests {
    use crate::domain::Number::PositiveInt;
    use crate::domain::events::Event::{BooleanPropertyChanged, DateTimePropertyChanged, NumberPropertyChanged};
    use crate::domain::property::PropertyLocator;
    use crate::hue::domain::{Motion, MotionChanged, MotionReport, MotionType, Owner, SensitivityChanged, SensitivityStatus};
    use crate::hue::map_motion_sensors_changed::map_motion_sensors_changed;
    use chrono::{TimeZone, Utc};

    fn owner() -> Owner {
        Owner {
            rid: "84a3be14-5d90-4165-ac64-818b7981bb32".to_string(),
            rtype: "device".to_string(),
        }
    }

    fn motion_changed() -> MotionChanged {
        MotionChanged {
            id: "42".to_string(),
            owner: owner(),
            enabled: None,
            motion: None,
            sensitivity: None,
            r#type: Some(MotionType::Motion),
        }
    }

    #[test]
    fn maps_no_changes() {
        let result = map_motion_sensors_changed(motion_changed());
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn maps_enabled_property() {
        let motion_changed = MotionChanged { enabled: Some(true), ..motion_changed() };

        let result = map_motion_sensors_changed(motion_changed);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0],
            BooleanPropertyChanged {
                device_id: owner().rid,
                property_id: PropertyLocator::Name("enabled".to_string()),
                value: true,
            }
        );
    }

    #[test]
    fn maps_motion_report() {
        let changed = Utc.with_ymd_and_hms(2006, 9, 22, 12, 42, 59).unwrap();
        let motion_changed = MotionChanged {
            motion: Some(Motion {
                motion_report: Some(MotionReport { changed, motion: true })
            }),
            ..motion_changed()
        };

        let result = map_motion_sensors_changed(motion_changed);
        assert_eq!(result.len(), 2);
        assert_eq!(
            result[0],
            BooleanPropertyChanged {
                device_id: owner().rid,
                property_id: PropertyLocator::Name("motion".to_string()),
                value: true,
            }
        );
        assert_eq!(
            result[1],
            DateTimePropertyChanged {
                device_id: owner().rid,
                property_id: PropertyLocator::Name("motionLastChanged".to_string()),
                value: Some(changed),
            }
        );
    }

    #[test]
    fn maps_sensitivity_when_confirmed() {
        let motion_changed = MotionChanged {
            sensitivity: Some(SensitivityChanged {
                status: Some(SensitivityStatus::Set),
                sensitivity: Some(3),
            }),
            ..motion_changed()
        };

        let result = map_motion_sensors_changed(motion_changed);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0],
            NumberPropertyChanged {
                device_id: owner().rid,
                property_id: "sensitivity".to_string(),
                value: Some(PositiveInt(3)),
            }
        );
    }

    #[test]
    fn ignores_sensitivity_while_changing() {
        let motion_changed = MotionChanged {
            sensitivity: Some(SensitivityChanged {
                status: Some(SensitivityStatus::Changing),
                sensitivity: Some(3),
            }),
            ..motion_changed()
        };

        let result = map_motion_sensors_changed(motion_changed);
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn ignores_sensitivity_without_a_status() {
        let motion_changed = MotionChanged {
            sensitivity: Some(SensitivityChanged { status: None, sensitivity: Some(3) }),
            ..motion_changed()
        };

        let result = map_motion_sensors_changed(motion_changed);
        assert_eq!(result.len(), 0);
    }
}