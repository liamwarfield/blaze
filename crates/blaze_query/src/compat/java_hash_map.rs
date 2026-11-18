//! The iteration order of a `java.util.HashMap` (or `HashSet`) filled by `put` in a given order.
//!
//! PORT: java-hash-order. `ProtoOutputFormatter.getFlattenedAttributeValues` merges the possible
//! values of a configurable dict attribute into a `new HashMap<>()`, whose order reaches the
//! output. This replays `java.util.HashMap`: `hash = h ^ (h >>> 16)`, a 16-slot table that
//! doubles when the size exceeds 3/4 of the capacity, appends at the bin tail, and resizes that
//! keep each bin's relative order. A bin of 8+ entries in a table under 64 slots resizes instead
//! of treeifying (`treeifyBin`); real treeified bins (64+ slots) keep insertion order here, while
//! Java's red-black bins may differ. Same model as blaze_repo_fetch's `java_hash_set_order`.

use blaze_query_engine::JavaHashCode;

const TREEIFY_THRESHOLD: usize = 8;
const MIN_TREEIFY_CAPACITY: usize = 64;

fn spread(h: i32) -> u32 {
    let h = h as u32;
    h ^ (h >> 16)
}

/// Inserts `entries` in order into a simulated `HashMap` (later puts of an equal key replace the
/// value in place) and returns its iteration order.
pub fn java_hash_map_order<K: JavaHashCode + Eq, V>(entries: Vec<(K, V)>) -> Vec<(K, V)> {
    let mut table: Vec<Vec<(u32, K, V)>> = (0..16).map(|_| Vec::new()).collect();
    let mut size = 0usize;

    fn resize<K, V>(table: &mut Vec<Vec<(u32, K, V)>>) {
        let new_cap = table.len() * 2;
        let mut new_table: Vec<Vec<(u32, K, V)>> = (0..new_cap).map(|_| Vec::new()).collect();
        for bin in table.drain(..) {
            for (h, k, v) in bin {
                new_table[(h as usize) & (new_cap - 1)].push((h, k, v));
            }
        }
        *table = new_table;
    }

    for (key, value) in entries {
        let h = spread(key.java_hash_code());
        let idx = (h as usize) & (table.len() - 1);
        if let Some(entry) = table[idx]
            .iter_mut()
            .find(|(hh, k, _)| *hh == h && *k == key)
        {
            entry.2 = value;
            continue;
        }
        table[idx].push((h, key, value));
        // putVal: `if (binCount >= TREEIFY_THRESHOLD - 1) treeifyBin(tab, hash);` where binCount
        // counts the nodes before the appended one.
        if table[idx].len() > TREEIFY_THRESHOLD && table.len() < MIN_TREEIFY_CAPACITY {
            // HashMap.treeifyBin resizes instead of treeifying small tables.
            resize(&mut table);
        }
        size += 1;
        if size > table.len() * 3 / 4 {
            resize(&mut table);
        }
    }
    table
        .into_iter()
        .flatten()
        .map(|(_, k, v)| (k, v))
        .collect()
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
