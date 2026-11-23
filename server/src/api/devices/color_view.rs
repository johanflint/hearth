use crate::domain::property::{CartesianCoordinate, Gamut};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(super) struct CartesianCoordinateView {
    x: f64,
    y: f64,
}

impl From<&CartesianCoordinate> for CartesianCoordinateView {
    fn from(coordinate: &CartesianCoordinate) -> Self {
        Self { x: coordinate.x(), y: coordinate.y() }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct GamutView {
    red: CartesianCoordinateView,
    green: CartesianCoordinateView,
    blue: CartesianCoordinateView,
}

impl From<&Gamut> for GamutView {
    fn from(gamut: &Gamut) -> Self {
        Self { red: gamut.red().into(), green: gamut.green().into(), blue: gamut.blue().into() }
    }
}
