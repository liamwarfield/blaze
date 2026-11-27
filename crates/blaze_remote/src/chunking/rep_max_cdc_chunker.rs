//! Port of `lib/remote/chunking/RepMaxCdcChunker.java`.

use std::io;

use super::gear_table::{GEAR, GEAR_HASH_WINDOW_SIZE};
use super::{ContentDefinedChunker, RepMaxCdcChunkingConfig, read_fully};
use crate::reapi::Digest;
use crate::util::DigestUtil;

/// RepMaxCDC implementation for splitting large blobs.
///
/// This module implements the RepMaxCDC algorithm as specified by the reference implementation
/// in buildbarn/go-cdc. RepMaxCDC expands upon MaxCDC, in that it repeatedly applies the chunking
/// process until all chunks are within `[minChunkSize, 2 * minChunkSize)` in size.
///
/// Cutting points are selected where the Gear rolling hash (a 64-byte window over the input) is
/// strictly maximal within a lookahead window. The size of the lookahead window is controlled by
/// the horizon size. Unlike FastCDC's maximum chunk size, the horizon size only denotes the
/// quality of the chunking that is performed: setting it to zero leads to uniform chunks of
/// `minChunkSize`, while setting it to a positive value `n` means that an optimal cutting point
/// within offsets `[minChunkSize, minChunkSize + n]` will always be respected.
///
/// All chunks are within `[minChunkSize, 2 * minChunkSize)` in size, except for the last chunk of
/// the input, which may be smaller than `minChunkSize` only if the entire input is. Chunk
/// boundaries are identical to those produced by the Go reference implementation for the same
/// parameters.
///
/// PORT: Java's `long` arithmetic wraps and its hashes are compared with
/// `Long.compareUnsigned`; the hashes here are `u64` with wrapping arithmetic.
// Java: com/google/devtools/build/lib/remote/chunking/RepMaxCdcChunker.java:61 RepMaxCdcChunker
pub struct RepMaxCdcChunker {
    min_size: i32,
    /// The amount of data that must be available (unless the end of the input has been reached)
    /// to determine the next chunk boundary: 2 * minSize + horizonSize. An optimal cutting point
    /// is searched within the horizonSize bytes following the earliest possible cutting point at
    /// minSize, and a further minSize bytes are left behind so that the final chunk of the input
    /// is at least minSize in size as well.
    peek_size: i32,
    digest_util: DigestUtil,
}

impl RepMaxCdcChunker {
    // Java: com/google/devtools/build/lib/remote/chunking/RepMaxCdcChunker.java:72 RepMaxCdcChunker.<init>
    pub fn new(digest_util: DigestUtil) -> RepMaxCdcChunker {
        Self::from_config(&RepMaxCdcChunkingConfig::defaults(), digest_util)
    }

    // Java: com/google/devtools/build/lib/remote/chunking/RepMaxCdcChunker.java:76 RepMaxCdcChunker.<init>
    pub fn from_config(
        config: &RepMaxCdcChunkingConfig,
        digest_util: DigestUtil,
    ) -> RepMaxCdcChunker {
        Self::with_sizes(config.min_chunk_size, config.horizon_size, digest_util)
    }

    /// Creates a chunker producing chunks of `[minSize, 2 * minSize)` bytes. Panics with Java's
    /// `IllegalArgumentException` text when the parameters are out of range.
    // Java: com/google/devtools/build/lib/remote/chunking/RepMaxCdcChunker.java:89 RepMaxCdcChunker.<init>
    pub fn with_sizes(min_size: i32, horizon_size: i32, digest_util: DigestUtil) -> Self {
        blaze_util::check_argument!(
            min_size >= GEAR_HASH_WINDOW_SIZE,
            "minSize must be at least {}, got {}",
            GEAR_HASH_WINDOW_SIZE,
            min_size
        );
        blaze_util::check_argument!(
            horizon_size >= 0,
            "horizonSize must not be negative, got {}",
            horizon_size
        );
        // The read buffer is sized at twice the peek size, so make sure that fits in an int.
        let peek = 2 * i64::from(min_size) + i64::from(horizon_size);
        blaze_util::check_argument!(
            peek <= i64::from(i32::MAX / 2),
            "2 * minSize + horizonSize must be at most {}, got {}",
            i32::MAX / 2,
            peek
        );
        RepMaxCdcChunker {
            min_size,
            peek_size: 2 * min_size + horizon_size,
            digest_util,
        }
    }
}

