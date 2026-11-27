//! Port of `lib/vfs/DigestUtils.java`.
//!
//! PORT: `DigestUtils` belongs to `lib/vfs`; it lives here, beside its users.

/// Keys used to cache the values of the digests for files where we don't have fast digests. The
/// cache keys are derived from many properties of the file metadata in an attempt to be able to
/// detect most file changes.
// Java: com/google/devtools/build/lib/vfs/DigestUtils.java:42 DigestUtils.CacheKey
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct CacheKey {
    path: blaze_vfs::PathFragment,
    node_id: i64,
    change_time: i64,
    last_modified_time: i64,
    size: i64,
}

impl CacheKey {
    // Java: com/google/devtools/build/lib/vfs/DigestUtils.java:51 DigestUtils.CacheKey.<init>
    fn new(path: &blaze_vfs::Path, status: &blaze_vfs::FileStatus) -> CacheKey {
        CacheKey {
            path: path.as_fragment().clone(),
            node_id: status.get_node_id(),
            change_time: status.get_last_change_time(),
            last_modified_time: status.get_last_modified_time(),
            size: status.get_size(),
        }
    }
}

/// The statistics of the digest cache (Caffeine's `CacheStats`, the counters this port records).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hit_count: i64,
    pub miss_count: i64,
    pub eviction_count: i64,
}

impl CacheStats {
    /// `hitCount / requestCount`, 1.0 when there were no requests.
    pub fn hit_rate(&self) -> f64 {
        let request_count = self.hit_count + self.miss_count;
        if request_count == 0 {
            1.0
        } else {
            self.hit_count as f64 / request_count as f64
        }
    }

    /// The difference of this and `other`; each count is at least zero.
    pub fn minus(&self, other: &CacheStats) -> CacheStats {
        CacheStats {
            hit_count: (self.hit_count - other.hit_count).max(0),
            miss_count: (self.miss_count - other.miss_count).max(0),
            eviction_count: (self.eviction_count - other.eviction_count).max(0),
        }
    }
}

/// `Caffeine.newBuilder().maximumSize(maximumSize).recordStats().build()`.
///
/// PORT: Caffeine evicts by its W-TinyLFU policy; this cache evicts the least recently used
/// entry. Hits and misses do not depend on the policy until the cache is full (default
/// capacity 50,000 entries).
struct DigestCache {
    maximum_size: u64,
    entries: std::collections::HashMap<CacheKey, (Vec<u8>, u64)>,
    clock: u64,
    stats: CacheStats,
}

impl DigestCache {
    fn new(maximum_size: u64) -> DigestCache {
        DigestCache {
            maximum_size,
            entries: std::collections::HashMap::new(),
            clock: 0,
            stats: CacheStats::default(),
        }
    }

    fn get_if_present(&mut self, key: &CacheKey) -> Option<Vec<u8>> {
        self.clock += 1;
        match self.entries.get_mut(key) {
            Some((digest, used)) => {
                *used = self.clock;
                self.stats.hit_count += 1;
                Some(digest.clone())
            }
            None => {
                self.stats.miss_count += 1;
                None
            }
        }
    }

    fn put(&mut self, key: CacheKey, digest: Vec<u8>) {
        self.clock += 1;
        self.entries.insert(key, (digest, self.clock));
        while self.entries.len() as u64 > self.maximum_size {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(k, _)| k.clone())
                .expect("non-empty");
            self.entries.remove(&oldest);
            self.stats.eviction_count += 1;
        }
    }

    fn invalidate_all(&mut self) {
        self.entries.clear();
    }
}

/// Global cache of files to their digests. This is `None` when the cache is disabled.
// Java: com/google/devtools/build/lib/vfs/DigestUtils.java:71 DigestUtils.globalCache
static GLOBAL_CACHE: parking_lot::Mutex<Option<DigestCache>> = parking_lot::Mutex::new(None);

/// Port of `DigestUtils`.
// Java: com/google/devtools/build/lib/vfs/DigestUtils.java:32 DigestUtils
pub struct DigestUtils;

impl DigestUtils {
    /// Estimated digest size: a SHA-256 digest is 32 bytes.
    // Java: com/google/devtools/build/lib/vfs/DigestUtils.java:34 DigestUtils.ESTIMATED_SIZE
    pub const ESTIMATED_SIZE: i32 = 32;

    /// Enables the caching of file digests based on file status data.
    ///
    /// If the cache was already enabled, this causes the cache to be reinitialized thus losing
    /// all contents. If the given size is zero, the cache is disabled altogether.
    // Java: com/google/devtools/build/lib/vfs/DigestUtils.java:84 DigestUtils.configureCache
    pub fn configure_cache(maximum_size: i64) {
        *GLOBAL_CACHE.lock() = if maximum_size == 0 {
            None
        } else {
            Some(DigestCache::new(maximum_size as u64))
        };
    }

