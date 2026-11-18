//! A single-threaded model of `java.util.concurrent.ConcurrentHashMap` that reproduces its
//! iteration order.
//!
//! PORT: java-hash-order. `QueryUtil.ThreadSafeMutableKeyExtractorBackedSetImpl` (the set every
//! `BlazeQueryEnvironment` set operation, `deps` frontier and `getFwdDeps`/`getReverseDeps` result
//! is built on) iterates a `ConcurrentHashMap<Label, Target>`, and that order reaches query
//! callbacks and output. This module replays the JDK algorithm (JDK 8 through 21 share it):
//!
//! - `spread(h) = (h ^ (h >>> 16)) & 0x7fffffff`; bin index `(n - 1) & hash`.
//! - Lazy table creation with `n = sizeCtl > 0 ? sizeCtl : 16`, then `sizeCtl = n - (n >>> 2)`.
//! - `putVal` appends to a list bin; `addCount` resizes (doubling) while `count >= sizeCtl`.
//! - A list bin that reaches 8 traversed nodes calls `treeifyBin`: below 64 slots it calls
//!   `tryPresize(n << 1)` (which may double several times), otherwise it becomes a tree bin. A
//!   tree bin iterates its `first` list; `putTreeVal` prepends to that list.
//! - `transfer` splits each list bin using the `lastRun` optimisation (the nodes before
//!   `lastRun` are prepended, so their relative order reverses), and each tree bin in order
//!   (untreeifying halves of 6 or fewer nodes).
//! - Iteration walks the table from index 0 and each bin from its head.
//!
//! Known gap: when a removal leaves a tree bin non-empty, Java untreeifies it if the red-black
//! tree's shape is "too small" (`TreeBin.removeTreeNode`); the shape depends on the red-black
//! balancing and on `System.identityHashCode` tie-breaks, so this model keeps it a tree bin. Only
//! reachable with 9+ colliding keys in one bin of a 64+ slot table followed by removals.

/// Java `Object.hashCode()` for the key types query sets use.
pub trait JavaHashCode {
    fn java_hash_code(&self) -> i32;
}

/// Java `String.hashCode()` of a string in Bazel's internal form (Latin-1 characters holding UTF-8
/// bytes, `lib/util/StringEncoding.java`): the polynomial runs over the UTF-8 bytes.
///
/// PORT: StringEncoding identity. Labels, package names and repository names are internal-form
/// strings in Java, so their hash codes are over UTF-8 bytes.
pub fn java_internal_string_hash_code(s: &str) -> i32 {
    let mut h: i32 = 0;
    for b in s.as_bytes() {
        h = h.wrapping_mul(31).wrapping_add(*b as i32);
    }
    h
}

impl JavaHashCode for String {
    fn java_hash_code(&self) -> i32 {
        java_internal_string_hash_code(self)
    }
}

impl JavaHashCode for str {
    fn java_hash_code(&self) -> i32 {
        java_internal_string_hash_code(self)
    }
}

impl JavaHashCode for i32 {
    /// Java `Integer.hashCode()`.
    fn java_hash_code(&self) -> i32 {
        *self
    }
}

/// `HashCodes.MULTIPLIER`.
const MULTIPLIER: i32 = 31;

/// Java `HashCodes.hashObjects(o1, o2)`.
// Java: com/google/devtools/build/lib/util/HashCodes.java:46 HashCodes.hashObjects
fn hash_objects2(h1: i32, h2: i32) -> i32 {
    h2.wrapping_add(MULTIPLIER.wrapping_mul(h1.wrapping_add(MULTIPLIER)))
}

/// Java `HashCodes.hashObjects(o1, o2, o3)`.
// Java: com/google/devtools/build/lib/util/HashCodes.java:52 HashCodes.hashObjects
fn hash_objects3(h1: i32, h2: i32, h3: i32) -> i32 {
    h3.wrapping_add(
        MULTIPLIER
            .wrapping_mul(h2.wrapping_add(MULTIPLIER.wrapping_mul(h1.wrapping_add(MULTIPLIER)))),
    )
}

