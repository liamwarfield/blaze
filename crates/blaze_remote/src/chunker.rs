//! Port of `lib/remote/Chunker.java`.

use std::cmp::min;
use std::io::{self, Read};
use std::sync::atomic::{AtomicI32, Ordering};

use bytes::Bytes;

use crate::common::remote_cache_client::Blob;
use crate::zstd::ZstdCompressingInputStream;

// Java: com/google/devtools/build/lib/remote/Chunker.java:48 Chunker.defaultChunkSize
static DEFAULT_CHUNK_SIZE: AtomicI32 = AtomicI32::new(1024 * 16);

/// A piece of a byte[] blob.
// Java: com/google/devtools/build/lib/remote/Chunker.java:65 Chunker.Chunk
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Chunk {
    offset: i64,
    data: Bytes,
}

impl Chunk {
    // Java: com/google/devtools/build/lib/remote/Chunker.java:70 Chunker.Chunk.<init>
    fn new(data: Bytes, offset: i64) -> Chunk {
        Chunk { data, offset }
    }

    // Java: com/google/devtools/build/lib/remote/Chunker.java:75 Chunker.Chunk.getOffset
    pub fn get_offset(&self) -> i64 {
        self.offset
    }

    // Java: com/google/devtools/build/lib/remote/Chunker.java:79 Chunker.Chunk.getData
    pub fn get_data(&self) -> &Bytes {
        &self.data
    }
}

/// The `Blob` a [`Chunker`] reads.
///
/// PORT: a Java `Chunker` owns its `Blob`; the Rust `Blob::get` returns a stream that borrows the
/// blob, so the chunker borrows the blob from its caller (as `RemoteCacheClient::upload_blob_impl`
/// receives it). The blob of `Builder.setInput(byte[])` is the bytes themselves.
enum ChunkerBlob<'a> {
    Blob(&'a dyn Blob),
    /// `() -> new ByteArrayInputStream(data)`.
    Bytes(Bytes),
}

impl<'a> ChunkerBlob<'a> {
    /// `blob.get()`.
    fn get(&self) -> io::Result<Box<dyn Read + Send + 'a>> {
        match self {
            ChunkerBlob::Blob(blob) => {
                let blob: &'a dyn Blob = *blob;
                blob.get()
            }
            ChunkerBlob::Bytes(data) => Ok(Box::new(io::Cursor::new(data.clone()))),
        }
    }
}

/// `ByteStreams.skipFully(in, n)`: discards `n` bytes of data from the input stream, failing
/// with an `EOFException` if the stream ends first.
// Java: com/google/common/io/ByteStreams.java ByteStreams.skipFully
fn skip_fully(in_: &mut dyn Read, n: i64) -> io::Result<()> {
    // ByteStreams.skipUpTo
    let mut total_skipped: i64 = 0;
    let mut buf: Vec<u8> = Vec::new();
    while total_skipped < n {
        let remaining = n - total_skipped;
        // Do a buffered read (BUFFER_SIZE is 8192).
        let skip = min(remaining, 8192) as usize;
        if buf.len() < skip {
            buf.resize(skip, 0);
        }
        let skipped = in_.read(&mut buf[..skip])?;
        if skipped == 0 {
            // Reached EOF
            break;
        }
        total_skipped += skipped as i64;
    }
    if total_skipped < n {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!(
                "reached end of stream after skipping {total_skipped} bytes; {n} bytes expected"
            ),
        ));
    }
    Ok(())
}

/// `Chunker.ChunkerInputStream`: a `PushbackInputStream` (with its pushback buffer of one byte)
/// that can tell whether it is at the end of the stream.
///
/// PORT: `close()` is dropping the stream, which drops (closes) the wrapped one.
// Java: com/google/devtools/build/lib/remote/Chunker.java:332 Chunker.ChunkerInputStream
struct ChunkerInputStream<'a> {
    in_: Box<dyn Read + Send + 'a>,
    /// The pushed back byte, if any.
    buf: Option<u8>,
}