    /// Clears the cache contents without changing its size. No-op if the cache hasn't yet been
    /// initialized.
    // Java: com/google/devtools/build/lib/vfs/DigestUtils.java:96 DigestUtils.clearCache
    pub fn clear_cache() {
        if let Some(cache) = GLOBAL_CACHE.lock().as_mut() {
            cache.invalidate_all();
        }
    }

    /// Obtains cache statistics. The cache must have previously been enabled by a call to
    /// [`DigestUtils::configure_cache`].
    // Java: com/google/devtools/build/lib/vfs/DigestUtils.java:109 DigestUtils.getCacheStats
    pub fn get_cache_stats() -> CacheStats {
        GLOBAL_CACHE
            .lock()
            .as_ref()
            .map(|cache| cache.stats)
            .expect("NullPointerException: configureCache() must have been called with a size >= 0")
    }

    /// Gets the digest of `path`, using a constant-time xattr call if the filesystem supports it
    /// (`path.getFastDigest()`, the `XattrProvider` of the path's file system), and calculating
    /// the digest manually otherwise.
    // Java: com/google/devtools/build/lib/vfs/DigestUtils.java:127 DigestUtils.getDigestWithManualFallback
    pub fn get_digest_with_manual_fallback(path: &blaze_vfs::Path) -> std::io::Result<Vec<u8>> {
        Self::get_digest_with_manual_fallback_and_status(path, None)
    }

    /// [`DigestUtils::get_digest_with_manual_fallback`] with a recently obtained file status (the
    /// key of the digest cache), if available.
    // Java: com/google/devtools/build/lib/vfs/DigestUtils.java:139 DigestUtils.getDigestWithManualFallback
    pub fn get_digest_with_manual_fallback_and_status(
        path: &blaze_vfs::Path,
        status: Option<&blaze_vfs::FileStatus>,
    ) -> std::io::Result<Vec<u8>> {
        match path.get_fast_digest()? {
            Some(digest) => Ok(digest),
            None => Self::manually_compute_digest(path, status),
        }
    }

    /// Calculates a digest manually (i.e., assuming that a fast digest can't obtained), through
    /// the digest cache when it is enabled; `status` is a recently obtained file status, if
    /// available.
    // Java: com/google/devtools/build/lib/vfs/DigestUtils.java:153 DigestUtils.manuallyComputeDigest
    // Java: com/google/devtools/build/lib/vfs/DigestUtils.java:164 DigestUtils.manuallyComputeDigest
    pub fn manually_compute_digest(
        path: &blaze_vfs::Path,
        status: Option<&blaze_vfs::FileStatus>,
    ) -> std::io::Result<Vec<u8>> {
        // Attempt a cache lookup if the cache is enabled.
        let mut key = None;
        if GLOBAL_CACHE.lock().is_some() {
            let k = match status {
                Some(status) => CacheKey::new(path, status),
                None => CacheKey::new(path, &path.stat()?),
            };
            if let Some(cache) = GLOBAL_CACHE.lock().as_mut()
                && let Some(digest) = cache.get_if_present(&k)
            {
                return Ok(digest);
            }
            key = Some(k);
        }

        let digest = path.get_digest()?;

        if let Some(key) = key
            && let Some(cache) = GLOBAL_CACHE.lock().as_mut()
        {
            cache.put(key, digest.clone());
        }
        Ok(digest)
    }

    /// Computes a digest corresponding to the unordered combination of two digests. The result
    /// has the length of the longer input; `lhs` is modified in place when it is at least as long
    /// as `rhs` (as in Java, where the longer array is returned after being added into).
    // Java: com/google/devtools/build/lib/vfs/DigestUtils.java:192 DigestUtils.combineUnordered
    pub fn combine_unordered(mut lhs: Vec<u8>, mut rhs: Vec<u8>) -> Vec<u8> {
        let n = rhs.len();
        if lhs.len() >= n {
            for i in 0..n {
                // Use + as in Guava's Hashing.combineUnordered.
                // This has a number of advantages over XOR, which was used in the past:
                // * Identical inputs will not cancel each other out.
                // * Due to the carry, addition isn't a linear operation on the level of bit vectors.
                //   This prevents adversaries from producing linear combinations (i.e., subsets of input
                //   sets) that collide with other inputs.
                lhs[i] = lhs[i].wrapping_add(rhs[i]);
            }
            return lhs;
        }
        std::mem::swap(&mut lhs, &mut rhs);
        DigestUtils::combine_unordered(lhs, rhs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combine_unordered_is_commutative_and_wraps() {
        let a = vec![0xff, 1, 2];
        let b = vec![2, 3];
        assert_eq!(
            DigestUtils::combine_unordered(a.clone(), b.clone()),
            vec![1, 4, 2]
        );
        assert_eq!(DigestUtils::combine_unordered(b, a), vec![1, 4, 2]);
    }
}
