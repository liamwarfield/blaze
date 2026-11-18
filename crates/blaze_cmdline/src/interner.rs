//! A weak-value interner, the Rust counterpart of Bazel's `BlazeInterners.newWeakInterner()` /
//! `SkyKeyInterner`: equal values created while an earlier one is alive share one `Arc`.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, Weak};

const SHARDS: usize = 16;
const SWEEP_EVERY: usize = 4096;

struct Shard<T> {
    buckets: HashMap<u64, Vec<Weak<T>>>,
    inserts_since_sweep: usize,
}

pub(crate) struct WeakInterner<T> {
    shards: [Mutex<Shard<T>>; SHARDS],
}

/// A stable hash (fixed SipHash keys) used for cached hash codes and interner buckets.
pub(crate) fn stable_hash<T: Hash + ?Sized>(value: &T) -> u64 {
    let mut h = std::hash::DefaultHasher::new();
    value.hash(&mut h);
    h.finish()
}

impl<T: Eq> WeakInterner<T> {
    pub(crate) fn new() -> Self {
        WeakInterner {
            shards: std::array::from_fn(|_| {
                Mutex::new(Shard {
                    buckets: HashMap::new(),
                    inserts_since_sweep: 0,
                })
            }),
        }
    }

    /// Returns the live instance equal to `value`, or interns `value`. `hash` must be a function
    /// of the value consistent with `Eq`.
    pub(crate) fn intern(&self, hash: u64, value: T) -> Arc<T> {
        let mut shard = self.shards[(hash as usize) % SHARDS]
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let bucket = shard.buckets.entry(hash).or_default();
        let mut found = None;
        bucket.retain(|w| match w.upgrade() {
            Some(a) => {
                if found.is_none() && *a == value {
                    found = Some(a);
                }
                true
            }
            None => false,
        });
        if let Some(a) = found {
            return a;
        }
        let a = Arc::new(value);
        bucket.push(Arc::downgrade(&a));
        shard.inserts_since_sweep += 1;
        if shard.inserts_since_sweep >= SWEEP_EVERY {
            shard.inserts_since_sweep = 0;
            shard.buckets.retain(|_, b| {
                b.retain(|w| w.strong_count() > 0);
                !b.is_empty()
            });
        }
        a
    }
}
