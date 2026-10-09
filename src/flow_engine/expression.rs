use crate::domain::color::Color;
use crate::domain::property::{BooleanProperty, DateTimeProperty, EnumProperty, NumberProperty, PropertyType, ValueKind};
use crate::domain::{Number, Problem, Time, WeekdayCondition, find_device, find_property};
use crate::extensions::date_time_ext::ToWeekday;
use crate::flow_engine::Context;
use crate::flow_engine::expression::ExpressionError::UnknownProperty;
use crate::flow_engine::solar_event::EventTime;
use crate::store::StoreSnapshot;
use chrono::{DateTime, NaiveTime, Utc};
use serde::Deserialize;
use std::cmp::Ordering;
use thiserror::Error;
use tracing::warn;

#[derive(PartialEq, Deserialize, Debug)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
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

    // Arithmetic
    Add { lhs: Box<Expression>, rhs: Box<Expression> },
    Subtract { lhs: Box<Expression>, rhs: Box<Expression> },

    // Literal
    Literal { value: Value },

    // Property
    PropertyChanged { device_id: String, property_id: String },
    PropertyValue { device_id: String, property_id: String },

    // Temporal
    Temporal { expression: TemporalExpression },
}

impl Expression {
    pub fn contains_property_changed(&self) -> bool {
        self.walk().any(|expression| matches!(expression, Expression::PropertyChanged { .. }))
    }

    pub fn constant_value(&self) -> Option<&Value> {
        match self {
            Expression::Literal { value } => Some(value),
            _ => None,
        }
    }

    pub fn value_kind(&self, snapshot: &StoreSnapshot) -> Result<ValueKind, Problem> {
        use Expression::*;

        let value_kind = match self {
            GreaterThanOrEqualTo { lhs, rhs } | GreaterThan { lhs, rhs } | LessThan { lhs, rhs } | LessThanOrEqualTo { lhs, rhs } => {
                match (lhs.value_kind(snapshot)?, rhs.value_kind(snapshot)?) {
                    (ValueKind::Number, ValueKind::Number) | (ValueKind::DateTime, ValueKind::DateTime) => ValueKind::Boolean,
                    (lhs, rhs) => return Err(self.incompatible_operands("Number|DateTime", vec![lhs, rhs])),
                }
            }
            EqualTo { lhs, rhs } | NotEqualTo { lhs, rhs } => {
                let (lhs, rhs) = (lhs.value_kind(snapshot)?, rhs.value_kind(snapshot)?);
                if lhs != rhs {
                    return Err(self.incompatible_operands("operands of the same kind", vec![lhs, rhs]));
                }
                ValueKind::Boolean
            }
            And { lhs, rhs } | Or { lhs, rhs } => {
                match (lhs.value_kind(snapshot)?, rhs.value_kind(snapshot)?) {
                    (ValueKind::Boolean, ValueKind::Boolean) => ValueKind::Boolean,
                    (lhs, rhs) => return Err(self.incompatible_operands("Boolean", vec![lhs, rhs])),
                }
            }
            Not { expression } => match expression.value_kind(snapshot)? {
                ValueKind::Boolean => ValueKind::Boolean,
                kind => return Err(self.incompatible_operands("Boolean", vec![kind])),
            },
            Add { lhs, rhs } | Subtract { lhs, rhs } => {
                match (lhs.value_kind(snapshot)?, rhs.value_kind(snapshot)?) {
                    (ValueKind::Number, ValueKind::Number) => ValueKind::Number,
                    (lhs, rhs) => return Err(self.incompatible_operands("Number", vec![lhs, rhs])),
                }
            },
            Literal { value } => value.kind(),
            PropertyChanged { .. } => ValueKind::Boolean,
            PropertyValue { device_id, property_id } => {
                let device = find_device(device_id, snapshot)?;
                find_property(device, property_id)?.value_kind()
            },
            Temporal { .. } => ValueKind::Boolean,
        };

        Ok(value_kind)
    }

    fn incompatible_operands(&self, expected: &'static str, actual: Vec<ValueKind>) -> Problem {
        use Expression::*;

        let operator = match self {
            GreaterThanOrEqualTo { .. } => "GreaterThanOrEqualTo",
            GreaterThan { .. } => "GreaterThan",
            LessThan { .. } => "LessThan",
            LessThanOrEqualTo { .. } => "LessThanOrEqualTo",
            EqualTo { .. } => "EqualTo",
            NotEqualTo { .. } => "NotEqualTo",
            And { .. } => "And",
            Or { .. } => "Or",
            Not { .. } => "Not",
            Add { .. } => "Add",
            Subtract { .. } => "Subtract",
            Literal { .. } | PropertyChanged { .. } | PropertyValue { .. } | Temporal { .. } => "Unknown",
        };

        Problem::IncompatibleOperands { operator, expected, actual }
    }

    /// Visits this expression and all its sub-expressions depth-first, left before right
    pub fn walk(&self) -> impl Iterator<Item=&Expression> + '_ {
        use Expression::*;
        let mut stack = vec![self];
        std::iter::from_fn(move || {
            let expression = stack.pop()?;
            match expression {
                GreaterThanOrEqualTo { lhs, rhs }
                | GreaterThan { lhs, rhs }
                | LessThan { lhs, rhs }
                | LessThanOrEqualTo { lhs, rhs }
                | EqualTo { lhs, rhs }
                | NotEqualTo { lhs, rhs }
                | And { lhs, rhs }
                | Or { lhs, rhs }
                | Add { lhs, rhs }
                | Subtract { lhs, rhs } => {
                    stack.push(rhs);
                    stack.push(lhs);
                }
                Not { expression } => stack.push(expression),
                Literal { .. } | PropertyChanged { .. } | PropertyValue { .. } | Temporal { .. } => {}
            }

            Some(expression)
        })
    }
}

#[derive(Eq, PartialEq, Hash, Debug, Clone)]
pub enum Value {
    Boolean(bool),
    Color(Color),
    DateTime(DateTime<Utc>),
    Number(Number),
    String(String),
    None,
}

