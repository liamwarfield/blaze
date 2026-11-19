//! The iteration order of a C++ `std::unordered_multiset<std::string>` as libstdc++ gives it.
//!
//! `AreStartupOptionsDifferent` (`src/main/cpp/blaze.cc:1021`) collects the differing startup
//! options in two `std::unordered_multiset<string>` and logs them, and
//! `KillRunningServerIfDifferentStartupOptions` prints them in the "Only in old server" / "Only
//! in new server" warning, in the sets' iteration order. That order is visible output, so it is
//! reproduced here.
//!
//! PORT: cpp-hash-order. Bazel's client (built with GCC 8.5, `libstdc++.so.6` linked
//! dynamically) iterates its sets in the order libstdc++'s `_Hashtable` keeps them: one singly
//! linked list in which the nodes of a bucket are contiguous, a new node going to the front of
//! its bucket's run, a node of an empty bucket starting a new run at the front of the list, a
//! rehash rebuilding the list from the old order. The hash is `std::_Hash_bytes` (seed
//! `0xc70f6907`); the bucket counts follow `std::__detail::_Prime_rehash_policy`
//! (`libstdc++-v3/src/c++11/hashtable_c++0x.cc` and `shared/hashtable-aux.cc`), which live in
//! the shared library and so are those of the system's libstdc++ at run time, unchanged since
//! GCC 5 in the parts used here.

/// `std::hash<std::string>`: `std::_Hash_bytes(data, len, 0xc70f6907)` for a 64-bit `size_t`
/// (`libstdc++-v3/libsupc++/hash_bytes.cc`, the 8-byte `size_t` branch).
pub fn std_hash_string(bytes: &[u8]) -> u64 {
    const SEED: u64 = 0xc70f6907;
    const MUL: u64 = (0xc6a4a793u64 << 32) + 0x5bd1e995u64;
    fn shift_mix(v: u64) -> u64 {
        v ^ (v >> 47)
    }
    let len = bytes.len();
    let len_aligned = len & !7usize;
    let mut hash: u64 = SEED ^ (len as u64).wrapping_mul(MUL);
    for chunk in bytes[..len_aligned].chunks_exact(8) {
        let word = u64::from_le_bytes(chunk.try_into().expect("8 bytes"));
        let data = shift_mix(word.wrapping_mul(MUL)).wrapping_mul(MUL);
        hash ^= data;
        hash = hash.wrapping_mul(MUL);
    }
    if len & 7 != 0 {
        // load_bytes(end, len & 7): the tail bytes as a little-endian number.
        let mut data: u64 = 0;
        for &b in bytes[len_aligned..].iter().rev() {
            data = (data << 8).wrapping_add(b as u64);
        }
        hash ^= data;
        hash = hash.wrapping_mul(MUL);
    }
    hash = shift_mix(hash).wrapping_mul(MUL);
    shift_mix(hash)
}

/// The start of libstdc++'s `__prime_list` (`shared/hashtable-aux.cc`), enough for tables of up
/// to 1,500 elements; larger tables are not reached by a startup command line.
const PRIME_LIST: &[u64] = &[
    2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37, 41, 43, 47, 53, 59, 61, 67, 71, 73, 79, 83, 89, 97,
    103, 109, 113, 127, 137, 139, 149, 157, 167, 179, 193, 199, 211, 227, 241, 257, 277, 293, 313,
    337, 359, 383, 409, 439, 467, 503, 541, 577, 619, 661, 709, 761, 823, 887, 953, 1031, 1109,
    1193, 1289, 1381, 1493, 1613, 1741, 1879, 2029, 2179, 2357, 2549,
];

/// `std::__detail::_Prime_rehash_policy` with the default maximum load factor 1.0.
struct PrimeRehashPolicy {
    next_resize: u64,
}

impl PrimeRehashPolicy {
    /// `_Prime_rehash_policy::_M_next_bkt`.
    fn next_bkt(&mut self, n: u64) -> u64 {
        const FAST_BKT: [u64; 14] = [2, 2, 2, 3, 5, 5, 7, 7, 11, 11, 11, 11, 13, 13];
        if n < FAST_BKT.len() as u64 {
            if n == 0 {
                return 1;
            }
            self.next_resize = FAST_BKT[n as usize];
            return FAST_BKT[n as usize];
        }
        // lower_bound(__prime_list + 6, __last_prime, n)
        let next = PRIME_LIST[6..]
            .iter()
            .copied()
            .find(|&p| p >= n)
            .unwrap_or_else(|| {
                panic!("cpp_unordered: a table of {n} buckets is beyond the embedded prime list")
            });
        self.next_resize = next;
        next
    }

