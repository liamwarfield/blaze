//! Port of `lib/remote/chunking/FastCdcChunker.java`.

use std::io;

use super::gear_table::GEAR;
use super::{ContentDefinedChunker, FastCdcChunkingConfig, read_fully};
use crate::reapi::Digest;
use crate::util::DigestUtil;

/// Masks for each of the desired number of bits, where 0 through 5 are unused.
///
/// The values for sizes 64 bytes through 128 kilo-bytes come from the C reference implementation
/// (found in the destor repository) while the extra values come from the restic-FastCDC
/// repository. The FastCDC paper claims that the deduplication ratio is slightly improved when
/// the mask bits are spread relatively evenly, hence these seemingly "magic" values.
// Java: com/google/devtools/build/lib/remote/chunking/FastCdcChunker.java:40 FastCdcChunker.MASKS
const MASKS: [u64; 26] = [
    0,                  // 0: padding
    0,                  // 1: padding
    0,                  // 2: padding
    0,                  // 3: padding
    0,                  // 4: padding
    0x0000000001804110, // 5: unused except for NC 3
    0x0000000001803110, // 6: 64B
    0x0000000018035100, // 7: 128B
    0x0000001800035300, // 8: 256B
    0x0000019000353000, // 9: 512B
    0x0000590003530000, // 10: 1KB
    0x0000d90003530000, // 11: 2KB
    0x0000d90103530000, // 12: 4KB
    0x0000d90303530000, // 13: 8KB
    0x0000d90313530000, // 14: 16KB
    0x0000d90f03530000, // 15: 32KB
    0x0000d90303537000, // 16: 64KB
    0x0000d90703537000, // 17: 128KB
    0x0000d90707537000, // 18: 256KB
    0x0000d91707537000, // 19: 512KB
    0x0000d91747537000, // 20: 1MB
    0x0000d91767537000, // 21: 2MB
    0x0000d93767537000, // 22: 4MB
    0x0000d93777537000, // 23: 8MB
    0x0000d93777577000, // 24: 16MB
    0x0000db3777577000, // 25: unused except for NC 3
];

/// `GEAR_LS`: the Gear table shifted left by one.
// Java: com/google/devtools/build/lib/remote/chunking/FastCdcChunker.java:76 FastCdcChunker.computeGearLs
const fn compute_gear_ls() -> [u64; 256] {
    let mut gear_ls = [0u64; 256];
    let mut i = 0;
    while i < 256 {
        gear_ls[i] = GEAR[i] << 1;
        i += 1;
    }
    gear_ls
}

const GEAR_LS: [u64; 256] = compute_gear_ls();

/// FastCDC 2020 implementation for splitting large blobs.
///
/// This module implements the canonical FastCDC algorithm as described in the paper by Wen Xia,
/// et al., in 2020 (<https://ieeexplore.ieee.org/document/9055082>).
///
/// PORT: Java's `long` arithmetic wraps; the hash here uses `wrapping_add` and shifts that drop
/// the high bits, the same values.
// Java: com/google/devtools/build/lib/remote/chunking/FastCdcChunker.java:31 FastCdcChunker
pub struct FastCdcChunker {
    min_size: i32,
    max_size: i32,
    avg_size: i32,
    mask_s: u64,
    mask_l: u64,
    mask_s_ls: u64,
    mask_l_ls: u64,
    seed: u64,
    shifted_seed: u64,
    digest_util: DigestUtil,
}

impl FastCdcChunker {
    // Java: com/google/devtools/build/lib/remote/chunking/FastCdcChunker.java:95 FastCdcChunker.<init>
    pub fn new(digest_util: DigestUtil) -> FastCdcChunker {
        Self::from_config(&FastCdcChunkingConfig::defaults(), digest_util)
    }

    // Java: com/google/devtools/build/lib/remote/chunking/FastCdcChunker.java:99 FastCdcChunker.<init>
    pub fn from_config(config: &FastCdcChunkingConfig, digest_util: DigestUtil) -> FastCdcChunker {
        Self::with_sizes(
            config.min_chunk_size(),
            config.avg_chunk_size,
            config.max_chunk_size(),
            config.normalization_level,
            // Integer.toUnsignedLong(config.seed())
            u64::from(config.seed as u32),
            digest_util,
        )
    }

