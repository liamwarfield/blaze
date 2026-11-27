//! Java `InputStream` semantics for the download pipeline, and the filtering streams
//! `HashInputStream`, `CheckContentLengthInputStream`, `RetryingInputStream`,
//! `ProgressInputStream` and `HashOutputStream`.
//!
//! PORT: Java streams check checksums and lengths both at end-of-stream and in `close()`, and
//! `close()` posts progress events, so the Rust pipeline uses an explicit [`InputStream`] trait
//! with `close` instead of `std::io::Read` + `Drop`.

use std::io::{self, Read, Write};
use std::sync::Arc;

use blaze_events::Event;
use blaze_events::handler::ExtendedEventHandler;
use blaze_events::ui::Clock;

use super::checksum::Checksum;
use super::download_progress_event::{DownloadProgressEvent, format_grouped};
use super::http_connector::UrlConnection;
use crate::cache::download_cache::KeyHasher;
use crate::errors::{IoException, IoExceptionKind};
use crate::uri::JavaUri;

/// A Java `InputStream`: `read` returns 0 only at end of stream (for a non-empty buffer).
pub trait InputStream: Send {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoException>;
    fn close(&mut self) -> Result<(), IoException>;
}

/// Reads to end of stream (`InputStream.transferTo`).
pub fn transfer_to(input: &mut dyn InputStream, out: &mut dyn Write) -> Result<u64, IoException> {
    let mut buf = vec![0u8; 8192];
    let mut total = 0u64;
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            return Ok(total);
        }
        out.write_all(&buf[..n])?;
        total += n as u64;
    }
}

/// An [`InputStream`] over a `std::io::Read` (closing drops nothing; the reader is dropped with
/// the stream).
pub struct ReadInputStream<R: Read + Send> {
    inner: Option<R>,
}

impl<R: Read + Send> ReadInputStream<R> {
    pub fn new(inner: R) -> Self {
        ReadInputStream { inner: Some(inner) }
    }
}

impl<R: Read + Send> InputStream for ReadInputStream<R> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoException> {
        match &mut self.inner {
            Some(r) => loop {
                match r.read(buf) {
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    other => return Ok(other?),
                }
            },
            None => Err(IoException::new("Stream closed")),
        }
    }

    fn close(&mut self) -> Result<(), IoException> {
        self.inner = None;
        Ok(())
    }
}

/// `std::io::Read` over an [`InputStream`] (errors are tunnelled, see `IoException::into_io_error`).
pub struct AsRead(pub Box<dyn InputStream>);

impl Read for AsRead {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf).map_err(IoException::into_io_error)
    }
}

/// `GZIPInputStream` (concatenated members are decoded, as in Java).
pub struct GzipInputStream {
    decoder: flate2::read::MultiGzDecoder<AsRead>,
    /// Data decoded eagerly by the constructor (Java's constructor reads the header).
    pending: Vec<u8>,
    pending_pos: usize,
}

impl GzipInputStream {
    /// Like Java's `GZIPInputStream` constructor, reads the gzip header eagerly: input that is
    /// not gzip fails here with `ZipException: Not in GZIP format`. On failure the wrapped
    /// stream is handed back so the caller can close it.
    pub fn new(
        mut inner: Box<dyn InputStream>,
    ) -> Result<Self, (IoException, Box<dyn InputStream>)> {
        // GZIPInputStream.readHeader: check the magic number first.
        let mut magic = [0u8; 2];
        let mut got = 0;
        while got < 2 {
            match inner.read(&mut magic[got..]) {
                Ok(0) => break,
                Ok(n) => got += n,
                Err(e) => return Err((e, inner)),
            }
        }
        if got < 2 {
            return Err((
                IoException::without_message(IoExceptionKind::EOFException),
                inner,
            ));
        }
        if magic != [0x1f, 0x8b] {
            return Err((
                IoException::of(IoExceptionKind::ZipException, "Not in GZIP format"),
                inner,
            ));
        }
        let inner: Box<dyn InputStream> =
            Box::new(PrefixedInputStream::new(magic.to_vec(), Some(inner)));
        let mut decoder = flate2::read::MultiGzDecoder::new(AsRead(inner));
        let mut pending = vec![0u8; 1];
        match decoder.read(&mut pending) {
            Ok(n) => {
                pending.truncate(n);
                Ok(GzipInputStream {
                    decoder,
                    pending,
                    pending_pos: 0,
                })
            }
            Err(e) => {
                let ex = if e.get_ref().is_some_and(|i| i.is::<IoException>()) {
                    IoException::from(e)
                } else if e.kind() == io::ErrorKind::UnexpectedEof {
                    IoException::without_message(IoExceptionKind::EOFException)
                } else {
                    IoException::of(IoExceptionKind::ZipException, e.to_string())
                };
                Err((ex, decoder.into_inner().0))
            }
        }
    }
}

