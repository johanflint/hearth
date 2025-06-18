use crate::domain::property::{Property, PropertyError, PropertyType};
use std::any::Any;
use std::cmp::Ordering;
use std::fmt::{Debug, Display};
use std::ops::{Add, Sub};

#[derive(PartialEq, Debug)]
pub struct NumberProperty {
    name: String,
    property_type: PropertyType,
    readonly: bool,
    external_id: Option<String>,
    unit: Unit,
    value: Number,
    minimum: Option<Number>,
    maximum: Option<Number>,
}

impl NumberProperty {
    pub fn builder(name: String, property_type: PropertyType, readonly: bool) -> NumberPropertyBuilder {
        NumberPropertyBuilder::new(name, property_type, readonly)
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self.value {
            Number::PositiveInt(value) => Some(value),
            Number::NegativeInt(_) | Number::Float(_) => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self.value {
            Number::PositiveInt(n) => {
                if n <= i64::MAX as u64 {
                    Some(n as i64)
                } else {
                    None
                }
            }
            Number::NegativeInt(n) => Some(n),
            Number::Float(_) => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self.value {
            Number::PositiveInt(n) => Some(n as f64),
            Number::NegativeInt(n) => Some(n as f64),
            Number::Float(n) => Some(n),
        }
    }

    pub fn value(&self) -> Number {
        self.value.clone()
    }

    pub fn validate_value(&self, value: Number) -> ValidatedValue {
        if self.readonly {
            return ValidatedValue::Invalid(PropertyError::ReadOnly);
        }

        if let Some(minimum) = &self.minimum {
            if &value < minimum {
                return ValidatedValue::Clamped(minimum.clone(), PropertyError::ValueTooSmall);
            }
        }

        if let Some(maximum) = &self.maximum {
            if &value > maximum {
                return ValidatedValue::Clamped(maximum.clone(), PropertyError::ValueTooLarge);
            }
        }

        ValidatedValue::Valid(value)
    }

    // This function does not validate the value as the value comes from an observer and the system
    // must be in sync with the observed system.
    pub fn set_value(&mut self, value: Number) -> Result<(), PropertyError> {
        if self.readonly {
            return Err(PropertyError::ReadOnly);
        }

        self.value = value;
        Ok(())
    }
}

#[derive(PartialEq, Debug)]
pub enum ValidatedValue {
    Valid(Number),
    Clamped(Number, PropertyError),
    Invalid(PropertyError),
}

impl Property for NumberProperty {
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
        self.value.to_string()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn eq_dyn(&self, other: &dyn Property) -> bool {
        other.as_any().downcast_ref::<NumberProperty>().map_or(false, |o| self == o)
    }
}

pub struct NumberPropertyBuilder {
    name: String,
    property_type: PropertyType,
    readonly: bool,
    external_id: Option<String>,
    unit: Unit,
    value: Number,
    minimum: Option<Number>,
    maximum: Option<Number>,
}

impl NumberPropertyBuilder {
    pub fn new(name: String, property_type: PropertyType, readonly: bool) -> Self {
        NumberPropertyBuilder {
            name,
            property_type,
            readonly,
            external_id: None,
            unit: Unit::Percentage,
            value: Number::PositiveInt(0),
            minimum: None,
            maximum: None,
        }
    }

    pub fn external_id(mut self, id: impl Into<String>) -> Self {
        self.external_id = Some(id.into());
        self
    }

    pub fn unit(mut self, value: Unit) -> Self {
        self.unit = value;
        self
    }

    pub fn positive_int(mut self, value: u64, minimum: Option<u64>, maximum: Option<u64>) -> Self {
        self.value = Number::PositiveInt(value);
        self.minimum = minimum.map(|v| Number::PositiveInt(v));
        self.maximum = maximum.map(|v| Number::PositiveInt(v));
        self
    }

    pub fn negative_int(mut self, value: i64, minimum: Option<i64>, maximum: Option<i64>) -> Self {
        self.value = Number::NegativeInt(value);
        self.minimum = minimum.map(|v| Number::NegativeInt(v));
        self.maximum = maximum.map(|v| Number::NegativeInt(v));
        self
    }

    pub fn float(mut self, value: f64, minimum: Option<f64>, maximum: Option<f64>) -> Self {
        self.value = Number::Float(value);
        self.minimum = minimum.map(|v| Number::Float(v));
        self.maximum = maximum.map(|v| Number::Float(v));
        self
    }

    pub fn build(self) -> NumberProperty {
        NumberProperty {
            name: self.name,
            property_type: self.property_type,
            readonly: self.readonly,
            external_id: self.external_id,
            unit: self.unit,
            value: self.value,
            minimum: self.minimum,
            maximum: self.maximum,
        }
    }
}

#[derive(PartialEq, Debug)]
pub enum Unit {
    Percentage,
    #[allow(dead_code)]
    Lux,
    #[allow(dead_code)]
    DegreesCelsius,
    Kelvin,
}

impl Unit {
    #[allow(dead_code)]
    pub fn symbol(&self) -> &str {
        match self {
            Unit::Percentage => "%",
            Unit::Lux => "l",
            Unit::DegreesCelsius => "°C",
            Unit::Kelvin => "k",
        }
    }
}

#[derive(Clone, Debug)]
pub enum Number {
    PositiveInt(u64),
    NegativeInt(i64),
    Float(f64),
}

impl Number {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Number::PositiveInt(n) => Some(n.clone() as f64),
            Number::NegativeInt(n) => Some(n.clone() as f64),
            Number::Float(n) => Some(n.clone()),
        }
    }
}

