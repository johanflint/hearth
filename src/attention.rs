use crate::domain::{BatteryState, Connectivity};
use chrono::{DateTime, Utc};

#[derive(Debug, PartialEq)]
pub struct Attention {
    pub subject: Subject,
    pub issue: Issue,
    pub since: DateTime<Utc>,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Subject {
    Device(String),
}

#[derive(Debug, PartialEq)]
pub enum Issue {
    Connectivity(Connectivity),
    Battery(BatteryState),
}

impl Issue {
    pub fn is_same_kind(&self, other: &Issue) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}