impl<'a> ChunkerInputStream<'a> {
    // Java: com/google/devtools/build/lib/remote/Chunker.java:333 Chunker.ChunkerInputStream.<init>
    fn new(in_: Box<dyn Read + Send + 'a>) -> ChunkerInputStream<'a> {
        ChunkerInputStream { in_, buf: None }
    }

    // Java: com/google/devtools/build/lib/remote/Chunker.java:337 Chunker.ChunkerInputStream.finished
    fn finished(&mut self) -> io::Result<bool> {
        // int c = super.read();
        let c = match self.buf.take() {
            Some(c) => Some(c),
            None => {
                let mut one = [0u8; 1];
                match self.in_.read(&mut one)? {
                    0 => None,
                    _ => Some(one[0]),
                }
            }
        };
        match c {
            None => Ok(true),
            Some(c) => {
                // super.unread(c);
                self.buf = Some(c);
                Ok(false)
            }
        }
    }
}

impl Read for ChunkerInputStream<'_> {
    /// `PushbackInputStream.read(b, off, len)`: the pushed back byte first, then the stream.
    // Java: java/io/PushbackInputStream.java PushbackInputStream.read
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        if b.is_empty() {
            return Ok(0);
        }
        let mut avail = 0;
        if let Some(c) = self.buf.take() {
            b[0] = c;
            avail = 1;
        }
        if b.len() > avail {
            let len = self.in_.read(&mut b[avail..])?;
            // `len == -1` (end of stream): `avail == 0 ? -1 : avail`.
            return Ok(avail + len);
        }
        Ok(avail)
    }
}

/// `java.lang.NullPointerException`, an unchecked exception of [`Chunker::seek`] (see
/// [`Chunker::seek_or_null_pointer`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NullPointerException;

/// Splits a data source into one or more [`Chunk`]s of at most `chunk_size` bytes.
///
/// After a data source has been fully consumed, that is until [`Chunker::has_next`] returns
/// `false`, the chunker closes the underlying data source (i.e. file) itself. However, in case of
/// error or when a data source does not get fully consumed, a user must call [`Chunker::reset`]
/// manually.
// Java: com/google/devtools/build/lib/remote/Chunker.java:46 Chunker
pub struct Chunker<'a> {
    blob: ChunkerBlob<'a>,
    uncompressed_size: i64,
    chunk_size: i32,
    empty_chunk: Chunk,

    data: Option<ChunkerInputStream<'a>>,
    offset: i64,
    chunk_cache: Option<Vec<u8>>,

    compressed: bool,

    // Set to true on the first call to next(). This is so that the Chunker can open its data
    // source lazily on the first call to next(), as opposed to opening it in the constructor or on
    // reset().
    initialized: bool,
}

impl<'a> Chunker<'a> {
    /// This method must only be called in tests!
    // Java: com/google/devtools/build/lib/remote/Chunker.java:52 Chunker.setDefaultChunkSizeForTesting
    pub fn set_default_chunk_size_for_testing(value: i32) {
        DEFAULT_CHUNK_SIZE.store(value, Ordering::SeqCst);
    }

    // Java: com/google/devtools/build/lib/remote/Chunker.java:56 Chunker.getDefaultChunkSize
    pub fn get_default_chunk_size() -> i32 {
        DEFAULT_CHUNK_SIZE.load(Ordering::SeqCst)
    }

    // Java: com/google/devtools/build/lib/remote/Chunker.java:60 Chunker.isCompressed
    pub fn is_compressed(&self) -> bool {
        self.compressed
    }

