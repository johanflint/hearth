use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::sync::Arc;

pub fn take_new<K: Eq + Hash + Clone, V>(map: &HashMap<K, Arc<V>>, seen: &mut HashSet<K>) -> Vec<Arc<V>> {
    map.iter().filter(|(key, _)| seen.insert((*key).clone())).map(|(_, value)| Arc::clone(value)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_new_returns_each_value_only_once() {
        let mut map = HashMap::from([("a", Arc::new(1))]);
        let mut seen = HashSet::new();

        assert_eq!(take_new(&map, &mut seen), vec![Arc::new(1)]);
        assert!(take_new(&map, &mut seen).is_empty());

        map.insert("b", Arc::new(2));
        assert_eq!(take_new(&map, &mut seen), vec![Arc::new(2)]);
    }
}
