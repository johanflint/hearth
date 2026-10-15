use crate::domain::Device;
use crate::domain::property::{ColorProperty, NumberProperty, PropertyType};
use crate::extensions::unsigned_ints_ext::MirekConversions;
use crate::hue::domain::LightChanged;

// Differences too small to see. Hue's corrections after a scene recall were at most 3 mirek and 0.0022 xy.
// When recalling a scene, the Hue bridge immediately reports the scene's values. It later seems to
// read the device's real value and reports that. These changes should never claim the device.
const UNNOTICEABLE_MIREK: u64 = 5;
const UNNOTICEABLE_XY: f64 = 0.005;

pub fn is_unnoticeable_correction(change: &LightChanged, device: &Device) -> bool {
    if change.on.is_some() || change.dimming.is_some() {
        return false;
    }

    let color_temperature_unnoticeable = match change.color_temperature.as_ref().and_then(|color_temperature| color_temperature.mirek) {
        Some(mirek) => current_mirek(device).is_some_and(|current| current.abs_diff(mirek) <= UNNOTICEABLE_MIREK),
        None => true,
    };

    let color_unnoticeable = match &change.color {
        Some(color) => device.get_property_of_type::<ColorProperty>(PropertyType::Color).is_some_and(|current| {
            (current.xy().x() - color.xy.x).abs() <= UNNOTICEABLE_XY && (current.xy().y() - color.xy.y).abs() <= UNNOTICEABLE_XY
        }),
        None => true, // Not part of this change
    };

    color_unnoticeable && color_temperature_unnoticeable
}

fn current_mirek(device: &Device) -> Option<u64> {
    device.get_property_of_type::<NumberProperty>(PropertyType::ColorTemperature)
        .and_then(|property| property.value())
        .and_then(|kelvin| kelvin.as_u64())
        .map(|kelvin| kelvin.kelvin_to_mirek())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::property::{CartesianCoordinate, Property};
    use crate::test_support::DeviceBuilder;
    use rstest::rstest;
    use serde_json::json;

    // The light after recalling the Concentrate scene
    fn device() -> Device {
        let color_temperature: Box<dyn Property> = Box::new(
            NumberProperty::builder("colorTemperature".to_string(), PropertyType::ColorTemperature, false)
                .positive_int(Some(233.mirek_to_kelvin()), Some(2000), Some(6535))
                .build(),
        );
        DeviceBuilder::new("device_id")
            .with_properties(vec![color_temperature])
            .with_color_property("color", CartesianCoordinate::new(0.3691, 0.3719), None)
            .build()
    }

    fn change(data: serde_json::Value) -> LightChanged {
        let mut value = json!({ "id": "light_id", "owner": { "rid": "device_id", "rtype": "device" } });
        value.as_object_mut().unwrap().extend(data.as_object().unwrap().clone());
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn the_correction_after_a_scene_recall_is_unnoticeable() {
        let change = change(json!({ "color_temperature": { "mirek": 230, "mirek_valid": true }, "color": { "xy": { "x": 0.3669, "y": 0.3706 } } }));

        assert!(is_unnoticeable_correction(&change, &device()));
    }

    #[rstest]
    #[case::within_the_limit(228, true)]
    #[case::beyond_the_limit(227, false)]
    fn a_color_temperature_is_unnoticeable_up_to_the_limit(#[case] mirek: u64, #[case] expected: bool) {
        let change = change(json!({ "color_temperature": { "mirek": mirek, "mirek_valid": true } }));

        assert_eq!(is_unnoticeable_correction(&change, &device()), expected);
    }

    #[rstest]
    #[case::within_the_limit(0.3651, true)]
    #[case::beyond_the_limit(0.3631, false)]
    fn a_color_is_unnoticeable_up_to_the_limit(#[case] x: f64, #[case] expected: bool) {
        let change = change(json!({ "color": { "xy": { "x": x, "y": 0.3719 } } }));

        assert_eq!(is_unnoticeable_correction(&change, &device()), expected);
    }

    #[test]
    fn a_noticeable_color_makes_the_whole_change_noticeable() {
        let change = change(json!({ "color_temperature": { "mirek": 233, "mirek_valid": true }, "color": { "xy": { "x": 0.5, "y": 0.4 } } }));

        assert!(!is_unnoticeable_correction(&change, &device()));
    }

    #[test]
    fn turning_a_light_on_or_off_is_never_a_correction() {
        let change = change(json!({ "on": { "on": true }, "color_temperature": { "mirek": 233, "mirek_valid": true } }));

        assert!(!is_unnoticeable_correction(&change, &device()));
    }

    #[test]
    fn dimming_a_light_is_never_a_correction() {
        let change = change(json!({ "dimming": { "brightness": 50.0 }, "color_temperature": { "mirek": 233, "mirek_valid": true } }));

        assert!(!is_unnoticeable_correction(&change, &device()));
    }

    #[test]
    fn a_light_without_a_known_color_temperature_is_not_corrected() {
        let device = DeviceBuilder::new("device_id").build();
        let change = change(json!({ "color_temperature": { "mirek": 233, "mirek_valid": true } }));

        assert!(!is_unnoticeable_correction(&change, &device));
    }

    #[test]
    fn a_light_without_a_known_color_is_not_corrected() {
        let device = DeviceBuilder::new("device_id").build();
        let change = change(json!({ "color": { "xy": { "x": 0.3691, "y": 0.3719 } } }));

        assert!(!is_unnoticeable_correction(&change, &device));
    }
}