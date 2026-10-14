use crate::domain::ContactState;
use crate::domain::events::{Event, InitiatedBy};
use crate::domain::property::PropertyLocator;
use crate::hue::domain::ContactChanged;

pub fn map_contact_sensors_changed(property: ContactChanged) -> Vec<Event> {
    let Some(report) = property.contact_report else {
        return Vec::new();
    };

    vec![
        Event::EnumPropertyChanged {
            device_id: property.owner.rid.to_string(),
            property_id: PropertyLocator::Name("contact".to_string()),
            value: Some(ContactState::from(&report.state).as_str().to_string()),
            initiated_by: InitiatedBy::External,
        },
        Event::DateTimePropertyChanged {
            device_id: property.owner.rid.to_string(),
            property_id: PropertyLocator::Name("contactLastChanged".to_string()),
            value: Some(report.changed),
            initiated_by: InitiatedBy::External,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hue::domain::{ContactReport, ContactState as HueContactState, Owner};
    use chrono::{TimeZone, Utc};
    use rstest::rstest;

    fn owner() -> Owner {
        Owner {
            rid: "c0a5a9b4-1b0a-4d6f-8d3a-2f4d6c1e9b77".to_string(),
            rtype: "device".to_string(),
        }
    }

    #[test]
    fn returns_no_events_without_a_report() {
        let property = ContactChanged {
            id: "6d8f3b41-5b3c-4a52-8d1a-9f1b6a0c7e21".to_string(),
            owner: owner(),
            contact_report: None,
        };

        assert!(map_contact_sensors_changed(property).is_empty());
    }

    #[rstest]
    #[case::closed(HueContactState::Contact, "closed")]
    #[case::open(HueContactState::NoContact, "open")]
    fn returns_the_contact_state_and_the_time_it_changed(#[case] state: HueContactState, #[case] expected: &str) {
        let changed = Utc.with_ymd_and_hms(2026, 9, 23, 15, 37, 22).unwrap();
        let property = ContactChanged {
            id: "6d8f3b41-5b3c-4a52-8d1a-9f1b6a0c7e21".to_string(),
            owner: owner(),
            contact_report: Some(ContactReport { changed, state }),
        };

        let events = map_contact_sensors_changed(property);

        assert_eq!(
            events,
            vec![
                Event::EnumPropertyChanged {
                    device_id: owner().rid,
                    property_id: PropertyLocator::Name("contact".to_string()),
                    value: Some(expected.to_string()),
                    initiated_by: InitiatedBy::External,
                },
                Event::DateTimePropertyChanged {
                    device_id: owner().rid,
                    property_id: PropertyLocator::Name("contactLastChanged".to_string()),
                    value: Some(changed),
                    initiated_by: InitiatedBy::External,
                },
            ]
        );
    }
}
