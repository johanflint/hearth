use crate::domain::Preset;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Default, Debug)]
pub struct PresetRegistry {
    presets: RwLock<HashMap<String, Arc<Preset>>>,
}

impl PresetRegistry {
    pub fn new(presets: Vec<Preset>) -> Self {
        let entries = presets.into_iter().map(|preset| (preset.id.clone(), Arc::new(preset))).collect();

        Self { presets: RwLock::new(entries) }
    }

    pub fn by_id(&self, id: &str) -> Option<Arc<Preset>> {
        self.presets.read().expect("preset registry lock poisoned").get(id).cloned()
    }

    pub fn register(&self, preset: Preset) -> RegisterResult {
        let mut entries = self.presets.write().expect("preset registry lock poisoned");
        match entries.insert(preset.id.clone(), Arc::new(preset)) {
            None => RegisterResult::Added,
            Some(_) => RegisterResult::Replaced,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum RegisterResult {
    Added,
    Replaced,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Number, Value};

    fn preset(id: &str, name: &str, value: Value) -> Preset {
        Preset { id: id.to_string(), name: name.to_string(), value }
    }

    #[test]
    fn by_id_returns_the_preset() {
        let registry = PresetRegistry::new(vec![
            preset("dim", "Dim", Value::Number(Number::PositiveInt(10))),
            preset("enabled", "Enabled", Value::Boolean(true)),
        ]);

        let preset = registry.by_id("dim").expect("preset exists");

        assert_eq!(preset.id, "dim");
        assert_eq!(preset.name, "Dim");
        assert_eq!(preset.value, Value::Number(Number::PositiveInt(10)));
    }

    #[test]
    fn by_id_returns_none_for_an_unknown_id() {
        let registry = PresetRegistry::new(vec![preset("dim", "Dim", Value::Boolean(true))]);

        assert!(registry.by_id("unknown").is_none());
    }

    #[test]
    fn by_id_returns_none_for_an_empty_registry() {
        assert!(PresetRegistry::default().by_id("dim").is_none());
    }

    #[test]
    fn by_id_shares_the_preset_instead_of_copying_it() {
        let registry = PresetRegistry::new(vec![preset("dim", "Dim", Value::Boolean(true))]);

        let first = registry.by_id("dim").expect("preset exists");
        let second = registry.by_id("dim").expect("preset exists");

        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn new_keeps_the_last_preset_for_a_duplicate_id() {
        let registry = PresetRegistry::new(vec![
            preset("dim", "First", Value::Boolean(true)),
            preset("dim", "Second", Value::Boolean(false)),
        ]);

        assert_eq!(registry.by_id("dim").expect("preset exists").name, "Second");
    }

    #[test]
    fn register_returns_added_for_an_unknown_preset() {
        let registry = PresetRegistry::default();

        let result = registry.register(preset("dim", "Dim", Value::Boolean(true)));

        assert_eq!(result, RegisterResult::Added);
        assert_eq!(registry.by_id("dim").expect("preset exists").name, "Dim");
    }

    #[test]
    fn register_returns_replaced_and_stores_the_new_preset_for_a_known_id() {
        let registry = PresetRegistry::new(vec![preset("dim", "Dim", Value::Boolean(true))]);

        let result = registry.register(preset("dim", "Dimmer", Value::Boolean(false)));

        let stored = registry.by_id("dim").expect("preset exists");
        assert_eq!(result, RegisterResult::Replaced);
        assert_eq!(stored.name, "Dimmer");
        assert_eq!(stored.value, Value::Boolean(false));
    }

    #[test]
    fn register_leaves_other_presets_untouched() {
        let registry = PresetRegistry::new(vec![
            preset("dim", "Dim", Value::Boolean(true)),
            preset("enabled", "Enabled", Value::Boolean(true)),
        ]);

        registry.register(preset("dim", "Dimmer", Value::Boolean(false)));

        assert_eq!(registry.by_id("enabled").expect("preset exists").name, "Enabled");
    }

    #[test]
    fn register_does_not_change_a_preset_a_caller_already_holds() {
        let registry = PresetRegistry::new(vec![preset("dim", "Dim", Value::Boolean(true))]);
        let before = registry.by_id("dim").expect("preset exists");

        registry.register(preset("dim", "Dimmer", Value::Boolean(false)));

        assert_eq!(before.name, "Dim");
        assert_eq!(before.value, Value::Boolean(true));
        assert!(!Arc::ptr_eq(&before, &registry.by_id("dim").expect("preset exists")));
    }
}
