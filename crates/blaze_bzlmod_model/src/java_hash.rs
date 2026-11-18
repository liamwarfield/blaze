//! Java `hashCode()` values and `java.util.HashMap` iteration order, for the places where a
//! `HashMap` whose keys are not strings reaches output.
//!
//! PORT: java-hash-order. `BazelModuleResolutionFunction.collectYankedVersionsValues` iterates a
//! `HashMap<ModuleKey, YankedVersionsValue.Key>` into an `ImmutableMap`, whose order becomes the
//! order of `selectedYankedVersions` in `MODULE.bazel.lock`. This module reproduces OpenJDK's
//! order for a map that only receives distinct keys (no removals): buckets in index order, and
//! within a bucket insertion order, which `HashMap.resize` keeps when it splits a bucket. Tree
//! bins (a ninth key in one bucket of a table of 64 or more slots) reorder a bucket and are not
//! modelled; reaching one fails loudly.

use blaze_util::unported;

/// Java `String.hashCode()`: `s[0]*31^(n-1) + ... + s[n-1]` over UTF-16 code units, wrapping.
// Java: java.base/java/lang/String.java String.hashCode
pub fn java_string_hash_code(s: &str) -> i32 {
    let mut h: i32 = 0;
    for c in s.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(c as i32);
    }
    h
}

/// Java `HashMap.hash`: `h ^ (h >>> 16)`.
// Java: java.base/java/util/HashMap.java HashMap.hash
fn spread(h: i32) -> u32 {
    let h = h as u32;
    h ^ (h >> 16)
}

/// `HashMap.TREEIFY_THRESHOLD`.
const TREEIFY_THRESHOLD: usize = 8;
/// `HashMap.MIN_TREEIFY_CAPACITY`.
const MIN_TREEIFY_CAPACITY: usize = 64;

// Java: java.base/java/util/HashMap.java HashMap.resize
fn resize<K>(table: &mut Vec<Vec<(u32, K)>>) {
    let new_cap = table.len() * 2;
    let mut new_table: Vec<Vec<(u32, K)>> = (0..new_cap).map(|_| Vec::new()).collect();
    for bin in table.drain(..) {
        // Each bin is split into a "lo" and a "hi" list, preserving relative order.
        for (h, k) in bin {
            new_table[(h as usize) & (new_cap - 1)].push((h, k));
        }
    }
    *table = new_table;
}

/// Returns the keys in the order a `new HashMap<>()` iterates them after receiving them with
/// `put` in the given order. Each item is `(key.hashCode(), key)`; the keys must be distinct.
// Java: java.base/java/util/HashMap.java HashMap.putVal, HashMap.HashIterator
pub fn java_hash_map_order<K>(keys: impl IntoIterator<Item = (i32, K)>) -> Vec<K> {
    // DEFAULT_INITIAL_CAPACITY, DEFAULT_LOAD_FACTOR
    let mut table: Vec<Vec<(u32, K)>> = (0..16).map(|_| Vec::new()).collect();
    let mut size = 0usize;
    for (hash_code, key) in keys {
        let h = spread(hash_code);
        let idx = (h as usize) & (table.len() - 1);
        table[idx].push((h, key));
        // putVal: `if (binCount >= TREEIFY_THRESHOLD - 1) treeifyBin(tab, hash)`, that is when
        // the bin already held 8 nodes.
        if table[idx].len() > TREEIFY_THRESHOLD {
            if table.len() < MIN_TREEIFY_CAPACITY {
                // treeifyBin resizes instead of treeifying a small table.
                resize(&mut table);
            } else {
                unported!("HashMap.treeifyBin", "iteration order of a tree bin");
            }
        }
        size += 1;
        // `if (++size > threshold) resize()`
        if size > table.len() / 4 * 3 {
            resize(&mut table);
        }
    }
    table.into_iter().flatten().map(|(_, k)| k).collect()
}

#[cfg(test)]
mod tests {
    //! The expected values were printed by OpenJDK 25 (the JDK Bazel 9.3.2 embeds is 25.0.2) for
    //! `record Version` with Bazel's `hashCode()` and `record ModuleKey(String name, Version
    //! version)`, filling a `new HashMap<ModuleKey, String>()` in the order of `KEYS`.
    use super::*;
    use crate::module_key::ModuleKey;
    use crate::version::Version;

