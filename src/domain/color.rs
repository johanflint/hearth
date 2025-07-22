use crate::domain::color::Color::{CIE_xyY, Hex, RGB};
use crate::domain::property::CartesianCoordinate;
use thiserror::Error;

#[derive(PartialEq, Clone, Debug)]
pub enum Color {
    RGB(u8, u8, u8),
    Hex(String),
    #[allow(non_camel_case_types)]
    CIE_xyY {
        xy: CartesianCoordinate,
        brightness: f64,
    },
}

impl Color {
    pub fn to_hex(self) -> Color {
        match self {
            RGB(r, g, b) => Hex(format!("#{:02x}{:02x}{:02x}", r, g, b)),
            Hex(_) => self,
            CIE_xyY { xy, brightness } => {
                let (r, g, b) = xyY_to_rgb(&xy, brightness);
                Hex(format!("#{:02x}{:02x}{:02x}", r, g, b))
            }
        }
    }

    pub fn to_rgb(self) -> Result<Color, ColorConversionError> {
        match self {
            RGB(_, _, _) => Ok(self),
            Hex(value) => {
                let (r, g, b) = hex_to_rgb(&value)?;
                Ok(RGB(r, g, b))
            }
            CIE_xyY { xy, brightness } => {
                let (r, g, b) = xyY_to_rgb(&xy, brightness);
                Ok(RGB(r, g, b))
            }
        }
    }

    #[allow(non_snake_case)]
    pub fn to_cie_xyY(self) -> Result<Color, ColorConversionError> {
        match self {
            RGB(r, g, b) => Ok(rgb_to_xyY(r, g, b)),
            Hex(value) => {
                let (r, g, b) = hex_to_rgb(&value)?;
                Ok(rgb_to_xyY(r, g, b))
            }
            CIE_xyY { .. } => Ok(self),
        }
    }
}

#[derive(Error, Debug)]
pub enum ColorConversionError {
    #[error("invalid hexadecimal value '{0}'")]
    InvalidHexFormat(String),
}

#[allow(non_snake_case)]
fn hex_to_rgb(hex: &str) -> Result<(u8, u8, u8), ColorConversionError> {
    let hex = hex.trim_start_matches('#');

    let red = u8::from_str_radix(&hex[0..2], 16).map_err(|_| ColorConversionError::InvalidHexFormat(hex.to_string()))?;
    let green = u8::from_str_radix(&hex[2..4], 16).map_err(|_| ColorConversionError::InvalidHexFormat(hex.to_string()))?;
    let blue = u8::from_str_radix(&hex[4..6], 16).map_err(|_| ColorConversionError::InvalidHexFormat(hex.to_string()))?;

    Ok((red, green, blue))
}

#[allow(non_snake_case)]
fn rgb_to_xyY(red: u8, green: u8, blue: u8) -> Color {
    // Convert to linear RGB
    let r = gamma_correct(red as f64 / 255.0);
    let g = gamma_correct(green as f64 / 255.0);
    let b = gamma_correct(blue as f64 / 255.0);

    // Convert to XYZ using sRGB D65
    let x = r * 0.4124 + g * 0.3576 + b * 0.1805;
    let y = r * 0.2126 + g * 0.7152 + b * 0.0722; // this is luminance
    let z = r * 0.0193 + g * 0.1192 + b * 0.9505;

    let sum = x + y + z;
    let (cx, cy) = if sum == 0.0 { (0.0, 0.0) } else { (x / sum, y / sum) };

    CIE_xyY {
        xy: CartesianCoordinate::new(cx, cy),
        brightness: y,
    }
}

fn gamma_correct(channel: f64) -> f64 {
    if channel > 0.04045 { ((channel + 0.055) / 1.055).powf(2.4) } else { channel / 12.92 }
}

