//! Port of `lib/remote/zstd/ZstdCompressingInputStream.java`.

use std::cmp::{max, min};
use std::collections::VecDeque;
use std::io::{self, Read};

use ::zstd::stream::raw::{Encoder, InBuffer, Operation, OutBuffer};

/// zstd-jni's `ZstdOutputStreamNoFinalizer`, as far as [`ZstdCompressingInputStream`] uses it
/// (`write`, `flush`, `close` at the default compression level, writing into the pipe).
// Java: com/github/luben/zstd/ZstdOutputStreamNoFinalizer.java ZstdOutputStreamNoFinalizer
struct ZstdOutputStreamNoFinalizer {
    stream: Encoder<'static>,
    /// `dst`, of size `ZSTD_CStreamOutSize()`.
    dst: Vec<u8>,
    frame_closed: bool,
    frame_started: bool,
}

impl ZstdOutputStreamNoFinalizer {
    fn new() -> io::Result<ZstdOutputStreamNoFinalizer> {
        Ok(ZstdOutputStreamNoFinalizer {
            // Zstd.defaultCompressionLevel()
            stream: Encoder::new(::zstd::DEFAULT_COMPRESSION_LEVEL)?,
            dst: vec![0u8; ::zstd::zstd_safe::CCtx::out_size()],
            frame_closed: true,
            frame_started: false,
        })
    }

    /// `write(byte[] src, int offset, int len)`; the compressed bytes go to `out`.
    fn write(&mut self, src: &[u8], out: &mut VecDeque<u8>) -> io::Result<()> {
        if self.frame_closed {
            self.stream.reinit()?;
            self.frame_closed = false;
            self.frame_started = true;
        }
        let mut input = InBuffer::around(src);
        while input.pos < src.len() {
            let mut output = OutBuffer::around(&mut self.dst[..]);
            self.stream.run(&mut input, &mut output)?;
            let dst_pos = output.pos();
            if dst_pos > 0 {
                out.extend(&self.dst[..dst_pos]);
            }
        }
        Ok(())
    }

    /// `flush()` (`closeFrameOnFlush` is false).
    fn flush(&mut self, out: &mut VecDeque<u8>) -> io::Result<()> {
        if !self.frame_closed {
            // compress the remaining input
            loop {
                let mut output = OutBuffer::around(&mut self.dst[..]);
                let size = self.stream.flush(&mut output)?;
                let dst_pos = output.pos();
                out.extend(&self.dst[..dst_pos]);
                if size == 0 {
                    break;
                }
            }
        }
        Ok(())
    }

    /// `close()`.
    fn close(mut self, out: &mut VecDeque<u8>) -> io::Result<()> {
        // Closing the stream withouth before writing anything should still produce valid zstd
        // frame. So reset the stream to start a frame if no frame was ever started.
        if !self.frame_started {
            self.stream.reinit()?;
            self.frame_closed = false;
        }
        if !self.frame_closed {
            // compress the remaining input and close the frame
            loop {
                let mut output = OutBuffer::around(&mut self.dst[..]);
                let size = self.stream.finish(&mut output, true)?;
                let dst_pos = output.pos();
                out.extend(&self.dst[..dst_pos]);
                if size == 0 {
                    break;
                }
            }
        }
        Ok(())
    }
}

/// A reader (`FilterInputStream`) that use zstd to compress the content.
///
/// PORT: Java's `PipedInputStream`/`PipedOutputStream` pair is the queue `pis`; `close()`
/// (ZstdCompressingInputStream.java:98) is dropping the stream, which drops the wrapped reader.
// Java: com/google/devtools/build/lib/remote/zstd/ZstdCompressingInputStream.java:27 ZstdCompressingInputStream
pub struct ZstdCompressingInputStream<R: Read> {
    in_: R,
    pis: VecDeque<u8>,
    zos: Option<ZstdOutputStreamNoFinalizer>,
    size: i32,
}

impl<R: Read> ZstdCompressingInputStream<R> {
    // We want the buffer to be able to contain at least:
    //   - Magic number: 4 bytes
    //   - FrameHeader 14 bytes
    //   - Block Header: 3 bytes
    //   - First block byte
    // This guarantees that we can always compress at least
    // 1 byte and write it to the pipe without blocking.
    // Java: com/google/devtools/build/lib/remote/zstd/ZstdCompressingInputStream.java:35 ZstdCompressingInputStream.MIN_BUFFER_SIZE
    pub const MIN_BUFFER_SIZE: i32 = 4 + 14 + 3 + 1;

