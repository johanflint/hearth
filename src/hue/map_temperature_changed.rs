use crate::domain::property::PropertyLocator;
use crate::domain::{Event, InitiatedBy, Number};
use crate::hue::domain::TemperatureChanged;

pub fn map_temperature_changed(property: TemperatureChanged) -> Vec<Event> {
    let Some(report) = property.temperature.and_then(|t| t.temperature_report) else {
        return vec![];
    };

    vec![
        Event::NumberPropertyChanged {
            device_id: property.owner.rid.to_string(),
            property_id: PropertyLocator::Name("temperature".to_string()),
            value: Some(Number::Float(report.temperature)),
            initiated_by: InitiatedBy::External,
        },
        Event::DateTimePropertyChanged {
            device_id: property.owner.rid.to_string(),
            property_id: PropertyLocator::Name("temperatureLastChanged".to_string()),
            value: Some(report.changed),
            initiated_by: InitiatedBy::External,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hue::domain::{Owner, Temperature, TemperatureReport};
    use chrono::{TimeZone, Utc};

    fn owner() -> Owner {
        Owner { rid: "dbe91174-8fe5-4309-b7d7-3f15ec1e57d3".to_string(), rtype: "device".to_string() }
    }

    #[test]
    fn returns_no_events_when_the_temperature_did_not_change() {
        let property = TemperatureChanged { id: "42".to_string(), owner: owner(), temperature: None };

        assert!(map_temperature_changed(property).is_empty());
    }

    #[test]
    fn returns_no_events_without_a_report() {
        let property = TemperatureChanged {
            id: "42".to_string(),
            owner: owner(),
            temperature: Some(Temperature { temperature_report: None }),
        };

        assert!(map_temperature_changed(property).is_empty());
    }

    #[test]
    fn returns_the_temperature_and_the_time_it_changed() {
        let changed = Utc.with_ymd_and_hms(2026, 9, 23, 15, 37, 21).unwrap();
        let property = TemperatureChanged {
            id: "42".to_string(),
            owner: owner(),
            temperature: Some(Temperature {
                temperature_report: Some(TemperatureReport { changed, temperature: 21.5 }),
            }),
        };

        let events = map_temperature_changed(property);

        assert_eq!(
            events,
            vec![
                Event::NumberPropertyChanged {
                    device_id: owner().rid,
                    property_id: PropertyLocator::Name("temperature".to_string()),
                    value: Some(Number::Float(21.5)),
                    initiated_by: InitiatedBy::External,
                },
                Event::DateTimePropertyChanged {
                    device_id: owner().rid,
                    property_id: PropertyLocator::Name("temperatureLastChanged".to_string()),
                    value: Some(changed),
                    initiated_by: InitiatedBy::External,
                },
            ]
        );
    }
}
