use crate::domain::Number;
use crate::domain::property::{BooleanProperty, NumberProperty, PropertyType};
use crate::flow_engine::Context;
use std::cmp::Ordering;
use tracing::warn;

pub enum Expression {
    // Comparison
    GreaterThanOrEqualTo(Box<Expression>, Box<Expression>),
    GreaterThan(Box<Expression>, Box<Expression>),
    LessThan(Box<Expression>, Box<Expression>),
    LessThanOrEqualTo(Box<Expression>, Box<Expression>),

    // Equality
    EqualTo(Box<Expression>, Box<Expression>),
    NotEqualTo(Box<Expression>, Box<Expression>),

    // Literal
    Literal(Value),

    // Property
    PropertyValue { device_id: String, property_id: String },
}

#[derive(PartialEq, Debug, Clone)]
pub enum Value {
    Boolean(bool),
    Number(Number),
}

pub fn evaluate(expression: &Expression, context: &Context) -> Result<Value, &'static str> {
    use Expression::*;

    match expression {
        // Comparison
        GreaterThanOrEqualTo(lhs, rhs) => compare(lhs, rhs, |o| o != Ordering::Less, context),
        GreaterThan(lhs, rhs) => compare(lhs, rhs, |o| o == Ordering::Greater, context),
        LessThan(lhs, rhs) => compare(lhs, rhs, |o| o == Ordering::Less, context),
        LessThanOrEqualTo(lhs, rhs) => compare(lhs, rhs, |o| o != Ordering::Greater, context),

        // Equality
        EqualTo(lhs, rhs) => match (evaluate(lhs, context)?, evaluate(rhs, context)?) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Boolean(a.eq(&b))),
            (Value::Boolean(a), Value::Boolean(b)) => Ok(Value::Boolean(a == b)),
            _ => Err("Invalid expression, must compare two values of the same type"),
        },
        NotEqualTo(lhs, rhs) => match (evaluate(lhs, context)?, evaluate(rhs, context)?) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Boolean(!a.eq(&b))),
            (Value::Boolean(a), Value::Boolean(b)) => Ok(Value::Boolean(a != b)),
            _ => Err("Invalid expression, must compare two values of the same type"),
        },

        // Literal
        Literal(value) => Ok(value.clone()),

        // Property
        PropertyValue { device_id, property_id } => {
            let Some(device) = context.snapshot().devices.get(device_id) else {
                warn!(device_id, "⚠️ Received property changed event for unknown device '{}'", device_id);
                return Err("Unknown device");
            };

            let Some(property) = device.properties.get(property_id) else {
                warn!(device_id = device.id, "⚠️ Unknown property '{}' for device '{}'", property_id, device.name);
                return Err("Unknown property");
            };

            match property.property_type() {
                PropertyType::Brightness => {
                    let value = property.as_any().downcast_ref::<NumberProperty>().unwrap();
                    Ok(Value::Number(value.value()))
                }
                PropertyType::Color => Err("Unsupported property type"),
                PropertyType::ColorTemperature => Err("Unsupported property type"),
                PropertyType::On => {
                    let value = property.as_any().downcast_ref::<BooleanProperty>().unwrap();
                    Ok(Value::Boolean(value.value()))
                }
            }
        }
    }
}

fn compare(lhs: &Expression, rhs: &Expression, cmp: fn(Ordering) -> bool, context: &Context) -> Result<Value, &'static str> {
    match (evaluate(lhs, context)?, evaluate(rhs, context)?) {
        (Value::Number(a), Value::Number(b)) => Ok(Value::Boolean(cmp(a.partial_cmp(&b).ok_or("not a number")?))),
        _ => Err("Comparison failed: expected numbers"),
    }
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
            &GreaterThanOrEqualTo(Box::new(Literal(Value::Number(lhs))), Box::new(Literal(Value::Number(rhs)))),
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
        let result = evaluate(&GreaterThan(Box::new(Literal(Value::Number(lhs))), Box::new(Literal(Value::Number(rhs)))), &context()).unwrap();
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
        let result = evaluate(&LessThan(Box::new(Literal(Value::Number(lhs))), Box::new(Literal(Value::Number(rhs)))), &context()).unwrap();
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
        let result = evaluate(&LessThanOrEqualTo(Box::new(Literal(Value::Number(lhs))), Box::new(Literal(Value::Number(rhs)))), &context()).unwrap();
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
        let result = evaluate(&EqualTo(Box::new(Literal(Value::Number(lhs))), Box::new(Literal(Value::Number(rhs)))), &context()).unwrap();
        assert_eq!(result, Value::Boolean(expected));
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
        let result = evaluate(&NotEqualTo(Box::new(Literal(Value::Number(lhs))), Box::new(Literal(Value::Number(rhs)))), &context()).unwrap();
        assert_eq!(result, Value::Boolean(expected));
    }

    #[rstest]
    #[case::unknown_device("unknown_device_id", "", Err("Unknown device"))]
    #[case::unknown_property("42", "unknown_property_id", Err("Unknown device"))]
    #[case::boolean("ab917a9a-a7d5-4853-9518-75909236a182", "on", Ok(Value::Boolean(true)))]
    #[case::number("ab917a9a-a7d5-4853-9518-75909236a182", "brightness", Ok(Value::Number(Number::Float(58.89))))]
    fn property_value(#[case] device_id: &str, #[case] property_id: &str, #[case] expected: Result<Value, &str>) {
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