impl ContentDefinedChunker for RepMaxCdcChunker {
    /// Chunks a blob and returns chunk digests.
    // Java: com/google/devtools/build/lib/remote/chunking/RepMaxCdcChunker.java:121 RepMaxCdcChunker.chunkToDigests
    fn chunk_to_digests(&self, input: &mut dyn io::Read) -> io::Result<Vec<Digest>> {
        let mut digests = Vec::new();
        let mut session = ChunkingSession::new(self, input);
        loop {
            let chunk_size = session.next_chunk_size()?;
            if chunk_size == 0 {
                break;
            }
            let offset = session.buf_start;
            digests.push(
                self.digest_util
                    .compute(&session.buf[offset..offset + chunk_size as usize]),
            );
        }
        Ok(digests)
    }
}

/// The chunking state for a single input stream.
///
/// This is a port of `repMaxContentDefinedChunker.ReadNextChunk()` from buildbarn/go-cdc on top
/// of a sliding read buffer, and mirrors its structure closely. Comments are largely taken from
/// the Go implementation.
// Java: com/google/devtools/build/lib/remote/chunking/RepMaxCdcChunker.java:139 RepMaxCdcChunker.ChunkingSession
struct ChunkingSession<'a> {
    min_size: i32,
    peek_size: i32,
    input: &'a mut dyn io::Read,
    buf: Vec<u8>,
    buf_start: usize,
    buf_end: usize,
    eof: bool,
    /// The size of the previous chunk returned by nextChunkSize(). This amount of data will be
    /// discarded from the buffer at the start of the next call to nextChunkSize().
    previous_chunk_size: i32,
    /// List of chunks for which no future data can influence their length. For each chunk, its
    /// size is stored. Chunks are stored in reverse order, so that they can be popped from the
    /// end.
    complete_chunks: IntList,
    /// List of cutting points that will determine the length of future chunks. The hashes at the
    /// positions of the cutting points in this list will be strictly monotonically increasing.
    ///
    /// Cutting points are addressed relative to the first eligible position at which they may be
    /// placed (i.e., the end of the last complete chunk, plus the minimum chunk size). This means
    /// that the first entry is always equal to zero.
    incomplete_chunks: IntList,
    /// The rolling hash value corresponding to the position up to where input data has been
    /// processed.
    current_hash: u64,
    /// The rolling hash value corresponding to the position of the last incomplete chunk. Any
    /// new incomplete chunk must have a hash value that is higher than this one.
    best_hash: u64,
}

