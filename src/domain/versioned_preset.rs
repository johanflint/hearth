use crate::domain::Preset;
use std::ops::Deref;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct VersionedPreset {
    pub preset: Arc<Preset>,
    pub revision: u64,
}

impl Deref for VersionedPreset {
    type Target = Preset;

    fn deref(&self) -> &Self::Target {
        &self.preset
    }
}
