use crate::domain::events::Event;
use crate::domain::property::PropertyLocator;
use crate::hue::domain::ButtonChanged;

pub fn map_remote_changed(property: ButtonChanged) -> Vec<Event> {
    let Some(report) = property.button.and_then(|b| b.button_report) else {
        return vec![];
    };

    vec![
        Event::EnumPropertyChanged {
            device_id: property.owner.rid.to_string(),
            property_id: PropertyLocator::ExternalId(property.id.to_string()),
            value: Some(report.event),
        },
        Event::DateTimePropertyChanged {
            device_id: property.owner.rid.to_string(),
            property_id: PropertyLocator::ExternalId(property.id),
            value: Some(report.updated),
        }
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hue::domain::{ButtonReport, ButtonUpdate, Owner};
    use chrono::{TimeZone, Utc};

    fn owner() -> Owner {
        Owner { rid: "3a3225cb-dcda-46fb-8f21-00a8c76024bc".to_string(), rtype: "device".to_string() }
    }

    #[test]
    fn map_remote_changed_maps_a_button_report_to_two_events() {
        let updated = Utc.with_ymd_and_hms(2026, 9, 24, 16, 59, 31).unwrap();
        let property = ButtonChanged {
            id: "5568ab93-2bef-4739-a667-8beb87898f78".to_string(),
            owner: owner(),
            button: Some(ButtonUpdate {
                button_report: Some(ButtonReport { updated, event: "short_release".to_string() }),
            }),
        };

        let events = map_remote_changed(property);

        assert_eq!(
            events,
            vec![
                Event::EnumPropertyChanged {
                    device_id: owner().rid,
                    property_id: PropertyLocator::ExternalId("5568ab93-2bef-4739-a667-8beb87898f78".to_string()),
                    value: Some("short_release".to_string()),
                },
                Event::DateTimePropertyChanged {
                    device_id: owner().rid,
                    property_id: PropertyLocator::ExternalId("5568ab93-2bef-4739-a667-8beb87898f78".to_string()),
                    value: Some(updated),
                },
            ]
        );
    }

    #[test]
    fn map_remote_changed_returns_no_events_if_button_report_is_missing() {
        let property = ButtonChanged { id: "5568ab93-2bef-4739-a667-8beb87898f78".to_string(), owner: owner(), button: None };

        assert_eq!(map_remote_changed(property), vec![]);
    }

    #[test]
    fn map_remote_changed_returns_no_events_if_button_report_is_omitted_from_a_partial_update() {
        let property = ButtonChanged { id: "5568ab93-2bef-4739-a667-8beb87898f78".to_string(), owner: owner(), button: Some(ButtonUpdate { button_report: None }) };

        assert_eq!(map_remote_changed(property), vec![]);
    }
}