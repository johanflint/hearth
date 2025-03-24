use crate::domain::property::{Property, PropertyType};
use std::any::Any;

#[derive(PartialEq, Debug)]
pub struct ColorProperty {
    name: String,
    property_type: PropertyType,
    readonly: bool,
    external_id: Option<String>,
    xy: CartesianCoordinate,
    gamut: Option<Gamut>,
}

impl ColorProperty {
    pub fn new(name: String, property_type: PropertyType, readonly: bool, external_id: Option<String>, xy: CartesianCoordinate, gamut: Option<Gamut>) -> Self {
        ColorProperty {
            name,
            property_type,
            readonly,
            external_id,
            xy,
            gamut,
        }
    }
}

impl Property for ColorProperty {
    fn name(&self) -> &str {
        &self.name
    }

    fn property_type(&self) -> PropertyType {
        self.property_type
    }

    fn readonly(&self) -> bool {
        self.readonly
    }

    fn external_id(&self) -> Option<&str> {
        self.external_id.as_deref()
    }

    fn value_string(&self) -> String {
        format!("CIE XY {{ x: {}, y: {} }}", self.xy.x, self.xy.y)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn eq_dyn(&self, other: &dyn Property) -> bool {
        other.as_any().downcast_ref::<ColorProperty>().map_or(false, |o| self == o)
    }
}

#[derive(PartialEq, Debug)]
pub struct CartesianCoordinate {
    x: f64,
    y: f64,
}

impl CartesianCoordinate {
    pub fn new(x: f64, y: f64) -> Self {
        CartesianCoordinate { x, y }
    }
}

#[derive(PartialEq, Debug)]
pub struct Gamut {
    red: CartesianCoordinate,
    green: CartesianCoordinate,
    blue: CartesianCoordinate,
}

impl Gamut {
    pub fn new(red: CartesianCoordinate, green: CartesianCoordinate, blue: CartesianCoordinate) -> Self {
        Gamut { red, green, blue }
    }
}