impl JavaHashCode for blaze_cmdline::RepositoryName {
    /// `HashCodes.hashObjects(name, contextRepoIfNotVisible, didYouMeanSuffix)`.
    // Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:167 RepositoryName.<init>
    fn java_hash_code(&self) -> i32 {
        if !self.is_visible() {
            // PORT: blaze_cmdline does not expose contextRepoIfNotVisible/didYouMeanSuffix yet.
            blaze_util::unported!(
                "RepositoryName.hashCode",
                "non-visible repository {}",
                self.get_name()
            );
        }
        hash_objects3(java_internal_string_hash_code(self.get_name()), 0, 0)
    }
}

impl JavaHashCode for blaze_cmdline::PackageIdentifier {
    /// `HashCodes.hashObjects(repository, pkgName)`; `PathFragment.hashCode()` is
    /// `normalizedPath.hashCode()`.
    // Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:142 PackageIdentifier.<init>
    fn java_hash_code(&self) -> i32 {
        hash_objects2(
            self.get_repository().java_hash_code(),
            java_internal_string_hash_code(self.get_package_fragment().get_path_string()),
        )
    }
}

impl JavaHashCode for blaze_cmdline::Label {
    /// `31 * (31 + name.hashCode()) + packageIdentifier.hashCode()`.
    // Java: com/google/devtools/build/lib/cmdline/Label.java:642 Label.hashCode
    fn java_hash_code(&self) -> i32 {
        let result = 31i32.wrapping_add(java_internal_string_hash_code(self.get_name()));
        31i32
            .wrapping_mul(result)
            .wrapping_add(self.get_package_identifier().java_hash_code())
    }
}

const MAXIMUM_CAPACITY: i32 = 1 << 30;
const DEFAULT_CAPACITY: usize = 16;
const TREEIFY_THRESHOLD: usize = 8;
const UNTREEIFY_THRESHOLD: usize = 6;
const MIN_TREEIFY_CAPACITY: usize = 64;
const HASH_BITS: i32 = 0x7fff_ffff;

/// Java `ConcurrentHashMap.spread`.
fn spread(h: i32) -> i32 {
    (h ^ ((h as u32) >> 16) as i32) & HASH_BITS
}

/// Java `ConcurrentHashMap.tableSizeFor`.
fn table_size_for(c: i32) -> i32 {
    // int n = -1 >>> Integer.numberOfLeadingZeros(c - 1); (Java masks the shift distance to 5 bits)
    let shift = (c.wrapping_sub(1) as u32).leading_zeros() & 31;
    let n = (u32::MAX >> shift) as i32;
    if n < 0 {
        1
    } else if n >= MAXIMUM_CAPACITY {
        MAXIMUM_CAPACITY
    } else {
        n + 1
    }
}

struct Node<K, V> {
    hash: i32,
    key: K,
    val: V,
}

struct Bin<K, V> {
    /// For a list bin, the `next` order; for a tree bin, the `first` list order.
    nodes: Vec<Node<K, V>>,
    is_tree: bool,
}

impl<K, V> Default for Bin<K, V> {
    fn default() -> Self {
        Bin {
            nodes: Vec::new(),
            is_tree: false,
        }
    }
}

/// See the module documentation.
pub struct JavaConcurrentHashMap<K, V> {
    table: Vec<Bin<K, V>>,
    size_ctl: i32,
    count: i64,
}

impl<K: JavaHashCode + Eq, V> JavaConcurrentHashMap<K, V> {
    /// Java `new ConcurrentHashMap<>()`.
    pub fn new() -> Self {
        JavaConcurrentHashMap {
            table: Vec::new(),
            size_ctl: 0,
            count: 0,
        }
    }

