use crate::domain::Number;
use crate::domain::property::{BooleanProperty, NumberProperty, PropertyType};
use crate::flow_engine::Context;
use crate::flow_engine::expression::ExpressionError::UnknownProperty;
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

    // Literal
    Literal { value: Value },

    // Property
    PropertyValue { device_id: String, property_id: String },
}

#[derive(PartialEq, Debug, Clone)]
pub enum Value {
    Boolean(bool),
    Number(Number),
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
            _ => Err(ExpressionError::OperandTypeMismatch {
                operand: "NotEqualTo",
                expected: "Boolean|Number",
                actual_lhs: format!("{:?}", lhs),
                actual_rhs: format!("{:?}", rhs),
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
                    let value = property.as_any().downcast_ref::<NumberProperty>().unwrap();
                    Ok(Value::Number(value.value().ok_or_else(|| ExpressionError::NoneValue)?))
                }
                PropertyType::Color => Err(ExpressionError::UnsupportedPropertyType(property.property_type())),
                PropertyType::ColorTemperature => Err(ExpressionError::UnsupportedPropertyType(property.property_type())),
                PropertyType::On => {
                    let value = property.as_any().downcast_ref::<BooleanProperty>().unwrap();
                    Ok(Value::Boolean(value.value()))
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
    #[error("operand type mismatch for operand {operand}")]
    OperandTypeMismatch {
        operand: &'static str,
        expected: &'static str,
        actual_lhs: String,
        actual_rhs: String,
    },
    #[error("unknown device '{0}'")]
    UnknownDevice(String),
    #[error("unknown property '{property_id}' for device '{device_id}'")]
    UnknownProperty { device_id: String, property_id: String },
    #[error("property type '{0:?}' is not supported as a property value")]
    UnsupportedPropertyType(PropertyType),
    #[error("unable to compare given Numbers {actual_lhs} and {actual_rhs}")]
    ComparisonFailed { actual_lhs: String, actual_rhs: String },
    #[error("value is None")]
    NoneValue,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::device::{Device, DeviceType};
    use crate::domain::property::{CartesianCoordinate, ColorProperty, Gamut, Property, Unit};
    use crate::flow_engine::expression::Expression::*;
    use crate::store::{DeviceMap, StoreSnapshot};
    use rstest::rstest;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn context() -> Context {
        Context::new(StoreSnapshot::default())
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
    #[case(Value::Boolean(true), Value::Number(Number::PositiveInt(2)), ExpressionError::OperandTypeMismatch {
                operand: "EqualTo",
                expected: "Boolean|Number",
                actual_lhs: "Literal { value: Boolean(true) }".to_string(),
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
    #[case(Value::Boolean(true), Value::Number(Number::PositiveInt(2)), ExpressionError::OperandTypeMismatch {
                operand: "NotEqualTo",
                expected: "Boolean|Number",
                actual_lhs: "Literal { value: Boolean(true) }".to_string(),
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
            &Context::new(snapshot),
        );

        assert_eq!(result, expected);
    }
}
