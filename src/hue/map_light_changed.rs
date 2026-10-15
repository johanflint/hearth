use crate::domain::property::{CartesianCoordinate, PropertyLocator};
use crate::domain::{Event, InitiatedBy, Number};
use crate::extensions::unsigned_ints_ext::MirekConversions;
use crate::hue::domain::LightChanged;
use tracing::warn;

pub fn map_light_changed_property(property: LightChanged, initiated_by: InitiatedBy) -> Vec<Event> {
    let mut events = Vec::<Event>::with_capacity(4);
    if let Some(on) = property.on {
        events.push(Event::BooleanPropertyChanged {
            device_id: property.owner.rid.to_string(),
            property_id: PropertyLocator::Name("on".to_string()),
            value: on.on,
            initiated_by,
        });
    }

    if let Some(dimming) = property.dimming {
        events.push(Event::NumberPropertyChanged {
            device_id: property.owner.rid.to_string(),
            property_id: PropertyLocator::Name("brightness".to_string()),
            value: Some(Number::Float(dimming.brightness)),
            initiated_by,
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
                property_id: PropertyLocator::Name("colorTemperature".to_string()),
                value: Some(Number::PositiveInt(clamped.mirek_to_kelvin())),
                initiated_by,
            });
        }
    }

    if let Some(color) = property.color {
        events.push(Event::ColorPropertyChanged {
            device_id: property.owner.rid.to_string(),
            property_id: PropertyLocator::Name("color".to_string()),
            xy: CartesianCoordinate::new(color.xy.x, color.xy.y),
            gamut: color.gamut.map(|mut g| g.take_gamut()),
            initiated_by,
        });
    }

    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Event::{BooleanPropertyChanged, ColorPropertyChanged, NumberPropertyChanged};
    use crate::domain::Number::{Float, PositiveInt};
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

        let result = map_light_changed_property(light_changed, InitiatedBy::Hearth);
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

        let result = map_light_changed_property(light_changed, InitiatedBy::Hearth);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0],
            BooleanPropertyChanged {
                device_id: "84a3be14-5d90-4165-ac64-818b7981bb32".to_string(),
                property_id: PropertyLocator::Name("on".to_string()),
                value: true,
                initiated_by: InitiatedBy::Hearth,
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

        let result = map_light_changed_property(light_changed, InitiatedBy::Hearth);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0],
            NumberPropertyChanged {
                device_id: "84a3be14-5d90-4165-ac64-818b7981bb32".to_string(),
                property_id: PropertyLocator::Name("brightness".to_string()),
                value: Some(Float(20.8)),
                initiated_by: InitiatedBy::Hearth,
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

        let result = map_light_changed_property(light_changed, InitiatedBy::Hearth);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0],
            NumberPropertyChanged {
                device_id: "84a3be14-5d90-4165-ac64-818b7981bb32".to_string(),
                property_id: PropertyLocator::Name("colorTemperature".to_string()),
                value: Some(PositiveInt(6535)),
                initiated_by: InitiatedBy::Hearth,
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

        let result = map_light_changed_property(light_changed, InitiatedBy::Hearth);
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

        let result = map_light_changed_property(light_changed, InitiatedBy::Hearth);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0],
            NumberPropertyChanged {
                device_id: "84a3be14-5d90-4165-ac64-818b7981bb32".to_string(),
                property_id: PropertyLocator::Name("colorTemperature".to_string()),
                value: Some(PositiveInt(expected_kelvin)),
                initiated_by: InitiatedBy::Hearth,
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

        let result = map_light_changed_property(light_changed, InitiatedBy::External);
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
                initiated_by: InitiatedBy::External,
            }
        );
    }

    #[test]
    fn passes_initiated_by_to_every_event() {
        let light_changed = LightChanged {
            id: "42".to_string(),
            owner: Owner { rid: "84a3be14-5d90-4165-ac64-818b7981bb32".to_string(), rtype: "device".to_string() },
            on: Some(On { on: true }),
            dimming: Some(Dimming { brightness: 50.0, min_dim_level: None }),
            color_temperature: Some(ChangedColorTemperature { mirek: Some(300), mirek_valid: true }),
            color: Some(ChangedColor { xy: Xy { x: 0.0, y: 0.0 }, gamut: None }),
        };

        let events = map_light_changed_property(light_changed, InitiatedBy::External);

        assert_eq!(events.len(), 4);
        for event in events {
            let initiated_by = match event {
                BooleanPropertyChanged { initiated_by, .. } | NumberPropertyChanged { initiated_by, .. } | ColorPropertyChanged { initiated_by, .. } => initiated_by,
                other => panic!("unexpected event: {other:?}"),
            };
            assert_eq!(initiated_by, InitiatedBy::External);
        }
    }
}
