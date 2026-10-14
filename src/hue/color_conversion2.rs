use crate::domain::color::ColorConversionError;
use crate::domain::property::{CartesianCoordinate, Gamut};
use thiserror::Error;

struct Color {}

pub enum ColorSpace {
    SrgbD65,       // For UI display
    HueWideRgbD65, // For the bulbs
}

// Algorithm that Hue uses: https://github.com/johnciech/PhilipsHueSDK/blob/master/ApplicationDesignNotes/RGB%20to%20xy%20Color%20conversion.md
fn rgb_to_xyY(red: u8, green: u8, blue: u8, gamut: Gamut) -> Result<(CartesianCoordinate, f64), ColorConversionError> {
    // Normalize RGB values to 0-1 range
    let r = red as f64 / 255.0;
    let g = green as f64 / 255.0;
    let b = blue as f64 / 255.0;

    let r = gamma_correct(r);
    let g = gamma_correct(g);
    let b = gamma_correct(b);

    // Convert to XYZ color space using Wide RGB D65 conversion
    let X = r * 0.649926 + g * 0.103455 + b * 0.197109;
    let Y = r * 0.234327 + g * 0.743075 + b * 0.022598;
    let Z = r * 0.000000 + g * 0.053077 + b * 1.035763;

    // Convert XYZ to xy chromaticity coordinates
    let sum = X + Y + Z;
    if sum == 0.0 {
        return Ok((CartesianCoordinate::new(0.0, 0.0), 0.0));
    }

    // Gamut correction
    let coordinate = CartesianCoordinate::new(X / sum, Y / sum);
    // let corrected = correct_for_gamut(coordinate, gamut);

    Ok((coordinate, Y))
}

pub fn xy_to_hex(xy: &CartesianCoordinate, brightness: f64, gamut: Gamut, color_space: ColorSpace) -> Result<String, ColorConversionError> {
    let (r, g, b) = xy_to_rgb(xy, brightness, gamut, color_space)?;
    Ok(format!("#{:02x}{:02x}{:02x}", r, g, b))
}

// The light does have a brightness! Maybe make it an option and default to 1.0 or so?
fn xy_to_rgb(xy: &CartesianCoordinate, brightness: f64, gamut: Gamut, color_space: ColorSpace) -> Result<(u8, u8, u8), ColorConversionError> {
    let coordinate = xy;

    // Convert xyY back to XYZ
    let z = 1.0 - coordinate.x - coordinate.y;
    let Y = brightness; // Brightness
    let X = (Y / coordinate.y) * coordinate.x;
    let Z = (Y / coordinate.y) * z;

    // sRGB D65 conversion
    let (mut r, mut g, mut b) = match color_space {
        ColorSpace::HueWideRgbD65 => (
            X * 1.656492 - Y * 0.354851 - Z * 0.255038,
            -X * 0.707196 + Y * 1.655397 + Z * 0.036152,
            X * 0.051713 - Y * 0.121364 + Z * 1.011530,
        ),
        ColorSpace::SrgbD65 => (
            X * 3.2406 - Y * 1.5372 - Z * 0.4986,
            -X * 0.9689 + Y * 1.8758 + Z * 0.0415,
            X * 0.0557 - Y * 0.2040 + Z * 1.0570,
        ),
    };

    // Step 3: Clamp negatives to 0
    let mut r = r.max(0.0);
    let mut g = g.max(0.0);
    let mut b = b.max(0.0);

    // Step 4: Normalize so max = 1.0
    let max_val = r.max(g).max(b);
    if max_val > 1.0 {
        r /= max_val;
        g /= max_val;
        b /= max_val;
    }

    // Step 5: Gamma correction
    let r = gamma_correct_rev(r);
    let g = gamma_correct_rev(g);
    let b = gamma_correct_rev(b);

    // Step 6: Convert to 0-255
    Ok((
        (r * 255.0).round().clamp(0.0, 255.0) as u8,
        (g * 255.0).round().clamp(0.0, 255.0) as u8,
        (b * 255.0).round().clamp(0.0, 255.0) as u8,
    ))
}

// Applies gamma correction according to the Philips Hue spec
fn gamma_correct(channel: f64) -> f64 {
    if channel > 0.04045 { ((channel + 0.055) / 1.055).powf(2.4) } else { channel / 12.92 }
}

// clip_xy_to_gamut
pub fn clip_to_gamut(coordinate: CartesianCoordinate, gamut: &Gamut) -> CartesianCoordinate {
    if is_in_gamut(&coordinate, gamut) {
        return coordinate;
    }

    // Find the closest point on each edge of the gamut triangle
    let p1 = closest_point_on_line(gamut.red(), gamut.green(), &coordinate);
    let p2 = closest_point_on_line(gamut.green(), gamut.blue(), &coordinate);
    let p3 = closest_point_on_line(gamut.blue(), gamut.red(), &coordinate);

    // Calculate distances to each edge
    let d1 = (coordinate.x() - p1.x()).powi(2) + (coordinate.y - p1.y).powi(2);
    let d2 = (coordinate.x - p2.x).powi(2) + (coordinate.y - p2.y).powi(2);
    let d3 = (coordinate.x - p3.x).powi(2) + (coordinate.y - p3.y).powi(2);

    // Return the closest point
    if d1 <= d2 && d1 <= d3 {
        p1
    } else if d2 <= d3 {
        p2
    } else {
        p3
    }
}

