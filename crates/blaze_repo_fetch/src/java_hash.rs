//! Replicates the iteration order of a Java `HashSet<String>`, for messages that print one.
//!
//! PORT: java-hash-order. `CouldNotFindPrefixException` prints the archive's candidate prefixes
//! from a `HashSet<String>`, so Blaze reproduces `java.util.HashMap`'s bucket layout: the
//! `String.hashCode` spread by `h ^ (h >>> 16)`, a power-of-two table starting at 16 slots that
//! doubles when the size exceeds 3/4 of the capacity, and per-bucket insertion order (which
//! `HashMap.resize` preserves). Treeified bins (9+ colliding entries in a table of 64+ slots)
//! keep insertion order here; Java's red-black bins could differ. Not reached by realistic
//! prefix sets.

/// Java `String.hashCode()` over UTF-16 code units.
pub fn java_string_hash_code(s: &str) -> i32 {
    let mut h: i32 = 0;
    for unit in s.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(unit as i32);
    }
    h
}

/// Java `HashMap.hash`.
fn spread(h: i32) -> u32 {
    let h = h as u32;
    h ^ (h >> 16)
}

/// Inserts `items` (in order) into a simulated `HashSet<String>` and returns its iteration order.
pub fn java_hash_set_order<I, S>(items: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    const TREEIFY_THRESHOLD: usize = 8;
    const MIN_TREEIFY_CAPACITY: usize = 64;
    let mut table: Vec<Vec<(u32, String)>> = vec![Vec::new(); 16];
    let mut size = 0usize;

    fn resize(table: &mut Vec<Vec<(u32, String)>>) {
        let new_cap = table.len() * 2;
        let mut new_table: Vec<Vec<(u32, String)>> = vec![Vec::new(); new_cap];
        for bin in table.drain(..) {
            for (h, k) in bin {
                new_table[(h as usize) & (new_cap - 1)].push((h, k));
            }
        }
        *table = new_table;
    }

    for item in items {
        let key: String = item.into();
        let h = spread(java_string_hash_code(&key));
        let idx = (h as usize) & (table.len() - 1);
        if table[idx].iter().any(|(hh, k)| *hh == h && *k == key) {
            continue;
        }
        table[idx].push((h, key));
        if table[idx].len() > TREEIFY_THRESHOLD && table.len() < MIN_TREEIFY_CAPACITY {
            // HashMap.treeifyBin resizes instead of treeifying small tables.
            resize(&mut table);
        }
        size += 1;
        if size > table.len() * 3 / 4 {
            resize(&mut table);
        }
    }
    table.into_iter().flatten().map(|(_, k)| k).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_hash_code_matches_java() {
        assert_eq!(java_string_hash_code(""), 0);
        assert_eq!(java_string_hash_code("a"), 97);
        assert_eq!(java_string_hash_code("abc"), 96354);
        // "polygenelubricants".hashCode() == Integer.MIN_VALUE
        assert_eq!(java_string_hash_code("polygenelubricants"), i32::MIN);
    }

    #[test]
    fn hash_set_order_matches_java() {
        // new HashSet<>(List.of("b", "a", "c")) iterates a, b, c (hashes 97, 98, 99).
        assert_eq!(java_hash_set_order(["b", "a", "c"]), vec!["a", "b", "c"]);
        // "foo" (101574) and "bar" (97299): buckets 6 and 3 of 16.
        assert_eq!(java_hash_set_order(["foo", "bar"]), vec!["bar", "foo"]);
    }
}
