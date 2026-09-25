use crate::domain::events::Event;
use crate::domain::property::PropertyLocator;
use crate::hue::connectivity::map_connectivity_status;
use crate::hue::domain::ZigbeeConnectivityChanged;

pub fn map_connectivity_changed(property: ZigbeeConnectivityChanged) -> Vec<Event> {
    let Some(status) = property.status else {
        return Vec::new();
    };

    vec![Event::EnumPropertyChanged {
        device_id: property.owner.rid,
        property_id: PropertyLocator::Name("connectivity".to_string()),
        value: Some(map_connectivity_status(&status).as_str().to_string()),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hue::domain::Owner;

    fn owner() -> Owner {
        Owner { rid: "3a3225cb-dcda-46fb-8f21-00a8c76024bc".to_string(), rtype: "device".to_string() }
    }

    #[test]
    fn map_connectivity_changed_maps_a_known_status_to_an_event() {
        let property = ZigbeeConnectivityChanged {
            id: "5568ab93-2bef-4739-a667-8beb87898f78".to_string(),
            owner: owner(),
            status: Some("connectivity_issue".to_string()),
        };

        let events = map_connectivity_changed(property);

        assert_eq!(
            events,
            vec![Event::EnumPropertyChanged {
                device_id: owner().rid,
                property_id: PropertyLocator::Name("connectivity".to_string()),
                value: Some("issues".to_string()),
            }]
        );
    }

    #[test]
    fn map_connectivity_changed_maps_an_unrecognized_status_to_unknown() {
        let property = ZigbeeConnectivityChanged {
            id: "5568ab93-2bef-4739-a667-8beb87898f78".to_string(),
            owner: owner(),
            status: Some("some_future_status".to_string()),
        };

        let events = map_connectivity_changed(property);

        assert_eq!(
            events,
            vec![Event::EnumPropertyChanged {
                device_id: owner().rid,
                property_id: PropertyLocator::Name("connectivity".to_string()),
                value: Some("unknown".to_string()),
            }]
        );
    }

    #[test]
    fn map_connectivity_changed_returns_no_events_if_status_is_omitted_from_a_partial_update() {
        let property = ZigbeeConnectivityChanged {
            id: "5568ab93-2bef-4739-a667-8beb87898f78".to_string(),
            owner: owner(),
            status: None,
        };

        assert_eq!(map_connectivity_changed(property), vec![]);
    }
}