impl Add for Number {
    type Output = Number;

    fn add(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            // Integer + Integer
            (Number::PositiveInt(a), Number::PositiveInt(b)) => Number::PositiveInt(a.saturating_add(b)),
            (Number::PositiveInt(a), Number::NegativeInt(b)) => match a as i128 + b as i128 {
                sum if sum >= 0 => Number::PositiveInt(sum as u64),
                sum => Number::NegativeInt(sum as i64),
            },
            (Number::NegativeInt(a), Number::PositiveInt(b)) => match a as i128 + b as i128 {
                sum if sum >= 0 => Number::PositiveInt(sum as u64),
                sum => Number::NegativeInt(sum as i64),
            },
            (Number::NegativeInt(a), Number::NegativeInt(b)) => Number::NegativeInt(a.saturating_add(b)),

            // Float involved
            (Number::Float(a), Number::Float(b)) => Number::Float(a + b),
            (Number::Float(a), Number::PositiveInt(b)) => Number::Float(a + b as f64),
            (Number::Float(a), Number::NegativeInt(b)) => Number::Float(a + b as f64),
            (Number::PositiveInt(a), Number::Float(b)) => Number::Float(a as f64 + b),
            (Number::NegativeInt(a), Number::Float(b)) => Number::Float(a as f64 + b),
        }
    }
}

impl Sub for Number {
    type Output = Number;

    fn sub(self, rhs: Number) -> Number {
        match (self, rhs) {
            // Integer - Integer
            (Number::PositiveInt(a), Number::PositiveInt(b)) => {
                if a >= b {
                    Number::PositiveInt(a - b)
                } else {
                    Number::NegativeInt((a as i128 - b as i128) as i64)
                }
            }
            (Number::PositiveInt(a), Number::NegativeInt(b)) => {
                let sum = a as i128 + b.abs() as i128;
                Number::PositiveInt(sum as u64)
            }
            (Number::NegativeInt(a), Number::PositiveInt(b)) => Number::NegativeInt(a.saturating_sub(b as i64)),
            (Number::NegativeInt(a), Number::NegativeInt(b)) => Number::NegativeInt(a.saturating_sub(b)),

            // Float involved
            (Number::Float(a), Number::Float(b)) => Number::Float(a - b),
            (Number::Float(a), Number::PositiveInt(b)) => Number::Float(a - b as f64),
            (Number::Float(a), Number::NegativeInt(b)) => Number::Float(a - b as f64),
            (Number::PositiveInt(a), Number::Float(b)) => Number::Float(a as f64 - b),
            (Number::NegativeInt(a), Number::Float(b)) => Number::Float(a as f64 - b),
        }
    }
}

