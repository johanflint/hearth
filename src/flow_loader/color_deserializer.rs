use crate::domain::color::Color;
use crate::domain::property::CartesianCoordinate;
use serde::de::{Error, Unexpected};
use serde::{Deserialize, Deserializer};

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum RawColor {
            Hex(String),
            Rgb {
                r: u8,
                g: u8,
                b: u8,
            },
            XyY {
                x: f64,
                y: f64,
                brightness: f64,
            },
            #[allow(non_snake_case)]
            XyYAlt {
                x: f64,
                y: f64,
                Y: f64,
            },
        }

        match RawColor::deserialize(deserializer)? {
            RawColor::Hex(s) => {
                let normalized = s.trim_start_matches('#').to_lowercase();
                if normalized.len() == 6 && normalized.chars().all(|c| c.is_ascii_hexdigit()) {
                    Ok(Color::Hex(format!("#{}", normalized)))
                } else {
                    Err(Error::invalid_value(Unexpected::Str(&s), &"a 6-digit hex color"))
                }
            }
            RawColor::Rgb { r, g, b } => Ok(Color::RGB(r, g, b)),
            RawColor::XyY { x, y, brightness } => Ok(Color::CIE_xyY {
                xy: CartesianCoordinate::new(x, y),
                brightness,
            }),
            RawColor::XyYAlt { x, y, Y } => Ok(Color::CIE_xyY {
                xy: CartesianCoordinate::new(x, y),
                brightness: Y,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::valid_hex("#000000", Ok(Color::Hex("#000000".to_string())))]
    #[case::valid_hex_without_hash("000000", Ok(Color::Hex("#000000".to_string())))]
    #[case::valid_hex_mixed_case("#8A7b4C", Ok(Color::Hex("#8a7b4c".to_string())))]
    #[case::invalid_hex("#000", Err(Error::custom("invalid value: string \"#000\", expected a 6-digit hex color")))]
    #[case::invalid_hex("#00000Z", Err(Error::custom("invalid value: string \"#00000Z\", expected a 6-digit hex color")))]
    fn deserializes_hex_values(#[case] json_value: String, #[case] expected: serde_json::Result<Color>) {
        let json = format!(r#""{}""#, json_value);

        let response = serde_json::from_str::<Color>(&json);

        // As serde_json::Error does not implement PartialEq, use debug print for comparison
        assert_eq!(format!("{:#?}", response), format!("{:#?}", expected));
    }

    #[rstest]
    #[case(0, 150, 255, Ok(Color::RGB(0, 150, 255)))]
    fn deserialize_rgb_values(#[case] r: u8, #[case] g: u8, #[case] b: u8, #[case] expected: serde_json::Result<Color>) {
        let json = format!(
            r#"{{
                "r": {},
                "g": {},
                "b": {}
            }}"#,
            r, g, b
        );

        let response = serde_json::from_str::<Color>(&json);

        // As serde_json::Error does not implement PartialEq, use debug print for comparison
        assert_eq!(format!("{:#?}", response), format!("{:#?}", expected));
    }

    #[rstest]
    #[case(0.4, 0.3, 0.9, Ok(Color::CIE_xyY { xy: CartesianCoordinate::new(x, y), brightness}))]
    fn deserialize_xy_brightness_values(#[case] x: f64, #[case] y: f64, #[case] brightness: f64, #[case] expected: serde_json::Result<Color>) {
        let json = format!(
            r#"{{
                "x": {},
                "y": {},
                "brightness": {}
            }}"#,
            x, y, brightness
        );

        let response = serde_json::from_str::<Color>(&json);

        // As serde_json::Error does not implement PartialEq, use debug print for comparison
        assert_eq!(format!("{:#?}", response), format!("{:#?}", expected));
    }

    #[rstest]
    #[case(0.4, 0.3, 0.9, Ok(Color::CIE_xyY { xy: CartesianCoordinate::new(x, y), brightness}))]
    #[allow(non_snake_case)]
    fn deserialize_xy_Y_values(#[case] x: f64, #[case] y: f64, #[case] brightness: f64, #[case] expected: serde_json::Result<Color>) {
        let json = format!(
            r#"{{
                "x": {},
                "y": {},
                "Y": {}
            }}"#,
            x, y, brightness
        );

        let response = serde_json::from_str::<Color>(&json);

        // As serde_json::Error does not implement PartialEq, use debug print for comparison
        assert_eq!(format!("{:#?}", response), format!("{:#?}", expected));
    }
}
