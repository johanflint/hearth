use crate::flow_engine::{Expression, Value};
use serde::Deserialize;
use std::time::Duration;

#[derive(PartialEq, Debug)]
pub struct PropertyCommand {
    pub operation: Operation,
    pub value: Expression,
    pub transition: Option<Duration>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct ResolvedPropertyCommand {
    pub operation: Operation,
    pub value: Value,
    pub transition: Option<Duration>,
}

#[derive(Clone, PartialEq, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub enum Operation { Set }