impl PartialOrd for Number {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match (self, other) {
            // Integer vs Integer
            (Number::PositiveInt(a), Number::PositiveInt(b)) => a.partial_cmp(b),
            (Number::NegativeInt(a), Number::NegativeInt(b)) => a.partial_cmp(b),
            (Number::PositiveInt(a), Number::NegativeInt(b)) => {
                if *b < 0 {
                    Some(Ordering::Greater)
                } else {
                    a.partial_cmp(&(*b as u64))
                }
            }
            (Number::NegativeInt(a), Number::PositiveInt(b)) => {
                if *a < 0 {
                    Some(Ordering::Less)
                } else {
                    a.partial_cmp(&(*b as i64))
                }
            }
            // Float vs Anything
            (Number::Float(a), Number::Float(b)) => a.partial_cmp(b),
            (Number::Float(a), Number::PositiveInt(b)) => a.partial_cmp(&(*b as f64)),
            (Number::Float(a), Number::NegativeInt(b)) => a.partial_cmp(&(*b as f64)),
            (Number::PositiveInt(a), Number::Float(b)) => (*a as f64).partial_cmp(b),
            (Number::NegativeInt(a), Number::Float(b)) => (*a as f64).partial_cmp(b),
        }
    }
}

impl PartialEq for Number {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Number::PositiveInt(a), Number::PositiveInt(b)) => a == b,
            (Number::NegativeInt(a), Number::NegativeInt(b)) => a == b,
            (Number::Float(a), Number::Float(b)) => a == b,
            (Number::Float(a), Number::PositiveInt(b)) => *a == *b as f64,
            (Number::Float(a), Number::NegativeInt(b)) => *a == *b as f64,
            (Number::PositiveInt(a), Number::Float(b)) => *a as f64 == *b,
            (Number::NegativeInt(a), Number::Float(b)) => *a as f64 == *b,
            _ => false,
        }
    }
}

impl Display for Number {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Number::PositiveInt(n) => write!(f, "{}", n),
            Number::NegativeInt(n) => write!(f, "{}", n),
            Number::Float(n) => write!(f, "{}", n),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    fn builder(readonly: bool) -> NumberPropertyBuilder {
        NumberProperty::builder("brightness".to_string(), PropertyType::Brightness, readonly).unit(Unit::Percentage)
    }

    #[rstest]
    #[case(builder(true).positive_int(42, None, None).build(), Some(42))]
    #[case(builder(true).negative_int(42, None, None).build(), None)]
    #[case(builder(true).float(42.0, None, None).build(), None)]
    fn as_u64(#[case] property: NumberProperty, #[case] expected: Option<u64>) {
        assert_eq!(property.as_u64(), expected);
    }

    #[rstest]
    #[case(builder(true).positive_int(42, None, None).build(), Some(42))]
    #[case(builder(true).positive_int(i64::MAX as u64 + 1, None, None).build(), None)]
    #[case(builder(true).negative_int(42, None, None).build(), Some(42))]
    #[case(builder(true).float(42.0, None, None).build(), None)]
    fn as_i64(#[case] property: NumberProperty, #[case] expected: Option<i64>) {
        assert_eq!(property.as_i64(), expected);
    }

    #[rstest]
    #[case(builder(true).positive_int(42, None, None).build(), Some(42.0))]
    #[case(builder(true).positive_int(i64::MAX as u64 + 1, None, None).build(), Some(i64::MAX as f64 + 1.0))]
    #[case(builder(true).negative_int(42, None, None).build(), Some(42.0))]
    #[case(builder(true).float(42.0, None, None).build(), Some(42.0))]
    fn as_f64(#[case] property: NumberProperty, #[case] expected: Option<f64>) {
        assert_eq!(property.as_f64(), expected);
    }

