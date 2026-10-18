use crate::domain::VersionedPreset;
use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Default, Debug)]
pub struct PresetRegistry {
    presets: RwLock<HashMap<String, VersionedPreset>>,
}

impl PresetRegistry {
    pub fn new(presets: Vec<VersionedPreset>) -> Self {
        let entries = presets.into_iter().map(|preset| (preset.id.clone(), preset)).collect();

        Self { presets: RwLock::new(entries) }
    }

    pub fn by_id(&self, id: &str) -> Option<VersionedPreset> {
        self.presets.read().expect("preset registry lock poisoned").get(id).cloned()
    }

    pub fn register(&self, preset: VersionedPreset) -> RegisterResult {
        let mut entries = self.presets.write().expect("preset registry lock poisoned");
        let Some(entry) = entries.get_mut(&preset.id) else {
            // Can happen with a stored preset that failed to load at boot and is now fixed
            entries.insert(preset.id.clone(), preset);
            return RegisterResult::Added;
        };

        if preset.revision <= entry.revision {
            return RegisterResult::Stale { current_revision: entry.revision };
        }

        *entry = preset;
        RegisterResult::Replaced
    }

    pub fn unregister(&self, id: &str, base_revision: u64) -> UnregisterResult {
        let mut entries = self.presets.write().expect("preset registry lock poisoned");
        let Some(entry) = entries.get(id) else {
            return UnregisterResult::NotFound;
        };

        if entry.revision > base_revision {
            return UnregisterResult::Stale { current_revision: entry.revision };
        }

        entries.remove(id);
        UnregisterResult::Deleted
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum RegisterResult {
    Added,
    Replaced,
    // A newer (or equal) revision is already stored; the registry is unchanged
    Stale { current_revision: u64 },
}

#[derive(Debug, PartialEq, Eq)]
pub enum UnregisterResult {
    NotFound,
    Deleted,
    // A newer revision is registered; the registry is unchanged
    Stale { current_revision: u64 },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Number, Preset, Value};
    use std::sync::Arc;

    fn preset(id: &str, name: &str, value: Value, revision: u64) -> VersionedPreset {
        VersionedPreset { preset: Arc::new(Preset { id: id.to_string(), name: name.to_string(), value }), revision }
    }

    #[test]
    fn by_id_returns_the_preset() {
        let registry = PresetRegistry::new(vec![
            preset("dim", "Dim", Value::Number(Number::PositiveInt(10)), 3),
            preset("enabled", "Enabled", Value::Boolean(true), 0),
        ]);

        let preset = registry.by_id("dim").expect("preset exists");

        assert_eq!(preset.id, "dim");
        assert_eq!(preset.name, "Dim");
        assert_eq!(preset.value, Value::Number(Number::PositiveInt(10)));
        assert_eq!(preset.revision, 3);
    }

    #[test]
    fn by_id_returns_none_for_an_unknown_id() {
        let registry = PresetRegistry::new(vec![preset("dim", "Dim", Value::Boolean(true), 0)]);

        assert!(registry.by_id("unknown").is_none());
    }

    #[test]
    fn by_id_returns_none_for_an_empty_registry() {
        assert!(PresetRegistry::default().by_id("dim").is_none());
    }

    #[test]
    fn by_id_shares_the_preset_instead_of_copying_it() {
        let registry = PresetRegistry::new(vec![preset("dim", "Dim", Value::Boolean(true), 0)]);

        let first = registry.by_id("dim").expect("preset exists");
        let second = registry.by_id("dim").expect("preset exists");

        assert!(Arc::ptr_eq(&first.preset, &second.preset));
    }

    #[test]
    fn new_keeps_the_last_preset_for_a_duplicate_id() {
        let registry = PresetRegistry::new(vec![
            preset("dim", "First", Value::Boolean(true), 0),
            preset("dim", "Second", Value::Boolean(false), 0),
        ]);

        assert_eq!(registry.by_id("dim").expect("preset exists").name, "Second");
    }

    #[test]
    fn register_returns_added_for_an_unknown_preset() {
        let registry = PresetRegistry::default();

        let result = registry.register(preset("dim", "Dim", Value::Boolean(true), 0));

        assert_eq!(result, RegisterResult::Added);
        assert_eq!(registry.by_id("dim").expect("preset exists").name, "Dim");
    }

    #[test]
    fn register_stores_a_newer_revision() {
        let registry = PresetRegistry::new(vec![preset("dim", "Dim", Value::Boolean(true), 0)]);

        let result = registry.register(preset("dim", "Dimmer", Value::Boolean(false), 1));

        let stored = registry.by_id("dim").expect("preset exists");
        assert_eq!(result, RegisterResult::Replaced);
        assert_eq!(stored.name, "Dimmer");
        assert_eq!(stored.value, Value::Boolean(false));
        assert_eq!(stored.revision, 1);
    }

    #[test]
    fn register_accepts_a_revision_gap() {
        let registry = PresetRegistry::new(vec![preset("dim", "Dim", Value::Boolean(true), 0)]);

        let result = registry.register(preset("dim", "Dimmer", Value::Boolean(false), 5));

        assert_eq!(result, RegisterResult::Replaced);
        assert_eq!(registry.by_id("dim").expect("preset exists").revision, 5);
    }

    #[test]
    fn register_rejects_an_older_revision() {
        let registry = PresetRegistry::new(vec![preset("dim", "Dim", Value::Boolean(true), 2)]);

        let result = registry.register(preset("dim", "Stale", Value::Boolean(false), 1));

        assert_eq!(result, RegisterResult::Stale { current_revision: 2 });
        let stored = registry.by_id("dim").expect("preset exists");
        assert_eq!(stored.name, "Dim");
        assert_eq!(stored.revision, 2);
    }

    #[test]
    fn register_rejects_the_same_revision() {
        let registry = PresetRegistry::new(vec![preset("dim", "Dim", Value::Boolean(true), 0)]);

        let result = registry.register(preset("dim", "Same", Value::Boolean(false), 0));

        assert_eq!(result, RegisterResult::Stale { current_revision: 0 });
        assert_eq!(registry.by_id("dim").expect("preset exists").name, "Dim");
    }

    #[test]
    fn register_leaves_other_presets_untouched() {
        let registry = PresetRegistry::new(vec![
            preset("dim", "Dim", Value::Boolean(true), 0),
            preset("enabled", "Enabled", Value::Boolean(true), 0),
        ]);

        registry.register(preset("dim", "Dimmer", Value::Boolean(false), 1));

        assert_eq!(registry.by_id("enabled").expect("preset exists").name, "Enabled");
    }

    #[test]
    fn register_does_not_change_a_preset_a_caller_already_holds() {
        let registry = PresetRegistry::new(vec![preset("dim", "Dim", Value::Boolean(true), 0)]);
        let before = registry.by_id("dim").expect("preset exists");

        registry.register(preset("dim", "Dimmer", Value::Boolean(false), 1));

        assert_eq!(before.name, "Dim");
        assert_eq!(before.value, Value::Boolean(true));
        assert_eq!(before.revision, 0);
        assert!(!Arc::ptr_eq(&before.preset, &registry.by_id("dim").expect("preset exists").preset));
    }

    #[test]
    fn unregister_removes_a_preset() {
        let registry = PresetRegistry::new(vec![preset("dim", "Dim", Value::Boolean(true), 0)]);

        let result = registry.unregister("dim", 0);

        assert_eq!(result, UnregisterResult::Deleted);
        assert!(registry.by_id("dim").is_none(), "expected the preset to be unregistered");
    }

    #[test]
    fn unregister_rejects_an_unknown_preset() {
        let registry = PresetRegistry::new(vec![preset("dim", "Dim", Value::Boolean(true), 0)]);

        let result = registry.unregister("unknown", 0);

        assert_eq!(result, UnregisterResult::NotFound);
        assert!(registry.by_id("dim").is_some(), "expected the preset to be there");
    }

    #[test]
    fn unregister_rejects_an_older_revision_than_registered() {
        let registry = PresetRegistry::new(vec![preset("dim", "Dim", Value::Boolean(true), 1)]);

        let result = registry.unregister("dim", 0);

        assert_eq!(result, UnregisterResult::Stale { current_revision: 1 });
        assert!(registry.by_id("dim").is_some(), "expected the preset to be there");
    }

    #[test]
    fn unregister_removes_an_entry_that_lags_behind() {
        let registry = PresetRegistry::new(vec![preset("dim", "Dim", Value::Boolean(true), 0)]);

        let result = registry.unregister("dim", 1);

        assert_eq!(result, UnregisterResult::Deleted);
        assert!(registry.by_id("dim").is_none(), "expected the preset to be unregistered");
    }

    #[test]
    fn unregister_leaves_other_presets_untouched() {
        let registry = PresetRegistry::new(vec![
            preset("dim", "Dim", Value::Boolean(true), 0),
            preset("enabled", "Enabled", Value::Boolean(true), 0),
        ]);

        registry.unregister("dim", 0);

        assert_eq!(registry.by_id("enabled").expect("preset exists").name, "Enabled");
    }

    #[test]
    fn unregister_does_not_change_a_preset_a_caller_already_holds() {
        let registry = PresetRegistry::new(vec![preset("dim", "Dim", Value::Boolean(true), 0)]);
        let before = registry.by_id("dim").expect("preset exists");

        registry.unregister("dim", 0);

        assert_eq!(before.name, "Dim");
        assert_eq!(before.revision, 0);
    }
}