    /// Panics with Java's `IllegalArgumentException` text when a parameter is out of range.
    // Java: com/google/devtools/build/lib/remote/chunking/FastCdcChunker.java:109 FastCdcChunker.<init>
    pub fn with_sizes(
        min_size: i32,
        avg_size: i32,
        max_size: i32,
        normalization: i32,
        seed: u64,
        digest_util: DigestUtil,
    ) -> FastCdcChunker {
        blaze_util::check_argument!(min_size > 0, "minSize must be positive");
        blaze_util::check_argument!(avg_size >= min_size, "avgSize must be >= minSize");
        blaze_util::check_argument!(max_size >= avg_size, "maxSize must be >= avgSize");
        blaze_util::check_argument!(
            (avg_size & (avg_size - 1)) == 0,
            "avgSize must be a power of 2, got {}",
            avg_size
        );
        blaze_util::check_argument!(
            (0..=3).contains(&normalization),
            "normalization must be 0-3"
        );
        let bits = 31 - avg_size.leading_zeros() as i32;
        let small_bits = bits + normalization;
        let large_bits = bits - normalization;
        blaze_util::check_argument!(
            small_bits <= 25 && large_bits >= 5,
            "normalization level too extreme for avgSize"
        );
        let mask_s = MASKS[small_bits as usize];
        let mask_l = MASKS[large_bits as usize];
        FastCdcChunker {
            min_size,
            max_size,
            avg_size,
            mask_s,
            mask_l,
            mask_s_ls: mask_s << 1,
            mask_l_ls: mask_l << 1,
            seed,
            shifted_seed: seed << 1,
            digest_util,
        }
    }

    /// Finds the next chunk boundary.
    // Java: com/google/devtools/build/lib/remote/chunking/FastCdcChunker.java:137 FastCdcChunker.cut
    fn cut(&self, buf: &[u8], off: usize, len: i32) -> i32 {
        if len <= self.min_size {
            return len;
        }
        let n = len.min(self.max_size);
        let center = n.min(self.avg_size);

        // Round down to even boundaries for 2-byte processing so we don't need to divide by 2 in
        // the loop.
        let min_limit = self.min_size & !1;
        let center_limit = center & !1;
        let remaining_limit = n & !1;

        let s = self.seed;
        let s_ls = self.shifted_seed;
        let mut hash: u64 = 0;
        let at = |a: i32| buf[off + a as usize] as usize;

        // Below avgSize: use maskS to discourage early cuts (too small chunks)
        let mut a = min_limit;
        while a < center_limit {
            hash = (hash << 2).wrapping_add(GEAR_LS[at(a)] ^ s_ls);
            if (hash & self.mask_s_ls) == 0 {
                return a;
            }
            hash = hash.wrapping_add(GEAR[at(a + 1)] ^ s);
            if (hash & self.mask_s) == 0 {
                return a + 1;
            }
            a += 2;
        }

        // Above avgSize: use maskL to encourage cuts (too large chunks)
        let mut a = center_limit;
        while a < remaining_limit {
            hash = (hash << 2).wrapping_add(GEAR_LS[at(a)] ^ s_ls);
            if (hash & self.mask_l_ls) == 0 {
                return a;
            }
            hash = hash.wrapping_add(GEAR[at(a + 1)] ^ s);
            if (hash & self.mask_l) == 0 {
                return a + 1;
            }
            a += 2;
        }

        n
    }
}

impl ContentDefinedChunker for FastCdcChunker {
    /// Chunks a file and returns chunk digests.
    ///
    /// This method is used for building MerkleTree entries for large files. It returns the
    /// content digests in order for each chunk.
    ///
    /// Note: We don't need the raw data here. We can read from the original file (seekable) when
    /// uploading, similar to how whole blobs work.
    // Java: com/google/devtools/build/lib/remote/chunking/FastCdcChunker.java:192 FastCdcChunker.chunkToDigests
    fn chunk_to_digests(&self, input: &mut dyn io::Read) -> io::Result<Vec<Digest>> {
        let mut digests = Vec::new();
        let max_size = self.max_size as usize;
        let mut buf = vec![0u8; max_size * 2];
        let mut cursor = 0usize;
        let mut end = 0usize;
        let mut eof = false;

        loop {
            let mut available = end - cursor;
            if available < max_size && !eof {
                if cursor > 0 && available > 0 {
                    buf.copy_within(cursor..end, 0);
                }
                cursor = 0;
                end = available;
                let (read, reached_end) = read_fully(input, &mut buf[end..])?;
                end += read;
                eof = reached_end;
                available = end - cursor;
            }

            if available == 0 {
                break;
            }

            let chunk_len = self.cut(&buf, cursor, available as i32) as usize;
            digests.push(self.digest_util.compute(&buf[cursor..cursor + chunk_len]));
            cursor += chunk_len;
        }

        Ok(digests)
    }
}
