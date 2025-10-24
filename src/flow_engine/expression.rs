use crate::domain::property::{BooleanProperty, NumberProperty, PropertyType};
use crate::domain::{Number, Weekday};
use crate::extensions::date_time_ext::ToWeekday;
use crate::flow_engine::Context;
use crate::flow_engine::expression::ExpressionError::UnknownProperty;
use Weekday::*;
use chrono::NaiveTime;
use serde::Deserialize;
use std::cmp::Ordering;
use thiserror::Error;
use tracing::warn;

#[derive(PartialEq, Deserialize, Debug)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Expression {
    // Comparison
    GreaterThanOrEqualTo { lhs: Box<Expression>, rhs: Box<Expression> },
    GreaterThan { lhs: Box<Expression>, rhs: Box<Expression> },
    LessThan { lhs: Box<Expression>, rhs: Box<Expression> },
    LessThanOrEqualTo { lhs: Box<Expression>, rhs: Box<Expression> },

    // Equality
    EqualTo { lhs: Box<Expression>, rhs: Box<Expression> },
    NotEqualTo { lhs: Box<Expression>, rhs: Box<Expression> },

    // Logic
    And { lhs: Box<Expression>, rhs: Box<Expression> },
    Or { lhs: Box<Expression>, rhs: Box<Expression> },
    Not { expression: Box<Expression> },

    // Literal
    Literal { value: Value },

    // Property
    PropertyValue { device_id: String, property_id: String },

    // Temporal
    Temporal { expression: TemporalExpression },
}

#[derive(PartialEq, Debug, Clone)]
pub enum Value {
    Boolean(bool),
    Number(Number),
    None,
}

#[derive(PartialEq, Deserialize, Debug)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TemporalExpression {
    IsToday { when: WeekdayCondition },
    IsBeforeTime { time: Time },
    IsAfterTime { time: Time },
    HasSunRisen, // Now >= sunrise
    HasSunSet,   // Now >= sunset
    IsDaytime,   // Now between sunrise and sunset
    IsNighttime, // Now < sunrise or now > sunset
}

#[derive(Clone, PartialEq, Debug)]
pub enum WeekdayCondition {
    Specific(Weekday),
    Range { start: Weekday, end: Weekday },
    Set(Vec<Weekday>),
    Weekdays,
    Weekend,
}

impl WeekdayCondition {
    pub fn included_days(&self) -> Vec<Weekday> {
        match self {
            WeekdayCondition::Specific(day) => vec![day.clone()],
            WeekdayCondition::Range { start, end } => {
                let all = Weekday::all();
                let start_index = start.as_index();
                let end_index = end.as_index();

                all[start_index..=end_index].to_vec()
            }
            WeekdayCondition::Set(days) => days.clone(),
            WeekdayCondition::Weekdays => vec![Monday, Tuesday, Wednesday, Thursday, Friday],
            WeekdayCondition::Weekend => vec![Saturday, Sunday],
        }
    }
}

#[derive(PartialEq, Debug, Clone)]
pub struct Time {
    hour: u8,
    minute: u8,
}

impl Time {
    pub fn new(hour: u8, minute: u8) -> Self {
        Self { hour, minute }
    }
}

