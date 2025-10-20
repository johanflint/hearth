use crate::store::StoreSnapshot;
use chrono::{DateTime, Local};

#[derive(Default, Debug)]
pub struct Context {
    snapshot: StoreSnapshot,
    now: DateTime<Local>,
}

impl Context {
    pub fn new(snapshot: StoreSnapshot) -> Self {
        Context { snapshot, now: Local::now() }
    }

    pub fn new_with_now(snapshot: StoreSnapshot, now: DateTime<Local>) -> Self {
        Context { snapshot, now }
    }

    pub fn snapshot(&self) -> &StoreSnapshot {
        &self.snapshot
    }

    pub fn now(&self) -> DateTime<Local> {
        self.now
    }
}
