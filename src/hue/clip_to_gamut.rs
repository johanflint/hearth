use crate::domain::property::{CartesianCoordinate, Gamut};

// Clip the xy coordinate to the given gamut
pub fn clip_to_gamut(coordinate: CartesianCoordinate, gamut: &Gamut) -> CartesianCoordinate {
    if is_in_gamut(&coordinate, gamut) {
        return coordinate;
    }

    // Find the closest point on each edge of the gamut triangle
    let point_red_green = closest_point_on_line(gamut.red(), gamut.green(), &coordinate);
    let point_green_blue = closest_point_on_line(gamut.green(), gamut.blue(), &coordinate);
    let point_blue_red = closest_point_on_line(gamut.blue(), gamut.red(), &coordinate);

    // Calculate distances to each edge
    let distance_red_green = squared_distance(&coordinate, &point_red_green);
    let distance_green_blue = squared_distance(&coordinate, &point_green_blue);
    let distance_blue_red = squared_distance(&coordinate, &point_blue_red);

    // Return the closest point
    if distance_red_green <= distance_green_blue && distance_red_green <= distance_blue_red {
        point_red_green
    } else if distance_green_blue <= distance_blue_red {
        point_green_blue
    } else {
        point_blue_red
    }
}

/// Check if a point is within the gamut triangle
fn is_in_gamut(coordinate: &CartesianCoordinate, gamut: &Gamut) -> bool {
    // Use cross product to check if point is inside triangle
    let v0 = CartesianCoordinate::new(gamut.blue().x() - gamut.red().x(), gamut.blue().y() - gamut.red().y());
    let v1 = CartesianCoordinate::new(gamut.green().x() - gamut.red().x(), gamut.green().y() - gamut.red().y());
    let v2 = CartesianCoordinate::new(coordinate.x() - gamut.red().x(), coordinate.y() - gamut.red().y());

    let dot00 = v0.x() * v0.x() + v0.y() * v0.y();
    let dot01 = v0.x() * v1.x() + v0.y() * v1.y();
    let dot02 = v0.x() * v2.x() + v0.y() * v2.y();
    let dot11 = v1.x() * v1.x() + v1.y() * v1.y();
    let dot12 = v1.x() * v2.x() + v1.y() * v2.y();

    let inv_denom = 1.0 / (dot00 * dot11 - dot01 * dot01);
    let u = (dot11 * dot02 - dot01 * dot12) * inv_denom;
    let v = (dot00 * dot12 - dot01 * dot02) * inv_denom;

    u >= 0.0 && v >= 0.0 && u + v <= 1.0
}

/// Find the closest point on a line segment to a given point
fn closest_point_on_line(a: &CartesianCoordinate, b: &CartesianCoordinate, p: &CartesianCoordinate) -> CartesianCoordinate {
    let ap = CartesianCoordinate::new(p.x() - a.x(), p.y() - a.y());
    let ab = CartesianCoordinate::new(b.x() - a.x(), b.y() - a.y());
    let ab2 = ab.x() * ab.x() + ab.y() * ab.y();
    let ap_ab = ap.x() * ab.x() + ap.y() * ab.y();
    let t = (ap_ab / ab2).max(0.0).min(1.0);
    CartesianCoordinate::new(a.x() + ab.x() * t, a.y() + ab.y() * t)
}

fn squared_distance(a: &CartesianCoordinate, b: &CartesianCoordinate) -> f64 {
    (a.x() - b.x()).powi(2) + (a.y() - b.y()).powi(2)
}