    /// Java `new ConcurrentHashMap<>(initialCapacity, loadFactor, concurrencyLevel)` (the
    /// two-argument constructor passes `concurrencyLevel = 1`).
    pub fn with_capacity(initial_capacity: i32, load_factor: f32, concurrency_level: i32) -> Self {
        assert!(
            load_factor > 0.0 && initial_capacity >= 0 && concurrency_level > 0,
            "IllegalArgumentException"
        );
        let initial_capacity = initial_capacity.max(concurrency_level);
        // long size = (long)(1.0 + (long)initialCapacity / loadFactor);
        let size = (1.0f64 + ((initial_capacity as i64) as f32 / load_factor) as f64) as i64;
        let cap = if size >= MAXIMUM_CAPACITY as i64 {
            MAXIMUM_CAPACITY
        } else {
            table_size_for(size as i32)
        };
        JavaConcurrentHashMap {
            table: Vec::new(),
            size_ctl: cap,
            count: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.count as usize
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn find(&self, key: &K) -> Option<(usize, usize)> {
        if self.table.is_empty() {
            return None;
        }
        let hash = spread(key.java_hash_code());
        let i = (self.table.len() - 1) & hash as usize;
        self.table[i]
            .nodes
            .iter()
            .position(|n| n.hash == hash && n.key == *key)
            .map(|j| (i, j))
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        self.find(key).map(|(i, j)| &self.table[i].nodes[j].val)
    }

    pub fn contains_key(&self, key: &K) -> bool {
        self.find(key).is_some()
    }

    /// Java `putIfAbsent`: returns false (and keeps the old value) if the key is present.
    pub fn put_if_absent(&mut self, key: K, value: V) -> bool {
        self.put_val(key, value, true)
    }

    /// Java `put`: returns false if the key was present (its value is replaced).
    pub fn put(&mut self, key: K, value: V) -> bool {
        self.put_val(key, value, false)
    }

    /// Java `ConcurrentHashMap.putVal`; returns true if a new mapping was added.
    fn put_val(&mut self, key: K, value: V, only_if_absent: bool) -> bool {
        let hash = spread(key.java_hash_code());
        if self.table.is_empty() {
            self.init_table();
        }
        let n = self.table.len();
        let i = (n - 1) & hash as usize;
        let bin_count;
        let mut existed = false;
        let bin = &mut self.table[i];
        if bin.nodes.is_empty() {
            bin.is_tree = false;
            bin.nodes.push(Node {
                hash,
                key,
                val: value,
            });
            bin_count = 0;
        } else if bin.is_tree {
            bin_count = 2;
            match bin
                .nodes
                .iter()
                .position(|e| e.hash == hash && e.key == key)
            {
                Some(j) => {
                    existed = true;
                    if !only_if_absent {
                        bin.nodes[j].val = value;
                    }
                }
                None => {
                    // TreeBin.putTreeVal: `first = x = new TreeNode<K,V>(h, k, v, f, xp);`
                    bin.nodes.insert(
                        0,
                        Node {
                            hash,
                            key,
                            val: value,
                        },
                    );
                }
            }
        } else {
            // Fast path for onlyIfAbsent with a matching first node returns before counting.
            if only_if_absent && bin.nodes[0].hash == hash && bin.nodes[0].key == key {
                return false;
            }
            let mut count = 1;
            let mut found = None;
            for (j, e) in bin.nodes.iter().enumerate() {
                if e.hash == hash && e.key == key {
                    found = Some(j);
                    break;
                }
                if j + 1 == bin.nodes.len() {
                    break;
                }
                count += 1;
            }
            match found {
                Some(j) => {
                    existed = true;
                    if !only_if_absent {
                        bin.nodes[j].val = value;
                    }
                }
                None => bin.nodes.push(Node {
                    hash,
                    key,
                    val: value,
                }),
            }
            bin_count = count;
        }
        if bin_count != 0 {
            if bin_count >= TREEIFY_THRESHOLD {
                self.treeify_bin(i);
            }
            if existed {
                return false;
            }
        }
        self.add_count(1, bin_count as i32);
        true
    }

    /// Java `ConcurrentHashMap.remove(key)`; returns the removed value.
    pub fn remove(&mut self, key: &K) -> Option<V> {
        let (i, j) = self.find(key)?;
        let node = self.table[i].nodes.remove(j);
        if self.table[i].nodes.is_empty() {
            self.table[i].is_tree = false;
        }
        // addCount(-1L, -1): no resize check.
        self.count -= 1;
        Some(node.val)
    }

    /// Values in Java iteration order.
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.table
            .iter()
            .flat_map(|b| b.nodes.iter().map(|n| &n.val))
    }

    /// Keys in Java iteration order.
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.table
            .iter()
            .flat_map(|b| b.nodes.iter().map(|n| &n.key))
    }

