use crate::store::StoreSnapshot;

#[derive(Default, Debug)]
pub struct Context {
    snapshot: StoreSnapshot,
}

impl Context {
    pub fn new(snapshot: StoreSnapshot) -> Self {
        Context { snapshot }
    }

    pub fn snapshot(&self) -> &StoreSnapshot {
        &self.snapshot
    }
}
