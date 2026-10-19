use std::collections::{BTreeSet, HashMap};
use std::sync::RwLock;

type DeviceId = String;
type GroupedLightId = String;

#[derive(Default, Debug)]
pub struct GroupedLightIndex {
    index: RwLock<HashMap<BTreeSet<DeviceId>, GroupedLightId>>,
}

impl GroupedLightIndex {
    pub fn replace_all(&self, index: HashMap<BTreeSet<DeviceId>, GroupedLightId>) {
        *self.index.write().expect("grouped light index lock poisoned") = index;
    }
}
