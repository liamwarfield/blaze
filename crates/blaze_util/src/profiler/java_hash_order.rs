//! `// PORT: java-hash-order` helpers: the iteration order of a `java.util.HashMap` whose keys
//! were inserted (never removed) in a given order.
//!
//! A `HashMap` starts with 16 buckets and doubles when its size exceeds 3/4 of the table; a
//! resize splits each bucket keeping the relative order of its entries. `put` / `putIfAbsent`
//! append a new entry to the tail of its bucket, `computeIfAbsent` puts it at the head. So
//! iteration is by the bucket index `(h ^ (h >>> 16)) & (capacity - 1)` in the final table, and
//! within a bucket in insertion order (`put`) or reverse insertion order (`computeIfAbsent`). Treeified buckets (8 or more
//! colliding keys in a table of at least 64) are not modelled; the profiler's maps never come
//! close.

/// `Long.hashCode(value)`.
pub fn long_hash_code(value: i64) -> i32 {
    (value ^ ((value as u64) >> 32) as i64) as i32
}

/// `String.hashCode()` of a string whose chars are UTF-16 code units (internal strings and the
/// ASCII names the profiler uses).
pub fn string_hash_code(value: &str) -> i32 {
    let mut h: i32 = 0;
    let mut buf = [0u16; 2];
    for c in value.chars() {
        for unit in c.encode_utf16(&mut buf) {
            h = h.wrapping_mul(31).wrapping_add(*unit as i32);
        }
    }
    h
}

/// The table size of a `new HashMap<>()` holding `n` entries.
fn capacity_for(n: usize) -> usize {
    let mut capacity = 16usize;
    while n > capacity * 3 / 4 {
        capacity *= 2;
    }
    capacity
}

/// How the map's entries were added.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Insertion {
    /// `put`, `putIfAbsent`: a new entry goes to the tail of its bucket.
    Put,
    /// `HashMap.computeIfAbsent` (and so `Collectors.groupingBy`): a new entry goes to the head.
    ComputeIfAbsent,
}

/// The keys of `insertion_order` in the order a `HashMap` iterates them.
pub fn java_hash_map_order<K: Clone>(
    insertion_order: &[K],
    hash_code: impl Fn(&K) -> i32,
    insertion: Insertion,
) -> Vec<K> {
    let capacity = capacity_for(insertion_order.len());
    let n = insertion_order.len();
    let mut keyed: Vec<(usize, usize, &K)> = insertion_order
        .iter()
        .enumerate()
        .map(|(i, k)| {
            let h = hash_code(k);
            let spread = h ^ ((h as u32) >> 16) as i32;
            let within = match insertion {
                Insertion::Put => i,
                Insertion::ComputeIfAbsent => n - i,
            };
            ((spread as u32 as usize) & (capacity - 1), within, k)
        })
        .collect();
    keyed.sort_by_key(|(bucket, within, _)| (*bucket, *within));
    keyed.into_iter().map(|(_, _, k)| k.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lane order Bazel 9.3.2 wrote (oracle profile of brotli `build //...`): the counter
    /// groups of `CollectLocalResourceUsage.logCollectedData`, a `groupingBy` into a HashMap.
    #[test]
    fn lane_names_iterate_like_the_oracle() {
        let inserted = [
            "CPU usage (Bazel)",
            "Memory usage (Bazel)",
            "CPU usage (total)",
            "Memory usage (total)",
            "Total worker memory usage",
            "System load average",
            "Network Up usage (total)",
            "Network Down usage (total)",
        ];
        let order = java_hash_map_order(&inserted, |s| string_hash_code(s), Insertion::ComputeIfAbsent);
        assert_eq!(
            order,
            vec![
                "Network Up usage (total)",
                "System load average",
                "CPU usage (total)",
                "Total worker memory usage",
                "CPU usage (Bazel)",
                "Memory usage (total)",
                "Network Down usage (total)",
                "Memory usage (Bazel)",
            ]
        );
    }
}
