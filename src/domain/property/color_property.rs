use crate::domain::property::property::ValueKind;
use crate::domain::property::{Property, PropertyError, PropertyType};
use ordered_float::OrderedFloat;
use std::any::Any;
use std::hash::{Hash, Hasher};

#[derive(Clone, PartialEq, Debug)]
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

    // This function does not check the readonly value as the value comes from an observer and the system
    // must be in sync with the observed system.
    pub fn set_value(&mut self, value: CartesianCoordinate, gamut: Option<Gamut>) -> Result<(), PropertyError> {
        self.xy = value;
        if gamut.is_some() {
            self.gamut = gamut;
        }

        Ok(())
    }

    pub fn gamut(&self) -> Option<&Gamut> {
        self.gamut.as_ref()
    }
}

impl Property for ColorProperty {
    fn name(&self) -> &str {
        &self.name
    }

    fn property_type(&self) -> PropertyType {
        self.property_type
    }

    fn value_kind(&self) -> ValueKind {
        ValueKind::Color
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

    fn clone_box(&self) -> Box<dyn Property> {
        Box::new(self.clone())
    }
}

#[derive(Clone, Debug)]
pub struct CartesianCoordinate {
    x: f64,
    y: f64,
}

impl CartesianCoordinate {
    pub fn new(x: f64, y: f64) -> Self {
        CartesianCoordinate { x, y }
    }

    pub fn x(&self) -> f64 {
        self.x
    }

    pub fn y(&self) -> f64 {
        self.y
    }
}

impl PartialEq for CartesianCoordinate {
    fn eq(&self, other: &Self) -> bool {
        OrderedFloat(self.x) == OrderedFloat(other.x) && OrderedFloat(self.y) == OrderedFloat(other.y)
    }
}

impl Eq for CartesianCoordinate {}

impl Hash for CartesianCoordinate {
    fn hash<H: Hasher>(&self, state: &mut H) {
        OrderedFloat(self.x).hash(state);
        OrderedFloat(self.y).hash(state);
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Gamut {
    red: CartesianCoordinate,
    green: CartesianCoordinate,
    blue: CartesianCoordinate,
}

impl Gamut {
    pub fn new(red: CartesianCoordinate, green: CartesianCoordinate, blue: CartesianCoordinate) -> Self {
        Gamut { red, green, blue }
    }

    pub fn red(&self) -> &CartesianCoordinate {
        &self.red
    }

    pub fn green(&self) -> &CartesianCoordinate {
        &self.green
    }

    pub fn blue(&self) -> &CartesianCoordinate {
        &self.blue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::DefaultHasher;

    fn hash_of<T: Hash>(value: &T) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn cartesian_coordinate_equal_values_are_equal_and_hash_equal() {
        let a = CartesianCoordinate::new(0.3, 0.4);
        let b = CartesianCoordinate::new(0.3, 0.4);

        assert_eq!(a, b);
        assert_eq!(hash_of(&a), hash_of(&b));
    }

    #[test]
    fn cartesian_coordinate_different_y_is_not_equal() {
        assert_ne!(CartesianCoordinate::new(0.3, 0.4), CartesianCoordinate::new(0.3, 0.5));
    }

    #[test]
    fn cartesian_coordinate_nan_is_equal_to_itself() {
        let a = CartesianCoordinate::new(f64::NAN, 0.4);

        assert_eq!(a, a.clone());
        assert_eq!(hash_of(&a), hash_of(&a.clone()));
    }

    #[test]
    fn cartesian_coordinate_signed_zeros_are_equal_and_hash_equal() {
        let a = CartesianCoordinate::new(0.0, 0.4);
        let b = CartesianCoordinate::new(-0.0, 0.4);

        assert_eq!(a, b);
        assert_eq!(hash_of(&a), hash_of(&b));
    }
}