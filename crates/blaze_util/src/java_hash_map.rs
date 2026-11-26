//! A port of `java.util.HashMap<String, String>`'s iteration order (JDK 25), shared by the two
//! ports of `PlatformUtils.getPlatformProto` (`blaze_remote::platform_utils` and
//! `blaze_exec`'s execution log).

/// A `java.util.HashMap<String, String>`, as far as its iteration order goes.
///
/// PORT: java-hash-order. Used where a Java `HashMap`'s iteration order reaches output (e.g.
/// `PlatformUtils.getPlatformProto` iterates its `properties` map before a stable sort by the
/// Unicode name: two internal names that are both invalid UTF-8 become the same name (U+FFFD)
/// after `internalToUnicode` and keep the map's order). The order of a `HashMap` is
/// deterministic: `String.hashCode()` (`31 * h + c` over the UTF-16 units) spread by
/// `h ^ (h >>> 16)`, a power-of-two table that doubles when the size exceeds 3/4 of the
/// capacity (or pre-sizes for `putAll` / `new HashMap<>(map)`), and per-bin insertion order
/// (which `resize` preserves). A bin that Java would treeify (more than 8 entries in a table of
/// 64 or more slots) is `unported!`.
// Java: java/util/HashMap.java HashMap (JDK 25)
pub struct JavaHashMap {
    /// `table`; empty until allocated.
    table: Vec<Vec<(u32, String, String)>>,
    size: usize,
    /// `threshold` (before allocation: the initial capacity, or 0 for the default).
    threshold: usize,
}

impl JavaHashMap {
    /// `HashMap.DEFAULT_INITIAL_CAPACITY`.
    const DEFAULT_INITIAL_CAPACITY: usize = 16;
    /// `HashMap.TREEIFY_THRESHOLD`.
    const TREEIFY_THRESHOLD: usize = 8;
    /// `HashMap.MIN_TREEIFY_CAPACITY`.
    const MIN_TREEIFY_CAPACITY: usize = 64;

    // Java: java/util/HashMap.java HashMap.<init>()
    pub fn new() -> JavaHashMap {
        JavaHashMap {
            table: Vec::new(),
            size: 0,
            threshold: 0,
        }
    }

    // Java: java/util/HashMap.java HashMap.<init>(Map)
    pub fn from_entries(entries: &[(String, String)]) -> JavaHashMap {
        let mut map = JavaHashMap::new();
        map.put_map_entries(entries);
        map
    }

    // Java: java/util/HashMap.java HashMap.hash
    fn hash(key: &str) -> u32 {
        // Java: java/lang/String.java String.hashCode
        let mut h: i32 = 0;
        for unit in key.encode_utf16() {
            h = h.wrapping_mul(31).wrapping_add(i32::from(unit));
        }
        let h = h as u32;
        h ^ (h >> 16)
    }

    // Java: java/util/HashMap.java HashMap.tableSizeFor
    fn table_size_for(cap: usize) -> usize {
        cap.max(1).next_power_of_two()
    }

    // Java: java/util/HashMap.java HashMap.resize
    fn resize(&mut self) {
        let old_cap = self.table.len();
        let old_thr = self.threshold;
        let (new_cap, mut new_thr) = if old_cap > 0 {
            let new_cap = old_cap << 1;
            let new_thr = if old_cap >= Self::DEFAULT_INITIAL_CAPACITY {
                old_thr << 1
            } else {
                0
            };
            (new_cap, new_thr)
        } else if old_thr > 0 {
            // initial capacity was placed in threshold
            (old_thr, 0)
        } else {
            // zero initial threshold signifies using defaults
            (
                Self::DEFAULT_INITIAL_CAPACITY,
                Self::DEFAULT_INITIAL_CAPACITY * 3 / 4,
            )
        };
        if new_thr == 0 {
            // `(int) (newCap * 0.75f)`
            new_thr = new_cap * 3 / 4;
        }
        self.threshold = new_thr;
        let mut new_table: Vec<Vec<(u32, String, String)>> = Vec::with_capacity(new_cap);
        new_table.resize_with(new_cap, Vec::new);
        // Each old bin is split into two new bins; the nodes keep their relative order.
        for bin in self.table.drain(..) {
            for node in bin {
                let index = (node.0 as usize) & (new_cap - 1);
                new_table[index].push(node);
            }
        }
        self.table = new_table;
    }

    /// `put(key, value)`.
    // Java: java/util/HashMap.java HashMap.putVal
    pub fn put(&mut self, key: String, value: String) {
        if self.table.is_empty() {
            self.resize();
        }
        let hash = JavaHashMap::hash(&key);
        let index = (hash as usize) & (self.table.len() - 1);
        if let Some(node) = self.table[index]
            .iter_mut()
            .find(|node| node.0 == hash && node.1 == key)
        {
            // An existing mapping keeps its place.
            node.2 = value;
            return;
        }
        let bin_count = self.table[index].len();
        self.table[index].push((hash, key, value));
        // `if (binCount >= TREEIFY_THRESHOLD - 1) treeifyBin(tab, hash)`, where Java's `binCount`
        // is one less than the number of nodes that were in the bin.
        if bin_count >= Self::TREEIFY_THRESHOLD {
            if self.table.len() < Self::MIN_TREEIFY_CAPACITY {
                // `treeifyBin` resizes a small table instead of treeifying the bin.
                self.resize();
            } else {
                crate::unported!("java.util.HashMap.treeifyBin (platform properties)");
            }
        }
        self.size += 1;
        if self.size > self.threshold {
            self.resize();
        }
    }

    /// `putAll(map)`, `map` given in its own iteration order.
    // Java: java/util/HashMap.java HashMap.putMapEntries
    pub fn put_map_entries(&mut self, entries: &[(String, String)]) {
        let s = entries.len();
        if s > 0 {
            if self.table.is_empty() {
                // pre-size: `Math.ceil(s / (double) loadFactor)`
                let t = (s as f64 / 0.75).ceil() as usize;
                if t > self.threshold {
                    self.threshold = JavaHashMap::table_size_for(t);
                }
            } else {
                while s > self.threshold {
                    self.resize();
                }
            }
            for (key, value) in entries {
                self.put(key.clone(), value.clone());
            }
        }
    }

    /// `entrySet()`, in Java's iteration order.
    // Java: java/util/HashMap.java HashMap.HashIterator
    pub fn into_entries(self) -> Vec<(String, String)> {
        self.table
            .into_iter()
            .flatten()
            .map(|(_, key, value)| (key, value))
            .collect()
    }
}

impl Default for JavaHashMap {
    fn default() -> Self {
        JavaHashMap::new()
    }
}