/// Converts xy + Y (luminance) back to gamma-corrected sRGB (0–255).
/// Result is lossless as long as the same gamma and transform are used.
#[allow(non_snake_case)]
fn xyY_to_rgb(xy: &CartesianCoordinate, brightness: f64) -> (u8, u8, u8) {
    let x = xy.x();
    let y = xy.y();
    let z = 1.0 - x - y;

    let X = (brightness / y) * x;
    let Y = brightness;
    let Z = (brightness / y) * z;

    // Convert back to linear sRGB
    let r_lin = X * 3.2406 + Y * -1.5372 + Z * -0.4986;
    let g_lin = X * -0.9689 + Y * 1.8758 + Z * 0.0415;
    let b_lin = X * 0.0557 + Y * -0.2040 + Z * 1.0570;

    // Clamp and gamma-correct
    let r = gamma_correct_rev(r_lin.max(0.0).min(1.0));
    let g = gamma_correct_rev(g_lin.max(0.0).min(1.0));
    let b = gamma_correct_rev(b_lin.max(0.0).min(1.0));

    ((r * 255.0).round() as u8, (g * 255.0).round() as u8, (b * 255.0).round() as u8)
}

fn gamma_correct_rev(channel: f64) -> f64 {
    if channel <= 0.0031308 {
        channel * 12.92
    } else {
        1.055 * channel.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod to_hex {
        use super::*;

        #[test]
        fn from_rgb() {
            assert_eq!(RGB(255, 0, 255).to_hex(), Hex("#ff00ff".to_string()));
            assert_eq!(RGB(50, 100, 150).to_hex(), Hex("#326496".to_string()));
        }

        #[test]
        fn from_hex() {
            assert_eq!(Hex("#ff00ff".to_string()).to_hex(), Hex("#ff00ff".to_string()));
            assert_eq!(Hex("#326496".to_string()).to_hex(), Hex("#326496".to_string()));
        }

        #[test]
        #[allow(non_snake_case)]
        fn from_cie_xyY() {
            assert_eq!(
                CIE_xyY {
                    xy: CartesianCoordinate::new(0.3209201623815967, 0.15415426251691475),
                    brightness: 0.2848
                }
                .to_hex(),
                Hex("#ff00ff".to_string())
            );
        }
    }

    mod to_rgb {
        use super::*;

        #[test]
        fn from_rgb() {
            assert_eq!(RGB(255, 0, 255).to_rgb().unwrap(), RGB(255, 0, 255));
        }

        #[test]
        fn from_hex() {
            assert_eq!(Hex("#ff00ff".to_string()).to_rgb().unwrap(), RGB(255, 0, 255));
            assert_eq!(Hex("#326496".to_string()).to_rgb().unwrap(), RGB(50, 100, 150));
        }

        #[test]
        #[allow(non_snake_case)]
        fn from_cie_xyY() {
            assert_eq!(
                CIE_xyY {
                    xy: CartesianCoordinate::new(0.32092016238159676, 0.15415426251691475),
                    brightness: 0.2848
                }
                .to_rgb()
                .unwrap(),
                RGB(255, 0, 255)
            );
        }
    }

    mod to_cie_xyy {
        use super::*;

        #[test]
        fn from_rgb() {
            assert_eq!(
                RGB(255, 0, 255).to_cie_xyY().unwrap(),
                CIE_xyY {
                    xy: CartesianCoordinate::new(0.32092016238159676, 0.15415426251691475),
                    brightness: 0.2848
                }
            );
        }

        #[test]
        fn from_hex() {
            assert_eq!(
                Hex("#ff00ff".to_string()).to_cie_xyY().unwrap(),
                CIE_xyY {
                    xy: CartesianCoordinate::new(0.32092016238159676, 0.15415426251691475),
                    brightness: 0.2848
                }
            );
        }

        #[test]
        #[allow(non_snake_case)]
        fn from_cie_xyY() {
            assert_eq!(
                CIE_xyY {
                    xy: CartesianCoordinate::new(0.3209201623815967, 0.15415426251691475),
                    brightness: 0.2848
                }
                .to_cie_xyY()
                .unwrap(),
                CIE_xyY {
                    xy: CartesianCoordinate::new(0.3209201623815967, 0.15415426251691475),
                    brightness: 0.2848
                }
            );
        }
    }
}