impl InputStream for GzipInputStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoException> {
        if self.pending_pos < self.pending.len() && !buf.is_empty() {
            let n = buf.len().min(self.pending.len() - self.pending_pos);
            buf[..n].copy_from_slice(&self.pending[self.pending_pos..self.pending_pos + n]);
            self.pending_pos += n;
            return Ok(n);
        }
        Ok(self.decoder.read(buf)?)
    }

    fn close(&mut self) -> Result<(), IoException> {
        self.decoder.get_mut().0.close()
    }
}

/// `ByteStreams.limit(new ByteArrayInputStream(buffer), n)` followed by an optional rest
/// (`SequenceInputStream`).
pub struct PrefixedInputStream {
    buffer: Vec<u8>,
    pos: usize,
    rest: Option<Box<dyn InputStream>>,
}

impl PrefixedInputStream {
    pub fn new(buffer: Vec<u8>, rest: Option<Box<dyn InputStream>>) -> Self {
        PrefixedInputStream {
            buffer,
            pos: 0,
            rest,
        }
    }
}

impl InputStream for PrefixedInputStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoException> {
        if self.pos < self.buffer.len() {
            let n = buf.len().min(self.buffer.len() - self.pos);
            buf[..n].copy_from_slice(&self.buffer[self.pos..self.pos + n]);
            self.pos += n;
            return Ok(n);
        }
        match &mut self.rest {
            Some(r) => r.read(buf),
            None => Ok(0),
        }
    }

    fn close(&mut self) -> Result<(), IoException> {
        match &mut self.rest {
            Some(r) => r.close(),
            None => Ok(()),
        }
    }
}

/// Input stream that guarantees its contents matches a hash code.
// PORT: an UnrecoverableHttpException is an IoException of kind UnrecoverableHttpException (errors.rs).
// Java: com/google/devtools/build/lib/bazel/repository/downloader/UnrecoverableHttpException.java:21 UnrecoverableHttpException.<init>
/// Input stream that interrupts I/O when the thread is interrupted (Java
/// `InterruptibleInputStream`, which hedges against JVMs that do not do it).
// Java: com/google/devtools/build/lib/bazel/repository/downloader/InterruptibleInputStream.java:30 InterruptibleInputStream
// PORT: skipped, java.io.InputStream methods the download pipeline never calls (the trait has `read` and `close` only): InterruptibleInputStream.available, InterruptibleInputStream.markSupported, InterruptibleInputStream.mark, InterruptibleInputStream.reset
pub struct InterruptibleInputStream {
    delegate: Box<dyn InputStream>,
}

impl InterruptibleInputStream {
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/InterruptibleInputStream.java:34 InterruptibleInputStream.<init>
    pub fn new(delegate: Box<dyn InputStream>) -> Self {
        InterruptibleInputStream { delegate }
    }

    /// `Thread.interrupted()` (which clears the status) then `new InterruptedIOException()`.
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/InterruptibleInputStream.java:83 InterruptibleInputStream.check
    fn check() -> Result<(), IoException> {
        if blaze_shell::thread_interrupt::interrupted() {
            return Err(IoException::without_message(
                IoExceptionKind::InterruptedIOException,
            ));
        }
        Ok(())
    }
}

