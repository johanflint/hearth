use crate::domain::Preset;
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

static PRESET_REGISTRY: LazyLock<PresetRegistry> = LazyLock::new(PresetRegistry::default);

#[derive(Default, Debug)]
pub struct PresetRegistry {
    presets: RwLock<HashMap<String, Preset>>,
}

impl PresetRegistry {
    pub fn new(presets: Vec<Preset>) -> Self {
        let entries = presets.into_iter().map(|preset| (preset.id.clone(), preset)).collect();

        Self { presets: RwLock::new(entries) }
    }
}
