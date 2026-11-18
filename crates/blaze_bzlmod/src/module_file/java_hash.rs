//! Java `HashMap` / `HashSet` iteration order for `String` keys.
//!
//! PORT: java-hash-order. `ModuleThreadContext` keeps several `java.util.HashMap`s whose
//! iteration order reaches output (`ModuleExtensionUsage.getRepoOverrides()` is copied from one,
//! error messages pick the first match from another, and spelling suggestions iterate a
//! `HashSet`). This module reproduces OpenJDK's order for maps that only ever grow (no removals),
//! which is all these maps do: buckets in index order, and within a bucket insertion order (resizes
//! split buckets without reordering entries). Tree bins (8+ colliding keys in a table of 64+) are
//! not modelled.

use indexmap::IndexMap;

/// Java `String.hashCode()`: `s[0]*31^(n-1) + ... + s[n-1]` over UTF-16 code units, wrapping.
// Java: java/lang/String.java String.hashCode
pub(crate) fn java_string_hash_code(s: &str) -> i32 {
    let mut h: i32 = 0;
    for c in s.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(c as i32);
    }
    h
}

/// Java `HashMap.hash`: `h ^ (h >>> 16)`.
// Java: java/util/HashMap.java HashMap.hash
fn spread(h: i32) -> u32 {
    let h = h as u32;
    h ^ (h >> 16)
}

/// Table size of a `new HashMap<>()` / `new HashSet<>()` after `n` insertions of distinct keys:
/// starts at 16 and doubles whenever the size exceeds 3/4 of the capacity.
// Java: java/util/HashMap.java HashMap.resize (DEFAULT_INITIAL_CAPACITY, DEFAULT_LOAD_FACTOR)
fn table_capacity(n: usize) -> usize {
    let mut cap = 16usize;
    while n > cap / 4 * 3 {
        cap *= 2;
    }
    cap
}

/// Returns `keys` (distinct, in insertion order) in the order a Java `HashMap`/`HashSet` that
/// received them in that order iterates them.
pub(crate) fn java_hash_order<'a>(keys: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    let keys: Vec<&'a str> = keys.into_iter().collect();
    let mask = (table_capacity(keys.len()) - 1) as u32;
    let mut indexed: Vec<(u32, usize, &'a str)> = keys
        .iter()
        .enumerate()
        .map(|(i, k)| (spread(java_string_hash_code(k)) & mask, i, *k))
        .collect();
    indexed.sort_by_key(|(bucket, i, _)| (*bucket, *i));
    indexed.into_iter().map(|(_, _, k)| k).collect()
}

/// A `java.util.HashMap<String, V>` that is never shrunk: storage in insertion order, iteration
/// in Java's order.
#[derive(Debug, Clone)]
pub(crate) struct JavaHashMap<V> {
    entries: IndexMap<String, V>,
}

impl<V> Default for JavaHashMap<V> {
    fn default() -> Self {
        JavaHashMap {
            entries: IndexMap::new(),
        }
    }
}

impl<V> JavaHashMap<V> {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Java `put`: replaces the value of an existing key (keeping its position) and returns the
    /// previous value.
    pub(crate) fn put(&mut self, key: impl Into<String>, value: V) -> Option<V> {
        self.entries.insert(key.into(), value)
    }

    pub(crate) fn get(&self, key: &str) -> Option<&V> {
        self.entries.get(key)
    }

    pub(crate) fn contains_key(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    /// Keys in Java iteration order (`keySet()`).
    pub(crate) fn key_set(&self) -> Vec<&str> {
        java_hash_order(self.entries.keys().map(|k| k.as_str()))
    }

    /// Entries in Java iteration order (`entrySet()`).
    pub(crate) fn entry_set(&self) -> Vec<(&str, &V)> {
        self.key_set()
            .into_iter()
            .map(|k| {
                let v = self.entries.get(k).expect("key from key_set");
                (k, v)
            })
            .collect()
    }
}