pub fn evaluate(expression: &Expression, context: &Context) -> Result<Value, ExpressionError> {
    use Expression::*;

    match expression {
        // Comparison
        GreaterThanOrEqualTo { lhs, rhs } => compare(lhs, rhs, |o| o != Ordering::Less, context),
        GreaterThan { lhs, rhs } => compare(lhs, rhs, |o| o == Ordering::Greater, context),
        LessThan { lhs, rhs } => compare(lhs, rhs, |o| o == Ordering::Less, context),
        LessThanOrEqualTo { lhs, rhs } => compare(lhs, rhs, |o| o != Ordering::Greater, context),

        // Equality
        EqualTo { lhs, rhs } => match (evaluate(lhs, context)?, evaluate(rhs, context)?) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Boolean(a.eq(&b))),
            (Value::Boolean(a), Value::Boolean(b)) => Ok(Value::Boolean(a == b)),
            (Value::None, Value::None) => Ok(Value::Boolean(true)),
            _ => Err(ExpressionError::OperandTypeMismatch {
                operand: "EqualTo",
                expected: "Boolean|Number",
                actual_lhs: format!("{:?}", lhs),
                actual_rhs: format!("{:?}", rhs),
            }),
        },
        NotEqualTo { lhs, rhs } => match (evaluate(lhs, context)?, evaluate(rhs, context)?) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Boolean(!a.eq(&b))),
            (Value::Boolean(a), Value::Boolean(b)) => Ok(Value::Boolean(a != b)),
            (Value::None, Value::None) => Ok(Value::Boolean(false)),
            _ => Err(ExpressionError::OperandTypeMismatch {
                operand: "NotEqualTo",
                expected: "Boolean|Number",
                actual_lhs: format!("{:?}", lhs),
                actual_rhs: format!("{:?}", rhs),
            }),
        },

        // Logic
        And { lhs, rhs } => match (evaluate(lhs, context)?, evaluate(rhs, context)?) {
            (Value::Boolean(a), Value::Boolean(b)) => Ok(Value::Boolean(a && b)),
            _ => Err(ExpressionError::OperandTypeMismatch {
                operand: "And",
                expected: "Boolean",
                actual_lhs: format!("{:?}", lhs),
                actual_rhs: format!("{:?}", rhs),
            }),
        },
        Or { lhs, rhs } => match (evaluate(lhs, context)?, evaluate(rhs, context)?) {
            (Value::Boolean(a), Value::Boolean(b)) => Ok(Value::Boolean(a || b)),
            _ => Err(ExpressionError::OperandTypeMismatch {
                operand: "Or",
                expected: "Boolean",
                actual_lhs: format!("{:?}", lhs),
                actual_rhs: format!("{:?}", rhs),
            }),
        },
        Not { expression } => match evaluate(expression, context)? {
            Value::Boolean(b) => Ok(Value::Boolean(!b)),
            _ => Err(ExpressionError::UnaryOperandTypeMismatch {
                operand: "Not",
                expected: "Boolean",
                actual: format!("{:?}", expression),
            }),
        },

        // Literal
        Literal { value } => Ok(value.clone()),

        // Property
        PropertyValue { device_id, property_id } => {
            let Some(device) = context.snapshot().devices.get(device_id) else {
                warn!(device_id, "⚠️ Received property changed event for unknown device '{}'", device_id);
                return Err(ExpressionError::UnknownDevice(device_id.clone()));
            };

            let Some(property) = device.properties.get(property_id) else {
                warn!(device_id = device.id, "⚠️ Unknown property '{}' for device '{}'", property_id, device.name);
                return Err(UnknownProperty {
                    device_id: device_id.clone(),
                    property_id: property_id.clone(),
                });
            };

            match property.property_type() {
                PropertyType::Brightness => {
                    let number_property = property.as_any().downcast_ref::<NumberProperty>().unwrap();
                    Ok(number_property.value().map(Value::Number).unwrap_or(Value::None))
                }
                PropertyType::Color => Err(ExpressionError::UnsupportedPropertyType(property.property_type())),
                PropertyType::ColorTemperature => Err(ExpressionError::UnsupportedPropertyType(property.property_type())),
                PropertyType::On => {
                    let value = property.as_any().downcast_ref::<BooleanProperty>().unwrap();
                    Ok(Value::Boolean(value.value()))
                }
            }
        }

        // Temporal
        Temporal { expression } => {
            let now = context.now();

            match expression {
                TemporalExpression::IsToday { when } => {
                    let included_days = when.included_days();
                    let matches = included_days.contains(&now.to_weekday());
                    Ok(Value::Boolean(matches))
                }
                TemporalExpression::IsBeforeTime { time } => Ok(Value::Boolean(
                    NaiveTime::from_hms_opt(time.hour as u32, time.minute as u32, 0)
                        .map(|target| now.time() < target)
                        .unwrap_or(false),
                )),
                TemporalExpression::IsAfterTime { time } => Ok(Value::Boolean(
                    NaiveTime::from_hms_opt(time.hour as u32, time.minute as u32, 0)
                        .map(|target| now.time() > target)
                        .unwrap_or(false),
                )),
                TemporalExpression::HasSunRisen => Ok(Value::Boolean(now.time() >= context.sunrise().time())),
                TemporalExpression::HasSunSet => Ok(Value::Boolean(now.time() >= context.sunset().time())),
                TemporalExpression::IsDaytime => {
                    let is_daytime = now.time() >= context.sunrise().time() && now.time() < context.sunset().time();
                    Ok(Value::Boolean(is_daytime))
                }
                TemporalExpression::IsNighttime => {
                    let is_nighttime = now.time() < context.sunrise().time() || now.time() >= context.sunset().time();
                    Ok(Value::Boolean(is_nighttime))
                }
            }
        }
    }
}

fn compare(lhs: &Expression, rhs: &Expression, cmp: fn(Ordering) -> bool, context: &Context) -> Result<Value, ExpressionError> {
    match (evaluate(lhs, context)?, evaluate(rhs, context)?) {
        (Value::Number(a), Value::Number(b)) => Ok(Value::Boolean(cmp(a.partial_cmp(&b).ok_or_else(|| ExpressionError::ComparisonFailed {
            actual_lhs: format!("{:?}", lhs),
            actual_rhs: format!("{:?}", rhs),
        })?))),
        _ => Err(ExpressionError::OperandTypeMismatch {
            operand: "Compare",
            expected: "Number",
            actual_lhs: format!("{:?}", lhs),
            actual_rhs: format!("{:?}", rhs),
        }),
    }
}

