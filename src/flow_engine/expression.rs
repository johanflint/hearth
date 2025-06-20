use crate::domain::Number;
use std::cmp::Ordering;

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
}

#[derive(PartialEq, Debug, Clone)]
pub enum Value {
    Boolean(bool),
    Number(Number),
}

pub fn evaluate(expression: &Expression) -> Result<Value, &'static str> {
    use Expression::*;

    match expression {
        // Comparison
        GreaterThanOrEqualTo(lhs, rhs) => compare(lhs, rhs, |o| o != Ordering::Less),
        GreaterThan(lhs, rhs) => compare(lhs, rhs, |o| o == Ordering::Greater),
        LessThan(lhs, rhs) => compare(lhs, rhs, |o| o == Ordering::Less),
        LessThanOrEqualTo(lhs, rhs) => compare(lhs, rhs, |o| o != Ordering::Greater),

        // Equality
        EqualTo(lhs, rhs) => match (evaluate(lhs)?, evaluate(rhs)?) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Boolean(a.eq(&b))),
            (Value::Boolean(a), Value::Boolean(b)) => Ok(Value::Boolean(a == b)),
            _ => Err("Invalid expression, must compare two values of the same type"),
        },
        NotEqualTo(lhs, rhs) => match (evaluate(lhs)?, evaluate(rhs)?) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Boolean(!a.eq(&b))),
            (Value::Boolean(a), Value::Boolean(b)) => Ok(Value::Boolean(a != b)),
            _ => Err("Invalid expression, must compare two values of the same type"),
        },

        // Literal
        Literal(value) => Ok(value.clone()),
    }
}

fn compare(lhs: &Expression, rhs: &Expression, cmp: fn(Ordering) -> bool) -> Result<Value, &'static str> {
    match (evaluate(lhs)?, evaluate(rhs)?) {
        (Value::Number(a), Value::Number(b)) => Ok(Value::Boolean(cmp(a.partial_cmp(&b).ok_or("not a number")?))),
        _ => Err("Comparison failed: expected numbers"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow_engine::expression::Expression::*;
    use rstest::rstest;

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
        let result = evaluate(&GreaterThanOrEqualTo(Box::new(Literal(Value::Number(lhs))), Box::new(Literal(Value::Number(rhs))))).unwrap();
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
        let result = evaluate(&GreaterThan(Box::new(Literal(Value::Number(lhs))), Box::new(Literal(Value::Number(rhs))))).unwrap();
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
        let result = evaluate(&LessThan(Box::new(Literal(Value::Number(lhs))), Box::new(Literal(Value::Number(rhs))))).unwrap();
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
        let result = evaluate(&LessThanOrEqualTo(Box::new(Literal(Value::Number(lhs))), Box::new(Literal(Value::Number(rhs))))).unwrap();
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
        let result = evaluate(&EqualTo(Box::new(Literal(Value::Number(lhs))), Box::new(Literal(Value::Number(rhs))))).unwrap();
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
        let result = evaluate(&NotEqualTo(Box::new(Literal(Value::Number(lhs))), Box::new(Literal(Value::Number(rhs))))).unwrap();
        assert_eq!(result, Value::Boolean(expected));
    }
}