    // Java: com/google/devtools/build/lib/remote/Chunker.java:115 Chunker.<init>
    pub fn new(
        blob: &'a dyn Blob,
        uncompressed_size: i64,
        chunk_size: i32,
        compressed: bool,
    ) -> Chunker<'a> {
        Chunker::from_chunker_blob(
            ChunkerBlob::Blob(blob),
            uncompressed_size,
            chunk_size,
            compressed,
        )
    }

    // Java: com/google/devtools/build/lib/remote/Chunker.java:115 Chunker.<init>
    fn from_chunker_blob(
        blob: ChunkerBlob<'a>,
        uncompressed_size: i64,
        chunk_size: i32,
        compressed: bool,
    ) -> Chunker<'a> {
        Chunker {
            blob,
            uncompressed_size,
            chunk_size,
            empty_chunk: Chunk::new(Bytes::new(), 0),
            data: None,
            offset: 0,
            chunk_cache: None,
            compressed,
            initialized: false,
        }
    }

    // Java: com/google/devtools/build/lib/remote/Chunker.java:123 Chunker.getOffset
    pub fn get_offset(&self) -> i64 {
        self.offset
    }

    // Java: com/google/devtools/build/lib/remote/Chunker.java:127 Chunker.getUncompressedSize
    pub fn get_uncompressed_size(&self) -> i64 {
        self.uncompressed_size
    }

    /// Reset the [`Chunker`] state to when it was newly constructed.
    ///
    /// Closes any open resources (file handles, ...).
    // Java: com/google/devtools/build/lib/remote/Chunker.java:136 Chunker.reset
    pub fn reset(&mut self) -> io::Result<()> {
        self.close_input()?;
        self.offset = 0;
        self.initialized = false;
        Ok(())
    }

    /// Seek to an offset in the source stream.
    ///
    /// May close and reopen resources in order to seek to an earlier offset.
    ///
    /// * `to_offset`: the offset from beginning of the source stream. If the source stream is
    ///   compressed, it refers to the offset in the uncompressed form to align with
    ///   `write_offset` in REAPI.
    // Java: com/google/devtools/build/lib/remote/Chunker.java:151 Chunker.seek
    pub fn seek(&mut self, to_offset: i64) -> io::Result<()> {
        match self.seek_or_null_pointer(to_offset) {
            Ok(result) => result,
            Err(NullPointerException) => panic!("NullPointerException"),
        }
    }

    /// [`Chunker::seek`], with the unchecked `NullPointerException` it throws when the chunker
    /// was consumed to the end (`data == null`) and is asked to stay there, as an `Err` instead
    /// of a panic, for the caller that catches it (`ByteStreamUploader.Writer.run`, whose
    /// exceptions gRPC turns into a cancelled call).
    // Java: com/google/devtools/build/lib/remote/Chunker.java:151 Chunker.seek
    pub(crate) fn seek_or_null_pointer(
        &mut self,
        to_offset: i64,
    ) -> Result<io::Result<()>, NullPointerException> {
        // For compressed stream, we need to reinitialize the stream since the offset refers to the
        // uncompressed form.
        if self.initialized
            && self.uncompressed_size > 0
            && to_offset >= self.offset
            && !self.compressed
        {
            let n = to_offset - self.offset;
            match self.data.as_mut() {
                Some(data) => {
                    if let Err(e) = skip_fully(data, n) {
                        return Ok(Err(e));
                    }
                }
                // `ByteStreams.skipFully(null, n)` only touches the stream when `n > 0`.
                None => {
                    if n > 0 {
                        return Err(NullPointerException);
                    }
                }
            }
            self.offset = to_offset;
        } else {
            if let Err(e) = self.reset() {
                return Ok(Err(e));
            }
            if let Err(e) = self.initialize(to_offset) {
                return Ok(Err(e));
            }
        }
        if self.uncompressed_size > 0 {
            // `data.finished()` dereferences `data` without a null check.
            let Some(data) = self.data.as_mut() else {
                return Err(NullPointerException);
            };
            match data.finished() {
                Ok(true) => {
                    if let Err(e) = self.close_input() {
                        return Ok(Err(e));
                    }
                }
                Ok(false) => {}
                Err(e) => return Ok(Err(e)),
            }
        }
        Ok(Ok(()))
    }

    /// `data`, which Java dereferences without a null check.
    fn data_mut(&mut self) -> &mut ChunkerInputStream<'a> {
        match self.data.as_mut() {
            Some(data) => data,
            None => panic!("NullPointerException"),
        }
    }

    /// Returns `true` if a subsequent call to [`Chunker::next`] returns a [`Chunk`] object;
    // Java: com/google/devtools/build/lib/remote/Chunker.java:169 Chunker.hasNext
    pub fn has_next(&self) -> bool {
        self.data.is_some() || !self.initialized
    }

    /// Closes the input stream and reset chunk cache
    // Java: com/google/devtools/build/lib/remote/Chunker.java:174 Chunker.closeInput
    fn close_input(&mut self) -> io::Result<()> {
        // data.close(); data = null;
        self.data = None;
        self.chunk_cache = None;
        Ok(())
    }

    // Java: com/google/devtools/build/lib/remote/Chunker.java:183 Chunker.close
    pub fn close(&mut self) -> io::Result<()> {
        self.reset()
    }

    /// Attempts reading at most a full chunk and stores it in the chunkCache buffer
    // Java: com/google/devtools/build/lib/remote/Chunker.java:188 Chunker.read
    fn read(&mut self) -> io::Result<usize> {
        let (Some(data), Some(chunk_cache)) = (self.data.as_mut(), self.chunk_cache.as_mut())
        else {
            panic!("NullPointerException");
        };
        let mut count = 0;
        while count < chunk_cache.len() {
            let c = data.read(&mut chunk_cache[count..])?;
            // `c < 0`: the end of the stream.
            if c == 0 {
                break;
            }
            count += c;
        }
        Ok(count)
    }

    /// Returns the next [`Chunk`] or panics with a `NoSuchElementException` if no data is left.
    ///
    /// Always call [`Chunker::has_next`] before calling this method.
    ///
    /// Zero byte inputs are treated special. Instead of throwing a `NoSuchElementException` on
    /// the first call to [`Chunker::next`], a [`Chunk`] with an empty `ByteString` is returned.
    // Java: com/google/devtools/build/lib/remote/Chunker.java:209 Chunker.next
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> io::Result<Chunk> {
        if !self.has_next() {
            panic!("NoSuchElementException");
        }

        self.maybe_initialize()?;

        if self.uncompressed_size == 0 {
            self.close_input()?;
            return Ok(self.empty_chunk.clone());
        }

        if self.data_mut().finished()? {
            self.chunk_cache = None;
            self.data = None;
            panic!("NoSuchElementException");
        }

        if self.chunk_cache.is_none() {
            // If the output is compressed we can't know how many bytes there are yet to read,
            // so we allocate the whole chunkSize, otherwise we try to compute the smallest
            // possible value. The cast to int is safe, because the return value is capped at
            // chunkSize.
            let cache_size: i32 = if self.compressed {
                self.chunk_size
            } else {
                min(
                    self.uncompressed_size - self.get_offset(),
                    self.chunk_size as i64,
                ) as i32
            };
            // Lazily allocate it in order to save memory on small data.
            // 1) bytesToRead < chunkSize: There will only ever be one next() call.
            // 2) bytesToRead == chunkSize: chunkCache will be set to its biggest possible value.
            // 3) bytestoRead > chunkSize: Not possible, due to Math.min above.
            if cache_size < 0 {
                panic!("NegativeArraySizeException: {cache_size}");
            }
            self.chunk_cache = Some(vec![0u8; cache_size as usize]);
        }

        let offset_before = self.offset;

        let bytes_read = self.read()?;

        let blob = match &self.chunk_cache {
            Some(chunk_cache) => Bytes::copy_from_slice(&chunk_cache[..bytes_read]),
            None => panic!("NullPointerException"),
        };

        // This has to happen after actualSize has been updated
        // or the guard in getActualSize won't work.
        self.offset += bytes_read as i64;
        if self.data_mut().finished()? {
            self.close_input()?;
        }

        Ok(Chunk::new(blob, offset_before))
    }

    // Java: com/google/devtools/build/lib/remote/Chunker.java:255 Chunker.maybeInitialize
    fn maybe_initialize(&mut self) -> io::Result<()> {
        if self.initialized {
            return Ok(());
        }
        self.initialize(0)
    }

    // Java: com/google/devtools/build/lib/remote/Chunker.java:262 Chunker.initialize
    fn initialize(&mut self, src_pos: i64) -> io::Result<()> {
        blaze_util::check_state!(!self.initialized);
        blaze_util::check_state!(self.data.is_none());
        blaze_util::check_state!(self.offset == 0);
        blaze_util::check_state!(self.chunk_cache.is_none());
        // PORT: Java unwraps an `IOException` that `blob.get()` smuggled out in a
        // `RuntimeException`; `Blob::get` returns its `io::Error` directly.
        let mut src = self.blob.get()?;
        skip_fully(&mut *src, src_pos)?;
        self.data = Some(if self.compressed {
            ChunkerInputStream::new(Box::new(ZstdCompressingInputStream::new(src)?))
        } else {
            ChunkerInputStream::new(src)
        });
        self.offset = src_pos;
        self.initialized = true;
        Ok(())
    }

    // Java: com/google/devtools/build/lib/remote/Chunker.java:285 Chunker.builder
    pub fn builder() -> ChunkerBuilder<'a> {
        ChunkerBuilder::new()
    }
}

