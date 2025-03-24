use crate::domain::events::Event;
use crate::domain::property::{CartesianCoordinate, Number};
use crate::extensions::unsigned_ints_ext::MirekConversions;
use crate::hue::domain::LightChanged;

pub fn map_light_changed_property(property: LightChanged) -> Vec<Event> {
    let mut events = Vec::<Event>::with_capacity(3);
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

    if let Some(color) = property.color {
        events.push(Event::ColorPropertyChanged {
            device_id: property.owner.rid.to_string(),
            property_id: "color".to_string(),
            xy: CartesianCoordinate::new(color.xy.x, color.xy.y),
            gamut: color.gamut.map(|mut g| g.take_gamut()),
        });
    }

    events
}
