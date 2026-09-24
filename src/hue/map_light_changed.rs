use crate::domain::Number;
use crate::domain::events::Event;
use crate::domain::property::{CartesianCoordinate, PropertyLocator};
use crate::extensions::unsigned_ints_ext::MirekConversions;
use crate::hue::domain::LightChanged;
use tracing::warn;

pub fn map_light_changed_property(property: LightChanged) -> Vec<Event> {
    let mut events = Vec::<Event>::with_capacity(4);
    if let Some(on) = property.on {
        events.push(Event::BooleanPropertyChanged {
            device_id: property.owner.rid.to_string(),
            property_id: PropertyLocator::Name("on".to_string()),
            value: on.on,
        });
    }

    if let Some(dimming) = property.dimming {
        events.push(Event::NumberPropertyChanged {
            device_id: property.owner.rid.to_string(),
            property_id: "brightness".to_string(),
            value: Some(Number::Float(dimming.brightness)),
        });
    }

    if let Some(color_temperature) = property.color_temperature {
        if color_temperature.mirek_valid && let Some(mirek) = color_temperature.mirek {
            let clamped = mirek.clamp(153, 500);
            if clamped != mirek {
                warn!("⚠️ Mirek value of '{mirek}' is out of range [153, 500], clamping to '{clamped}'")
            }
            events.push(Event::NumberPropertyChanged {
                device_id: property.owner.rid.to_string(),
                property_id: "colorTemperature".to_string(),
                value: Some(Number::PositiveInt(clamped.mirek_to_kelvin())),
            });
        }
    }

    if let Some(color) = property.color {
        events.push(Event::ColorPropertyChanged {
            device_id: property.owner.rid.to_string(),
            property_id: PropertyLocator::Name("color".to_string()),
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
    use rstest::rstest;

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
                property_id: PropertyLocator::Name("on".to_string()),
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
                value: Some(Float(20.8)),
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
            color_temperature: Some(ChangedColorTemperature { mirek: Some(153), mirek_valid: true }),
            color: None,
        };

        let result = map_light_changed_property(light_changed);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0],
            NumberPropertyChanged {
                device_id: "84a3be14-5d90-4165-ac64-818b7981bb32".to_string(),
                property_id: "colorTemperature".to_string(),
                value: Some(PositiveInt(6535))
            }
        );
    }

    #[test]
    fn ignores_color_temperature_if_mirek_is_invalid() {
        let light_changed = LightChanged {
            id: "42".to_string(),
            owner: Owner {
                rid: "84a3be14-5d90-4165-ac64-818b7981bb32".to_string(),
                rtype: "device".to_string(),
            },
            on: None,
            dimming: None,
            color_temperature: Some(ChangedColorTemperature { mirek: Some(100), mirek_valid: false }),
            color: None,
        };

        let result = map_light_changed_property(light_changed);
        assert_eq!(result.len(), 0);
    }

    #[rstest]
    #[case::mirek_too_small(152, 6535)]
    #[case::mirek_too_big(501, 2000)]
    fn ignores_color_temperature_if_mirek_is_out_of_bounds(#[case] mirek: u64, #[case] expected_kelvin: u64) {
        let light_changed = LightChanged {
            id: "42".to_string(),
            owner: Owner {
                rid: "84a3be14-5d90-4165-ac64-818b7981bb32".to_string(),
                rtype: "device".to_string(),
            },
            on: None,
            dimming: None,
            color_temperature: Some(ChangedColorTemperature { mirek: Some(mirek), mirek_valid: true }),
            color: None,
        };

        let result = map_light_changed_property(light_changed);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0],
            NumberPropertyChanged {
                device_id: "84a3be14-5d90-4165-ac64-818b7981bb32".to_string(),
                property_id: "colorTemperature".to_string(),
                value: Some(PositiveInt(expected_kelvin))
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
                property_id: PropertyLocator::Name("color".to_string()),
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
