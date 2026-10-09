use crate::flow_engine::{Expression, Value};
use std::time::Duration;

#[derive(PartialEq, Debug)]
pub struct PropertyCommand {
    pub value: Expression,
    pub transition: Option<Duration>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct ResolvedPropertyCommand {
    pub value: Value,
    pub transition: Option<Duration>,
}
