use crate::domain::controller::Controller;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

static CONTROLLER_REGISTRY: LazyLock<RwLock<HashMap<&'static str, Arc<dyn Controller>>>> = LazyLock::new(|| RwLock::new(HashMap::new()));

pub fn register(controller: Arc<dyn Controller>) {
    CONTROLLER_REGISTRY.write().unwrap().insert(controller.id(), controller);
}

pub fn get(controller_id: &str) -> Option<Arc<dyn Controller>> {
    CONTROLLER_REGISTRY.read().unwrap().get(controller_id).cloned()
}
