use std::collections::{HashMap, HashSet};

pub fn group_duplicates<T: Clone>(entries: impl Iterator<Item=(String, T)>) -> HashMap<String, Vec<T>> {
    let mut grouped: HashMap<String, Vec<T>> = HashMap::new();
    for (id, item) in entries {
        grouped.entry(id).or_default().push(item);
    }
    grouped.retain(|_, items| items.len() > 1);
    grouped
}

pub fn duplicate_id_set<T>(grouped: &HashMap<String, Vec<T>>) -> HashSet<&str> {
    grouped.keys().map(String::as_str).collect()
}
