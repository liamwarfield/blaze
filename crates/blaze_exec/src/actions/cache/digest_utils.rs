//! The parts of `lib/vfs/DigestUtils.java` the action cache uses.
//!
//! PORT: `DigestUtils` belongs to `lib/vfs`; these members live here until `blaze_vfs` ports
//! them.

/// Port of `DigestUtils` (the action-cache subset).
// Java: com/google/devtools/build/lib/vfs/DigestUtils.java:32 DigestUtils
pub struct DigestUtils;

impl DigestUtils {
    /// Estimated digest size: a SHA-256 digest is 32 bytes.
    // Java: com/google/devtools/build/lib/vfs/DigestUtils.java:34 DigestUtils.ESTIMATED_SIZE
    pub const ESTIMATED_SIZE: i32 = 32;

    /// Gets the digest of `path`, using a constant-time xattr call if the filesystem supports it
    /// (`path.getFastDigest()`, the `XattrProvider` of the path's file system), and calculating
    /// the digest manually otherwise.
    ///
    /// PORT: the digest cache of `manuallyComputeDigest` (`DigestUtils.configureCache`) is not
    /// ported; the manual digest is `path.getDigest()`, and `status` (the recently obtained
    /// `FileStatus` that only serves as that cache's key) is not taken.
    // Java: com/google/devtools/build/lib/vfs/DigestUtils.java:139 DigestUtils.getDigestWithManualFallback
    pub fn get_digest_with_manual_fallback(path: &blaze_vfs::Path) -> std::io::Result<Vec<u8>> {
        match path.get_fast_digest()? {
            Some(digest) => Ok(digest),
            // Java: com/google/devtools/build/lib/vfs/DigestUtils.java:164 DigestUtils.manuallyComputeDigest
            None => path.get_digest(),
        }
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
