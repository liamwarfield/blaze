//! Port of `runtime/SynchronizedOutputStream.java`: the command's stdout or stderr as the build
//! event streamer collects it into progress events.

use std::io;
use std::sync::{Arc, OnceLock};

use blaze_events::OutputStream;
use blaze_util::check_argument;
use parking_lot::Mutex;

use crate::runtime::build_event_streamer::BuildEventStreamer;

/// `OutputStream` suitably synchronized for producer-consumer use cases. The method
/// [`SynchronizedOutputStream::read_and_reset`] allows to read the bytes accumulated so far and
/// simultaneously truncate precisely the bytes read. Moreover, upon such a reset the amount of
/// memory retained is reset to a small constant.
///
/// This class is expected to be used with the `BuildEventStreamer`.
// Java: com/google/devtools/build/lib/runtime/SynchronizedOutputStream.java:38 SynchronizedOutputStream
pub struct SynchronizedOutputStream {
    /// The maximal amount of bytes we intend to store in the buffer. However, the requirement
    /// that a single write be written in one go is more important, so the actual size we store
    /// in this buffer can be the maximum (not the sum) of this value and the amount of bytes
    /// written in a single call to the `write` method.
    max_buffered_length: i32,
    /// `Splitter.fixedLength(Math.max(maxChunkSize, maxBufferedLength))`.
    max_chunk_size: usize,
    is_stderr: bool,
    buf: Mutex<Vec<u8>>,
    /// The event streamer that is supposed to flush stdout/stderr.
    streamer: OnceLock<Arc<BuildEventStreamer>>,
}

impl SynchronizedOutputStream {
    // Java: com/google/devtools/build/lib/runtime/SynchronizedOutputStream.java:58 SynchronizedOutputStream.<init>
    pub fn new(max_buffered_length: i32, max_chunk_size: i32, is_stderr: bool) -> Self {
        check_argument!(max_chunk_size > 0);
        SynchronizedOutputStream {
            max_buffered_length,
            max_chunk_size: max_chunk_size.max(max_buffered_length) as usize,
            is_stderr,
            buf: Mutex::new(Vec::with_capacity(64)),
            streamer: OnceLock::new(),
        }
    }

    // Java: com/google/devtools/build/lib/runtime/SynchronizedOutputStream.java:67 SynchronizedOutputStream.registerStreamer
    pub fn register_streamer(&self, streamer: Arc<BuildEventStreamer>) {
        let _ = self.streamer.set(streamer);
    }

    /// Read the contents of the stream and simultaneously clear them. Also, reset the amount of
    /// memory retained to a constant amount.
    // Java: com/google/devtools/build/lib/runtime/SynchronizedOutputStream.java:75 SynchronizedOutputStream.readAndReset
    pub fn read_and_reset(&self) -> Vec<String> {
        let bytes = std::mem::replace(&mut *self.buf.lock(), Vec::with_capacity(64));
        // `new String(buf, 0, count, UTF_8)`: Java's decoder replaces each malformed sequence
        // with U+FFFD, as `from_utf8_lossy` does.
        let content = String::from_utf8_lossy(&bytes).into_owned();
        if content.is_empty() {
            return Vec::new();
        }
        // maxChunkSizeSplitter.split(content): pieces of maxChunkSize chars, the last shorter.
        // PORT: Java counts UTF-16 code units; a character outside the Basic Multilingual Plane
        // counts once here, and a chunk boundary never splits it.
        let chars: Vec<char> = content.chars().collect();
        chars
            .chunks(self.max_chunk_size)
            .map(|chunk| chunk.iter().collect())
            .collect()
    }
}

impl OutputStream for SynchronizedOutputStream {
    // Java: com/google/devtools/build/lib/runtime/SynchronizedOutputStream.java:91 SynchronizedOutputStream.write
    fn write(&self, buffer: &[u8]) -> io::Result<()> {
        let count = buffer.len() as i64;
        let mut should_flush = false;
        // As we have to do the flushing outside the synchronized block, we have to expect other
        // writes to come immediately after flushing, so we have to do the check inside a while
        // loop.
        let mut did_write = false;
        while !did_write {
            {
                let mut buf = self.buf.lock();
                let this_count = buf.len() as i64;
                let streamer = self
                    .streamer
                    .get()
                    .unwrap_or_else(|| panic!("NullPointerException: streamer"));
                if (this_count + count < self.max_buffered_length as i64 || this_count == 0)
                    && streamer.can_buffer_progress_write(self.is_stderr)
                {
                    buf.extend_from_slice(buffer);
                    did_write = true;
                } else {
                    should_flush = true;
                }
                if buf.len() as i64 >= self.max_buffered_length as i64 {
                    should_flush = true;
                }
            }
            if should_flush && let Some(streamer) = self.streamer.get() {
                streamer.flush();
                should_flush = false;
            }
        }
        Ok(())
    }
}
