//! Port of `DigestMap.java`.

use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

use blaze_util::check_argument;
use blaze_util::fingerprint::{DigestHashFunction, Fingerprint};
use parking_lot::RwLock;

struct Entry {
    /// Java's table holds a strong reference to each key; this keeps the keyed allocation alive
    /// so its address is never reused while it is a key.
    _key: Box<dyn Any + Send + Sync>,
    digest: Box<[u8]>,
}

/// Map of key -> [digest bytes].
///
/// Keys use reference equality (here: the address of an `Arc` allocation, which the map keeps
/// alive).
// Java: com/google/devtools/build/lib/collect/nestedset/DigestMap.java:36 DigestMap
// PORT: Java's lock-free open-addressing table (StampedLock, AtomicReferenceArray, Murmur smear,
// resizing at 75% load) is a memory/GC optimization whose layout is not observable; this port
// keeps the same operations and results on a locked hash map. As in Java, the first digest
// inserted for a key wins.
pub(crate) struct DigestMap {
    #[allow(dead_code)]
    digest_hash_function: DigestHashFunction,
    table: RwLock<HashMap<usize, Entry>>,
}

fn key_identity<K: ?Sized>(key: &Arc<K>) -> usize {
    Arc::as_ptr(key) as *const () as usize
}

impl DigestMap {
    // Java: com/google/devtools/build/lib/collect/nestedset/DigestMap.java:58 DigestMap.<init>
    pub(crate) fn new(digest_hash_function: DigestHashFunction, initial_size: i32) -> Self {
        check_argument!(
            initial_size > 0 && (initial_size & (initial_size - 1)) == 0,
            "initialSize must be a power of 2 greater than 0"
        );
        DigestMap {
            digest_hash_function,
            table: RwLock::new(HashMap::with_capacity(initial_size as usize)),
        }
    }

    /// Finds the digest for the corresponding key and adds it to the passed fingerprint.
    // Java: com/google/devtools/build/lib/collect/nestedset/DigestMap.java:68 DigestMap.readDigest
    pub(crate) fn read_digest<K: ?Sized>(
        &self,
        key: &Arc<K>,
        fingerprint: &mut Fingerprint,
    ) -> bool {
        let table = self.table.read();
        match table.get(&key_identity(key)) {
            Some(entry) => {
                fingerprint.add_bytes(&entry.digest);
                true
            }
            None => false,
        }
    }

    /// Inserts a digest for the corresponding key, then immediately reads it into another
    /// fingerprint.
    ///
    /// This is equivalent to `digest_map.insert_digest(key, digest.digest_and_reset());
    /// digest_map.read_digest(key, read_to);` but it will be faster.
    ///
    /// `digest` is the fingerprint to insert. This will reset the fingerprint instance (only if
    /// the key was not already present, as in Java). `read_to` is a fingerprint to read the
    /// just-added fingerprint into.
    // Java: com/google/devtools/build/lib/collect/nestedset/DigestMap.java:104 DigestMap.insertAndReadDigest
    pub(crate) fn insert_and_read_digest<K: ?Sized + Send + Sync + 'static>(
        &self,
        key: &Arc<K>,
        digest: &mut Fingerprint,
        read_to: &mut Fingerprint,
    ) {
        let mut table = self.table.write();
        let entry = table.entry(key_identity(key)).or_insert_with(|| Entry {
            _key: Box::new(Arc::clone(key)),
            digest: digest.digest_and_reset().into_boxed_slice(),
        });
        read_to.add_bytes(&entry.digest);
    }
}

#[cfg(test)]
mod digest_map_test {
    //! Port of `DigestMapTest.java`.
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn fingerprint(f: DigestHashFunction) -> Fingerprint {
        Fingerprint::with_digest_function(f)
    }

    fn simple_test(f: DigestHashFunction) {
        let count = 128; // Must be smaller than byte for this test or we'll overflow
        let keys: Vec<Arc<i32>> = (0..count).map(Arc::new).collect();

        let digest_map = DigestMap::new(f, 4);
        for (i, key) in keys.iter().enumerate() {
            let mut digest = fingerprint(f);
            digest.add_int(i as i32);
            let mut fp = fingerprint(f);
            digest_map.insert_and_read_digest(key, &mut digest, &mut fp);
            let mut reference = fingerprint(f);
            reference.add_bytes(&fingerprint(f).add_int(i as i32).digest_and_reset());
            assert_eq!(fp.hex_digest_and_reset(), reference.hex_digest_and_reset());
        }
        for (i, key) in keys.iter().enumerate() {
            let mut fp = fingerprint(f);
            assert!(digest_map.read_digest(key, &mut fp));
            let mut reference = fingerprint(f);
            reference.add_bytes(&fingerprint(f).add_int(i as i32).digest_and_reset());
            assert_eq!(fp.hex_digest_and_reset(), reference.hex_digest_and_reset());
        }
    }

    fn concurrency_test(f: DigestHashFunction) {
        let count = 128usize; // Must be smaller than byte for this test or we'll overflow
        let keys: Vec<Arc<i32>> = (0..count as i32).map(Arc::new).collect();
        let digest_map = DigestMap::new(f, 4);
        let done = AtomicBool::new(false);
        let failure: parking_lot::Mutex<Option<String>> = parking_lot::Mutex::new(None);
        let thread_count = 16;
        std::thread::scope(|s| {
            for t in 0..thread_count {
                let (keys, digest_map, done, failure) = (&keys, &digest_map, &done, &failure);
                s.spawn(move || {
                    // Deterministic per-thread pseudo-random sequence (Java: new Random()).
                    let mut state: u64 = 0x9E37_79B9_7F4A_7C15 ^ (t as u64 + 1);
                    while !done.load(Ordering::SeqCst) {
                        state ^= state << 13;
                        state ^= state >> 7;
                        state ^= state << 17;
                        let index = (state % count as u64) as usize;
                        let key = &keys[index];
                        let mut fp = fingerprint(f);
                        if !digest_map.read_digest(key, &mut fp) {
                            let mut digest = fingerprint(f);
                            digest.add_int(index as i32);
                            digest_map.insert_and_read_digest(key, &mut digest, &mut fp);
                        }
                        let mut reference = fingerprint(f);
                        reference.add_bytes(&fingerprint(f).add_int(index as i32).digest_and_reset());
                        let hex_digest = fp.hex_digest_and_reset();
                        let reference_digest = reference.hex_digest_and_reset();
                        if hex_digest != reference_digest {
                            *failure.lock() = Some(format!(
                                "Digests are not equal: {hex_digest} != {reference_digest}, index {index}"
                            ));
                            done.store(true, Ordering::SeqCst);
                        }
                    }
                });
            }
            std::thread::sleep(std::time::Duration::from_millis(1000));
            done.store(true, Ordering::SeqCst);
        });
        if let Some(e) = failure.lock().take() {
            panic!("IllegalStateException: {e}");
        }
    }

    #[test]
    fn simple_test_sha256() {
        simple_test(DigestHashFunction::Sha256);
    }

    #[test]
    fn simple_test_sha1() {
        simple_test(DigestHashFunction::Sha1);
    }

    #[test]
    fn concurrency_test_sha256() {
        concurrency_test(DigestHashFunction::Sha256);
    }

    #[test]
    fn concurrency_test_sha1() {
        concurrency_test(DigestHashFunction::Sha1);
    }
}
