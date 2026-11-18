//! Replicates the iteration order of Java `HashMap`s that Bazel's `mod` command lets reach its
//! output.
//!
//! PORT: java-hash-order. `BazelModuleInspectorFunction.computeAugmentedGraph` collects the
//! augmented graph in a `HashMap<ModuleKey, AugmentedModule.Builder>` and copies it into an
//! `ImmutableMap`, and `compute` groups it by module name with `Collectors.groupingBy` (another
//! `HashMap`). That order reaches the user through `modulesIndex` (error messages such as
//! "available versions: [...]", and the order of resolved `--from` keys, which seeds the BFS of
//! `mod path`). Both maps are filled only through `HashMap.computeIfAbsent`, which this module
//! simulates exactly:
//!
//! - the table is allocated lazily with 16 buckets;
//! - before each call (even one that finds its key), the table doubles if `size > threshold`
//!   (`threshold = 0.75 * capacity`);
//! - a new entry is linked at the *head* of its bucket (`tab[i] = newNode(hash, key, v, first)`);
//! - a resize splits each bucket into a "lo" and a "hi" list keeping relative order;
//! - a bucket that reaches `TREEIFY_THRESHOLD` (8) entries before the insertion resizes the table
//!   while it has fewer than 64 buckets (`treeifyBin`).
//!
//! Treeified bins (9+ colliding entries in a table of 64+ buckets) are kept as plain lists here;
//! Java's red-black bins could iterate differently. Realistic module graphs never reach that.

/// Java `String.hashCode()` over UTF-16 code units.
// Java: java/lang/String.java String.hashCode
pub fn java_string_hash_code(s: &str) -> i32 {
    let mut h: i32 = 0;
    for unit in s.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(unit as i32);
    }
    h
}

/// Java `Version.hashCode()`: `Objects.hash("version", getNormalized().hashCode())`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/Version.java:204 Version.hashCode
pub fn java_version_hash_code(version: &blaze_bzlmod_model::Version) -> i32 {
    // Arrays.hashCode(new Object[] {"version", Integer.valueOf(h)}).
    let mut result: i32 = 1;
    result = result
        .wrapping_mul(31)
        .wrapping_add(java_string_hash_code("version"));
    result
        .wrapping_mul(31)
        .wrapping_add(java_string_hash_code(version.get_normalized()))
}

/// Java `ModuleKey.hashCode()`: the record's generated hash, `31 * name.hashCode() +
/// version.hashCode()` (`ObjectMethods` combines components as `acc * 31 + h`, starting at 0).
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleKey.java:34 ModuleKey
pub fn java_module_key_hash_code(key: &blaze_bzlmod_model::ModuleKey) -> i32 {
    let mut acc: i32 = 0;
    acc = acc
        .wrapping_mul(31)
        .wrapping_add(java_string_hash_code(key.name()));
    acc.wrapping_mul(31)
        .wrapping_add(java_version_hash_code(key.version()))
}

/// Java `HashMap.hash`.
fn spread(h: i32) -> u32 {
    let h = h as u32;
    h ^ (h >> 16)
}

const DEFAULT_INITIAL_CAPACITY: usize = 16;
const TREEIFY_THRESHOLD: usize = 8;
const MIN_TREEIFY_CAPACITY: usize = 64;

/// A simulated `java.util.HashMap` that is only written with `computeIfAbsent`. It tracks keys
/// only; callers keep the values in their own map and ask for [`JavaHashMapOrder::iteration_order`].
#[derive(Debug, Clone)]
pub struct JavaHashMapOrder<K: PartialEq + Clone> {
    /// Each bucket in iteration order: (spread hash, key).
    table: Vec<Vec<(u32, K)>>,
    size: usize,
}

impl<K: PartialEq + Clone> Default for JavaHashMapOrder<K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: PartialEq + Clone> JavaHashMapOrder<K> {
    /// `new HashMap<>()`: no table until the first insertion.
    pub fn new() -> Self {
        JavaHashMapOrder {
            table: Vec::new(),
            size: 0,
        }
    }

    fn threshold(&self) -> usize {
        // HashMap.resize: newThr = (int)(DEFAULT_LOAD_FACTOR * newCap) for the default table,
        // and oldThr << 1 afterwards, which is the same 3/4 of the capacity.
        self.table.len() * 3 / 4
    }

    // Java: java/util/HashMap.java HashMap.resize
    fn resize(&mut self) {
        let new_cap = if self.table.is_empty() {
            DEFAULT_INITIAL_CAPACITY
        } else {
            self.table.len() * 2
        };
        let mut new_table: Vec<Vec<(u32, K)>> = vec![Vec::new(); new_cap];
        for bin in self.table.drain(..) {
            for (h, k) in bin {
                new_table[(h as usize) & (new_cap - 1)].push((h, k));
            }
        }
        self.table = new_table;
    }

    /// `HashMap.computeIfAbsent(key, ...)` with a mapping function that returns non-null and
    /// does not touch the map. Returns whether the key was absent (and is now inserted).
    // Java: java/util/HashMap.java HashMap.computeIfAbsent
    pub fn compute_if_absent(&mut self, key: &K, hash_code: i32) -> bool {
        let hash = spread(hash_code);
        if self.size > self.threshold() || self.table.is_empty() {
            self.resize();
        }
        let n = self.table.len();
        let i = (hash as usize) & (n - 1);
        let mut bin_count = 0;
        for (h, k) in &self.table[i] {
            if *h == hash && k == key {
                return false;
            }
            bin_count += 1;
        }
        // tab[i] = newNode(hash, key, v, first): the new node becomes the head of the bin.
        self.table[i].insert(0, (hash, key.clone()));
        if bin_count >= TREEIFY_THRESHOLD - 1 && n < MIN_TREEIFY_CAPACITY {
            // HashMap.treeifyBin resizes instead of treeifying small tables.
            self.resize();
        }
        self.size += 1;
        true
    }

    /// The keys in the order `HashMap.entrySet()` iterates them.
    pub fn iteration_order(&self) -> Vec<K> {
        self.table
            .iter()
            .flat_map(|bin| bin.iter().map(|(_, k)| k.clone()))
            .collect()
    }
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
    fn compute_if_absent_links_new_entries_at_bucket_head() {
        // "a" (97) and "q" (113) share bucket 1 of 16; the later insertion iterates first.
        let mut m = JavaHashMapOrder::new();
        assert!(m.compute_if_absent(&"a", java_string_hash_code("a")));
        assert!(m.compute_if_absent(&"q", java_string_hash_code("q")));
        assert!(!m.compute_if_absent(&"a", java_string_hash_code("a")));
        assert!(m.compute_if_absent(&"b", java_string_hash_code("b")));
        assert_eq!(m.iteration_order(), vec!["q", "a", "b"]);
    }
}