/// Builder class for the Chunker
// Java: com/google/devtools/build/lib/remote/Chunker.java:290 Chunker.Builder
pub struct ChunkerBuilder<'a> {
    chunk_size: i32,
    size: i64,
    compressed: bool,
    input_stream: Option<ChunkerBlob<'a>>,
}

impl<'a> ChunkerBuilder<'a> {
    fn new() -> ChunkerBuilder<'a> {
        ChunkerBuilder {
            chunk_size: Chunker::get_default_chunk_size(),
            size: 0,
            compressed: false,
            input_stream: None,
        }
    }

    /// `setInput(long size, Blob in)`.
    // Java: com/google/devtools/build/lib/remote/Chunker.java:297 Chunker.Builder.setInput
    pub fn set_input(mut self, size: i64, in_: &'a dyn Blob) -> Self {
        blaze_util::check_state!(self.input_stream.is_none());
        self.size = size;
        self.input_stream = Some(ChunkerBlob::Blob(in_));
        self
    }

    /// `setInput(byte[] data)` (`@VisibleForTesting`).
    // Java: com/google/devtools/build/lib/remote/Chunker.java:307 Chunker.Builder.setInput
    pub fn set_input_bytes(mut self, data: impl Into<Bytes>) -> Self {
        blaze_util::check_state!(self.input_stream.is_none());
        let data: Bytes = data.into();
        self.size = data.len() as i64;
        self.input_stream = Some(ChunkerBlob::Bytes(data));
        self
    }

    // Java: com/google/devtools/build/lib/remote/Chunker.java:315 Chunker.Builder.setCompressed
    pub fn set_compressed(mut self, compressed: bool) -> Self {
        self.compressed = compressed;
        self
    }

    // Java: com/google/devtools/build/lib/remote/Chunker.java:321 Chunker.Builder.setChunkSize
    pub fn set_chunk_size(mut self, chunk_size: i32) -> Self {
        self.chunk_size = chunk_size;
        self
    }

    // Java: com/google/devtools/build/lib/remote/Chunker.java:326 Chunker.Builder.build
    pub fn build(self) -> Chunker<'a> {
        let Some(input_stream) = self.input_stream else {
            panic!("NullPointerException");
        };
        Chunker::from_chunker_blob(input_stream, self.size, self.chunk_size, self.compressed)
    }
}