impl Value {
    pub fn kind(&self) -> ValueKind {
        match self {
            Value::Boolean(_) => ValueKind::Boolean,
            Value::Color(_) => ValueKind::Color,
            Value::DateTime(_) => ValueKind::DateTime,
            Value::Number(_) => ValueKind::Number,
            Value::None => ValueKind::None,
            Value::String(_) => ValueKind::Enum,
        }
    }
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
            (Value::Color(a), Value::Color(b)) => Ok(Value::Boolean(a == b)),
            (Value::DateTime(a), Value::DateTime(b)) => Ok(Value::Boolean(a == b)),
            (Value::String(a), Value::String(b)) => Ok(Value::Boolean(a.eq(&b))),
            (Value::None, Value::None) => Ok(Value::Boolean(true)),
            (lhs, rhs) => Err(ExpressionError::OperandTypeMismatch {
                operand: "EqualTo",
                expected: EQUATABLE,
                actual_lhs: lhs.kind(),
                actual_rhs: rhs.kind(),
            }),
        },
        NotEqualTo { lhs, rhs } => match (evaluate(lhs, context)?, evaluate(rhs, context)?) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Boolean(!a.eq(&b))),
            (Value::Boolean(a), Value::Boolean(b)) => Ok(Value::Boolean(a != b)),
            (Value::Color(a), Value::Color(b)) => Ok(Value::Boolean(a != b)),
            (Value::DateTime(a), Value::DateTime(b)) => Ok(Value::Boolean(a != b)),
            (Value::String(a), Value::String(b)) => Ok(Value::Boolean(!a.eq(&b))),
            (Value::None, Value::None) => Ok(Value::Boolean(false)),
            (lhs, rhs) => Err(ExpressionError::OperandTypeMismatch {
                operand: "NotEqualTo",
                expected: EQUATABLE,
                actual_lhs: lhs.kind(),
                actual_rhs: rhs.kind(),
            }),
        },

        // Logic
        And { lhs, rhs } => match (evaluate(lhs, context)?, evaluate(rhs, context)?) {
            (Value::Boolean(a), Value::Boolean(b)) => Ok(Value::Boolean(a && b)),
            (lhs, rhs) => Err(ExpressionError::OperandTypeMismatch {
                operand: "And",
                expected: BOOLEAN,
                actual_lhs: lhs.kind(),
                actual_rhs: rhs.kind(),
            }),
        },
        Or { lhs, rhs } => match (evaluate(lhs, context)?, evaluate(rhs, context)?) {
            (Value::Boolean(a), Value::Boolean(b)) => Ok(Value::Boolean(a || b)),
            (lhs, rhs) => Err(ExpressionError::OperandTypeMismatch {
                operand: "Or",
                expected: BOOLEAN,
                actual_lhs: lhs.kind(),
                actual_rhs: rhs.kind(),
            }),
        },
        Not { expression } => match evaluate(expression, context)? {
            Value::Boolean(b) => Ok(Value::Boolean(!b)),
            value => Err(ExpressionError::UnaryOperandTypeMismatch {
                operand: "Not",
                expected: BOOLEAN,
                actual: value.kind(),
            }),
        },

        // Arithmetic
        Add { lhs, rhs } => match (evaluate(lhs, context)?, evaluate(rhs, context)?) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a + b)),
            (lhs, rhs) => Err(ExpressionError::OperandTypeMismatch {
                operand: "Add",
                expected: NUMBER,
                actual_lhs: lhs.kind(),
                actual_rhs: rhs.kind(),
            }),
        },
        Subtract { lhs, rhs } => match (evaluate(lhs, context)?, evaluate(rhs, context)?) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a - b)),
            (lhs, rhs) => Err(ExpressionError::OperandTypeMismatch {
                operand: "Subtract",
                expected: NUMBER,
                actual_lhs: lhs.kind(),
                actual_rhs: rhs.kind(),
            }),
        },

        // Literal
        Literal { value } => Ok(value.clone()),

        // Property
        PropertyChanged { device_id, property_id } => {
            let changed = context.changed().is_some_and(|c| &c.device_id == device_id && &c.property_id == property_id);
            Ok(Value::Boolean(changed))
        },
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
                PropertyType::BatteryLevel => {
                    let number_property = property.as_any().downcast_ref::<NumberProperty>().unwrap();
                    Ok(number_property.value().map(Value::Number).unwrap_or(Value::None))
                }
                PropertyType::BatteryState => {
                    let enum_property = property.as_any().downcast_ref::<EnumProperty>().unwrap();
                    Ok(enum_property.value().map(|v| Value::String(v.to_string())).unwrap_or(Value::None))
                }
                PropertyType::Brightness => {
                    let number_property = property.as_any().downcast_ref::<NumberProperty>().unwrap();
                    Ok(number_property.value().map(Value::Number).unwrap_or(Value::None))
                }
                PropertyType::Button => {
                    let enum_property = property.as_any().downcast_ref::<EnumProperty>().unwrap();
                    Ok(enum_property.value().map(|v| Value::String(v.to_string())).unwrap_or(Value::None))
                },
                PropertyType::ButtonLastChanged => {
                    let button_last_changed = property.as_any().downcast_ref::<DateTimeProperty>().unwrap();
                    Ok(button_last_changed.value().map(Value::DateTime).unwrap_or(Value::None))
                }
                PropertyType::Color => Err(ExpressionError::UnsupportedPropertyType(property.property_type())),
                PropertyType::ColorTemperature => Err(ExpressionError::UnsupportedPropertyType(property.property_type())),
                PropertyType::Connectivity => Err(ExpressionError::UnsupportedPropertyType(property.property_type())),
                PropertyType::MotionLastChanged => {
                    let motion_last_changed_property = property.as_any().downcast_ref::<DateTimeProperty>().unwrap();
                    Ok(motion_last_changed_property.value().map(Value::DateTime).unwrap_or(Value::None))
                },
                PropertyType::Enabled => {
                    let enabled_property = property.as_any().downcast_ref::<BooleanProperty>().unwrap();
                    Ok(Value::Boolean(enabled_property.value()))
                }
                PropertyType::Motion => {
                    let motion_property = property.as_any().downcast_ref::<BooleanProperty>().unwrap();
                    Ok(Value::Boolean(motion_property.value()))
                }
                PropertyType::On => {
                    let value = property.as_any().downcast_ref::<BooleanProperty>().unwrap();
                    Ok(Value::Boolean(value.value()))
                }
                PropertyType::MotionSensitivity => {
                    let sensitivity_property = property.as_any().downcast_ref::<NumberProperty>().unwrap();
                    Ok(sensitivity_property.value().map(Value::Number).unwrap_or(Value::None))
                }
                PropertyType::Illuminance => {
                    let illuminance_property = property.as_any().downcast_ref::<NumberProperty>().unwrap();
                    Ok(illuminance_property.value().map(Value::Number).unwrap_or(Value::None))
                }
                PropertyType::IlluminanceLastChanged => {
                    let illuminance_last_changed_property = property.as_any().downcast_ref::<DateTimeProperty>().unwrap();
                    Ok(illuminance_last_changed_property.value().map(Value::DateTime).unwrap_or(Value::None))
                }
            }
        }

        // Temporal
        // Uses wall-clock so it needs to use NaiveTime
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
                TemporalExpression::HasSunRisen => Ok(Value::Boolean(
                    match context.sunrise() {
                        EventTime::At(sunrise) => now.time() >= sunrise.time(),
                        EventTime::PolarDay => true,
                        EventTime::PolarNight => false,
                    }
                )),
                TemporalExpression::HasSunSet => Ok(Value::Boolean(
                    match context.sunset() {
                        EventTime::At(sunrise) => now.time() >= sunrise.time(),
                        EventTime::PolarDay => true,
                        EventTime::PolarNight => false,
                    }
                )),
                TemporalExpression::IsDaytime => {
                    let is_daytime = match (context.sunrise(), context.sunset()) {
                        (EventTime::At(sunrise), EventTime::At(sunset)) => now.time() >= sunrise.time() && now.time() < sunset.time(),
                        (EventTime::PolarDay, EventTime::PolarDay) => true,
                        (EventTime::PolarNight, EventTime::PolarNight) => false,
                        (sunrise, sunset) => {
                            warn!(?sunrise, ?sunset, "⚠️ Inconsistent sunrise/sunset classification, defaulting IsDaytime to false");
                            false
                        }
                    };
                    Ok(Value::Boolean(is_daytime))
                }
                TemporalExpression::IsNighttime => {
                    let is_nighttime = match (context.sunrise(), context.sunset()) {
                        (EventTime::At(sunrise), EventTime::At(sunset)) => now.time() < sunrise.time() || now.time() >= sunset.time(),
                        (EventTime::PolarDay, EventTime::PolarDay) => false,
                        (EventTime::PolarNight, EventTime::PolarNight) => true,
                        (sunrise, sunset) => {
                            warn!(?sunrise, ?sunset, "⚠️ Inconsistent sunrise/sunset classification, defaulting IsNighttime to false");
                            false
                        }
                    };
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
        (Value::DateTime(a), Value::DateTime(b)) => Ok(Value::Boolean(cmp(a.cmp(&b)))),
        (lhs, rhs) => Err(ExpressionError::OperandTypeMismatch {
            operand: "Compare",
            expected: COMPARABLE,
            actual_lhs: lhs.kind(),
            actual_rhs: rhs.kind(),
        }),
    }
}

const BOOLEAN: &[ValueKind] = &[ValueKind::Boolean];
const NUMBER: &[ValueKind] = &[ValueKind::Number];
const COMPARABLE: &[ValueKind] = &[ValueKind::Number, ValueKind::DateTime];
const EQUATABLE: &[ValueKind] = &[ValueKind::Boolean, ValueKind::Color, ValueKind::DateTime, ValueKind::Enum, ValueKind::None, ValueKind::Number];

#[derive(Error, PartialEq, Debug)]
pub enum ExpressionError {
    #[error("operand type mismatch for operand {operand}, expected {}, but got {actual_lhs} and {actual_rhs}", join_kinds(.expected))]
    OperandTypeMismatch {
        operand: &'static str,
        expected: &'static [ValueKind],
        actual_lhs: ValueKind,
        actual_rhs: ValueKind,
    },
    #[error("operand type mismatch for operand {operand}, expected {} but got {actual}", join_kinds(.expected))]
    UnaryOperandTypeMismatch { operand: &'static str, expected: &'static [ValueKind], actual: ValueKind },
    #[error("unknown device '{0}'")]
    UnknownDevice(String),
    #[error("unknown property '{property_id}' for device '{device_id}'")]
    UnknownProperty { device_id: String, property_id: String },
    #[error("property type '{0:?}' is not supported as a property value")]
    UnsupportedPropertyType(PropertyType),
    #[error("unable to compare given Numbers {actual_lhs} and {actual_rhs}")]
    ComparisonFailed { actual_lhs: String, actual_rhs: String },
}

