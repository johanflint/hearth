use crate::domain::Color;
use serde::{Serialize, Serializer};
use serde_json::json;

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Color::Hex(hex) => serializer.serialize_str(hex),
            Color::RGB(r, g, b) => json!({ "r": r, "g": g, "b": b }).serialize(serializer),
            Color::CIE_xyY { xy, brightness } => {
                json!({ "x": xy.x(), "y": xy.y(), "brightness": brightness }).serialize(serializer)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::property::CartesianCoordinate;
    use rstest::rstest;

    #[rstest]
    #[case::hex(Color::Hex("#ff0000".to_string()), json!("#ff0000"))]
    #[case::rgb(Color::RGB(255, 0, 0), json!({ "r": 255, "g": 0, "b": 0 }))]
    #[case::xy_brightness(Color::CIE_xyY { xy: CartesianCoordinate::new(0.675, 0.322), brightness: 0.2126 }, json!({ "x": 0.675, "y": 0.322, "brightness": 0.2126 }))]
    fn serialize_color_formats(#[case] color: Color, #[case] expected: serde_json::Value) {
        let json = serde_json::to_value(&color).unwrap();

        assert_eq!(json, expected);
        assert_eq!(serde_json::from_value::<Color>(json).unwrap(), color, "must deserialize to the same color");
    }
}