    /// Entries in Java iteration order.
    pub fn entries(&self) -> impl Iterator<Item = (&K, &V)> {
        self.table
            .iter()
            .flat_map(|b| b.nodes.iter().map(|n| (&n.key, &n.val)))
    }

    /// Java `ConcurrentHashMap.initTable`.
    fn init_table(&mut self) {
        let sc = self.size_ctl;
        let n = if sc > 0 {
            sc as usize
        } else {
            DEFAULT_CAPACITY
        };
        self.table = (0..n).map(|_| Bin::default()).collect();
        self.size_ctl = (n - (n >> 2)) as i32;
    }

    /// Java `ConcurrentHashMap.addCount` (single-threaded).
    fn add_count(&mut self, x: i64, check: i32) {
        self.count += x;
        if check >= 0 {
            while self.count >= self.size_ctl as i64
                && !self.table.is_empty()
                && (self.table.len() as i32) < MAXIMUM_CAPACITY
            {
                self.transfer();
            }
        }
    }

    /// Java `ConcurrentHashMap.treeifyBin`.
    fn treeify_bin(&mut self, index: usize) {
        let n = self.table.len();
        if n < MIN_TREEIFY_CAPACITY {
            self.try_presize((n << 1) as i32);
        } else if !self.table[index].nodes.is_empty() && !self.table[index].is_tree {
            self.table[index].is_tree = true;
        }
    }

    /// Java `ConcurrentHashMap.tryPresize` (table already initialised).
    fn try_presize(&mut self, size: i32) {
        let c = if size >= (MAXIMUM_CAPACITY >> 1) {
            MAXIMUM_CAPACITY
        } else {
            table_size_for(size + (size >> 1) + 1)
        };
        loop {
            let sc = self.size_ctl;
            let n = self.table.len() as i32;
            if c <= sc || n >= MAXIMUM_CAPACITY {
                break;
            }
            self.transfer();
        }
    }

    /// Java `ConcurrentHashMap.transfer` with one thread; ends with
    /// `sizeCtl = (n << 1) - (n >>> 1)`.
    fn transfer(&mut self) {
        let n = self.table.len();
        let mut next: Vec<Bin<K, V>> = (0..n << 1).map(|_| Bin::default()).collect();
        let old = std::mem::take(&mut self.table);
        for (i, bin) in old.into_iter().enumerate() {
            if bin.nodes.is_empty() {
                continue;
            }
            if !bin.is_tree {
                let nodes = bin.nodes;
                let mut run_bit = nodes[0].hash as usize & n;
                let mut last_run = 0;
                for (j, p) in nodes.iter().enumerate().skip(1) {
                    let b = p.hash as usize & n;
                    if b != run_bit {
                        run_bit = b;
                        last_run = j;
                    }
                }
                let mut nodes = nodes;
                let run: Vec<Node<K, V>> = nodes.split_off(last_run);
                let mut ln: Vec<Node<K, V>> = Vec::new();
                let mut hn: Vec<Node<K, V>> = Vec::new();
                if run_bit == 0 {
                    ln = run;
                } else {
                    hn = run;
                }
                // `ln = new Node<K,V>(ph, pk, pv, ln)`: each earlier node is prepended.
                for p in nodes {
                    if p.hash as usize & n == 0 {
                        ln.insert(0, p);
                    } else {
                        hn.insert(0, p);
                    }
                }
                next[i] = Bin {
                    nodes: ln,
                    is_tree: false,
                };
                next[i + n] = Bin {
                    nodes: hn,
                    is_tree: false,
                };
            } else {
                let mut lo = Vec::new();
                let mut hi = Vec::new();
                for e in bin.nodes {
                    if e.hash as usize & n == 0 {
                        lo.push(e);
                    } else {
                        hi.push(e);
                    }
                }
                let lc = lo.len();
                let hc = hi.len();
                next[i] = Bin {
                    is_tree: lc > UNTREEIFY_THRESHOLD,
                    nodes: lo,
                };
                next[i + n] = Bin {
                    is_tree: hc > UNTREEIFY_THRESHOLD,
                    nodes: hi,
                };
            }
        }
        self.table = next;
        self.size_ctl = ((n << 1) - (n >> 1)) as i32;
    }
}

