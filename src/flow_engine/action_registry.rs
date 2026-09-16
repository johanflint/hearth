use crate::flow_engine::action::Action;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

pub(in crate::flow_engine) static ACTION_REGISTRY: LazyLock<RwLock<HashMap<String, fn(&serde_json::Value) -> Result<Box<dyn Action>, serde_json::Error>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

pub(in crate::flow_engine) fn register_action<T: Action + Send + Sync + Default + DeserializeOwned + 'static>() {
    let kind = T::default().kind().to_owned();
    ACTION_REGISTRY
        .write()
        .unwrap()
        .insert(kind, |json| serde_json::from_value::<T>(json.clone()).map(|a| Box::new(a) as Box<dyn Action>));
}

pub(in crate::flow_engine) fn known_actions() -> Vec<String> {
    let registry = ACTION_REGISTRY.read().unwrap();
    registry.keys().map(|v| v.as_str().to_owned()).collect()
}