    #[rstest]
    #[case(Number::PositiveInt(42), Number::PositiveInt(42))]
    #[case(Number::PositiveInt(42), Number::NegativeInt(42))]
    #[case(Number::PositiveInt(42), Number::Float(42.0))]
    #[case(Number::NegativeInt(42), Number::PositiveInt(42))]
    #[case(Number::NegativeInt(-42), Number::NegativeInt(-42))]
    #[case(Number::NegativeInt(42), Number::Float(42.0))]
    #[case(Number::Float(42.0), Number::PositiveInt(42))]
    #[case(Number::Float(42.0), Number::NegativeInt(42))]
    #[case(Number::Float(42.7), Number::Float(42.7))]
    fn compare_equals(#[case] a: Number, #[case] b: Number) {
        assert_eq!(a.partial_cmp(&b), Some(Ordering::Equal));
    }

    #[rstest]
    #[case(Number::PositiveInt(42), Number::PositiveInt(7))]
    #[case(Number::PositiveInt(42), Number::NegativeInt(7))]
    #[case(Number::PositiveInt(42), Number::Float(41.999))]
    #[case(Number::NegativeInt(42), Number::PositiveInt(41))]
    #[case(Number::NegativeInt(-42), Number::NegativeInt(-43))]
    #[case(Number::NegativeInt(42), Number::Float(41.999))]
    #[case(Number::Float(42.1), Number::PositiveInt(42))]
    #[case(Number::Float(42.1), Number::NegativeInt(42))]
    #[case(Number::Float(42.7), Number::Float(42.699))]
    fn compare_greater_than(#[case] a: Number, #[case] b: Number) {
        assert_eq!(a.partial_cmp(&b), Some(Ordering::Greater));
    }

    #[test]
    fn returns_the_value() {
        let property = builder(false).positive_int(42, None, None).build();

        assert!(property.as_i64().is_some());
        assert_eq!(property.as_i64().unwrap(), 42i64);

        assert!(property.as_u64().is_some());
        assert_eq!(property.as_u64().unwrap(), 42u64);

        assert!(property.as_f64().is_some());
        assert_eq!(property.as_f64().unwrap(), 42f64);
    }

    #[rstest]
    #[case(Number::PositiveInt(7))]
    #[case(Number::NegativeInt(-7))]
    #[case(Number::Float(7.0))]
    fn validate_value_returns_valid_if_property_is_editable(#[case] value: Number) {
        let property = builder(false).negative_int(42, Some(-100), Some(100)).build();

        let result = property.validate_value(value.clone());

        assert_eq!(result, ValidatedValue::Valid(value));
    }

    #[rstest]
    #[case(Number::PositiveInt(7))]
    #[case(Number::NegativeInt(-7))]
    #[case(Number::Float(7.0))]
    fn validate_value_returns_invalid_if_property_is_readonly(#[case] value: Number) {
        let property = builder(true).positive_int(42, Some(1), Some(100)).build();

        let result = property.validate_value(value);

        assert_eq!(result, ValidatedValue::Invalid(PropertyError::ReadOnly));
    }

    #[rstest]
    #[case(Number::PositiveInt(7))]
    #[case(Number::NegativeInt(-7))]
    #[case(Number::Float(7.0))]
    fn validate_value_returned_clamped_if_value_is_too_small(#[case] value: Number) {
        let property = builder(false).positive_int(42, Some(10), Some(100)).build();

        let result = property.validate_value(value);

        assert_eq!(result, ValidatedValue::Clamped(Number::PositiveInt(10), PropertyError::ValueTooSmall));
    }

    #[rstest]
    #[case(Number::PositiveInt(42))]
    #[case(Number::NegativeInt(42))]
    #[case(Number::Float(42.7))]
    fn validate_value_returned_clamped_if_value_is_too_large(#[case] value: Number) {
        let property = builder(false).positive_int(42, Some(1), Some(10)).build();

        let result = property.validate_value(value);

        assert_eq!(result, ValidatedValue::Clamped(Number::PositiveInt(10), PropertyError::ValueTooLarge));
    }
}