    const KEYS: [(&str, &str); 26] = [
        ("aaa", "1.0"),
        ("bbb", "1.1"),
        ("ccc", "2.0"),
        ("ddd", "0.0.1"),
        ("rules_cc", "0.0.9"),
        ("rules_java", "7.6.5"),
        ("platforms", "0.0.10"),
        ("bazel_skylib", "1.6.1"),
        ("protobuf", "27.0"),
        ("abseil-cpp", "20240116.2"),
        ("googletest", "1.14.0.bcr.1"),
        ("zlib", "1.3.1.bcr.3"),
        ("rules_python", "0.33.2"),
        ("rules_license", "0.0.8"),
        ("rules_proto", "6.0.2"),
        ("re2", "2024-07-02"),
        ("yanked1", "1.0"),
        ("yanked2", "1.0-rc1"),
        ("b", "2.0"),
        ("c", "3.0"),
        ("e", "5.0"),
        ("f", "6.0"),
        ("g", "7.0"),
        ("h", "8.0"),
        ("i", "9.0"),
        ("local", ""),
    ];

    fn key(name: &str, version: &str) -> ModuleKey {
        ModuleKey::new(name, Version::parse(version).unwrap())
    }

    fn order(n: usize) -> String {
        let keys = KEYS[..n].iter().map(|(name, version)| {
            let k = key(name, version);
            (k.java_hash_code(), k)
        });
        java_hash_map_order(keys)
            .iter()
            .map(|k| k.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }

    #[test]
    fn string_hash_code_matches_java() {
        assert_eq!(java_string_hash_code(""), 0);
        assert_eq!(java_string_hash_code("a"), 97);
        assert_eq!(java_string_hash_code("abc"), 96354);
        // "polygenelubricants".hashCode() == Integer.MIN_VALUE
        assert_eq!(java_string_hash_code("polygenelubricants"), i32::MIN);
    }

    #[test]
    fn hash_codes_match_java() {
        assert_eq!(key("rules_cc", "0.0.9").java_hash_code(), 281489790);
        assert_eq!(ModuleKey::root().java_hash_code(), -1985052183);
        assert_eq!(
            Version::parse("1.2.3").unwrap().java_hash_code(),
            -1938379741
        );
    }

    #[test]
    fn hash_map_order_matches_java() {
        assert_eq!(order(3), "bbb@1.1, aaa@1.0, ccc@2.0");
        assert_eq!(
            order(7),
            "rules_java@7.6.5, bbb@1.1, aaa@1.0, ccc@2.0, rules_cc@0.0.9, ddd@0.0.1, \
             platforms@0.0.10"
        );
        // One resize (13 > 12).
        assert_eq!(
            order(13),
            "bbb@1.1, aaa@1.0, ccc@2.0, protobuf@27.0, rules_python@0.33.2, ddd@0.0.1, \
             abseil-cpp@20240116.2, rules_java@7.6.5, zlib@1.3.1.bcr.3, bazel_skylib@1.6.1, \
             googletest@1.14.0.bcr.1, rules_cc@0.0.9, platforms@0.0.10"
        );
        assert_eq!(
            order(18),
            "bbb@1.1, aaa@1.0, ccc@2.0, protobuf@27.0, rules_python@0.33.2, ddd@0.0.1, \
             yanked1@1.0, yanked2@1.0-rc1, abseil-cpp@20240116.2, rules_proto@6.0.2, \
             rules_java@7.6.5, rules_license@0.0.8, zlib@1.3.1.bcr.3, re2@2024-07-02, \
             bazel_skylib@1.6.1, googletest@1.14.0.bcr.1, rules_cc@0.0.9, platforms@0.0.10"
        );
        // Two resizes (25 > 24).
        assert_eq!(
            order(26),
            "protobuf@27.0, yanked1@1.0, yanked2@1.0-rc1, rules_proto@6.0.2, zlib@1.3.1.bcr.3, \
             b@2.0, f@6.0, h@8.0, bazel_skylib@1.6.1, local@_, platforms@0.0.10, bbb@1.1, \
             aaa@1.0, ccc@2.0, rules_python@0.33.2, ddd@0.0.1, abseil-cpp@20240116.2, \
             rules_java@7.6.5, rules_license@0.0.8, re2@2024-07-02, c@3.0, e@5.0, g@7.0, i@9.0, \
             googletest@1.14.0.bcr.1, rules_cc@0.0.9"
        );
    }
}
