use crate::domain::Number;
use crate::domain::events::{Event, InitiatedBy};
use crate::domain::property::PropertyLocator;
use crate::hue::domain::LightLevelChanged;
use crate::hue::illuminance::light_level_to_lux;

pub fn map_light_level_changed(property: LightLevelChanged) -> Vec<Event> {
    let Some(report) = property.light.and_then(|light| light.light_level_report) else {
        return vec![];
    };

    vec![
        Event::NumberPropertyChanged {
            device_id: property.owner.rid.to_string(),
            property_id: PropertyLocator::Name("illuminance".to_string()),
            value: Some(Number::Float(light_level_to_lux(report.light_level))),
            initiated_by: InitiatedBy::External,
        },
        Event::DateTimePropertyChanged {
            device_id: property.owner.rid.to_string(),
            property_id: PropertyLocator::Name("illuminanceLastChanged".to_string()),
            value: Some(report.changed),
            initiated_by: InitiatedBy::External,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hue::domain::{Light, LightLevelReport, Owner};
    use chrono::{TimeZone, Utc};

    fn owner() -> Owner {
        Owner {
            rid: "dbe91174-8fe5-4309-b7d7-3f15ec1e57d3".to_string(),
            rtype: "device".to_string(),
        }
    }

    #[test]
    fn returns_no_events_when_the_light_level_did_not_change() {
        let property = LightLevelChanged {
            id: "42".to_string(),
            owner: owner(),
            light: None,
        };

        assert!(map_light_level_changed(property).is_empty());
    }

    #[test]
    fn returns_no_events_without_a_report() {
        let property = LightLevelChanged {
            id: "42".to_string(),
            owner: owner(),
            light: Some(Light { light_level_report: None }),
        };

        assert!(map_light_level_changed(property).is_empty());
    }

    #[test]
    fn returns_the_illuminance_in_lux_and_the_time_it_changed() {
        let changed = Utc.with_ymd_and_hms(2026, 9, 23, 15, 37, 21).unwrap();
        let property = LightLevelChanged {
            id: "42".to_string(),
            owner: owner(),
            light: Some(Light {
                light_level_report: Some(LightLevelReport { changed, light_level: 10_001 }),
            }),
        };

        let events = map_light_level_changed(property);

        assert_eq!(events.len(), 2);
        let Event::NumberPropertyChanged { device_id, property_id, value: Some(Number::Float(lux)), initiated_by } = &events[0] else {
            panic!("expected a NumberPropertyChanged event with a float value");
        };
        assert_eq!(device_id, &owner().rid);
        assert_eq!(property_id, &PropertyLocator::Name("illuminance".to_string()));
        assert!((lux - 10.0).abs() < 1e-9);
        assert_eq!(initiated_by, &InitiatedBy::External);
        assert_eq!(
            events[1],
            Event::DateTimePropertyChanged {
                device_id: owner().rid,
                property_id: PropertyLocator::Name("illuminanceLastChanged".to_string()),
                value: Some(changed),
                initiated_by: InitiatedBy::External,
            }
        );
    }

    #[test]
    fn returns_zero_lux_for_a_light_level_of_zero() {
        let changed = Utc.with_ymd_and_hms(2026, 9, 23, 15, 37, 21).unwrap();
        let property = LightLevelChanged {
            id: "42".to_string(),
            owner: owner(),
            light: Some(Light {
                light_level_report: Some(LightLevelReport { changed, light_level: 0 }),
            }),
        };

        let events = map_light_level_changed(property);

        assert!(matches!(&events[0], Event::NumberPropertyChanged { value: Some(Number::Float(lux)), .. } if *lux == 0.0));
    }
}
