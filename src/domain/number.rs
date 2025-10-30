use ordered_float::OrderedFloat;
use std::cmp::Ordering;
use std::fmt::Display;
use std::hash::{Hash, Hasher};
use std::ops::{Add, Sub};

#[derive(Copy, Clone, Debug)]
pub enum Number {
    PositiveInt(u64),
    NegativeInt(i64),
    Float(f64),
}

impl Hash for Number {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Number::PositiveInt(v) => v.hash(state),
            Number::NegativeInt(v) => v.hash(state),
            Number::Float(v) => OrderedFloat(v.clone()).hash(state),
        }
    }
}

impl Eq for Number {}

impl Number {
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Number::PositiveInt(value) => Some(value.clone()),
            Number::NegativeInt(value) if value.is_positive() => Some(value.clone() as u64),
            _ => None,
        }
    }

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
                sum if sum >= 0 => Number::PositiveInt(a.saturating_add(b as u64)),
                sum => Number::NegativeInt(sum as i64),
            },
            (Number::NegativeInt(a), Number::PositiveInt(b)) => match a as i128 + b as i128 {
                sum if sum >= 0 => Number::PositiveInt(b.saturating_add(a as u64)),
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
            (Number::PositiveInt(a), Number::NegativeInt(b)) => match a as i128 - b as i128 {
                result if result >= 0 => Number::PositiveInt(result as u64),
                result => Number::NegativeInt(result as i64),
            },
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

    #[rstest]
    #[case(Number::PositiveInt(1), Number::PositiveInt(2), Number::PositiveInt(3))]
    #[case(Number::PositiveInt(1), Number::NegativeInt(2), Number::PositiveInt(3))]
    #[case(Number::PositiveInt(1), Number::NegativeInt(-2), Number::NegativeInt(-1))]
    #[case(Number::NegativeInt(1), Number::PositiveInt(2), Number::PositiveInt(3))]
    #[case(Number::NegativeInt(-3), Number::PositiveInt(2), Number::NegativeInt(-1))]
    #[case(Number::NegativeInt(-3), Number::NegativeInt(2), Number::NegativeInt(-1))]
    #[case(Number::Float(3.3), Number::Float(2.0), Number::Float(5.3))]
    #[case(Number::Float(3.3), Number::PositiveInt(2), Number::Float(5.3))]
    #[case(Number::Float(3.3), Number::NegativeInt(2), Number::Float(5.3))]
    #[case(Number::PositiveInt(2), Number::Float(3.3), Number::Float(5.3))]
    #[case(Number::NegativeInt(2), Number::Float(3.3), Number::Float(5.3))]
    // Overflows
    #[case(Number::PositiveInt(u64::MAX), Number::PositiveInt(1), Number::PositiveInt(u64::MAX))]
    #[case(Number::PositiveInt(u64::MAX), Number::NegativeInt(1), Number::PositiveInt(u64::MAX))]
    #[case(Number::PositiveInt(u64::MAX), Number::NegativeInt(i64::MIN), Number::PositiveInt(u64::MAX))]
    #[case(Number::PositiveInt(0), Number::NegativeInt(i64::MIN), Number::NegativeInt(i64::MIN))]
    #[case(Number::NegativeInt(1), Number::PositiveInt(u64::MAX), Number::PositiveInt(u64::MAX))]
    #[case(Number::NegativeInt(0), Number::NegativeInt(i64::MIN), Number::NegativeInt(i64::MIN))]
    fn add(#[case] a: Number, #[case] b: Number, #[case] expected: Number) {
        assert_eq!(a + b, expected);
    }

    #[rstest]
    #[case(Number::PositiveInt(3), Number::PositiveInt(2), Number::PositiveInt(1))]
    #[case(Number::PositiveInt(3), Number::PositiveInt(5), Number::NegativeInt(-2))]
    #[case(Number::PositiveInt(3), Number::NegativeInt(5), Number::NegativeInt(-2))]
    #[case(Number::PositiveInt(3), Number::NegativeInt(1), Number::PositiveInt(2))]
    #[case(Number::NegativeInt(3), Number::PositiveInt(5), Number::NegativeInt(-2))]
    #[case(Number::NegativeInt(3), Number::PositiveInt(1), Number::NegativeInt(2))]
    #[case(Number::NegativeInt(-3), Number::NegativeInt(-3), Number::NegativeInt(0))]
    #[case(Number::Float(3.5), Number::Float(2.0), Number::Float(1.5))]
    #[case(Number::Float(3.5), Number::PositiveInt(2), Number::Float(1.5))]
    #[case(Number::Float(3.5), Number::NegativeInt(2), Number::Float(1.5))]
    #[case(Number::PositiveInt(2), Number::Float(3.5), Number::Float(-1.5))]
    #[case(Number::NegativeInt(2), Number::Float(3.5), Number::Float(-1.5))]
    fn subtract(#[case] a: Number, #[case] b: Number, #[case] expected: Number) {
        assert_eq!(a - b, expected);
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

    #[rstest]
    #[case(Number::PositiveInt(42), "42")]
    #[case(Number::NegativeInt(-42), "-42")]
    #[case(Number::Float(-1.234), "-1.234")]
    fn display(#[case] number: Number, #[case] expected: &str) {
        assert_eq!(format!("{}", number), expected);
    }
}