/// Check if a point is within the gamut triangle
fn is_in_gamut(coordinate: &CartesianCoordinate, gamut: &Gamut) -> bool {
    // Use cross product to check if point is inside triangle
    let v0 = CartesianCoordinate::new(gamut.blue().x() - gamut.red().x(), gamut.blue().y() - gamut.red().y());
    let v1 = CartesianCoordinate::new(gamut.green().x() - gamut.red().x(), gamut.green().y() - gamut.red().y());
    let v2 = CartesianCoordinate::new(coordinate.x() - gamut.red().x(), coordinate.y - gamut.red().y());

    let dot00 = v0.x * v0.x + v0.y * v0.y;
    let dot01 = v0.x * v1.x + v0.y * v1.y;
    let dot02 = v0.x * v2.x + v0.y * v2.y;
    let dot11 = v1.x * v1.x + v1.y * v1.y;
    let dot12 = v1.x * v2.x + v1.y * v2.y;

    let inv_denom = 1.0 / (dot00 * dot11 - dot01 * dot01);
    let u = (dot11 * dot02 - dot01 * dot12) * inv_denom;
    let v = (dot00 * dot12 - dot01 * dot02) * inv_denom;

    u >= 0.0 && v >= 0.0 && u + v <= 1.0
}

/// Find the closest point on a line segment to a given point
fn closest_point_on_line(a: &CartesianCoordinate, b: &CartesianCoordinate, p: &CartesianCoordinate) -> CartesianCoordinate {
    let ap = CartesianCoordinate::new(p.x - a.x, p.y - a.y);
    let ab = CartesianCoordinate::new(b.x - a.x, b.y - a.y);
    let ab2 = ab.x * ab.x + ab.y * ab.y;
    let ap_ab = ap.x * ab.x + ap.y * ab.y;
    let t = (ap_ab / ab2).max(0.0).min(1.0);
    CartesianCoordinate::new(a.x + ab.x * t, a.y + ab.y * t)
}

fn squared_distance(a: &CartesianCoordinate, b: &CartesianCoordinate) -> f64 {
    (a.x - b.x).powi(2) + (a.y - b.y).powi(2)
}

/// Applies reverse gamma correction to linear RGB
fn gamma_correct_rev(channel: f64) -> f64 {
    if channel <= 0.0031308 {
        channel * 12.92
    } else {
        1.055 * channel.powf(1.0 / 2.4) - 0.055
    }
}

#[derive(Error, Debug)]
pub enum ColorConversionError2 {
    #[error("invalid hexadecimal value '{0}'")]
    InvalidHexFormat(String),
}

// make it test-only?
enum HueGamut {
    /// Gamut A (Original Hue bulbs)
    A,
    /// Gamut B (Hue bulbs with better color reproduction)
    B,
    /// Gamut C (Hue bulbs with best color reproduction)
    C,
}

impl HueGamut {
    fn value(&self) -> Gamut {
        match self {
            HueGamut::A => Gamut::new(
                CartesianCoordinate::new(0.7040, 0.2960),
                CartesianCoordinate::new(0.2151, 0.7106),
                CartesianCoordinate::new(0.1380, 0.0800),
            ),
            HueGamut::B => Gamut::new(
                CartesianCoordinate::new(0.6750, 0.3220),
                CartesianCoordinate::new(0.4090, 0.5180),
                CartesianCoordinate::new(0.1670, 0.0400),
            ),
            HueGamut::C => Gamut::new(
                CartesianCoordinate::new(0.6915, 0.3038),
                CartesianCoordinate::new(0.1700, 0.7000),
                CartesianCoordinate::new(0.1532, 0.0475),
            ),
        }
    }
}

// derive brightness (Y) for conversion
fn derive(brightness: f64) -> f64 {
    brightness / 254.0
}

#[cfg(test)]
mod tests {
    use super::*;

    // #[test]
    // fn test_color_conversion() {
    //     let gamut = HueGamut::C.value();
    //
    //     let (xy, Y) = hex_to_xyY("#ff00ff", gamut.clone()).unwrap();
    //     assert_eq!(xy, CartesianCoordinate::new(0.37643066767381234, 0.17099120886491775));
    //     assert_eq!(Y, 0.256925);
    //
    //     let hex = xy_to_hex(&xy, Y, gamut.clone(), ColorSpace::HueWideRgbD65).unwrap();
    //     assert_eq!(hex, "#ff00ff");
    //     println!("{:?}", xy_to_hex(&xy, Y, gamut.clone(), ColorSpace::HueWideRgbD65,));
    // }
}
