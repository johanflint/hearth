use crate::domain::Number;
use crate::domain::events::Event;
use crate::domain::property::CartesianCoordinate;
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

    if let Some(color_temperature) = property.color_temperature {
        events.push(Event::NumberPropertyChanged {
            device_id: property.owner.rid.to_string(),
            property_id: "colorTemperature".to_string(),
            value: Number::PositiveInt(color_temperature.mirek.mirek_to_kelvin()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Number::{Float, PositiveInt};
    use crate::domain::events::Event::{BooleanPropertyChanged, ColorPropertyChanged, NumberPropertyChanged};
    use crate::domain::property::Gamut;
    use crate::hue::domain::{ChangedColor, ChangedColorTemperature, ColorGamut, Dimming, On, Owner, Xy};
    use pretty_assertions::assert_eq;

    #[test]
    fn maps_no_changes() {
        let light_changed = LightChanged {
            id: "42".to_string(),
            owner: Owner {
                rid: "84a3be14-5d90-4165-ac64-818b7981bb32".to_string(),
                rtype: "device".to_string(),
            },
            on: None,
            dimming: None,
            color_temperature: None,
            color: None,
        };

        let result = map_light_changed_property(light_changed);
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn maps_on_property() {
        let light_changed = LightChanged {
            id: "42".to_string(),
            owner: Owner {
                rid: "84a3be14-5d90-4165-ac64-818b7981bb32".to_string(),
                rtype: "device".to_string(),
            },
            on: Some(On { on: true }),
            dimming: None,
            color_temperature: None,
            color: None,
        };

        let result = map_light_changed_property(light_changed);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0],
            BooleanPropertyChanged {
                device_id: "84a3be14-5d90-4165-ac64-818b7981bb32".to_string(),
                property_id: "on".to_string(),
                value: true
            }
        );
    }

    #[test]
    fn maps_dimming_property() {
        let light_changed = LightChanged {
            id: "42".to_string(),
            owner: Owner {
                rid: "84a3be14-5d90-4165-ac64-818b7981bb32".to_string(),
                rtype: "device".to_string(),
            },
            on: None,
            dimming: Some(Dimming {
                brightness: 20.8,
                min_dim_level: None,
            }),
            color_temperature: None,
            color: None,
        };

        let result = map_light_changed_property(light_changed);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0],
            NumberPropertyChanged {
                device_id: "84a3be14-5d90-4165-ac64-818b7981bb32".to_string(),
                property_id: "brightness".to_string(),
                value: Float(20.8),
            }
        );
    }

    #[test]
    fn maps_color_temperature_property() {
        let light_changed = LightChanged {
            id: "42".to_string(),
            owner: Owner {
                rid: "84a3be14-5d90-4165-ac64-818b7981bb32".to_string(),
                rtype: "device".to_string(),
            },
            on: None,
            dimming: None,
            color_temperature: Some(ChangedColorTemperature { mirek: 153, mirek_valid: true }),
            color: None,
        };

        let result = map_light_changed_property(light_changed);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0],
            NumberPropertyChanged {
                device_id: "84a3be14-5d90-4165-ac64-818b7981bb32".to_string(),
                property_id: "colorTemperature".to_string(),
                value: PositiveInt(6535)
            }
        );
    }

    #[test]
    fn maps_color_property() {
        let light_changed = LightChanged {
            id: "42".to_string(),
            owner: Owner {
                rid: "84a3be14-5d90-4165-ac64-818b7981bb32".to_string(),
                rtype: "device".to_string(),
            },
            on: None,
            dimming: None,
            color_temperature: None,
            color: Some(ChangedColor {
                xy: Xy { x: 0.0, y: 0.0 },
                gamut: Some(ColorGamut {
                    red: Xy { x: 0.1, y: 0.2 },
                    green: Xy { x: 0.3, y: 0.4 },
                    blue: Xy { x: 0.5, y: 0.6 },
                }),
            }),
        };

        let result = map_light_changed_property(light_changed);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0],
            ColorPropertyChanged {
                device_id: "84a3be14-5d90-4165-ac64-818b7981bb32".to_string(),
                property_id: "color".to_string(),
                xy: CartesianCoordinate::new(0.0, 0.0),
                gamut: Some(Gamut::new(
                    CartesianCoordinate::new(0.1, 0.2),
                    CartesianCoordinate::new(0.3, 0.4),
                    CartesianCoordinate::new(0.5, 0.6)
                )),
            }
        );
    }
}