impl<'a> ChunkingSession<'a> {
    fn new(chunker: &RepMaxCdcChunker, input: &'a mut dyn io::Read) -> ChunkingSession<'a> {
        ChunkingSession {
            min_size: chunker.min_size,
            peek_size: chunker.peek_size,
            input,
            buf: vec![0u8; chunker.peek_size as usize * 2],
            buf_start: 0,
            buf_end: 0,
            eof: false,
            previous_chunk_size: 0,
            complete_chunks: IntList::default(),
            incomplete_chunks: IntList::default(),
            current_hash: 0,
            best_hash: 0,
        }
    }

    /// Makes up to `n` bytes available at `buf_start` and returns how many of them are available,
    /// which is only less than `n` at the end of the input.
    // Java: com/google/devtools/build/lib/remote/chunking/RepMaxCdcChunker.java:192 RepMaxCdcChunker.ChunkingSession.peek
    fn peek(&mut self, n: i32) -> io::Result<i32> {
        let mut available = self.buf_end - self.buf_start;
        if (available as i64) < i64::from(n) && !self.eof {
            if self.buf_start > 0 && available > 0 {
                self.buf.copy_within(self.buf_start..self.buf_end, 0);
            }
            self.buf_start = 0;
            self.buf_end = available;
            let (read, reached_end) = read_fully(self.input, &mut self.buf[self.buf_end..])?;
            self.buf_end += read;
            self.eof = reached_end;
            available = self.buf_end - self.buf_start;
        }
        Ok(n.min(available as i32))
    }

    /// The byte at `index` of the buffer, as a Gear table index.
    fn at(&self, index: i32) -> usize {
        self.buf[index as usize] as usize
    }

    /// Returns the size of the next chunk, whose data is available in the buffer at `buf_start`,
    /// or zero at the end of the input.
    // Java: com/google/devtools/build/lib/remote/chunking/RepMaxCdcChunker.java:217 RepMaxCdcChunker.ChunkingSession.nextChunkSize
    fn next_chunk_size(&mut self) -> io::Result<i32> {
        let min_size = self.min_size;
        // Discard data that was handed out by the previous call.
        self.buf_start += self.previous_chunk_size as usize;
        self.previous_chunk_size = 0;

        // If the previous iteration yielded multiple chunks, we can return them without peeking
        // the full horizon.
        if !self.complete_chunks.is_empty() {
            let first_chunk = self.complete_chunks.pop_last();
            blaze_util::check_state!(
                self.peek(first_chunk)? == first_chunk,
                "complete chunk no longer buffered"
            );
            self.previous_chunk_size = first_chunk;
            return Ok(first_chunk);
        }

        // Gain access to the data corresponding to the next chunk(s). If we're reaching the end
        // of the input, either consume all data or leave at least minSize bytes behind. This
        // ensures that all chunks of the file are at least minSize in size, assuming the file is
        // as well.
        let mut d_len = self.peek(self.peek_size)?;
        if d_len < 2 * min_size {
            if d_len == 0 {
                return Ok(0);
            }
            self.previous_chunk_size = d_len;
            return Ok(d_len);
        }
        d_len -= min_size;

        let base = self.buf_start as i32;

        // Extract the last incomplete chunk from the list, as it denotes where the previous call
        // stopped hashing the input. incompleteChunks takes over the role of the Go
        // implementation's oldChunks from here on.
        let mut current_chunk: i32;
        let mut hash: u64;
        let mut best: u64;
        if self.incomplete_chunks.size() >= 2 {
            current_chunk = self.incomplete_chunks.pop_last();
            hash = self.current_hash;
            best = self.best_hash;
        } else {
            // This is the very first chunk. We know that the first minSize positions can't
            // contain a cut. Skip them.
            self.incomplete_chunks.clear();
            self.incomplete_chunks.add(0);
            hash = 0;
            for i in (min_size - GEAR_HASH_WINDOW_SIZE)..min_size {
                hash = (hash << 1).wrapping_add(GEAR[self.at(base + i)]);
            }
            best = hash;
            current_chunk = 0;
        }

        // The position of the next byte to hash, relative to the start of the peeked data.
        let mut pos = min_size + current_chunk;
        loop {
            // Start hashing data where the previous call left off. Stop hashing before the
            // distance between two consecutive potential cutting points becomes minSize in size,
            // as this allows us to complete a chunk.
            let mut hash_region_len = d_len - pos;
            let mut original_chunk_count: i32 = -1;
            let bytes_before_min_chunk_size =
                self.incomplete_chunks.last() + min_size - 1 - current_chunk;
            if hash_region_len > bytes_before_min_chunk_size {
                hash_region_len = bytes_before_min_chunk_size;
                original_chunk_count = self.incomplete_chunks.size();
            } else if hash_region_len == 0 {
                break;
            }

            // Preserve all offsets at which the hash increases. Hash values are compared as
            // unsigned 64-bit integers, matching the Go reference implementation.
            //
            // The Gear hash recurrence is linear, so the hashes of the next four positions can all
            // be computed directly from the hash at the block start: h(i+k) = (hash << k) + s(k),
            // where s(k) is the Gear sum of the k block bytes.
            let mut idx: i32 = 0;
            let mut p = base + pos;
            while idx + 4 <= hash_region_len {
                let s1 = GEAR[self.at(p)];
                let s2 = (s1 << 1).wrapping_add(GEAR[self.at(p + 1)]);
                let s3 = (s2 << 1).wrapping_add(GEAR[self.at(p + 2)]);
                let s4 = (s3 << 1).wrapping_add(GEAR[self.at(p + 3)]);
                let h1 = (hash << 1).wrapping_add(s1);
                let h2 = (hash << 2).wrapping_add(s2);
                let h3 = (hash << 3).wrapping_add(s3);
                let h4 = (hash << 4).wrapping_add(s4);
                hash = h4;
                if best < h1 {
                    best = h1;
                    self.incomplete_chunks.add(current_chunk + idx + 1);
                }
                if best < h2 {
                    best = h2;
                    self.incomplete_chunks.add(current_chunk + idx + 2);
                }
                if best < h3 {
                    best = h3;
                    self.incomplete_chunks.add(current_chunk + idx + 3);
                }
                if best < h4 {
                    best = h4;
                    self.incomplete_chunks.add(current_chunk + idx + 4);
                }
                idx += 4;
                p += 4;
            }
            while idx < hash_region_len {
                hash = (hash << 1).wrapping_add(GEAR[self.at(base + pos + idx)]);
                if best < hash {
                    best = hash;
                    self.incomplete_chunks.add(current_chunk + idx + 1);
                }
                idx += 1;
            }

            if self.incomplete_chunks.size() == original_chunk_count {
                // The loop above did not yield any new cutting points, and the next byte is
                // minSize away from the last cutting point. This means we can complete all chunks
                // up to this point.
                let previous_complete_chunk_count = self.complete_chunks.size();
                let mut next_chunk = self.incomplete_chunks.last();
                let mut i = self.incomplete_chunks.size() - 3;
                while next_chunk >= min_size {
                    let chunk = self.incomplete_chunks.get(i);
                    if next_chunk - chunk >= min_size {
                        self.complete_chunks.add(next_chunk - chunk);
                        next_chunk = chunk;
                        i -= 1;
                    }
                    i -= 1;
                }
                self.complete_chunks.add(min_size + next_chunk);
                let size = self.complete_chunks.size();
                self.complete_chunks
                    .reverse(previous_complete_chunk_count, size);
                self.incomplete_chunks.truncate(1);
                current_chunk = 0;
                hash = (hash << 1).wrapping_add(GEAR[self.at(base + pos + hash_region_len)]);
                best = hash;
                pos += hash_region_len + 1;
            } else {
                current_chunk += hash_region_len;
                pos += hash_region_len;
            }
        }

        // Processed the full horizon. Return the first chunk.
        self.incomplete_chunks.add(current_chunk);
        let first_chunk: i32;
        if !self.complete_chunks.is_empty() {
            let size = self.complete_chunks.size();
            self.complete_chunks.reverse(0, size);
            first_chunk = self.complete_chunks.pop_last();
        } else {
            // The process above did not yield any complete chunks, either because we reached the
            // end of the file or the horizon size wasn't large enough.
            //
            // Ensure that we pick a cutting point respecting the maximum chunk size, that still
            // allows us to pick the most optimal cutting point in the horizon later on.
            let mut first_chunk_index = self.incomplete_chunks.size() - 2;
            let mut max_chunk = self.incomplete_chunks.get(first_chunk_index) - min_size;
            let mut i = first_chunk_index - 2;
            while max_chunk >= 0 {
                let chunk = self.incomplete_chunks.get(i);
                if chunk <= max_chunk {
                    first_chunk_index = i;
                    max_chunk = chunk - min_size;
                    i -= 1;
                }
                i -= 1;
            }
            first_chunk = min_size + self.incomplete_chunks.get(first_chunk_index);

            // There will be potential cutting points after the selected one that are no longer
            // eligible, as those would violate the minimum chunk size. These should be removed
            // from the list.
            let mut reusable_chunk_index = first_chunk_index + 1;
            loop {
                let offset_in_second_chunk =
                    self.incomplete_chunks.get(reusable_chunk_index) - first_chunk;
                if offset_in_second_chunk >= 0 {
                    // This cutting point and the ones after it should be kept.
                    for i in reusable_chunk_index..self.incomplete_chunks.size() {
                        let value = self.incomplete_chunks.get(i) - first_chunk;
                        self.incomplete_chunks.set(i, value);
                    }
                    if offset_in_second_chunk == 0 {
                        // There is no need to recompute any cutting points.
                        self.incomplete_chunks.remove_prefix(reusable_chunk_index);
                    } else {
                        // Because the first cutting point to keep resides at an offset beyond
                        // the minimum chunk size, we may have glossed over potential cutting
                        // points before it. Recompute these.
                        //
                        // This should only happen rarely, especially if the horizon size is
                        // sufficiently large.
                        let region_start = base + first_chunk;
                        let mut recomputed_hash: u64 = 0;
                        for i in (min_size - GEAR_HASH_WINDOW_SIZE)..min_size {
                            recomputed_hash = (recomputed_hash << 1)
                                .wrapping_add(GEAR[self.at(region_start + i)]);
                        }
                        self.incomplete_chunks.set(0, 0);
                        let mut best_recomputed_hash = recomputed_hash;
                        let mut recomputed_chunk_index = 1;
                        let original_chunks_count = self.incomplete_chunks.size();
                        for i in 0..offset_in_second_chunk - 1 {
                            recomputed_hash = (recomputed_hash << 1)
                                .wrapping_add(GEAR[self.at(region_start + min_size + i)]);
                            if best_recomputed_hash < recomputed_hash {
                                best_recomputed_hash = recomputed_hash;
                                let recomputed_chunk = i + 1;
                                if recomputed_chunk_index < reusable_chunk_index {
                                    self.incomplete_chunks
                                        .set(recomputed_chunk_index, recomputed_chunk);
                                    recomputed_chunk_index += 1;
                                } else {
                                    self.incomplete_chunks.add(recomputed_chunk);
                                }
                            }
                        }
                        if recomputed_chunk_index < reusable_chunk_index {
                            // Recomputing yielded fewer cutting points than we had previously.
                            // Make the cutting points contiguous again.
                            self.incomplete_chunks
                                .remove_range(recomputed_chunk_index, reusable_chunk_index);
                        } else if self.incomplete_chunks.size() > original_chunks_count {
                            // Recomputing yielded more cutting points than we had previously.
                            // The excess cutting points were stored at the end. Rotate them into
                            // place, so that the list remains sorted.
                            let size = self.incomplete_chunks.size();
                            self.incomplete_chunks
                                .reverse(reusable_chunk_index, original_chunks_count);
                            self.incomplete_chunks.reverse(original_chunks_count, size);
                            self.incomplete_chunks.reverse(reusable_chunk_index, size);
                        }
                    }
                    break;
                }

                // The cutting point should be removed.
                reusable_chunk_index += 1;
                if reusable_chunk_index == self.incomplete_chunks.size() {
                    self.incomplete_chunks.truncate(1);
                    break;
                }
            }
        }
        self.previous_chunk_size = first_chunk;
        self.current_hash = hash;
        self.best_hash = best;
        Ok(first_chunk)
    }
}

/// A minimal growable list of ints (Java's `IntList`, to avoid boxing on the chunking path);
/// indices are Java `int`s, and an index out of range panics as Java's array access throws.
// Java: com/google/devtools/build/lib/remote/chunking/RepMaxCdcChunker.java:478 RepMaxCdcChunker.IntList
#[derive(Default)]
struct IntList {
    elements: Vec<i32>,
}

impl IntList {
    fn size(&self) -> i32 {
        self.elements.len() as i32
    }

    fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    fn get(&self, index: i32) -> i32 {
        self.elements[usize::try_from(index).expect("ArrayIndexOutOfBoundsException")]
    }

    fn set(&mut self, index: i32, value: i32) {
        self.elements[usize::try_from(index).expect("ArrayIndexOutOfBoundsException")] = value;
    }

    fn last(&self) -> i32 {
        *self
            .elements
            .last()
            .expect("ArrayIndexOutOfBoundsException")
    }

    fn add(&mut self, value: i32) {
        self.elements.push(value);
    }

    fn pop_last(&mut self) -> i32 {
        self.elements
            .pop()
            .expect("ArrayIndexOutOfBoundsException")
    }

    fn clear(&mut self) {
        self.elements.clear();
    }

    fn truncate(&mut self, new_size: i32) {
        self.elements.truncate(new_size as usize);
    }

    /// Removes the first `count` elements, shifting the rest to the front.
    fn remove_prefix(&mut self, count: i32) {
        self.elements.drain(..count as usize);
    }

    /// Removes the elements in `[from, to)`, shifting later elements down.
    fn remove_range(&mut self, from: i32, to: i32) {
        self.elements.drain(from as usize..to as usize);
    }

    /// Reverses the elements in `[from, to)`.
    fn reverse(&mut self, from: i32, to: i32) {
        if from < to {
            self.elements[from as usize..to as usize].reverse();
        }
    }
}
