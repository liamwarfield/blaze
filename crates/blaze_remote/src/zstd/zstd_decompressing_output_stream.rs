//! Port of `lib/remote/zstd/ZstdDecompressingOutputStream.java`.

use std::io::{self, Write};

use ::zstd::stream::raw::{Decoder, InBuffer, Operation, OutBuffer};

/// A writer (`OutputStream`) that use zstd to decompress the content.
///
/// PORT: Java feeds each written slice to a continuous `ZstdInputStreamNoFinalizer` and
/// `transferTo`s what it yields to `out`; here the same `ZSTD_decompressStream` loop runs
/// directly on the slice. "Continuous" means that input ending inside a frame is not an error:
/// the decoder keeps its state for the next write.
// Java: com/google/devtools/build/lib/remote/zstd/ZstdDecompressingOutputStream.java:23 ZstdDecompressingOutputStream
pub struct ZstdDecompressingOutputStream<W: Write> {
    out: W,
    zis: Decoder<'static>,
    /// The decompression buffer (`ZSTD_DStreamOutSize()`).
    dst: Vec<u8>,
    /// `ZstdInputStreamNoFinalizer.frameFinished`.
    frame_finished: bool,
}

impl<W: Write> ZstdDecompressingOutputStream<W> {
    // Java: com/google/devtools/build/lib/remote/zstd/ZstdDecompressingOutputStream.java:28 ZstdDecompressingOutputStream.<init>
    pub fn new(out: W) -> io::Result<ZstdDecompressingOutputStream<W>> {
        Ok(ZstdDecompressingOutputStream {
            out,
            zis: Decoder::new()?,
            dst: vec![0u8; ::zstd::zstd_safe::DCtx::out_size()],
            frame_finished: false,
        })
    }

    /// `write(int b)`.
    // Java: com/google/devtools/build/lib/remote/zstd/ZstdDecompressingOutputStream.java:47 ZstdDecompressingOutputStream.write
    pub fn write_byte(&mut self, b: u8) -> io::Result<()> {
        self.write_bytes(&[b])
    }

    /// `write(byte[] b, int off, int len)`: decompresses `b` and writes everything that can be
    /// decoded so far to the underlying stream.
    // Java: com/google/devtools/build/lib/remote/zstd/ZstdDecompressingOutputStream.java:57 ZstdDecompressingOutputStream.write
    pub fn write_bytes(&mut self, b: &[u8]) -> io::Result<()> {
        // inner = new ByteArrayInputStream(b, off, len); zis.transferTo(out);
        let mut input = InBuffer::around(b);
        loop {
            if self.frame_finished {
                if input.pos == b.len() {
                    // A finished frame is fully flushed; nothing is pending.
                    break;
                }
                // The next frame of a multi-frame stream.
                self.zis.reinit()?;
                self.frame_finished = false;
            }
            let mut output = OutBuffer::around(&mut self.dst[..]);
            let size = self.zis.run(&mut input, &mut output)?;
            let dst_pos = output.pos();
            // we have completed a frame
            if size == 0 {
                self.frame_finished = true;
            }
            self.out.write_all(&self.dst[..dst_pos])?;
            // size > 0, so more input is required but there is data left in the decompressor
            // buffers if we have filled the dst buffer
            if input.pos == b.len() && dst_pos < self.dst.len() {
                break;
            }
        }
        Ok(())
    }

    /// `close()`: frees the decompression resources and closes (here: flushes and returns) the
    /// underlying stream.
    // Java: com/google/devtools/build/lib/remote/zstd/ZstdDecompressingOutputStream.java:63 ZstdDecompressingOutputStream.close
    pub fn close(mut self) -> io::Result<W> {
        self.close_shallow()?;
        // PORT: `out.close()`; a Rust writer is closed by dropping it, so it is flushed and
        // handed back.
        self.out.flush()?;
        Ok(self.out)
    }

    /// Free resources related to decompression without closing the underlying stream.
    ///
    /// PORT: the decoder is freed when `self` is dropped; nothing else to do.
    // Java: com/google/devtools/build/lib/remote/zstd/ZstdDecompressingOutputStream.java:71 ZstdDecompressingOutputStream.closeShallow
    pub fn close_shallow(&mut self) -> io::Result<()> {
        Ok(())
    }

    /// The underlying stream (`out`).
    pub fn get_ref(&self) -> &W {
        &self.out
    }

    /// The underlying stream (`out`).
    pub fn get_mut(&mut self) -> &mut W {
        &mut self.out
    }
}

impl<W: Write> Write for ZstdDecompressingOutputStream<W> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.write_bytes(b)?;
        Ok(b.len())
    }

    /// `OutputStream.flush()` is not overridden in Java: it does nothing (in particular it does
    /// not flush the underlying stream).
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
