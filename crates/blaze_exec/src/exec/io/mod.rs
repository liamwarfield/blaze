//! Port of the message streams of `lib/util/io` (`MessageOutputStream`, `MessageInputStream`,
//! `MessageOutputStreamWrapper`, `MessageInputStreamWrapper`, `AsynchronousMessageOutputStream`),
//! as used by the execution log.

pub mod asynchronous_message_output_stream;
pub mod message_input_stream_wrapper;
pub mod message_output_stream_wrapper;

use std::io::{self, Write};

pub use asynchronous_message_output_stream::AsynchronousMessageOutputStream;
pub use message_input_stream_wrapper::{
    BinaryInputStreamWrapper, JsonInputStreamWrapper, parse_delimited_from,
};
pub use message_output_stream_wrapper::{BinaryOutputStreamWrapper, JsonOutputStreamWrapper};

/// A variation of `OutputStream` for protobuf messages.
// Java: com/google/devtools/build/lib/util/io/MessageOutputStream.java:21 MessageOutputStream
pub trait MessageOutputStream<T>: Send {
    /// Writes a protobuf message to the underlying stream.
    // Java: com/google/devtools/build/lib/util/io/MessageOutputStream.java:23 MessageOutputStream.write
    fn write(&mut self, m: &T) -> io::Result<()>;

    /// Closes the underlying stream. Any following writes will fail.
    // Java: com/google/devtools/build/lib/util/io/MessageOutputStream.java:27 MessageOutputStream.close
    fn close(&mut self) -> io::Result<()>;
}

/// A variation of `InputStream` for protobuf messages.
// Java: com/google/devtools/build/lib/util/io/MessageInputStream.java:22 MessageInputStream
pub trait MessageInputStream<T>: Send {
    /// Reads a protobuf message from the underlying stream, or `None` if there are no more
    /// messages.
    // Java: com/google/devtools/build/lib/util/io/MessageInputStream.java:25 MessageInputStream.read
    fn read(&mut self) -> io::Result<Option<T>>;

    /// Closes the underlying stream. Any following reads will fail.
    // Java: com/google/devtools/build/lib/util/io/MessageInputStream.java:29 MessageInputStream.close
    fn close(&mut self) -> io::Result<()>;
}

/// A `java.io.OutputStream`: a byte sink with an explicit, fallible `close()`.
///
/// PORT: Rust's `Write` has no `close`; dropping a writer cannot report errors (and a zstd
/// encoder must be finished explicitly), so streams whose `close()` matters implement this.
pub trait OutputStream: Write + Send {
    /// `OutputStream.close()`: flushes and releases the stream.
    fn close(self: Box<Self>) -> io::Result<()>;
}

/// An `OutputStream` whose `close()` is a flush followed by drop (`FileOutputStream`,
/// `BufferedOutputStream`).
pub struct FlushingOutputStream<W: Write + Send>(pub W);

impl<W: Write + Send> Write for FlushingOutputStream<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.0.write_all(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl<W: Write + Send> OutputStream for FlushingOutputStream<W> {
    fn close(mut self: Box<Self>) -> io::Result<()> {
        self.0.flush()
    }
}

/// Port of the parts of `com.github.luben.zstd.ZstdOutputStream` Bazel uses: a zstd frame at the
/// library's default level, finished on `close()`.
pub struct ZstdOutputStream<W: Write + Send> {
    encoder: Option<zstd::stream::write::Encoder<'static, W>>,
}

/// `Zstd.defaultCompressionLevel()` (`ZSTD_CLEVEL_DEFAULT`), the level `new ZstdOutputStream(out)`
/// uses.
const ZSTD_DEFAULT_COMPRESSION_LEVEL: i32 = 3;

impl<W: Write + Send> ZstdOutputStream<W> {
    /// `new ZstdOutputStream(out)`.
    pub fn new(out: W) -> io::Result<ZstdOutputStream<W>> {
        Ok(ZstdOutputStream {
            encoder: Some(zstd::stream::write::Encoder::new(
                out,
                ZSTD_DEFAULT_COMPRESSION_LEVEL,
            )?),
        })
    }

    fn encoder(&mut self) -> io::Result<&mut zstd::stream::write::Encoder<'static, W>> {
        self.encoder
            .as_mut()
            .ok_or_else(|| io::Error::other("Stream closed"))
    }
}

impl<W: Write + Send> Write for ZstdOutputStream<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.encoder()?.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.encoder()?.flush()
    }
}

impl<W: Write + Send> OutputStream for ZstdOutputStream<W> {
    fn close(mut self: Box<Self>) -> io::Result<()> {
        match self.encoder.take() {
            None => Ok(()),
            Some(encoder) => {
                let mut out = encoder.finish()?;
                out.flush()
            }
        }
    }
}
