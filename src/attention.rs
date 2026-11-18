use crate::domain::{BatteryState, Connectivity};
use crate::integration::IntegrationStatus;
use chrono::{DateTime, Utc};
use std::sync::Arc;

#[derive(Debug, PartialEq)]
pub struct Attention {
    pub subject: Subject,
    pub issue: Issue,
    pub since: DateTime<Utc>,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Subject {
    Device(String),
    Integration(String),
}

#[derive(Debug, PartialEq)]
pub enum Issue {
    Connectivity(Connectivity),
    Battery(BatteryState),
    Integration(IntegrationStatus),
}

impl Issue {
    pub fn is_same_kind(&self, other: &Issue) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AttentionList {
    pub devices: Arc<[Attention]>,
    pub integrations: Arc<[Attention]>,
}

impl AttentionList {
    pub fn iter(&self) -> impl Iterator<Item = &Attention> {
        self.devices.iter().chain(self.integrations.iter())
    }
}
