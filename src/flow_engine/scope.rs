use std::any::Any;
use std::collections::HashMap;
use std::fmt::Debug;

#[derive(Debug)]
pub struct Scope {
    data: HashMap<String, Box<dyn Any + Send + Sync>>,
}

impl Scope {
    pub fn new() -> Self {
        Scope { data: HashMap::new() }
    }

    pub fn store<T: 'static + Send + Sync>(&mut self, key: String, value: T) -> Option<Box<dyn Any + Send + Sync>> {
        self.data.insert(key, Box::new(value))
    }

    pub fn get<T: 'static + Send + Sync>(&self, k: &str) -> Option<&T> {
        self.data.get(k).and_then(|v| v.downcast_ref::<T>())
    }

    pub fn get_mut<T: 'static + Send + Sync>(&mut self, k: &str) -> Option<&mut T> {
        self.data.get_mut(k).and_then(|v| v.downcast_mut::<T>())
    }

    pub fn ensure_entry_mut<T: 'static + Send + Sync, F: FnOnce() -> T>(&mut self, k: String, default: F) -> Option<&mut T> {
        self.data.entry(k).or_insert_with(|| Box::new(default())).downcast_mut::<T>()
    }
}
