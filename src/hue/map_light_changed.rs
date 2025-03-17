use crate::domain::events::Event;
use crate::hue::domain::LightChanged;

pub fn map_light_changed_property(property: LightChanged) -> Vec<Event> {
    let mut events = Vec::<Event>::with_capacity(1);
    if let Some(on) = property.on {
        events.push(Event::BooleanPropertyChanged {
            device_id: property.owner.rid.to_string(),
            property_id: "on".to_string(),
            value: on.on,
        });
    }

    events
}