impl InputStream for InterruptibleInputStream {
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/InterruptibleInputStream.java:38 InterruptibleInputStream.read
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/InterruptibleInputStream.java:44 InterruptibleInputStream.read
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/InterruptibleInputStream.java:50 InterruptibleInputStream.read
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoException> {
        Self::check()?;
        self.delegate.read(buf)
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/InterruptibleInputStream.java:78 InterruptibleInputStream.close
    fn close(&mut self) -> Result<(), IoException> {
        self.delegate.close()
    }
}

// Java: com/google/devtools/build/lib/bazel/repository/downloader/HashInputStream.java:34 HashInputStream
pub struct HashInputStream {
    delegate: Box<dyn InputStream>,
    hasher: Option<KeyHasher>,
    checksum: Checksum,
    actual: Option<Vec<u8>>,
}

impl HashInputStream {
    pub fn new(delegate: Box<dyn InputStream>, checksum: Checksum) -> Self {
        HashInputStream {
            delegate,
            hasher: Some(checksum.get_key_type().new_hasher()),
            checksum,
            actual: None,
        }
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/HashInputStream.java:81 HashInputStream.check
    fn check(&mut self) -> Result<(), IoException> {
        if self.actual.is_none() {
            self.actual = Some(self.hasher.take().unwrap().hash());
        }
        let actual = self.actual.as_ref().unwrap();
        if self.checksum.get_hash_code() != actual.as_slice() {
            return Err(IoException::of(
                IoExceptionKind::UnrecoverableHttpException,
                format!(
                    "Checksum was {} but wanted {}",
                    self.checksum.emit_other_hash_in_same_format(actual),
                    self.checksum
                        .emit_other_hash_in_same_format(self.checksum.get_hash_code())
                ),
            ));
        }
        Ok(())
    }
}

// PORT: skipped, a java.io stream method the download pipeline never calls (the trait has `read` and `close` only): HashInputStream.available
impl InputStream for HashInputStream {
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/HashInputStream.java:49 HashInputStream.read
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/HashInputStream.java:60 HashInputStream.read
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoException> {
        let amount = self.delegate.read(buf)?;
        if amount == 0 && !buf.is_empty() {
            self.check()?;
        } else if let Some(h) = &mut self.hasher {
            h.put_bytes(&buf[..amount]);
        }
        Ok(amount)
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/HashInputStream.java:76 HashInputStream.close
    fn close(&mut self) -> Result<(), IoException> {
        self.delegate.close()?;
        self.check()
    }
}

/// Input stream that guarantees its contents match a size.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/CheckContentLengthInputStream.java:27 CheckContentLengthInputStream
pub struct CheckContentLengthInputStream {
    delegate: Box<dyn InputStream>,
    expected_size: i64,
    actual_size: i64,
}

/// Whether `e` is the transport's report of a response body that ended before its
/// `Content-Length` (`io::ErrorKind::UnexpectedEof`, which `From<io::Error>` turns into
/// [`IoExceptionKind::EOFException`]).
///
/// PORT: no JDK stream below `CheckContentLengthInputStream` throws `EOFException` (a short
/// fixed-length body ends with -1), so the kind identifies the transport's report.
fn is_short_body(e: &IoException) -> bool {
    e.kind == IoExceptionKind::EOFException
}

impl CheckContentLengthInputStream {
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/CheckContentLengthInputStream.java:31 CheckContentLengthInputStream.<init>
    pub fn new(delegate: Box<dyn InputStream>, expected_size: i64) -> Self {
        CheckContentLengthInputStream {
            delegate,
            expected_size,
            actual_size: 0,
        }
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/CheckContentLengthInputStream.java:69 CheckContentLengthInputStream.checkContentLength
    fn check_content_length(&self) -> Result<(), IoException> {
        if self.actual_size != self.expected_size {
            // PORT: skipped, the sizes are in the message; no caller reads them: ContentLengthMismatchException.getActualSize, ContentLengthMismatchException.getExpectedSize
            // Java: com/google/devtools/build/lib/bazel/repository/downloader/ContentLengthMismatchException.java:25 ContentLengthMismatchException.<init>
            return Err(IoException::of(
                IoExceptionKind::ContentLengthMismatchException,
                format!(
                    "Bytes read {} but wanted {}",
                    self.actual_size, self.expected_size
                ),
            ));
        }
        Ok(())
    }
}

// PORT: skipped, a java.io stream method the download pipeline never calls (the trait has `read` and `close` only): CheckContentLengthInputStream.available
impl InputStream for CheckContentLengthInputStream {
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/CheckContentLengthInputStream.java:48 CheckContentLengthInputStream.read
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/CheckContentLengthInputStream.java:36 CheckContentLengthInputStream.read
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoException> {
        let amount = match self.delegate.read(buf) {
            Ok(amount) => amount,
            // PORT: a body that ends before its Content-Length is a normal end of stream for
            // HttpURLConnection (`sun.net.www.MeteredStream.read` hands on the socket's -1), so
            // Java reaches `checkContentLength()` here and throws
            // ContentLengthMismatchException with the bytes read so far, which
            // `DownloadManager.shouldRetryDownload` retries. The default transport
            // (`socket_transport`) ends the stream the same way; a transport that reports the
            // condition as an `UnexpectedEof` error instead is treated as the end of stream.
            Err(e) if is_short_body(&e) => 0,
            Err(e) => return Err(e),
        };
        if amount == 0 && !buf.is_empty() {
            self.check_content_length()?;
        } else {
            self.actual_size += amount as i64;
        }
        Ok(amount)
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/CheckContentLengthInputStream.java:63 CheckContentLengthInputStream.close
    fn close(&mut self) -> Result<(), IoException> {
        self.delegate.close()?;
        self.check_content_length()
    }
}

/// Lambda for establishing a connection with the same parameters as initially.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/RetryingInputStream.java:42 RetryingInputStream.Reconnector.connect
pub type Reconnector = Arc<
    dyn Fn(&IoException, Vec<(String, Vec<String>)>) -> Result<UrlConnection, IoException>
        + Send
        + Sync,
>;

const MAX_RESUMES: i32 = 3;

/// Input stream that reconnects on read timeouts and errors.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/RetryingInputStream.java:34 RetryingInputStream
pub struct RetryingInputStream {
    delegate: Box<dyn InputStream>,
    reconnector: Reconnector,
    toto: i64,
    resumes: i32,
    suppressed: Vec<IoException>,
    /// See [`RetryingInputStream::with_fixed_length_body`].
    fixed_length_body: bool,
}

impl RetryingInputStream {
    pub fn new(delegate: Box<dyn InputStream>, reconnector: Reconnector) -> Self {
        RetryingInputStream {
            delegate,
            reconnector,
            toto: 0,
            resumes: 0,
            suppressed: Vec::new(),
            fixed_length_body: false,
        }
    }