    // Java: com/google/devtools/build/lib/remote/zstd/ZstdCompressingInputStream.java:41 ZstdCompressingInputStream.<init>
    pub fn new(in_: R) -> io::Result<ZstdCompressingInputStream<R>> {
        ZstdCompressingInputStream::with_size(in_, 512)
    }

    // Java: com/google/devtools/build/lib/remote/zstd/ZstdCompressingInputStream.java:45 ZstdCompressingInputStream.<init>
    pub fn with_size(in_: R, size: i32) -> io::Result<ZstdCompressingInputStream<R>> {
        blaze_util::check_argument!(
            size >= Self::MIN_BUFFER_SIZE,
            "The buffer size must be at least {} bytes",
            Self::MIN_BUFFER_SIZE
        );
        Ok(ZstdCompressingInputStream {
            in_,
            size,
            pis: VecDeque::with_capacity(size as usize),
            zos: Some(ZstdOutputStreamNoFinalizer::new()?),
        })
    }

    /// `pis.available()`.
    // Java: java/io/PipedInputStream.java PipedInputStream.available
    fn pis_available(&self) -> i32 {
        self.pis.len() as i32
    }

    /// `pis.read(b, off, len)` for `len > 0`: the bytes that are in the pipe, at most `b.len()`;
    /// `-1` when the pipe is empty and its writer is closed.
    // Java: java/io/PipedInputStream.java PipedInputStream.read
    fn pis_read(&mut self, b: &mut [u8]) -> i32 {
        if self.pis.is_empty() {
            // PORT: a `PipedInputStream` blocks here while its writer is open; `reFill()` always
            // writes before this is reached, so only the closed case remains.
            return -1;
        }
        let n = min(b.len(), self.pis.len());
        for (slot, byte) in b[..n].iter_mut().zip(self.pis.drain(..n)) {
            *slot = byte;
        }
        n as i32
    }

    // Java: com/google/devtools/build/lib/remote/zstd/ZstdCompressingInputStream.java:55 ZstdCompressingInputStream.reFill
    fn re_fill(&mut self) -> io::Result<()> {
        let mut buf = vec![0u8; self.size as usize];
        let to_read = max(
            0,
            self.size - self.pis_available() - Self::MIN_BUFFER_SIZE + 1,
        ) as usize;
        // PORT: `InputStream.read(buf, 0, 0)` is 0, not end of stream; `Read::read` of a
        // non-empty buffer returns 0 only at the end of the stream.
        let len: i32 = if to_read == 0 {
            0
        } else {
            match self.in_.read(&mut buf[..to_read])? {
                0 => -1,
                n => n as i32,
            }
        };
        if len == -1 {
            if let Some(zos) = self.zos.take() {
                zos.close(&mut self.pis)?;
            }
        } else if let Some(zos) = self.zos.as_mut() {
            zos.write(&buf[..len as usize], &mut self.pis)?;
            zos.flush(&mut self.pis)?;
        }
        Ok(())
    }

    /// `read()`: the next byte, or `-1` at the end of the stream.
    // Java: com/google/devtools/build/lib/remote/zstd/ZstdCompressingInputStream.java:68 ZstdCompressingInputStream.read
    pub fn read_byte(&mut self) -> io::Result<i32> {
        if self.pis_available() == 0 {
            if self.zos.is_none() {
                return Ok(-1);
            }
            self.re_fill()?;
        }
        Ok(match self.pis.pop_front() {
            Some(b) => b as i32,
            None => -1,
        })
    }

    /// `read(byte[] b, int off, int len)`: the number of bytes read, or `-1` at the end of the
    /// stream (`0` for an empty `b`).
    // Java: com/google/devtools/build/lib/remote/zstd/ZstdCompressingInputStream.java:84 ZstdCompressingInputStream.read
    pub fn read_bytes(&mut self, b: &mut [u8]) -> io::Result<i32> {
        let len = b.len();
        let mut count: usize = 0;
        let mut n: i32 = if len > 0 { -1 } else { 0 };
        while count < len && (self.pis_available() > 0 || self.zos.is_some()) {
            if self.pis_available() == 0 {
                self.re_fill()?;
            }
            n = self.pis_read(&mut b[count..]);
            count += max(0, n) as usize;
        }
        Ok(if count > 0 { count as i32 } else { n })
    }
}

impl<R: Read> Read for ZstdCompressingInputStream<R> {
    /// [`ZstdCompressingInputStream::read_bytes`] with Rust's convention for the end of the
    /// stream (`Ok(0)`).
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        Ok(max(0, self.read_bytes(b)?) as usize)
    }
}