impl<K: JavaHashCode + Eq, V> Default for JavaConcurrentHashMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(PartialEq, Eq, Debug, Clone)]
    struct H(i32, &'static str);

    impl JavaHashCode for H {
        fn java_hash_code(&self) -> i32 {
            self.0
        }
    }

    #[test]
    fn table_size_for_matches_java() {
        assert_eq!(table_size_for(0), 1);
        assert_eq!(table_size_for(1), 1);
        assert_eq!(table_size_for(2), 2);
        assert_eq!(table_size_for(3), 4);
        assert_eq!(table_size_for(17), 32);
        assert_eq!(table_size_for(49), 64);
    }

    #[test]
    fn string_hash_code_matches_java() {
        assert_eq!(java_internal_string_hash_code(""), 0);
        assert_eq!(java_internal_string_hash_code("abc"), 96354);
        assert_eq!(
            java_internal_string_hash_code("polygenelubricants"),
            i32::MIN
        );
    }

    #[test]
    fn small_map_resizes_like_java() {
        // new ConcurrentHashMap<>(1, 0.75f): table of 2 slots, sizeCtl 2 after init.
        let mut m: JavaConcurrentHashMap<H, ()> = JavaConcurrentHashMap::with_capacity(1, 0.75, 1);
        assert!(m.put_if_absent(H(1, "a"), ()));
        assert_eq!(m.table.len(), 2);
        assert!(m.put_if_absent(H(3, "b"), ()));
        // count 2 >= sizeCtl 2: resize to 4 (sizeCtl 3).
        assert_eq!(m.table.len(), 4);
        assert_eq!(m.size_ctl, 3);
        assert!(!m.put_if_absent(H(3, "b"), ()));
        let keys: Vec<_> = m.keys().map(|k| k.1).collect();
        assert_eq!(keys, vec!["a", "b"]);
    }

    #[test]
    fn transfer_reverses_nodes_before_last_run() {
        // Default table of 16 slots, sizeCtl 12: the 12th insertion doubles the table. Bin 0
        // holds hashes 0, 32, 16; 16 moves to bin 16 (lastRun), 0 and 32 are prepended in turn.
        let mut m: JavaConcurrentHashMap<H, ()> = JavaConcurrentHashMap::new();
        m.put(H(0, "a"), ());
        m.put(H(32, "b"), ());
        m.put(H(16, "c"), ());
        let fillers = ["f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9"];
        for (i, f) in fillers.iter().enumerate() {
            m.put(H(i as i32 + 1, *f), ());
        }
        assert_eq!(m.table.len(), 32);
        let keys: Vec<_> = m.keys().map(|k| k.1).collect();
        let mut expected = vec!["b", "a"];
        expected.extend(fillers);
        expected.push("c");
        assert_eq!(keys, expected);
    }
}