    /// `_Prime_rehash_policy::_M_need_rehash`: the new bucket count, if a rehash is needed.
    fn need_rehash(&mut self, n_bkt: u64, n_elt: u64, n_ins: u64) -> Option<u64> {
        if n_elt + n_ins > self.next_resize {
            // If _M_next_resize is 0 it means that we have nothing allocated so far and that we
            // start inserting elements. In this case we start with an initial bucket size of 11.
            let min_bkts = std::cmp::max(n_elt + n_ins, if self.next_resize != 0 { 0 } else { 11 });
            if min_bkts >= n_bkt {
                return Some(self.next_bkt(std::cmp::max(min_bkts + 1, n_bkt * 2)));
            }
            self.next_resize = n_bkt;
            return None;
        }
        None
    }
}

/// A `std::unordered_multiset<std::string>` (default-constructed) reduced to what decides its
/// iteration order: the buckets' runs in list order.
pub struct CppUnorderedMultiset {
    bucket_count: u64,
    policy: PrimeRehashPolicy,
    /// The list, as runs of (bucket index, elements of that bucket in list order).
    runs: Vec<(u64, Vec<String>)>,
    len: usize,
}

impl CppUnorderedMultiset {
    pub fn new() -> Self {
        CppUnorderedMultiset {
            bucket_count: 1,
            policy: PrimeRehashPolicy { next_resize: 0 },
            runs: Vec::new(),
            len: 0,
        }
    }

    fn hash(s: &str) -> u64 {
        // The C++ hashes the raw bytes of the std::string: the internal string's Latin-1 bytes.
        std_hash_string(&blaze_util::string_encoding::latin1_encode(s))
    }

    fn bucket_index(&self, s: &str) -> u64 {
        Self::hash(s) % self.bucket_count
    }

    /// `_Hashtable::_M_rehash_aux(n, false_type)`: rebuilds the list in the new buckets,
    /// visiting the old list in order.
    fn rehash(&mut self, n: u64) {
        let old: Vec<String> = self.runs.drain(..).flat_map(|(_, v)| v).collect();
        self.bucket_count = n;
        // (run index of the previous node's bucket, position of the previous node in it)
        let mut prev: Option<(u64, usize)> = None;
        for s in old {
            let bkt = self.bucket_index(&s);
            match prev {
                Some((prev_bkt, prev_pos)) if prev_bkt == bkt => {
                    // Previous insert was already in this bucket: insert after it to preserve
                    // equivalent elements' relative order.
                    let run = self.run_mut(bkt).expect("bucket of the previous node");
                    run.insert(prev_pos + 1, s);
                    prev = Some((bkt, prev_pos + 1));
                }
                _ => {
                    if let Some(run) = self.run_mut(bkt) {
                        // Bucket not empty: after the bucket's before-begin node.
                        run.insert(0, s);
                    } else {
                        // Empty bucket: at the beginning of the list.
                        self.runs.insert(0, (bkt, vec![s]));
                    }
                    prev = Some((bkt, 0));
                }
            }
        }
    }

    fn run_mut(&mut self, bkt: u64) -> Option<&mut Vec<String>> {
        self.runs
            .iter_mut()
            .find(|(b, _)| *b == bkt)
            .map(|(_, v)| v)
    }

    /// `insert(value)`: `_M_insert_multi_node(nullptr, code, node)`.
    pub fn insert(&mut self, s: String) {
        if let Some(n) = self
            .policy
            .need_rehash(self.bucket_count, self.len as u64, 1)
        {
            self.rehash(n);
        }
        let bkt = self.bucket_index(&s);
        match self.runs.iter_mut().find(|(b, _)| *b == bkt) {
            Some((_, run)) => {
                if let Some(i) = run.iter().position(|e| *e == s) {
                    // Next to an equivalent element (before or after it is not observable: the
                    // elements are equal).
                    run.insert(i, s);
                } else {
                    // No equivalent: at the beginning of the bucket.
                    run.insert(0, s);
                }
            }
            None => self.runs.insert(0, (bkt, vec![s])),
        }
        self.len += 1;
    }

    /// `auto it = find(s); if (it != end()) erase(it);`: removes one instance. Returns whether
    /// there was one.
    pub fn erase_one(&mut self, s: &str) -> bool {
        let bkt = self.bucket_index(s);
        let Some(ri) = self.runs.iter().position(|(b, _)| *b == bkt) else {
            return false;
        };
        let run = &mut self.runs[ri].1;
        let Some(i) = run.iter().position(|e| e == s) else {
            return false;
        };
        run.remove(i);
        if run.is_empty() {
            self.runs.remove(ri);
        }
        self.len -= 1;
        true
    }

    /// `find(s) != end()`.
    pub fn contains(&self, s: &str) -> bool {
        let bkt = self.bucket_index(s);
        self.runs
            .iter()
            .any(|(b, run)| *b == bkt && run.iter().any(|e| e == s))
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The elements in iteration order (`for (const string &a : set)`).
    pub fn iter(&self) -> impl Iterator<Item = &String> {
        self.runs.iter().flat_map(|(_, v)| v.iter())
    }
}

impl Default for CppUnorderedMultiset {
    fn default() -> Self {
        Self::new()
    }
}
