use crate::domain::events::Event;
use crate::domain::property::PropertyLocator;
use crate::hue::domain::ButtonChanged;

pub fn map_remote_changed(property: ButtonChanged) -> Vec<Event> {
    let mut events = Vec::<Event>::with_capacity(2);

    let report = property.button.and_then(|b| b.button_report).take();
    let value = report.map(|report| report.event);

    events.push(Event::EnumPropertyChanged {
        device_id: property.owner.rid.to_string(),
        property_id: PropertyLocator::ExternalId(property.id.to_string()),
        value,
    });

    events
}