    /// Marks the response as having a `Content-Length`, for a stream that is wrapped in a
    /// [`CheckContentLengthInputStream`].
    ///
    /// PORT: with HttpURLConnection a fixed-length body that ends early is a normal end of
    /// stream, so Java's `RetryingInputStream` sees no exception and does not reconnect; the
    /// `CheckContentLengthInputStream` above it throws. The Rust transport reports the short
    /// body as an error (see `is_short_body`), which this stream then hands on unchanged instead
    /// of resuming with a `Range` request.
    pub fn with_fixed_length_body(mut self) -> Self {
        self.fixed_length_body = true;
        self
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/RetryingInputStream.java:139 RetryingInputStream.propagate
    fn propagate(&self, mut error: IoException) -> IoException {
        for e in &self.suppressed {
            error.add_suppressed(e.clone());
        }
        error
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/RetryingInputStream.java:94 RetryingInputStream.tryAgainIfPossible
    fn try_again_if_possible(&mut self, cause: IoException) -> Result<(), IoException> {
        if cause.kind == IoExceptionKind::InterruptedIOException {
            return Err(cause);
        }
        if self.resumes >= MAX_RESUMES {
            return Err(self.propagate(cause));
        }
        self.resumes += 1;
        // We know this connection failed so if it reminds us we're going to ignore it.
        let _ = self.delegate.close();
        self.suppressed.push(cause.clone());
        self.reconnect_where_we_left_off(&cause)
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/RetryingInputStream.java:110 RetryingInputStream.reconnectWhereWeLeftOff
    fn reconnect_where_we_left_off(&mut self, cause: &IoException) -> Result<(), IoException> {
        let amount_read = self.toto;
        let result = (|| {
            let mut connection = if amount_read == 0 {
                (self.reconnector)(cause, Vec::new())?
            } else {
                let connection = (self.reconnector)(
                    cause,
                    vec![("Range".to_string(), vec![format!("bytes={}-", amount_read)])],
                )?;
                if !connection
                    .get_header_field("Content-Range")
                    .unwrap_or_default()
                    .starts_with(&format!("bytes {}-", amount_read))
                {
                    return Err(IoException::new(format!(
                        "Tried to reconnect at offset {} but server didn't support it",
                        format_grouped(amount_read)
                    )));
                }
                connection
            };
            connection.get_input_stream()
        })();
        match result {
            Ok(s) => {
                self.delegate = s;
                Ok(())
            }
            Err(e) if e.kind == IoExceptionKind::InterruptedIOException => Err(e),
            Err(e) => Err(self.propagate(e)),
        }
    }
}

// PORT: skipped, a java.io stream method the download pipeline never calls (the trait has `read` and `close` only): RetryingInputStream.available
impl InputStream for RetryingInputStream {
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/RetryingInputStream.java:57 RetryingInputStream.read
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/RetryingInputStream.java:72 RetryingInputStream.read
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoException> {
        loop {
            match self.delegate.read(buf) {
                Ok(amount) => {
                    self.toto += amount as i64;
                    return Ok(amount);
                }
                // PORT: see `with_fixed_length_body`.
                Err(e) if self.fixed_length_body && is_short_body(&e) => return Err(e),
                Err(e) => self.try_again_if_possible(e)?,
            }
        }
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/RetryingInputStream.java:92 RetryingInputStream.close
    fn close(&mut self) -> Result<(), IoException> {
        self.delegate.close()
    }
}

const PROGRESS_INTERVAL_MS: i64 = 200;

/// Input stream that reports progress on total bytes read as the download progresses.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/ProgressInputStream.java:36 ProgressInputStream
pub struct ProgressInputStream {
    clock: Arc<dyn Clock>,
    event_handler: Arc<dyn ExtendedEventHandler>,
    delegate: Box<dyn InputStream>,
    interval_ms: i64,
    url: JavaUri,
    original_url: JavaUri,
    total_bytes: Option<i64>,
    toto: i64,
    next_event: i64,
}

impl ProgressInputStream {
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/ProgressInputStream.java:83 ProgressInputStream.<init>
    pub fn new(
        clock: Arc<dyn Clock>,
        event_handler: Arc<dyn ExtendedEventHandler>,
        interval_ms: i64,
        delegate: Box<dyn InputStream>,
        url: JavaUri,
        original_url: JavaUri,
        total_bytes: Option<i64>,
    ) -> Self {
        assert!(interval_ms >= 0, "IllegalArgumentException");
        let next_event = clock.current_time_millis() + interval_ms;
        event_handler.post(Arc::new(DownloadProgressEvent::new(
            original_url.clone(),
            Some(url.clone()),
            0,
            total_bytes,
            false,
        )));
        ProgressInputStream {
            clock,
            event_handler,
            delegate,
            interval_ms,
            url,
            original_url,
            total_bytes,
            toto: 0,
            next_event,
        }
    }

    /// `ProgressInputStream.Factory.create` (interval 200 ms).
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/ProgressInputStream.java:48 ProgressInputStream.Factory.<init>
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/ProgressInputStream.java:54 ProgressInputStream.Factory.create
    pub fn create(
        clock: Arc<dyn Clock>,
        event_handler: Arc<dyn ExtendedEventHandler>,
        delegate: Box<dyn InputStream>,
        url: JavaUri,
        original_url: JavaUri,
        total_bytes: Option<i64>,
    ) -> Self {
        Self::new(
            clock,
            event_handler,
            PROGRESS_INTERVAL_MS,
            delegate,
            url,
            original_url,
            total_bytes,
        )
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/ProgressInputStream.java:130 ProgressInputStream.reportProgress
    fn report_progress(&mut self, bytes_read: i64) {
        let now = self.clock.current_time_millis();
        if now < self.next_event {
            return;
        }
        let mut via = String::new();
        if let Some(h) = self.url.get_host()
            && Some(h) != self.original_url.get_host()
        {
            via = format!(" via {}", h);
        }
        self.event_handler.post(Arc::new(DownloadProgressEvent::new(
            self.original_url.clone(),
            Some(self.url.clone()),
            bytes_read,
            self.total_bytes,
            false,
        )));
        self.event_handler.handle(&Event::progress(format!(
            "Downloading {}{}: {} bytes",
            self.original_url,
            via,
            format_grouped(bytes_read)
        )));
        self.next_event = now + self.interval_ms;
    }
}

// PORT: skipped, a java.io stream method the download pipeline never calls (the trait has `read` and `close` only): ProgressInputStream.available
impl InputStream for ProgressInputStream {
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/ProgressInputStream.java:104 ProgressInputStream.read
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/ProgressInputStream.java:113 ProgressInputStream.read
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoException> {
        let amount = self.delegate.read(buf)?;
        if amount > 0 {
            self.toto += amount as i64;
            let t = self.toto;
            self.report_progress(t);
        }
        Ok(amount)
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/ProgressInputStream.java:127 ProgressInputStream.close
    fn close(&mut self) -> Result<(), IoException> {
        self.delegate.close()?;
        self.event_handler.post(Arc::new(DownloadProgressEvent::new(
            self.original_url.clone(),
            Some(self.url.clone()),
            self.toto,
            self.total_bytes,
            true,
        )));
        Ok(())
    }
}

/// Output stream that guarantees its contents matches a hash code (checked on `close`).
// Java: com/google/devtools/build/lib/bazel/repository/downloader/HashOutputStream.java:37 HashOutputStream
pub struct HashOutputStream<W: Write> {
    delegate: W,
    hasher: Option<KeyHasher>,
    checksum: Checksum,
    actual: Option<Vec<u8>>,
}

impl<W: Write> HashOutputStream<W> {
    pub fn new(delegate: W, checksum: Checksum) -> Self {
        HashOutputStream {
            delegate,
            hasher: Some(checksum.get_key_type().new_hasher()),
            checksum,
            actual: None,
        }
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/HashOutputStream.java:79 HashOutputStream.close
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/HashOutputStream.java:80 HashOutputStream.check
    pub fn close(&mut self) -> Result<(), IoException> {
        self.delegate.flush()?;
        if self.actual.is_none() {
            self.actual = Some(self.hasher.take().unwrap().hash());
        }
        let actual = self.actual.as_ref().unwrap();
        if self.checksum.get_hash_code() != actual.as_slice() {
            return Err(IoException::of(
                IoExceptionKind::UnrecoverableHttpException,
                format!(
                    "Checksum was {} but wanted {}",
                    self.checksum.emit_other_hash_in_same_format(actual),
                    self.checksum
                        .emit_other_hash_in_same_format(self.checksum.get_hash_code())
                ),
            ));
        }
        Ok(())
    }
}

impl<W: Write> Write for HashOutputStream<W> {
    // PORT: one `write` for Java's three overloads (byte, array, slice).
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/HashOutputStream.java:51 HashOutputStream.write
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/HashOutputStream.java:57 HashOutputStream.write
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/HashOutputStream.java:63 HashOutputStream.write
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.delegate.write(buf)?;
        if let Some(h) = &mut self.hasher {
            h.put_bytes(&buf[..n]);
        }
        Ok(n)
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/HashOutputStream.java:69 HashOutputStream.flush
    fn flush(&mut self) -> io::Result<()> {
        self.delegate.flush()
    }
}