#[derive(Error, PartialEq, Debug)]
pub enum ExpressionError {
    #[error("operand type mismatch for operand {operand}, expected {expected}, but got {actual_lhs} and {actual_rhs}")]
    OperandTypeMismatch {
        operand: &'static str,
        expected: &'static str,
        actual_lhs: String,
        actual_rhs: String,
    },
    #[error("operand type mismatch for operand {operand}, expected {expected} but got {actual}")]
    UnaryOperandTypeMismatch { operand: &'static str, expected: &'static str, actual: String },
    #[error("unknown device '{0}'")]
    UnknownDevice(String),
    #[error("unknown property '{property_id}' for device '{device_id}'")]
    UnknownProperty { device_id: String, property_id: String },
    #[error("property type '{0:?}' is not supported as a property value")]
    UnsupportedPropertyType(PropertyType),
    #[error("unable to compare given Numbers {actual_lhs} and {actual_rhs}")]
    ComparisonFailed { actual_lhs: String, actual_rhs: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::GeoLocation;
    use crate::domain::device::{Device, DeviceType};
    use crate::domain::property::{CartesianCoordinate, ColorProperty, Gamut, Property, Unit};
    use crate::flow_engine::expression::Expression::*;
    use crate::flow_engine::expression::ExpressionError::{OperandTypeMismatch, UnaryOperandTypeMismatch};
    use crate::flow_engine::expression::TemporalExpression::{HasSunRisen, HasSunSet, IsAfterTime, IsBeforeTime, IsDaytime, IsNighttime, IsToday};
    use crate::store::{DeviceMap, StoreSnapshot};
    use chrono::{Local, TimeZone};
    use rstest::rstest;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn context() -> Context {
        Context::default()
    }

    fn device() -> Device {
        let on_property: Box<dyn Property> = Box::new(BooleanProperty::new(
            "on".to_string(),
            PropertyType::On,
            false,
            Some("43e4f3a7-8b35-4b0c-a2ba-e6ca8f4c099b".to_string()),
            true,
        ));

        let brightness_property: Box<dyn Property> = Box::new(
            NumberProperty::builder("brightness".to_string(), PropertyType::Brightness, false)
                .external_id("43e4f3a7-8b35-4b0c-a2ba-e6ca8f4c099b".to_string())
                .unit(Unit::Percentage)
                .float(58.89, Some(2.0), Some(100.0))
                .build(),
        );

        let color_temperature_property: Box<dyn Property> = Box::new(
            NumberProperty::builder("colorTemperature".to_string(), PropertyType::ColorTemperature, false)
                .external_id("43e4f3a7-8b35-4b0c-a2ba-e6ca8f4c099b".to_string())
                .unit(Unit::Kelvin)
                .positive_int(6535, Some(2000), Some(6535))
                .build(),
        );

        let color_property: Box<dyn Property> = Box::new(ColorProperty::new(
            "color".to_string(),
            PropertyType::Color,
            false,
            Some("43e4f3a7-8b35-4b0c-a2ba-e6ca8f4c099b".to_string()),
            CartesianCoordinate::new(0.4851, 0.4331),
            Some(Gamut::new(
                CartesianCoordinate::new(0.675, 0.322),
                CartesianCoordinate::new(0.409, 0.518),
                CartesianCoordinate::new(0.167, 0.04),
            )),
        ));

        Device {
            id: "ab917a9a-a7d5-4853-9518-75909236a182".to_string(),
            r#type: DeviceType::Light,
            manufacturer: "Signify Netherlands B.V.".to_string(),
            model_id: "LCT007".to_string(),
            product_name: "Hue color lamp".to_string(),
            name: "Lamp".to_string(),
            properties: HashMap::from([
                (on_property.name().to_string(), on_property),
                (brightness_property.name().to_string(), brightness_property),
                (color_temperature_property.name().to_string(), color_temperature_property),
                (color_property.name().to_string(), color_property),
            ]),
            external_id: None,
            address: None,
            controller_id: Some("hue"),
        }
    }

    #[rstest]
    #[case(Number::PositiveInt(5), Number::PositiveInt(2), true)]
    #[case(Number::PositiveInt(2), Number::PositiveInt(5), false)]
    #[case(Number::PositiveInt(5), Number::PositiveInt(5), true)]
    #[case(Number::NegativeInt(5), Number::PositiveInt(2), true)]
    #[case(Number::NegativeInt(5), Number::NegativeInt(5), true)]
    #[case(Number::NegativeInt(5), Number::Float(5.0), true)]
    #[case(Number::Float(5.1), Number::Float(5.0), true)]
    #[case(Number::Float(4.999), Number::Float(5.0), false)]
    #[case(Number::Float(5.1), Number::Float(5.0), true)]
    #[case(Number::Float(5.1), Number::Float(5.1), true)]
    fn greater_than_or_equal_to(#[case] lhs: Number, #[case] rhs: Number, #[case] expected: bool) {
        let result = evaluate(
            &GreaterThanOrEqualTo {
                lhs: Box::new(Literal { value: Value::Number(lhs) }),
                rhs: Box::new(Literal { value: Value::Number(rhs) }),
            },
            &context(),
        )
        .unwrap();
        assert_eq!(result, Value::Boolean(expected));
    }

    #[rstest]
    #[case(Number::PositiveInt(5), Number::PositiveInt(2), true)]
    #[case(Number::PositiveInt(2), Number::PositiveInt(5), false)]
    #[case(Number::PositiveInt(5), Number::PositiveInt(5), false)]
    #[case(Number::NegativeInt(5), Number::PositiveInt(2), true)]
    #[case(Number::NegativeInt(5), Number::NegativeInt(5), false)]
    #[case(Number::NegativeInt(5), Number::Float(5.0), false)]
    #[case(Number::Float(5.1), Number::Float(5.0), true)]
    #[case(Number::Float(4.999), Number::Float(5.0), false)]
    #[case(Number::Float(5.1), Number::Float(5.0), true)]
    #[case(Number::Float(5.0), Number::Float(5.0), false)]
    fn greater_than(#[case] lhs: Number, #[case] rhs: Number, #[case] expected: bool) {
        let result = evaluate(
            &GreaterThan {
                lhs: Box::new(Literal { value: Value::Number(lhs) }),
                rhs: Box::new(Literal { value: Value::Number(rhs) }),
            },
            &context(),
        )
        .unwrap();
        assert_eq!(result, Value::Boolean(expected));
    }

    #[rstest]
    #[case(Number::PositiveInt(2), Number::PositiveInt(5), true)]
    #[case(Number::PositiveInt(5), Number::PositiveInt(2), false)]
    #[case(Number::PositiveInt(5), Number::PositiveInt(5), false)]
    #[case(Number::NegativeInt(2), Number::PositiveInt(5), true)]
    #[case(Number::NegativeInt(5), Number::NegativeInt(5), false)]
    #[case(Number::NegativeInt(5), Number::Float(5.0), false)]
    #[case(Number::Float(5.0), Number::Float(5.1), true)]
    #[case(Number::Float(4.999), Number::Float(5.0), true)]
    #[case(Number::Float(5.1), Number::Float(5.0), false)]
    #[case(Number::Float(5.0), Number::Float(5.0), false)]
    fn less_than(#[case] lhs: Number, #[case] rhs: Number, #[case] expected: bool) {
        let result = evaluate(
            &LessThan {
                lhs: Box::new(Literal { value: Value::Number(lhs) }),
                rhs: Box::new(Literal { value: Value::Number(rhs) }),
            },
            &context(),
        )
        .unwrap();
        assert_eq!(result, Value::Boolean(expected));
    }

    #[rstest]
    #[case(Number::PositiveInt(2), Number::PositiveInt(5), true)]
    #[case(Number::PositiveInt(5), Number::PositiveInt(2), false)]
    #[case(Number::PositiveInt(5), Number::PositiveInt(5), true)]
    #[case(Number::NegativeInt(2), Number::PositiveInt(5), true)]
    #[case(Number::NegativeInt(5), Number::NegativeInt(5), true)]
    #[case(Number::NegativeInt(5), Number::Float(5.0), true)]
    #[case(Number::Float(5.0), Number::Float(5.1), true)]
    #[case(Number::Float(4.999), Number::Float(5.0), true)]
    #[case(Number::Float(5.1), Number::Float(5.0), false)]
    #[case(Number::Float(5.0), Number::Float(5.0), true)]
    fn less_than_or_equal_to(#[case] lhs: Number, #[case] rhs: Number, #[case] expected: bool) {
        let result = evaluate(
            &LessThanOrEqualTo {
                lhs: Box::new(Literal { value: Value::Number(lhs) }),
                rhs: Box::new(Literal { value: Value::Number(rhs) }),
            },
            &context(),
        )
        .unwrap();
        assert_eq!(result, Value::Boolean(expected));
    }

    #[rstest]
    #[case(Number::PositiveInt(2), Number::PositiveInt(5), false)]
    #[case(Number::PositiveInt(5), Number::PositiveInt(2), false)]
    #[case(Number::PositiveInt(5), Number::PositiveInt(5), true)]
    #[case(Number::NegativeInt(2), Number::PositiveInt(5), false)]
    #[case(Number::NegativeInt(5), Number::NegativeInt(5), true)]
    #[case(Number::NegativeInt(5), Number::Float(5.0), true)]
    #[case(Number::Float(5.0), Number::Float(5.1), false)]
    #[case(Number::Float(4.999), Number::Float(5.0), false)]
    #[case(Number::Float(5.1), Number::Float(5.0), false)]
    #[case(Number::Float(5.0), Number::Float(5.0), true)]
    fn equal_to(#[case] lhs: Number, #[case] rhs: Number, #[case] expected: bool) {
        let result = evaluate(
            &EqualTo {
                lhs: Box::new(Literal { value: Value::Number(lhs) }),
                rhs: Box::new(Literal { value: Value::Number(rhs) }),
            },
            &context(),
        )
        .unwrap();
        assert_eq!(result, Value::Boolean(expected));
    }

    #[rstest]
    #[case(Value::None, Value::None, true)]
    fn equal_to_none(#[case] lhs: Value, #[case] rhs: Value, #[case] expected: bool) {
        let result = evaluate(
            &EqualTo {
                lhs: Box::new(Literal { value: lhs }),
                rhs: Box::new(Literal { value: rhs }),
            },
            &context(),
        )
        .unwrap();

        assert_eq!(result, Value::Boolean(expected));
    }

    #[rstest]
    #[case(true, true, true)]
    #[case(true, false, false)]
    #[case(false, true, false)]
    #[case(false, false, true)]
    fn equal_to_bool(#[case] lhs: bool, #[case] rhs: bool, #[case] expected: bool) {
        let result = evaluate(
            &EqualTo {
                lhs: Box::new(Literal { value: Value::Boolean(lhs) }),
                rhs: Box::new(Literal { value: Value::Boolean(rhs) }),
            },
            &context(),
        )
        .unwrap();
        assert_eq!(result, Value::Boolean(expected));
    }

    #[rstest]
    #[case(Value::Boolean(true), Value::Number(Number::PositiveInt(2)), OperandTypeMismatch {
                operand: "EqualTo",
                expected: "Boolean|Number",
                actual_lhs: "Literal { value: Boolean(true) }".to_string(),
                actual_rhs: "Literal { value: Number(PositiveInt(2)) }".to_string(),
        })]
    #[case(Value::None, Value::Number(Number::PositiveInt(2)), OperandTypeMismatch {
                operand: "EqualTo",
                expected: "Boolean|Number",
                actual_lhs: "Literal { value: None }".to_string(),
                actual_rhs: "Literal { value: Number(PositiveInt(2)) }".to_string(),
        })]
    fn equal_to_mismatch(#[case] lhs: Value, #[case] rhs: Value, #[case] expected: ExpressionError) {
        let result = evaluate(
            &EqualTo {
                lhs: Box::new(Literal { value: lhs }),
                rhs: Box::new(Literal { value: rhs }),
            },
            &context(),
        )
        .unwrap_err();
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case(Number::PositiveInt(2), Number::PositiveInt(5), true)]
    #[case(Number::PositiveInt(5), Number::PositiveInt(2), true)]
    #[case(Number::PositiveInt(5), Number::PositiveInt(5), false)]
    #[case(Number::NegativeInt(2), Number::PositiveInt(5), true)]
    #[case(Number::NegativeInt(5), Number::NegativeInt(5), false)]
    #[case(Number::NegativeInt(5), Number::Float(5.0), false)]
    #[case(Number::Float(5.0), Number::Float(5.1), true)]
    #[case(Number::Float(4.999), Number::Float(5.0), true)]
    #[case(Number::Float(5.1), Number::Float(5.0), true)]
    #[case(Number::Float(5.0), Number::Float(5.0), false)]
    fn not_equal_to(#[case] lhs: Number, #[case] rhs: Number, #[case] expected: bool) {
        let result = evaluate(
            &NotEqualTo {
                lhs: Box::new(Literal { value: Value::Number(lhs) }),
                rhs: Box::new(Literal { value: Value::Number(rhs) }),
            },
            &context(),
        )
        .unwrap();
        assert_eq!(result, Value::Boolean(expected));
    }

    #[rstest]
    #[case(Value::None, Value::None, false)]
    fn not_equal_to_none(#[case] lhs: Value, #[case] rhs: Value, #[case] expected: bool) {
        let result = evaluate(
            &NotEqualTo {
                lhs: Box::new(Literal { value: lhs }),
                rhs: Box::new(Literal { value: rhs }),
            },
            &context(),
        )
        .unwrap();

        assert_eq!(result, Value::Boolean(expected));
    }

    #[rstest]
    #[case(true, true, false)]
    #[case(true, false, true)]
    #[case(false, true, true)]
    #[case(false, false, false)]
    fn not_equal_to_bool(#[case] lhs: bool, #[case] rhs: bool, #[case] expected: bool) {
        let result = evaluate(
            &NotEqualTo {
                lhs: Box::new(Literal { value: Value::Boolean(lhs) }),
                rhs: Box::new(Literal { value: Value::Boolean(rhs) }),
            },
            &context(),
        )
        .unwrap();
        assert_eq!(result, Value::Boolean(expected));
    }

    #[rstest]
    #[case(Value::Boolean(true), Value::Number(Number::PositiveInt(2)), OperandTypeMismatch {
                operand: "NotEqualTo",
                expected: "Boolean|Number",
                actual_lhs: "Literal { value: Boolean(true) }".to_string(),
                actual_rhs: "Literal { value: Number(PositiveInt(2)) }".to_string(),
        })]
    #[case(Value::None, Value::Number(Number::PositiveInt(2)), OperandTypeMismatch {
                operand: "NotEqualTo",
                expected: "Boolean|Number",
                actual_lhs: "Literal { value: None }".to_string(),
                actual_rhs: "Literal { value: Number(PositiveInt(2)) }".to_string(),
        })]
    fn not_equal_to_mismatch(#[case] lhs: Value, #[case] rhs: Value, #[case] expected: ExpressionError) {
        let result = evaluate(
            &NotEqualTo {
                lhs: Box::new(Literal { value: lhs }),
                rhs: Box::new(Literal { value: rhs }),
            },
            &context(),
        )
        .unwrap_err();
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case(true, true, true)]
    #[case(true, false, false)]
    #[case(false, true, false)]
    #[case(false, false, false)]
    fn and(#[case] lhs: bool, #[case] rhs: bool, #[case] expected: bool) {
        let result = evaluate(
            &And {
                lhs: Box::new(Literal { value: Value::Boolean(lhs) }),
                rhs: Box::new(Literal { value: Value::Boolean(rhs) }),
            },
            &context(),
        )
        .unwrap();
        assert_eq!(result, Value::Boolean(expected));
    }

    #[rstest]
    #[case(Value::Boolean(true), Value::Number(Number::PositiveInt(2)), OperandTypeMismatch {
                operand: "And",
                expected: "Boolean",
                actual_lhs: "Literal { value: Boolean(true) }".to_string(),
                actual_rhs: "Literal { value: Number(PositiveInt(2)) }".to_string(),
        })]
    #[case(Value::Boolean(false), Value::None, OperandTypeMismatch {
                operand: "And",
                expected: "Boolean",
                actual_lhs: "Literal { value: Boolean(false) }".to_string(),
                actual_rhs: "Literal { value: None }".to_string(),
            })]
    #[case(Value::None, Value::Number(Number::PositiveInt(2)), OperandTypeMismatch {
                operand: "And",
                expected: "Boolean",
                actual_lhs: "Literal { value: None }".to_string(),
                actual_rhs: "Literal { value: Number(PositiveInt(2)) }".to_string(),
        })]
    fn and_mismatch(#[case] lhs: Value, #[case] rhs: Value, #[case] expected: ExpressionError) {
        let result = evaluate(
            &And {
                lhs: Box::new(Literal { value: lhs }),
                rhs: Box::new(Literal { value: rhs }),
            },
            &context(),
        )
        .unwrap_err();
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case(true, true, true)]
    #[case(true, false, true)]
    #[case(false, true, true)]
    #[case(false, false, false)]
    fn or(#[case] lhs: bool, #[case] rhs: bool, #[case] expected: bool) {
        let result = evaluate(
            &Or {
                lhs: Box::new(Literal { value: Value::Boolean(lhs) }),
                rhs: Box::new(Literal { value: Value::Boolean(rhs) }),
            },
            &context(),
        )
        .unwrap();
        assert_eq!(result, Value::Boolean(expected));
    }

    #[rstest]
    #[case(Value::Boolean(true), Value::Number(Number::PositiveInt(2)), OperandTypeMismatch {
                operand: "Or",
                expected: "Boolean",
                actual_lhs: "Literal { value: Boolean(true) }".to_string(),
                actual_rhs: "Literal { value: Number(PositiveInt(2)) }".to_string(),
        })]
    #[case(Value::Boolean(false), Value::None, OperandTypeMismatch {
                operand: "Or",
                expected: "Boolean",
                actual_lhs: "Literal { value: Boolean(false) }".to_string(),
                actual_rhs: "Literal { value: None }".to_string(),
        })]
    #[case(Value::None, Value::Number(Number::PositiveInt(2)), OperandTypeMismatch {
                operand: "Or",
                expected: "Boolean",
                actual_lhs: "Literal { value: None }".to_string(),
                actual_rhs: "Literal { value: Number(PositiveInt(2)) }".to_string(),
        })]
    fn or_mismatch(#[case] lhs: Value, #[case] rhs: Value, #[case] expected: ExpressionError) {
        let result = evaluate(
            &Or {
                lhs: Box::new(Literal { value: lhs }),
                rhs: Box::new(Literal { value: rhs }),
            },
            &context(),
        )
        .unwrap_err();
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case(true, false)]
    #[case(false, true)]
    fn not(#[case] value: bool, #[case] expected: bool) {
        let result = evaluate(
            &Not {
                expression: Box::new(Literal { value: Value::Boolean(value) }),
            },
            &context(),
        )
        .unwrap();
        assert_eq!(result, Value::Boolean(expected));
    }

    #[rstest]
    #[case(Value::None, UnaryOperandTypeMismatch {
                operand: "Not",
                expected: "Boolean",
                actual: "Literal { value: None }".to_string(),
        })]
    #[case(Value::Number(Number::PositiveInt(2)), UnaryOperandTypeMismatch {
                operand: "Not",
                expected: "Boolean",
                actual: "Literal { value: Number(PositiveInt(2)) }".to_string(),
        })]
    fn not_mismatch(#[case] value: Value, #[case] expected: ExpressionError) {
        let result = evaluate(
            &Not {
                expression: Box::new(Literal { value }),
            },
            &context(),
        )
        .unwrap_err();
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case::unknown_device("unknown_device_id", "", Err(ExpressionError::UnknownDevice("unknown_device_id".to_string())))]
    #[case::unknown_property("ab917a9a-a7d5-4853-9518-75909236a182", "unknown_property_id", Err(UnknownProperty { device_id: "ab917a9a-a7d5-4853-9518-75909236a182".to_string(), property_id: "unknown_property_id".to_string() }))]
    #[case::boolean("ab917a9a-a7d5-4853-9518-75909236a182", "on", Ok(Value::Boolean(true)))]
    #[case::number("ab917a9a-a7d5-4853-9518-75909236a182", "brightness", Ok(Value::Number(Number::Float(58.89))))]
    #[case::color("ab917a9a-a7d5-4853-9518-75909236a182", "color", Err(ExpressionError::UnsupportedPropertyType(PropertyType::Color)))]
    #[case::color_temperature(
        "ab917a9a-a7d5-4853-9518-75909236a182",
        "colorTemperature",
        Err(ExpressionError::UnsupportedPropertyType(PropertyType::ColorTemperature))
    )]
    fn property_value(#[case] device_id: &str, #[case] property_id: &str, #[case] expected: Result<Value, ExpressionError>) {
        let device = device();
        let devices: DeviceMap = HashMap::from([(device.id.clone(), Arc::new(device))]);
        let snapshot = StoreSnapshot { devices: Arc::new(devices) };

        let result = evaluate(
            &PropertyValue {
                device_id: device_id.to_string(),
                property_id: property_id.to_string(),
            },
            &Context::new(snapshot, GeoLocation::default()),
        );

        assert_eq!(result, expected);
    }

    #[rstest]
    #[case(Monday, false)]
    #[case(Tuesday, false)]
    #[case(Wednesday, false)]
    #[case(Thursday, false)]
    #[case(Friday, true)]
    #[case(Saturday, false)]
    #[case(Sunday, false)]
    fn is_today(#[case] weekday: Weekday, #[case] expected: bool) {
        let fixed_date_time = Local.with_ymd_and_hms(2000, 8, 4, 12, 0, 0).unwrap(); // A Friday
        let result = evaluate(
            &Temporal {
                expression: IsToday {
                    when: WeekdayCondition::Specific(weekday),
                },
            },
            &Context::new_with_now(StoreSnapshot::default(), fixed_date_time, GeoLocation::default()),
        )
        .unwrap();
        assert_eq!(result, Value::Boolean(expected));
    }

    #[rstest]
    #[case::midnight(Time::new(0, 0), false)]
    #[case::before_time(Time::new(11, 59), false)]
    #[case::same_time(Time::new(12, 0), false)]
    #[case::after_time(Time::new(12, 1), true)]
    #[case::before_midnight(Time::new(23, 59), true)]
    fn is_before_time(#[case] time: Time, #[case] expected: bool) {
        let fixed_date_time = Local.with_ymd_and_hms(2000, 8, 4, 12, 0, 0).unwrap();
        let result = evaluate(
            &Temporal { expression: IsBeforeTime { time } },
            &Context::new_with_now(StoreSnapshot::default(), fixed_date_time, GeoLocation::default()),
        )
        .unwrap();
        assert_eq!(result, Value::Boolean(expected));
    }

    #[rstest]
    #[case::midnight(Time::new(0, 0), true)]
    #[case::before_time(Time::new(11, 59), true)]
    #[case::same_time(Time::new(12, 0), false)]
    #[case::after_time(Time::new(12, 1), false)]
    #[case::before_midnight(Time::new(23, 59), false)]
    fn is_after_time(#[case] time: Time, #[case] expected: bool) {
        let fixed_date_time = Local.with_ymd_and_hms(2000, 8, 4, 12, 0, 0).unwrap();
        let result = evaluate(
            &Temporal { expression: IsAfterTime { time } },
            &Context::new_with_now(StoreSnapshot::default(), fixed_date_time, GeoLocation::default()),
        )
        .unwrap();
        assert_eq!(result, Value::Boolean(expected));
    }

    #[rstest]
    #[case(Time::new(0, 0), false)]
    #[case(Time::new(14, 0), true)]
    #[case(Time::new(23, 0), true)]
    fn has_sun_risen(#[case] time: Time, #[case] expected: bool) {
        // Sunrise at given location and date: 2000-08-04T06:09:31+02:00
        let fixed_date_time = Local.with_ymd_and_hms(2000, 8, 4, time.hour as u32, time.minute as u32, 0).unwrap();
        let result = evaluate(
            &Temporal { expression: HasSunRisen },
            &Context::new_with_now(
                StoreSnapshot::default(),
                fixed_date_time,
                GeoLocation {
                    latitude: 51.9244,
                    longitude: 4.4777,
                    altitude: 0.0,
                },
            ),
        )
        .unwrap();
        assert_eq!(result, Value::Boolean(expected));
    }

    #[rstest]
    #[case(Time::new(0, 0), false)]
    #[case(Time::new(14, 0), false)]
    #[case(Time::new(23, 0), true)]
    fn has_sun_set(#[case] time: Time, #[case] expected: bool) {
        // Sunset at given location and date: 2000-08-04T21:26:42+02:00
        let fixed_date_time = Local.with_ymd_and_hms(2000, 8, 4, time.hour as u32, time.minute as u32, 0).unwrap();
        let result = evaluate(
            &Temporal { expression: HasSunSet },
            &Context::new_with_now(
                StoreSnapshot::default(),
                fixed_date_time,
                GeoLocation {
                    latitude: 51.9244,
                    longitude: 4.4777,
                    altitude: 0.0,
                },
            ),
        )
        .unwrap();
        assert_eq!(result, Value::Boolean(expected));
    }

    #[rstest]
    #[case(Time::new(0, 0), false)]
    #[case(Time::new(14, 0), true)]
    #[case(Time::new(23, 0), false)]
    fn is_daytime(#[case] time: Time, #[case] expected: bool) {
        // Sunrise and sunset at given location and date: 2000-08-04T06:09:31+02:00 and 2000-08-04T21:26:42+02:00
        let fixed_date_time = Local.with_ymd_and_hms(2000, 8, 4, time.hour as u32, time.minute as u32, 0).unwrap();
        let result = evaluate(
            &Temporal { expression: IsDaytime },
            &Context::new_with_now(
                StoreSnapshot::default(),
                fixed_date_time,
                GeoLocation {
                    latitude: 51.9244,
                    longitude: 4.4777,
                    altitude: 0.0,
                },
            ),
        )
        .unwrap();
        assert_eq!(result, Value::Boolean(expected));
    }

    #[rstest]
    #[case(Time::new(0, 0), true)]
    #[case(Time::new(14, 0), false)]
    #[case(Time::new(23, 0), true)]
    fn is_nighttime(#[case] time: Time, #[case] expected: bool) {
        // Sunrise and sunset at given location and date: 2000-08-04T06:09:31+02:00 and 2000-08-04T21:26:42+02:00
        let fixed_date_time = Local.with_ymd_and_hms(2000, 8, 4, time.hour as u32, time.minute as u32, 0).unwrap();
        let result = evaluate(
            &Temporal { expression: IsNighttime },
            &Context::new_with_now(
                StoreSnapshot::default(),
                fixed_date_time,
                GeoLocation {
                    latitude: 51.9244,
                    longitude: 4.4777,
                    altitude: 0.0,
                },
            ),
        )
        .unwrap();
        assert_eq!(result, Value::Boolean(expected));
    }
}
