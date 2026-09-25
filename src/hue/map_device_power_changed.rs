use crate::domain::events::Event;
use crate::domain::property::PropertyLocator;
use crate::domain::{BatteryState, Number};
use crate::hue::domain::DevicePowerChanged;

pub fn map_device_power_changed(property: DevicePowerChanged) -> Vec<Event> {
    let Some(power_state) = property.power_state else {
        return Vec::new();
    };

    let battery_state = power_state.battery_level.map(|b| BatteryState::from_percent(u64::from(b)).as_str().to_string());
    vec![
        Event::NumberPropertyChanged {
            device_id: property.owner.rid.to_string(),
            property_id: PropertyLocator::Name("batteryLevel".to_string()),
            value: power_state.battery_level.map(|level| Number::PositiveInt(u64::from(level))),
        },
        Event::EnumPropertyChanged {
            device_id: property.owner.rid.to_string(),
            property_id: PropertyLocator::Name("batteryState".to_string()),
            value: battery_state,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hue::domain::{Owner, PowerState};

    fn owner() -> Owner {
        Owner {
            rid: "3a3225cb-dcda-46fb-8f21-00a8c76024bc".to_string(),
            rtype: "device".to_string(),
        }
    }

    #[test]
    fn map_device_power_changed_maps_a_battery_level_to_an_event() {
        let property = DevicePowerChanged {
            id: "5568ab93-2bef-4739-a667-8beb87898f78".to_string(),
            owner: owner(),
            power_state: Some(PowerState {
                battery_level: Some(56),
                battery_state: Some("normal".to_string()),
            }),
        };

        let events = map_device_power_changed(property);

        assert_eq!(
            events,
            vec![
                Event::NumberPropertyChanged {
                    device_id: owner().rid,
                    property_id: PropertyLocator::Name("batteryLevel".to_string()),
                    value: Some(Number::PositiveInt(56)),
                },
                Event::EnumPropertyChanged {
                    device_id: owner().rid,
                    property_id: PropertyLocator::Name("batteryState".to_string()),
                    value: Some("normal".to_string()),
                }
            ]
        );
    }

    #[test]
    fn map_device_power_changed_maps_a_missing_battery_level_to_none() {
        let property = DevicePowerChanged {
            id: "5568ab93-2bef-4739-a667-8beb87898f78".to_string(),
            owner: owner(),
            power_state: Some(PowerState {
                battery_level: None,
                battery_state: None,
            }),
        };

        let events = map_device_power_changed(property);

        assert_eq!(
            events,
            vec![
                Event::NumberPropertyChanged {
                    device_id: owner().rid,
                    property_id: PropertyLocator::Name("batteryLevel".to_string()),
                    value: None,
                },
                Event::EnumPropertyChanged {
                    device_id: owner().rid,
                    property_id: PropertyLocator::Name("batteryState".to_string()),
                    value: None,
                }
            ]
        );
    }

    #[test]
    fn map_device_power_changed_returns_no_events_if_power_state_is_omitted_from_a_partial_update() {
        let property = DevicePowerChanged {
            id: "5568ab93-2bef-4739-a667-8beb87898f78".to_string(),
            owner: owner(),
            power_state: None,
        };

        assert_eq!(map_device_power_changed(property), vec![]);
    }
}
