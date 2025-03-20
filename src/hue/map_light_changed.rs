use crate::domain::events::Event;
use crate::domain::property::Number;
use crate::hue::domain::LightChanged;

pub fn map_light_changed_property(property: LightChanged) -> Vec<Event> {
    let mut events = Vec::<Event>::with_capacity(2);
    if let Some(on) = property.on {
        events.push(Event::BooleanPropertyChanged {
            device_id: property.owner.rid.to_string(),
            property_id: "on".to_string(),
            value: on.on,
        });
    }

    if let Some(dimming) = property.dimming {
        events.push(Event::NumberPropertyChanged {
            device_id: property.owner.rid.to_string(),
            property_id: "brightness".to_string(),
            value: Number::Float(dimming.brightness),
        });
    }

    events
}