fn join_kinds(kinds: &[ValueKind]) -> String {
    kinds.iter().map(ValueKind::to_string).collect::<Vec<_>>().join("|")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Weekday::*;
    use crate::domain::device::Device;
    use crate::domain::property::{CartesianCoordinate, Gamut, Property, Unit};
    use crate::domain::{GeoLocation, Weekday};
    use crate::flow_engine::context::ContextBuilder;
    use crate::flow_engine::expression::Expression::*;
    use crate::flow_engine::expression::ExpressionError::{OperandTypeMismatch, UnaryOperandTypeMismatch};
    use crate::flow_engine::expression::TemporalExpression::{HasSunRisen, HasSunSet, IsAfterTime, IsBeforeTime, IsDaytime, IsNighttime, IsToday};
    use crate::store::{DeviceMap, PropertyChange, StoreSnapshot};
    use crate::test_support::DeviceBuilder;
    use chrono::{Local, TimeZone};
    use rstest::rstest;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn context_with_location() -> ContextBuilder {
        Context::builder().location(GeoLocation {
            latitude: 51.9244,
            longitude: 4.4777,
            altitude: 0.0,
        })
    }

    fn context_with_polar_location() -> ContextBuilder {
        // Longyearbyen, Svalbard: far enough north to experience both a polar day (sun never sets,
        // roughly mid-April to late-August) and a polar night (sun never rises, roughly late-October
        // to mid-February).
        Context::builder().location(GeoLocation {
            latitude: 78.2232,
            longitude: 15.6267,
            altitude: 0.0,
        })
    }

    fn device() -> Device {
        let brightness_property: Box<dyn Property> = Box::new(
            NumberProperty::builder("brightness".to_string(), PropertyType::Brightness, false)
                .external_id("43e4f3a7-8b35-4b0c-a2ba-e6ca8f4c099b".to_string())
                .unit(Unit::Percentage)
                .float(Some(58.89), Some(2.0), Some(100.0))
                .build(),
        );

        let color_temperature_property: Box<dyn Property> = Box::new(
            NumberProperty::builder("colorTemperature".to_string(), PropertyType::ColorTemperature, false)
                .external_id("43e4f3a7-8b35-4b0c-a2ba-e6ca8f4c099b".to_string())
                .unit(Unit::Kelvin)
                .positive_int(Some(6535), Some(2000), Some(6535))
                .build(),
        );

        let motion_last_changed_property: Box<dyn Property> = Box::new(
            DateTimeProperty::new("motionLastChanged".to_string(), PropertyType::MotionLastChanged, true, None, Some(Utc.with_ymd_and_hms(2000, 8, 4, 12, 0, 0).unwrap()))
        );

        let illuminance_property: Box<dyn Property> = Box::new(
            NumberProperty::builder("illuminance".to_string(), PropertyType::Illuminance, true)
                .external_id("ab917a9a-a7d5-4853-9518-75909236a182".to_string())
                .unit(Unit::Lux)
                .float(Some(316.23), Some(0.0), None)
                .build(),
        );

        let illuminance_last_changed_property: Box<dyn Property> = Box::new(
            DateTimeProperty::new("illuminanceLastChanged".to_string(), PropertyType::IlluminanceLastChanged, true, None, Some(Utc.with_ymd_and_hms(2000, 8, 4, 12, 0, 0).unwrap()))
        );

        let battery_level_property: Box<dyn Property> = Box::new(
            NumberProperty::builder("batteryLevel".to_string(), PropertyType::BatteryLevel, true)
                .unit(Unit::Percentage)
                .positive_int(Some(42), Some(0), Some(100))
                .build(),
        );

        let battery_state_property: Box<dyn Property> = Box::new(
            EnumProperty::new("batteryState".to_string(), PropertyType::BatteryState, true, None, Some("normal".to_string()), vec!["normal".to_string(), "low".to_string(), "critical".to_string()]).unwrap(),
        );

        DeviceBuilder::new("ab917a9a-a7d5-4853-9518-75909236a182")
            .with_boolean_property("on", true)
            .with_color_property(
                "color",
                CartesianCoordinate::new(0.4851, 0.4331),
                Some(Gamut::new(
                    CartesianCoordinate::new(0.675, 0.322),
                    CartesianCoordinate::new(0.409, 0.518),
                    CartesianCoordinate::new(0.167, 0.04),
                )),
            )
            .with_properties(vec![
                brightness_property,
                color_temperature_property,
                motion_last_changed_property,
                illuminance_property,
                illuminance_last_changed_property,
                battery_level_property,
                battery_state_property
            ])
            .build()
    }

    fn utc_with_ymd(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap()
    }

    fn cie_xyy(x: f64, y: f64, brightness: f64) -> Color {
        Color::CIE_xyY {
            xy: CartesianCoordinate::new(x, y),
            brightness,
        }
    }

    mod greater_than_or_equal_to {
        use super::*;

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
        fn number(#[case] lhs: Number, #[case] rhs: Number, #[case] expected: bool) {
            let result = evaluate(
                &GreaterThanOrEqualTo {
                    lhs: Box::new(Literal { value: Value::Number(lhs) }),
                    rhs: Box::new(Literal { value: Value::Number(rhs) }),
                },
                &Context::default(),
            ).unwrap();
            assert_eq!(result, Value::Boolean(expected));
        }

        #[rstest]
        #[case(true, false)]
        fn boolean(#[case] lhs: bool, #[case] rhs: bool) {
            let result = evaluate(
                &GreaterThanOrEqualTo {
                    lhs: Box::new(Literal { value: Value::Boolean(lhs) }),
                    rhs: Box::new(Literal { value: Value::Boolean(rhs) }),
                },
                &Context::default(),
            ).unwrap_err();
            assert_eq!(result, OperandTypeMismatch {
                operand: "Compare",
                expected: COMPARABLE,
                actual_lhs: ValueKind::Boolean,
                actual_rhs: ValueKind::Boolean,
            });
        }

        #[rstest]
        #[case(utc_with_ymd(2000, 8, 4), utc_with_ymd(2000, 8, 3), true)]
        #[case(utc_with_ymd(2000, 8, 4), utc_with_ymd(2000, 8, 4), true)]
        #[case(utc_with_ymd(2000, 8, 4), utc_with_ymd(2000, 8, 5), false)]
        fn date_time(#[case] lhs: DateTime<Utc>, #[case] rhs: DateTime<Utc>, #[case] expected: bool) {
            let result = evaluate(
                &GreaterThanOrEqualTo {
                    lhs: Box::new(Literal { value: Value::DateTime(lhs) }),
                    rhs: Box::new(Literal { value: Value::DateTime(rhs) }),
                },
                &Context::default(),
            ).unwrap();
            assert_eq!(result, Value::Boolean(expected));
        }

        #[test]
        fn color() {
            let result = evaluate(
                &GreaterThanOrEqualTo {
                    lhs: Box::new(Literal {
                        value: Value::Color(Color::RGB(255, 0, 0)),
                    }),
                    rhs: Box::new(Literal {
                        value: Value::Color(Color::RGB(0, 0, 255)),
                    }),
                },
                &Context::default(),
            )
                .unwrap_err();
            assert_eq!(
                result,
                OperandTypeMismatch {
                    operand: "Compare",
                    expected: COMPARABLE,
                    actual_lhs: ValueKind::Color,
                    actual_rhs: ValueKind::Color,
                }
            );
        }
    }

    mod greater_than {
        use super::*;

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
        fn number(#[case] lhs: Number, #[case] rhs: Number, #[case] expected: bool) {
            let result = evaluate(
                &GreaterThan {
                    lhs: Box::new(Literal { value: Value::Number(lhs) }),
                    rhs: Box::new(Literal { value: Value::Number(rhs) }),
                },
                &Context::default(),
            ).unwrap();
            assert_eq!(result, Value::Boolean(expected));
        }

        #[rstest]
        #[case(true, false)]
        fn boolean(#[case] lhs: bool, #[case] rhs: bool) {
            let result = evaluate(
                &GreaterThan {
                    lhs: Box::new(Literal { value: Value::Boolean(lhs) }),
                    rhs: Box::new(Literal { value: Value::Boolean(rhs) }),
                },
                &Context::default(),
            ).unwrap_err();
            assert_eq!(result, OperandTypeMismatch {
                operand: "Compare",
                expected: COMPARABLE,
                actual_lhs: ValueKind::Boolean,
                actual_rhs: ValueKind::Boolean,
            });
        }

        #[rstest]
        #[case(utc_with_ymd(2000, 8, 4), utc_with_ymd(2000, 8, 3), true)]
        #[case(utc_with_ymd(2000, 8, 4), utc_with_ymd(2000, 8, 4), false)]
        #[case(utc_with_ymd(2000, 8, 4), utc_with_ymd(2000, 8, 5), false)]
        fn date_time(#[case] lhs: DateTime<Utc>, #[case] rhs: DateTime<Utc>, #[case] expected: bool) {
            let result = evaluate(
                &GreaterThan {
                    lhs: Box::new(Literal { value: Value::DateTime(lhs) }),
                    rhs: Box::new(Literal { value: Value::DateTime(rhs) }),
                },
                &Context::default(),
            ).unwrap();
            assert_eq!(result, Value::Boolean(expected));
        }

        #[test]
        fn color() {
            let result = evaluate(
                &GreaterThan {
                    lhs: Box::new(Literal {
                        value: Value::Color(Color::RGB(255, 0, 0)),
                    }),
                    rhs: Box::new(Literal {
                        value: Value::Color(Color::RGB(0, 0, 255)),
                    }),
                },
                &Context::default(),
            )
                .unwrap_err();
            assert_eq!(
                result,
                OperandTypeMismatch {
                    operand: "Compare",
                    expected: COMPARABLE,
                    actual_lhs: ValueKind::Color,
                    actual_rhs: ValueKind::Color,
                }
            );
        }
    }

    mod less_than {
        use super::*;

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
        fn number(#[case] lhs: Number, #[case] rhs: Number, #[case] expected: bool) {
            let result = evaluate(
                &LessThan {
                    lhs: Box::new(Literal { value: Value::Number(lhs) }),
                    rhs: Box::new(Literal { value: Value::Number(rhs) }),
                },
                &Context::default(),
            ).unwrap();
            assert_eq!(result, Value::Boolean(expected));
        }

        #[rstest]
        #[case(true, false)]
        fn boolean(#[case] lhs: bool, #[case] rhs: bool) {
            let result = evaluate(
                &LessThan {
                    lhs: Box::new(Literal { value: Value::Boolean(lhs) }),
                    rhs: Box::new(Literal { value: Value::Boolean(rhs) }),
                },
                &Context::default(),
            ).unwrap_err();
            assert_eq!(result, OperandTypeMismatch {
                operand: "Compare",
                expected: COMPARABLE,
                actual_lhs: ValueKind::Boolean,
                actual_rhs: ValueKind::Boolean,
            });
        }

        #[rstest]
        #[case(utc_with_ymd(2000, 8, 4), utc_with_ymd(2000, 8, 3), false)]
        #[case(utc_with_ymd(2000, 8, 4), utc_with_ymd(2000, 8, 4), false)]
        #[case(utc_with_ymd(2000, 8, 4), utc_with_ymd(2000, 8, 5), true)]
        fn date_time(#[case] lhs: DateTime<Utc>, #[case] rhs: DateTime<Utc>, #[case] expected: bool) {
            let result = evaluate(
                &LessThan {
                    lhs: Box::new(Literal { value: Value::DateTime(lhs) }),
                    rhs: Box::new(Literal { value: Value::DateTime(rhs) }),
                },
                &Context::default(),
            ).unwrap();
            assert_eq!(result, Value::Boolean(expected));
        }
    }

    mod less_than_or_equal_to {
        use super::*;

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
        fn number(#[case] lhs: Number, #[case] rhs: Number, #[case] expected: bool) {
            let result = evaluate(
                &LessThanOrEqualTo {
                    lhs: Box::new(Literal { value: Value::Number(lhs) }),
                    rhs: Box::new(Literal { value: Value::Number(rhs) }),
                },
                &Context::default(),
            ).unwrap();
            assert_eq!(result, Value::Boolean(expected));
        }

        #[rstest]
        #[case(true, false)]
        fn boolean(#[case] lhs: bool, #[case] rhs: bool) {
            let result = evaluate(
                &LessThanOrEqualTo {
                    lhs: Box::new(Literal { value: Value::Boolean(lhs) }),
                    rhs: Box::new(Literal { value: Value::Boolean(rhs) }),
                },
                &Context::default(),
            ).unwrap_err();
            assert_eq!(result, OperandTypeMismatch {
                operand: "Compare",
                expected: COMPARABLE,
                actual_lhs: ValueKind::Boolean,
                actual_rhs: ValueKind::Boolean,
            });
        }

        #[rstest]
        #[case(utc_with_ymd(2000, 8, 4), utc_with_ymd(2000, 8, 3), false)]
        #[case(utc_with_ymd(2000, 8, 4), utc_with_ymd(2000, 8, 4), true)]
        #[case(utc_with_ymd(2000, 8, 4), utc_with_ymd(2000, 8, 5), true)]
        fn date_time(#[case] lhs: DateTime<Utc>, #[case] rhs: DateTime<Utc>, #[case] expected: bool) {
            let result = evaluate(
                &LessThanOrEqualTo {
                    lhs: Box::new(Literal { value: Value::DateTime(lhs) }),
                    rhs: Box::new(Literal { value: Value::DateTime(rhs) }),
                },
                &Context::default(),
            )
                .unwrap();
            assert_eq!(result, Value::Boolean(expected));
        }

        #[test]
        fn color() {
            let result = evaluate(
                &LessThan {
                    lhs: Box::new(Literal {
                        value: Value::Color(Color::RGB(255, 0, 0)),
                    }),
                    rhs: Box::new(Literal {
                        value: Value::Color(Color::RGB(0, 0, 255)),
                    }),
                },
                &Context::default(),
            )
                .unwrap_err();
            assert_eq!(
                result,
                OperandTypeMismatch {
                    operand: "Compare",
                    expected: COMPARABLE,
                    actual_lhs: ValueKind::Color,
                    actual_rhs: ValueKind::Color,
                }
            );
        }
    }

    mod equal_to {
        use super::*;

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
        fn number(#[case] lhs: Number, #[case] rhs: Number, #[case] expected: bool) {
            let result = evaluate(
                &EqualTo {
                    lhs: Box::new(Literal { value: Value::Number(lhs) }),
                    rhs: Box::new(Literal { value: Value::Number(rhs) }),
                },
                &Context::default(),
            ).unwrap();
            assert_eq!(result, Value::Boolean(expected));
        }

        #[rstest]
        #[case(true, false, false)]
        #[case(true, true, true)]
        #[case(false, true, false)]
        #[case(false, false, true)]
        fn boolean(#[case] lhs: bool, #[case] rhs: bool, #[case] expected: bool) {
            let result = evaluate(
                &EqualTo {
                    lhs: Box::new(Literal { value: Value::Boolean(lhs) }),
                    rhs: Box::new(Literal { value: Value::Boolean(rhs) }),
                },
                &Context::default(),
            ).unwrap();
            assert_eq!(result, Value::Boolean(expected));
        }

        #[rstest]
        #[case::same_rgb(Color::RGB(255, 0, 0), Color::RGB(255, 0, 0), true)]
        #[case::different_rgb(Color::RGB(255, 0, 0), Color::RGB(0, 0, 255), false)]
        #[case::same_hex(Color::Hex("#ff0000".to_string()), Color::Hex("#ff0000".to_string()), true)]
        #[case::different_hex(Color::Hex("#ff0000".to_string()), Color::Hex("#0000ff".to_string()), false)]
        #[case::same_cie_xyy(cie_xyy(0.675, 0.322, 0.2126), cie_xyy(0.675, 0.322, 0.2126), true)]
        #[case::different_cie_xyy_coordinate(cie_xyy(0.675, 0.322, 0.2126), cie_xyy(0.167, 0.04, 0.2126), false)]
        #[case::different_cie_xyy_brightness(cie_xyy(0.675, 0.322, 0.2126), cie_xyy(0.675, 0.322, 0.5), false)]
        #[case::different_representations(Color::RGB(255, 0, 0), Color::Hex("#ff0000".to_string()), false)]
        fn color(#[case] lhs: Color, #[case] rhs: Color, #[case] expected: bool) {
            let result = evaluate(
                &EqualTo {
                    lhs: Box::new(Literal { value: Value::Color(lhs) }),
                    rhs: Box::new(Literal { value: Value::Color(rhs) }),
                },
                &Context::default(),
            )
                .unwrap();
            assert_eq!(result, Value::Boolean(expected));
        }

        #[rstest]
        #[case(utc_with_ymd(2000, 8, 4), utc_with_ymd(2000, 8, 3), false)]
        #[case(utc_with_ymd(2000, 8, 4), utc_with_ymd(2000, 8, 4), true)]
        #[case(utc_with_ymd(2000, 8, 4), utc_with_ymd(2000, 8, 5), false)]
        fn date_time(#[case] lhs: DateTime<Utc>, #[case] rhs: DateTime<Utc>, #[case] expected: bool) {
            let result = evaluate(
                &EqualTo {
                    lhs: Box::new(Literal { value: Value::DateTime(lhs) }),
                    rhs: Box::new(Literal { value: Value::DateTime(rhs) }),
                },
                &Context::default(),
            ).unwrap();
            assert_eq!(result, Value::Boolean(expected));
        }

        #[rstest]
        #[case("initial_press", "short_release", false)]
        #[case("initial_press", "initial_press", true)]
        fn string(#[case] lhs: String, #[case] rhs: String, #[case] expected: bool) {
            let result = evaluate(
                &EqualTo {
                    lhs: Box::new(Literal { value: Value::String(lhs) }),
                    rhs: Box::new(Literal { value: Value::String(rhs) }),
                },
                &Context::default(),
            ).unwrap();
            assert_eq!(result, Value::Boolean(expected));
        }

        #[rstest]
        #[case(Value::None, Value::None, true)]
        fn none(#[case] lhs: Value, #[case] rhs: Value, #[case] expected: bool) {
            let result = evaluate(
                &EqualTo {
                    lhs: Box::new(Literal { value: lhs }),
                    rhs: Box::new(Literal { value: rhs }),
                },
                &Context::default(),
            )
                .unwrap();

            assert_eq!(result, Value::Boolean(expected));
        }

        #[rstest]
        #[case(Value::Boolean(true), Value::Number(Number::PositiveInt(2)), OperandTypeMismatch{
                operand: "EqualTo",
                expected: EQUATABLE,
                actual_lhs: ValueKind::Boolean,
                actual_rhs: ValueKind::Number,
            })]
        #[case(Value::None, Value::Number(Number::PositiveInt(2)), OperandTypeMismatch{
                operand: "EqualTo",
                expected: EQUATABLE,
                actual_lhs: ValueKind::None,
                actual_rhs: ValueKind::Number,
            })]
        #[case(Value::Color(Color::Hex("#ff0000".to_string())), Value::String("#ff0000".to_string()), OperandTypeMismatch{
                operand: "EqualTo",
                expected: EQUATABLE,
                actual_lhs: ValueKind::Color,
                actual_rhs: ValueKind::Enum,
            })]
        fn mismatch(#[case] lhs: Value, #[case] rhs: Value, #[case] expected: ExpressionError) {
            let result = evaluate(
                &EqualTo {
                    lhs: Box::new(Literal { value: lhs }),
                    rhs: Box::new(Literal { value: rhs }),
                },
                &Context::default(),
            )
                .unwrap_err();
            assert_eq!(result, expected);
        }
    }

    mod not_equal_to {
        use super::*;

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
        fn number(#[case] lhs: Number, #[case] rhs: Number, #[case] expected: bool) {
            let result = evaluate(
                &NotEqualTo {
                    lhs: Box::new(Literal { value: Value::Number(lhs) }),
                    rhs: Box::new(Literal { value: Value::Number(rhs) }),
                },
                &Context::default(),
            )
                .unwrap();
            assert_eq!(result, Value::Boolean(expected));
        }

        #[rstest]
        #[case(true, true, false)]
        #[case(true, false, true)]
        #[case(false, true, true)]
        #[case(false, false, false)]
        fn bool(#[case] lhs: bool, #[case] rhs: bool, #[case] expected: bool) {
            let result = evaluate(
                &NotEqualTo {
                    lhs: Box::new(Literal { value: Value::Boolean(lhs) }),
                    rhs: Box::new(Literal { value: Value::Boolean(rhs) }),
                },
                &Context::default(),
            )
                .unwrap();
            assert_eq!(result, Value::Boolean(expected));
        }

        #[rstest]
        #[case::same_rgb(Color::RGB(255, 0, 0), Color::RGB(255, 0, 0), false)]
        #[case::different_rgb(Color::RGB(255, 0, 0), Color::RGB(0, 0, 255), true)]
        #[case::same_hex(Color::Hex("#ff0000".to_string()), Color::Hex("#ff0000".to_string()), false)]
        #[case::different_hex(Color::Hex("#ff0000".to_string()), Color::Hex("#0000ff".to_string()), true)]
        #[case::same_cie_xyy(cie_xyy(0.675, 0.322, 0.2126), cie_xyy(0.675, 0.322, 0.2126), false)]
        #[case::different_cie_xyy_coordinate(cie_xyy(0.675, 0.322, 0.2126), cie_xyy(0.167, 0.04, 0.2126), true)]
        #[case::different_cie_xyy_brightness(cie_xyy(0.675, 0.322, 0.2126), cie_xyy(0.675, 0.322, 0.5), true)]
        #[case::different_representations(Color::RGB(255, 0, 0), Color::Hex("#ff0000".to_string()), true)]
        fn color(#[case] lhs: Color, #[case] rhs: Color, #[case] expected: bool) {
            let result = evaluate(
                &NotEqualTo {
                    lhs: Box::new(Literal { value: Value::Color(lhs) }),
                    rhs: Box::new(Literal { value: Value::Color(rhs) }),
                },
                &Context::default(),
            )
                .unwrap();
            assert_eq!(result, Value::Boolean(expected));
        }

        #[rstest]
        #[case(utc_with_ymd(2000, 8, 4), utc_with_ymd(2000, 8, 3), true)]
        #[case(utc_with_ymd(2000, 8, 4), utc_with_ymd(2000, 8, 4), false)]
        #[case(utc_with_ymd(2000, 8, 4), utc_with_ymd(2000, 8, 5), true)]
        fn date_time(#[case] lhs: DateTime<Utc>, #[case] rhs: DateTime<Utc>, #[case] expected: bool) {
            let result = evaluate(
                &NotEqualTo {
                    lhs: Box::new(Literal { value: Value::DateTime(lhs) }),
                    rhs: Box::new(Literal { value: Value::DateTime(rhs) }),
                },
                &Context::default(),
            ).unwrap();
            assert_eq!(result, Value::Boolean(expected));
        }

        #[rstest]
        #[case("initial_press", "short_release", true)]
        #[case("initial_press", "initial_press", false)]
        fn string(#[case] lhs: String, #[case] rhs: String, #[case] expected: bool) {
            let result = evaluate(
                &NotEqualTo {
                    lhs: Box::new(Literal { value: Value::String(lhs) }),
                    rhs: Box::new(Literal { value: Value::String(rhs) }),
                },
                &Context::default(),
            ).unwrap();
            assert_eq!(result, Value::Boolean(expected));
        }

        #[rstest]
        #[case(Value::None, Value::None, false)]
        fn none(#[case] lhs: Value, #[case] rhs: Value, #[case] expected: bool) {
            let result = evaluate(
                &NotEqualTo {
                    lhs: Box::new(Literal { value: lhs }),
                    rhs: Box::new(Literal { value: rhs }),
                },
                &Context::default(),
            )
                .unwrap();

            assert_eq!(result, Value::Boolean(expected));
        }

        #[rstest]
        #[case(Value::Boolean(true), Value::Number(Number::PositiveInt(2)), OperandTypeMismatch{
                operand: "NotEqualTo",
                expected: EQUATABLE,
                actual_lhs: ValueKind::Boolean,
                actual_rhs: ValueKind::Number,
            })]
        #[case(Value::None, Value::Number(Number::PositiveInt(2)), OperandTypeMismatch{
                operand: "NotEqualTo",
                expected: EQUATABLE,
                actual_lhs: ValueKind::None,
                actual_rhs: ValueKind::Number,
            })]
        #[case(Value::Color(Color::Hex("#ff0000".to_string())), Value::String("#ff0000".to_string()), OperandTypeMismatch{
                operand: "NotEqualTo",
                expected: EQUATABLE,
                actual_lhs: ValueKind::Color,
                actual_rhs: ValueKind::Enum,
            })]
        fn mismatch(#[case] lhs: Value, #[case] rhs: Value, #[case] expected: ExpressionError) {
            let result = evaluate(
                &NotEqualTo {
                    lhs: Box::new(Literal { value: lhs }),
                    rhs: Box::new(Literal { value: rhs }),
                },
                &Context::default(),
            )
                .unwrap_err();
            assert_eq!(result, expected);
        }
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
            &Context::default(),
        )
        .unwrap();
        assert_eq!(result, Value::Boolean(expected));
    }

    #[rstest]
    #[case(Value::Boolean(true), Value::Number(Number::PositiveInt(2)), OperandTypeMismatch {
                operand: "And",
                expected: BOOLEAN,
                actual_lhs: ValueKind::Boolean,
                actual_rhs: ValueKind::Number,
        })]
    #[case(Value::Boolean(false), Value::None, OperandTypeMismatch {
                operand: "And",
                expected: BOOLEAN,
                actual_lhs: ValueKind::Boolean,
                actual_rhs: ValueKind::None,
            })]
    #[case(Value::None, Value::Number(Number::PositiveInt(2)), OperandTypeMismatch {
                operand: "And",
                expected: BOOLEAN,
                actual_lhs: ValueKind::None,
                actual_rhs: ValueKind::Number,
        })]
    fn and_mismatch(#[case] lhs: Value, #[case] rhs: Value, #[case] expected: ExpressionError) {
        let result = evaluate(
            &And {
                lhs: Box::new(Literal { value: lhs }),
                rhs: Box::new(Literal { value: rhs }),
            },
            &Context::default(),
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
            &Context::default(),
        )
        .unwrap();
        assert_eq!(result, Value::Boolean(expected));
    }

    #[rstest]
    #[case(Value::Boolean(true), Value::Number(Number::PositiveInt(2)), OperandTypeMismatch {
                operand: "Or",
                expected: BOOLEAN,
                actual_lhs: ValueKind::Boolean,
                actual_rhs: ValueKind::Number,
        })]
    #[case(Value::Boolean(false), Value::None, OperandTypeMismatch {
                operand: "Or",
                expected: BOOLEAN,
                actual_lhs: ValueKind::Boolean,
                actual_rhs: ValueKind::None,
        })]
    #[case(Value::None, Value::Number(Number::PositiveInt(2)), OperandTypeMismatch {
                operand: "Or",
                expected: BOOLEAN,
                actual_lhs: ValueKind::None,
                actual_rhs: ValueKind::Number,
        })]
    fn or_mismatch(#[case] lhs: Value, #[case] rhs: Value, #[case] expected: ExpressionError) {
        let result = evaluate(
            &Or {
                lhs: Box::new(Literal { value: lhs }),
                rhs: Box::new(Literal { value: rhs }),
            },
            &Context::default(),
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
            &Context::default(),
        )
        .unwrap();
        assert_eq!(result, Value::Boolean(expected));
    }

    #[rstest]
    #[case(Value::None, UnaryOperandTypeMismatch {
                operand: "Not",
                expected: BOOLEAN,
                actual: ValueKind::None,
        })]
    #[case(Value::Number(Number::PositiveInt(2)), UnaryOperandTypeMismatch {
                operand: "Not",
                expected: BOOLEAN,
                actual: ValueKind::Number,
        })]
    fn not_mismatch(#[case] value: Value, #[case] expected: ExpressionError) {
        let result = evaluate(
            &Not {
                expression: Box::new(Literal { value }),
            },
            &Context::default(),
        )
        .unwrap_err();
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case::positive_ints(Number::PositiveInt(3), Number::PositiveInt(2), Number::PositiveInt(5))]
    #[case::negative_result(Number::PositiveInt(3), Number::NegativeInt(-5), Number::NegativeInt(-2))]
    #[case::float(Number::PositiveInt(3), Number::Float(0.5), Number::Float(3.5))]
    fn add(#[case] lhs: Number, #[case] rhs: Number, #[case] expected: Number) {
        let result = evaluate(
            &Add {
                lhs: Box::new(Literal { value: Value::Number(lhs) }),
                rhs: Box::new(Literal { value: Value::Number(rhs) }),
            },
            &Context::default(),
        )
            .unwrap();
        assert_eq!(result, Value::Number(expected));
    }

    #[rstest]
    #[case::positive_ints(Number::PositiveInt(3), Number::PositiveInt(2), Number::PositiveInt(1))]
    #[case::negative_result(Number::PositiveInt(3), Number::PositiveInt(5), Number::NegativeInt(-2))]
    #[case::float(Number::PositiveInt(3), Number::Float(0.5), Number::Float(2.5))]
    fn subtract(#[case] lhs: Number, #[case] rhs: Number, #[case] expected: Number) {
        let result = evaluate(
            &Subtract {
                lhs: Box::new(Literal { value: Value::Number(lhs) }),
                rhs: Box::new(Literal { value: Value::Number(rhs) }),
            },
            &Context::default(),
        )
            .unwrap();
        assert_eq!(result, Value::Number(expected));
    }

    #[rstest]
    #[case::add_boolean(
        Add{ lhs: Box::new(Literal { value: Value::Number(Number::PositiveInt(1)) }), rhs: Box::new(Literal { value: Value::Boolean(true) }) },
        OperandTypeMismatch{ operand: "Add", expected: NUMBER, actual_lhs: ValueKind::Number, actual_rhs: ValueKind::Boolean }
    )]
    #[case::add_none(
        Add{ lhs: Box::new(Literal { value: Value::None }), rhs: Box::new(Literal { value: Value::Number(Number::PositiveInt(1)) }) },
        OperandTypeMismatch{ operand: "Add", expected: NUMBER, actual_lhs: ValueKind::None, actual_rhs: ValueKind::Number }
    )]
    #[case::subtract_string(
        Subtract{ lhs: Box::new(Literal { value: Value::String("low".to_string()) }), rhs: Box::new(Literal { value: Value::Number(Number::PositiveInt(1)) }) },
        OperandTypeMismatch{ operand: "Subtract", expected: NUMBER, actual_lhs: ValueKind::Enum, actual_rhs: ValueKind::Number }
    )]
    fn arithmetic_mismatch(#[case] expression: Expression, #[case] expected: ExpressionError) {
        let result = evaluate(&expression, &Context::default()).unwrap_err();
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case::rgb(Color::RGB(255, 0, 0))]
    #[case::hex(Color::Hex("#ff0000".to_string()))]
    #[case::cie_xyy(cie_xyy(0.675, 0.322, 0.2126))]
    fn color_literal_evaluates_to_its_value(#[case] color: Color) {
        let result = evaluate(&Literal { value: Value::Color(color.clone()) }, &Context::default());

        assert_eq!(result, Ok(Value::Color(color)));
    }

    #[test]
    fn property_changed_evaluates_to_true_when_it_matches_the_context() {
        let context = Context::builder()
            .changed(Some(PropertyChange { device_id: "ab917a9a-a7d5-4853-9518-75909236a182".to_string(), property_id: "on".to_string() }))
            .build();
        let expression = PropertyChanged { device_id: "ab917a9a-a7d5-4853-9518-75909236a182".to_string(), property_id: "on".to_string() };
        assert_eq!(evaluate(&expression, &context), Ok(Value::Boolean(true)));
    }

    #[test]
    fn property_changed_evaluates_to_false_for_a_different_property() {
        let context = Context::builder()
            .changed(Some(PropertyChange { device_id: "ab917a9a-a7d5-4853-9518-75909236a182".to_string(), property_id: "brightness".to_string() }))
            .build();
        let expression = PropertyChanged { device_id: "ab917a9a-a7d5-4853-9518-75909236a182".to_string(), property_id: "on".to_string() };
        assert_eq!(evaluate(&expression, &context), Ok(Value::Boolean(false)));
    }

    #[test]
    fn property_changed_evaluates_to_false_when_the_context_has_no_change() {
        let context = Context::builder().build(); // e.g. a scheduled flow's context
        let expression = PropertyChanged { device_id: "ab917a9a-a7d5-4853-9518-75909236a182".to_string(), property_id: "on".to_string() };
        assert_eq!(evaluate(&expression, &context), Ok(Value::Boolean(false)));
    }

    #[rstest]
    #[case::unknown_device("unknown_device_id", "", Err(ExpressionError::UnknownDevice("unknown_device_id".to_string())))]
    #[case::unknown_property("ab917a9a-a7d5-4853-9518-75909236a182", "unknown_property_id", Err(UnknownProperty { device_id: "ab917a9a-a7d5-4853-9518-75909236a182".to_string(), property_id: "unknown_property_id".to_string() }))]
    #[case::boolean("ab917a9a-a7d5-4853-9518-75909236a182", "on", Ok(Value::Boolean(true)))]
    #[case::date_time("ab917a9a-a7d5-4853-9518-75909236a182", "motionLastChanged", Ok(Value::DateTime(Utc.with_ymd_and_hms(2000, 8, 4, 12, 0, 0).unwrap())))]
    #[case::number("ab917a9a-a7d5-4853-9518-75909236a182", "brightness", Ok(Value::Number(Number::Float(58.89))))]
    #[case::color("ab917a9a-a7d5-4853-9518-75909236a182", "color", Err(ExpressionError::UnsupportedPropertyType(PropertyType::Color)))]
    #[case::color_temperature(
        "ab917a9a-a7d5-4853-9518-75909236a182",
        "colorTemperature",
        Err(ExpressionError::UnsupportedPropertyType(PropertyType::ColorTemperature))
    )]
    #[case::illuminance("ab917a9a-a7d5-4853-9518-75909236a182", "illuminance", Ok(Value::Number(Number::Float(316.23))))]
    #[case::illuminance_last_changed("ab917a9a-a7d5-4853-9518-75909236a182", "illuminanceLastChanged", Ok(Value::DateTime(Utc.with_ymd_and_hms(2000, 8, 4, 12, 0, 0).unwrap())))]
    #[case::battery_level("ab917a9a-a7d5-4853-9518-75909236a182", "batteryLevel", Ok(Value::Number(Number::PositiveInt(42))))]
    #[case::battery_state("ab917a9a-a7d5-4853-9518-75909236a182", "batteryState", Ok(Value::String("normal".to_string())))]
    fn property_value(#[case] device_id: &str, #[case] property_id: &str, #[case] expected: Result<Value, ExpressionError>) {
        let device = device();
        let devices: DeviceMap = HashMap::from([(device.id.clone(), Arc::new(device))]);
        let snapshot = StoreSnapshot { devices: Arc::new(devices) };

        let result = evaluate(
            &PropertyValue {
                device_id: device_id.to_string(),
                property_id: property_id.to_string(),
            },
            &Context::builder().snapshot(snapshot).build(),
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
            &Context::builder().now(fixed_date_time).build(),
        )
        .unwrap();
        assert_eq!(result, Value::Boolean(expected));
    }

    #[rstest]
    #[case::midnight(Time { hour: 0, minute: 0 }, false)]
    #[case::before_time(Time { hour: 11, minute: 59 }, false)]
    #[case::same_time(Time { hour: 12, minute: 0 }, false)]
    #[case::after_time(Time { hour: 12, minute: 1 }, true)]
    #[case::before_midnight(Time { hour: 23, minute: 59 }, true)]
    fn is_before_time(#[case] time: Time, #[case] expected: bool) {
        let fixed_date_time = Local.with_ymd_and_hms(2000, 8, 4, 12, 0, 0).unwrap();
        let context = &context_with_location().now(fixed_date_time).build();
        let result = evaluate(&Temporal { expression: IsBeforeTime { time } }, &context).unwrap();
        assert_eq!(result, Value::Boolean(expected));
    }

    #[rstest]
    #[case::midnight(Time { hour: 0, minute: 0 }, true)]
    #[case::before_time(Time { hour: 11, minute: 59 }, true)]
    #[case::same_time(Time { hour: 12, minute: 0 }, false)]
    #[case::after_time(Time { hour: 12, minute: 1 }, false)]
    #[case::before_midnight(Time { hour: 23, minute: 59 }, false)]
    fn is_after_time(#[case] time: Time, #[case] expected: bool) {
        let fixed_date_time = Local.with_ymd_and_hms(2000, 8, 4, 12, 0, 0).unwrap();
        let context = &context_with_location().now(fixed_date_time).build();
        let result = evaluate(&Temporal { expression: IsAfterTime { time } }, &context).unwrap();
        assert_eq!(result, Value::Boolean(expected));
    }

    #[rstest]
    #[case(Time { hour: 0, minute: 0 }, false)]
    #[case(Time { hour: 14, minute: 0 }, true)]
    #[case(Time { hour: 23, minute: 0 }, true)]
    fn has_sun_risen(#[case] time: Time, #[case] expected: bool) {
        // Sunrise at given location and date: 2000-08-04T06:09:31+02:00
        let fixed_date_time = Local.with_ymd_and_hms(2000, 8, 4, time.hour as u32, time.minute as u32, 0).unwrap();
        let context = &context_with_location().now(fixed_date_time).build();
        let result = evaluate(&Temporal { expression: HasSunRisen }, &context).unwrap();
        assert_eq!(result, Value::Boolean(expected));
    }

    #[rstest]
    #[case(Time { hour: 0, minute: 0 }, false)]
    #[case(Time { hour: 14, minute: 0 }, false)]
    #[case(Time { hour: 23, minute: 0 }, true)]
    fn has_sun_set(#[case] time: Time, #[case] expected: bool) {
        // Sunset at given location and date: 2000-08-04T21:26:42+02:00
        let fixed_date_time = Local.with_ymd_and_hms(2000, 8, 4, time.hour as u32, time.minute as u32, 0).unwrap();
        let context = &context_with_location().now(fixed_date_time).build();
        let result = evaluate(&Temporal { expression: HasSunSet }, &context).unwrap();
        assert_eq!(result, Value::Boolean(expected));
    }

    #[rstest]
    #[case(Time { hour: 0, minute: 0 }, false)]
    #[case(Time { hour: 14, minute: 0 }, true)]
    #[case(Time { hour: 23, minute: 0 }, false)]
    fn is_daytime(#[case] time: Time, #[case] expected: bool) {
        // Sunrise and sunset at given location and date: 2000-08-04T06:09:31+02:00 and 2000-08-04T21:26:42+02:00
        let fixed_date_time = Local.with_ymd_and_hms(2000, 8, 4, time.hour as u32, time.minute as u32, 0).unwrap();
        let context = &context_with_location().now(fixed_date_time).build();
        let result = evaluate(&Temporal { expression: IsDaytime }, &context).unwrap();
        assert_eq!(result, Value::Boolean(expected));
    }

    #[rstest]
    #[case(Time { hour: 0, minute: 0 }, true)]
    #[case(Time { hour: 14, minute: 0 }, false)]
    #[case(Time { hour: 23, minute: 0 }, true)]
    fn is_nighttime(#[case] time: Time, #[case] expected: bool) {
        // Sunrise and sunset at given location and date: 2000-08-04T06:09:31+02:00 and 2000-08-04T21:26:42+02:00
        let fixed_date_time = Local.with_ymd_and_hms(2000, 8, 4, time.hour as u32, time.minute as u32, 0).unwrap();
        let context = &context_with_location().now(fixed_date_time).build();
        let result = evaluate(&Temporal { expression: IsNighttime }, &context).unwrap();
        assert_eq!(result, Value::Boolean(expected));
    }

    // Polar day/night: the sun never sets (day) or never rises (night) that date, so these expressions
    // must hold regardless of the time of day - verified below at midnight, noon and just before midnight.

    #[rstest]
    #[case::just_after_midnight(Time { hour: 0, minute: 0 })]
    #[case::midday(Time { hour: 12, minute: 0 })]
    #[case::just_before_midnight(Time { hour: 23, minute: 59 })]
    fn has_sun_risen_on_polar_day(#[case] time: Time) {
        let fixed_date_time = Local.with_ymd_and_hms(2000, 6, 21, time.hour as u32, time.minute as u32, 0).unwrap();
        let context = &context_with_polar_location().now(fixed_date_time).build();
        let result = evaluate(&Temporal { expression: HasSunRisen }, &context).unwrap();
        assert_eq!(result, Value::Boolean(true));
    }

    #[rstest]
    #[case::just_after_midnight(Time { hour: 0, minute: 0 })]
    #[case::midday(Time { hour: 12, minute: 0 })]
    #[case::just_before_midnight(Time { hour: 23, minute: 59 })]
    fn has_sun_risen_on_polar_night(#[case] time: Time) {
        let fixed_date_time = Local.with_ymd_and_hms(2000, 2, 13, time.hour as u32, time.minute as u32, 0).unwrap();
        let context = &context_with_polar_location().now(fixed_date_time).build();
        let result = evaluate(&Temporal { expression: HasSunRisen }, &context).unwrap();
        assert_eq!(result, Value::Boolean(false));
    }

    #[rstest]
    #[case::just_after_midnight(Time { hour: 0, minute: 0 })]
    #[case::midday(Time { hour: 12, minute: 0 })]
    #[case::just_before_midnight(Time { hour: 23, minute: 59 })]
    fn has_sun_set_on_polar_day(#[case] time: Time) {
        let fixed_date_time = Local.with_ymd_and_hms(2000, 6, 21, time.hour as u32, time.minute as u32, 0).unwrap();
        let context = &context_with_polar_location().now(fixed_date_time).build();
        let result = evaluate(&Temporal { expression: HasSunSet }, &context).unwrap();
        assert_eq!(result, Value::Boolean(true));
    }

    #[rstest]
    #[case::just_after_midnight(Time { hour: 0, minute: 0 })]
    #[case::midday(Time { hour: 12, minute: 0 })]
    #[case::just_before_midnight(Time { hour: 23, minute: 59 })]
    fn has_sun_set_on_polar_night(#[case] time: Time) {
        let fixed_date_time = Local.with_ymd_and_hms(2000, 2, 13, time.hour as u32, time.minute as u32, 0).unwrap();
        let context = &context_with_polar_location().now(fixed_date_time).build();
        let result = evaluate(&Temporal { expression: HasSunSet }, &context).unwrap();
        assert_eq!(result, Value::Boolean(false));
    }

    #[rstest]
    #[case::just_after_midnight(Time { hour: 0, minute: 0 })]
    #[case::midday(Time { hour: 12, minute: 0 })]
    #[case::just_before_midnight(Time { hour: 23, minute: 59 })]
    fn is_daytime_on_polar_day(#[case] time: Time) {
        let fixed_date_time = Local.with_ymd_and_hms(2000, 6, 21, time.hour as u32, time.minute as u32, 0).unwrap();
        let context = &context_with_polar_location().now(fixed_date_time).build();
        let result = evaluate(&Temporal { expression: IsDaytime }, &context).unwrap();
        assert_eq!(result, Value::Boolean(true));
    }

    #[rstest]
    #[case::just_after_midnight(Time { hour: 0, minute: 0 })]
    #[case::midday(Time { hour: 12, minute: 0 })]
    #[case::just_before_midnight(Time { hour: 23, minute: 59 })]
    fn is_daytime_on_polar_night(#[case] time: Time) {
        let fixed_date_time = Local.with_ymd_and_hms(2000, 2, 13, time.hour as u32, time.minute as u32, 0).unwrap();
        let context = &context_with_polar_location().now(fixed_date_time).build();
        let result = evaluate(&Temporal { expression: IsDaytime }, &context).unwrap();
        assert_eq!(result, Value::Boolean(false));
    }

    #[rstest]
    #[case::just_after_midnight(Time { hour: 0, minute: 0 })]
    #[case::midday(Time { hour: 12, minute: 0 })]
    #[case::just_before_midnight(Time { hour: 23, minute: 59 })]
    fn is_nighttime_on_polar_day(#[case] time: Time) {
        let fixed_date_time = Local.with_ymd_and_hms(2000, 6, 21, time.hour as u32, time.minute as u32, 0).unwrap();
        let context = &context_with_polar_location().now(fixed_date_time).build();
        let result = evaluate(&Temporal { expression: IsNighttime }, &context).unwrap();
        assert_eq!(result, Value::Boolean(false));
    }

    #[rstest]
    #[case::just_after_midnight(Time { hour: 0, minute: 0 })]
    #[case::midday(Time { hour: 12, minute: 0 })]
    #[case::just_before_midnight(Time { hour: 23, minute: 59 })]
    fn is_nighttime_on_polar_night(#[case] time: Time) {
        let fixed_date_time = Local.with_ymd_and_hms(2000, 2, 13, time.hour as u32, time.minute as u32, 0).unwrap();
        let context = &context_with_polar_location().now(fixed_date_time).build();
        let result = evaluate(&Temporal { expression: IsNighttime }, &context).unwrap();
        assert_eq!(result, Value::Boolean(true));
    }

    #[test]
    fn property_value_evaluates_to_none_for_a_battery_state_without_a_reading() {
        let battery_state_property: Box<dyn Property> = Box::new(
            EnumProperty::new("batteryState".to_string(), PropertyType::BatteryState, true, None, None, vec!["normal".to_string(), "low".to_string(), "critical".to_string()]).unwrap(),
        );
        let device = DeviceBuilder::new("device")
            .with_properties(vec![battery_state_property])
            .build();
        let devices: DeviceMap = HashMap::from([(device.id.clone(), Arc::new(device))]);
        let snapshot = StoreSnapshot { devices: Arc::new(devices) };

        let result = evaluate(
            &PropertyValue { device_id: "device".to_string(), property_id: "batteryState".to_string() },
            &Context::builder().snapshot(snapshot).build(),
        );

        assert_eq!(result, Ok(Value::None));
    }

    #[rstest]
    #[case::direct(PropertyChanged{ device_id: "light".to_string(), property_id: "on".to_string() }, true)]
    #[case::wrapped_in_not(Not{ expression: Box::new(PropertyChanged { device_id: "light".to_string(), property_id: "on".to_string() }) }, true)]
    #[case::wrapped_in_and(And{
        lhs: Box::new(PropertyChanged { device_id: "light".to_string(), property_id: "on".to_string() }),
        rhs: Box::new(Literal { value: Value::Boolean(true) })
        }, true)]
    #[case::property_value_only(PropertyValue{ device_id: "light".to_string(), property_id: "on".to_string() }, false)]
    #[case::literal_only(Literal{ value: Value::Boolean(true) }, false)]
    fn contains_property_changed_detects_it_anywhere_in_the_tree(#[case] expression: Expression, #[case] expected: bool) {
        assert_eq!(expression.contains_property_changed(), expected);
    }

    #[test]
    fn walk_visits_all_expressions_depth_first_left_before_right() {
        let a = Literal { value: Value::Number(Number::PositiveInt(1)) };
        let b = PropertyValue { device_id: "d".to_string(), property_id: "p".to_string() };
        let c = Literal { value: Value::Boolean(true) };
        let expression = Or {
            lhs: Box::new(GreaterThan { lhs: Box::new(a), rhs: Box::new(b) }),
            rhs: Box::new(Not { expression: Box::new(c) }),
        };

        let visited: Vec<&str> = expression.walk().map(|e| match e {
            Or { .. } => "or",
            GreaterThan { .. } => "gt",
            Literal { value: Value::Number(_) } => "number",
            PropertyValue { .. } => "property",
            Not { .. } => "not",
            Literal { value: Value::Boolean(_) } => "boolean",
            _ => "other",
        }).collect();

        assert_eq!(visited, vec!["or", "gt", "number", "property", "not", "boolean"]);
    }

    #[rstest]
    #[case::add(Add{ lhs: Box::new(Literal { value: Value::Number(Number::PositiveInt(1)) }), rhs: Box::new(Literal { value: Value::Number(Number::PositiveInt(2)) }) })]
    #[case::subtract(Subtract{ lhs: Box::new(Literal { value: Value::Number(Number::PositiveInt(1)) }), rhs: Box::new(Literal { value: Value::Number(Number::PositiveInt(2)) }) })]
    fn walk_visits_both_operands_of_an_arithmetic_expression(#[case] expression: Expression) {
        let visited: Vec<Option<&Value>> = expression.walk().map(Expression::constant_value).collect();

        assert_eq!(visited, vec![None, Some(&Value::Number(Number::PositiveInt(1))), Some(&Value::Number(Number::PositiveInt(2)))]);
    }

    #[rstest]
    #[case::number_literal(Literal{ value: Value::Number(Number::PositiveInt(42)) }, Some(Value::Number(Number::PositiveInt(42))))]
    #[case::boolean_literal(Literal{ value: Value::Boolean(true) }, Some(Value::Boolean(true)))]
    #[case::property_value(PropertyValue{ device_id: "light".to_string(), property_id: "brightness".to_string() }, None)]
    #[case::operator_on_literals(
        EqualTo{ lhs: Box::new(Literal { value: Value::Number(Number::PositiveInt(1)) }), rhs: Box::new(Literal{ value: Value::Number(Number::PositiveInt(1)) }) },
        None
    )]
    #[case::temporal(Temporal{ expression: IsDaytime }, None)]
    fn constant_value_is_only_known_for_a_literal(#[case] expression: Expression, #[case] expected: Option<Value>) {
        assert_eq!(expression.constant_value(), expected.as_ref());
    }

    mod value_kind {
        use super::*;
        use crate::domain::property::ValueKind;

        const DEVICE_ID: &str = "ab917a9a-a7d5-4853-9518-75909236a182";

        fn snapshot() -> StoreSnapshot {
            let device = device();
            StoreSnapshot { devices: Arc::new(HashMap::from([(device.id.clone(), Arc::new(device))])) }
        }

        fn literal(value: Value) -> Box<Expression> {
            Box::new(Literal { value })
        }

        fn property_value(device_id: &str, property_id: &str) -> Box<Expression> {
            Box::new(PropertyValue { device_id: device_id.to_string(), property_id: property_id.to_string() })
        }

        #[rstest]
        #[case::boolean(*literal(Value::Boolean(true)), ValueKind::Boolean)]
        #[case::color(*literal(Value::Color(Color::Hex("#ff0000".to_string()))), ValueKind::Color)]
        #[case::date_time(*literal(Value::DateTime(utc_with_ymd(2000, 8, 4))), ValueKind::DateTime)]
        #[case::string(*literal(Value::String("normal".to_string())), ValueKind::Enum)]
        #[case::number(*literal(Value::Number(Number::PositiveInt(42))), ValueKind::Number)]
        #[case::boolean_property(*property_value(DEVICE_ID, "on"), ValueKind::Boolean)]
        #[case::number_property(*property_value(DEVICE_ID, "brightness"), ValueKind::Number)]
        #[case::enum_property(*property_value(DEVICE_ID, "batteryState"), ValueKind::Enum)]
        #[case::property_changed(PropertyChanged{ device_id: DEVICE_ID.to_string(), property_id: "on".to_string() }, ValueKind::Boolean)]
        #[case::temporal(Temporal{ expression: IsDaytime }, ValueKind::Boolean)]
        #[case::compare_numbers(GreaterThan{ lhs: property_value(DEVICE_ID, "brightness"), rhs: literal(Value::Number(Number::PositiveInt(50))) }, ValueKind::Boolean)]
        #[case::compare_date_times(
            LessThan{ lhs: literal(Value::DateTime(utc_with_ymd(2000, 8, 4))), rhs: literal(Value::DateTime(utc_with_ymd(2000, 8, 5))) },
            ValueKind::Boolean
        )]
        #[case::equal_enums(EqualTo{ lhs: property_value(DEVICE_ID, "batteryState"), rhs: literal(Value::String("low".to_string())) }, ValueKind::Boolean)]
        #[case::not_equal_colors(NotEqualTo{ lhs: property_value(DEVICE_ID, "color"), rhs: literal(Value::Color(Color::Hex("#ff0000".to_string()))) }, ValueKind::Boolean)]
        #[case::and(And{ lhs: literal(Value::Boolean(true)), rhs: property_value(DEVICE_ID, "on") }, ValueKind::Boolean)]
        #[case::or(Or{ lhs: literal(Value::Boolean(true)), rhs: literal(Value::Boolean(false)) }, ValueKind::Boolean)]
        #[case::not(Not{ expression: property_value(DEVICE_ID, "on") }, ValueKind::Boolean)]
        #[case::add(Add{ lhs: property_value(DEVICE_ID, "brightness"), rhs: literal(Value::Number(Number::PositiveInt(10))) }, ValueKind::Number)]
        #[case::subtract(Subtract{ lhs: property_value(DEVICE_ID, "brightness"), rhs: literal(Value::Number(Number::Float(0.5))) }, ValueKind::Number)]
        #[case::nested_arithmetic(
            GreaterThan{ lhs: Box::new(Add { lhs: property_value(DEVICE_ID, "brightness"), rhs: literal(Value::Number(Number::PositiveInt(10))) }), rhs: literal(Value::Number(Number::PositiveInt(50))) },
            ValueKind::Boolean
        )]
        fn infers_the_kind(#[case] expression: Expression, #[case] expected: ValueKind) {
            assert_eq!(expression.value_kind(&snapshot()), Ok(expected));
        }

        #[rstest]
        #[case::compare_booleans(
            GreaterThanOrEqualTo{ lhs: literal(Value::Boolean(true)), rhs: literal(Value::Boolean(false)) },
            Problem::IncompatibleOperands{ operator: "GreaterThanOrEqualTo", expected: "Number|DateTime", actual: vec![ValueKind::Boolean, ValueKind::Boolean] }
        )]
        #[case::compare_number_with_date_time(
            LessThanOrEqualTo{ lhs: literal(Value::Number(Number::PositiveInt(1))), rhs: literal(Value::DateTime(utc_with_ymd(2000, 8, 4))) },
            Problem::IncompatibleOperands{ operator: "LessThanOrEqualTo", expected: "Number|DateTime", actual: vec![ValueKind::Number, ValueKind::DateTime] }
        )]
        #[case::equal_different_kinds(
            EqualTo{ lhs: property_value(DEVICE_ID, "on"), rhs: literal(Value::Number(Number::PositiveInt(1))) },
            Problem::IncompatibleOperands{ operator: "EqualTo", expected: "operands of the same kind", actual: vec![ValueKind::Boolean, ValueKind::Number] }
        )]
        #[case::not_equal_different_kinds(
            NotEqualTo{ lhs: literal(Value::String("low".to_string())), rhs: literal(Value::Boolean(true)) },
            Problem::IncompatibleOperands{ operator: "NotEqualTo", expected: "operands of the same kind", actual: vec![ValueKind::Enum, ValueKind::Boolean] }
        )]
        #[case::and_with_a_number(
            And{ lhs: literal(Value::Boolean(true)), rhs: property_value(DEVICE_ID, "brightness") },
            Problem::IncompatibleOperands{ operator: "And", expected: "Boolean", actual: vec![ValueKind::Boolean, ValueKind::Number] }
        )]
        #[case::or_with_a_color(
            Or{ lhs: literal(Value::Color(Color::Hex("#ff0000".to_string()))), rhs: literal(Value::Boolean(false)) },
            Problem::IncompatibleOperands{ operator: "Or", expected: "Boolean", actual: vec![ValueKind::Color, ValueKind::Boolean] }
        )]
        #[case::not_a_number(
            Not{ expression: literal(Value::Number(Number::PositiveInt(1))) },
            Problem::IncompatibleOperands{ operator: "Not", expected: "Boolean", actual: vec![ValueKind::Number] }
        )]
        #[case::nested(
            And{ lhs: Box::new(Not { expression: literal(Value::Number(Number::PositiveInt(1))) }), rhs: literal(Value::Boolean(true)) },
            Problem::IncompatibleOperands{ operator: "Not", expected: "Boolean", actual: vec![ValueKind::Number] }
        )]
        #[case::add_a_boolean(
            Add{ lhs: property_value(DEVICE_ID, "brightness"), rhs: property_value(DEVICE_ID, "on") },
            Problem::IncompatibleOperands{ operator: "Add", expected: "Number", actual: vec![ValueKind::Number, ValueKind::Boolean] }
        )]
        #[case::subtract_a_date_time(
            Subtract{ lhs: literal(Value::DateTime(utc_with_ymd(2000, 8, 4))), rhs: literal(Value::Number(Number::PositiveInt(1))) },
            Problem::IncompatibleOperands{ operator: "Subtract", expected: "Number", actual: vec![ValueKind::DateTime, ValueKind::Number] }
        )]
        #[case::unknown_device(
            *property_value("missing", "on"),
            Problem::UnknownDevice { device_id: "missing".to_string() }
        )]
        #[case::unknown_property(
            EqualTo{ lhs: property_value(DEVICE_ID, "missing"), rhs: literal(Value::Boolean(true)) },
            Problem::UnknownProperty{ device_id: DEVICE_ID.to_string(), property_id: "missing".to_string() }
        )]
        fn reports_a_problem(#[case] expression: Expression, #[case] expected: Problem) {
            assert_eq!(expression.value_kind(&snapshot()), Err(expected));
        }

        #[test]
        fn incompatible_operands_displays_the_operator_and_the_operand_kinds() {
            let problem = Problem::IncompatibleOperands { operator: "And", expected: "Boolean", actual: vec![ValueKind::Boolean, ValueKind::Number] };

            assert_eq!(problem.to_string(), "incompatible operands for 'And': expected Boolean, got boolean and number");
        }
    }
}
