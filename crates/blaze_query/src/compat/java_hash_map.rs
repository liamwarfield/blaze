//! The iteration order of a `java.util.HashMap` (or `HashSet`) filled by `put` in a given order.
//!
//! PORT: java-hash-order. `ProtoOutputFormatter.getFlattenedAttributeValues` merges the possible
//! values of a configurable dict attribute into a `new HashMap<>()`, whose order reaches the
//! output. The replay of `java.util.HashMap` lives in
//! `blaze_query_engine::java_concurrent_hash_map::java_hash_map_order` (next to `JavaHashCode`),
//! so that blaze_packages can use it too; a treeified bin (9+ colliding keys in a table of 64+
//! slots) fails loudly there.

use blaze_query_engine::JavaHashCode;

/// Inserts `entries` in order into a simulated `HashMap` (later puts of an equal key replace the
/// value in place) and returns its iteration order.
pub fn java_hash_map_order<K: JavaHashCode + Eq, V>(entries: Vec<(K, V)>) -> Vec<(K, V)> {
    blaze_query_engine::java_concurrent_hash_map::java_hash_map_order(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_string_map_iterates_by_bucket() {
        // "a"=97 -> bucket 1, "b"=98 -> bucket 2, "q"=113 -> bucket 1.
        let order = java_hash_map_order(vec![
            ("q".to_owned(), 1),
            ("b".to_owned(), 2),
            ("a".to_owned(), 3),
        ]);
        let keys: Vec<&str> = order.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, vec!["q", "a", "b"]);
    }
}
