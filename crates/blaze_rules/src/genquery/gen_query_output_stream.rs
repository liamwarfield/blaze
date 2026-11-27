//! Port of `rules/genquery/GenQueryOutputStream.java`.

use std::io::{self, Read, Write};

use blaze_util::Fingerprint;
use flate2::write::GzEncoder;

/// When compression is enabled, the threshold at which the stream will switch to compressing
/// output. The value of this constant is arbitrary but effective.
///
/// If genquery definition explicitly sets `compressed_output` parameter to `True`, the stream
/// will be compressed regardless of whether its size reaches this threshold.
// Java: com/google/devtools/build/lib/rules/genquery/GenQueryOutputStream.java:46 GenQueryOutputStream.COMPRESSION_THRESHOLD
const COMPRESSION_THRESHOLD: i32 = 1 << 20;

/// `new GZIPOutputStream(out, GZIP_BYTES_BUFFER)`: Java's header (`GZIPOutputStream.writeHeader`:
/// no flags, mtime 0, XFL 0, OS byte 255) and its `Deflater(Deflater.DEFAULT_COMPRESSION,
/// true)`: zlib level 6, raw deflate with a 15-bit window, memory level 8, the default strategy
/// (flate2's `GzBuilder` makes the same `deflateInit2` call; XFL is 0 at level 6).
///
/// The deflate bytes are zlib's: flate2 is built with its C zlib backend (workspace
/// `Cargo.toml`), linked to the system `libz.so.1` as the JDK's `libzip.so` that Bazel's server
/// runs with is. Neither feeds input with a flush, so the stream does not depend on how the bytes
/// were chunked (`DeflaterOutputStream` writes through an 8192-byte buffer, this stream does not).
// Java: com/google/devtools/build/lib/rules/genquery/GenQueryOutputStream.java:48 GenQueryOutputStream.GZIP_BYTES_BUFFER
fn gzip_output_stream(out: Vec<u8>) -> GzEncoder<Vec<u8>> {
    flate2::GzBuilder::new()
        .mtime(0)
        .operating_system(255)
        .write(out, flate2::Compression::default())
}

/// Encapsulates the output of a `GenQuery`'s query. CPU and memory overhead of individual
/// methods depends on the underlying content and settings.
// Java: com/google/devtools/build/lib/rules/genquery/GenQueryOutputStream.java:54 GenQueryOutputStream.GenQueryResult
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenQueryResult {
    /// Used when input and output GenQuery result data are in the same format, so no
    /// decompression or other data transformation is needed.
    // Java: com/google/devtools/build/lib/rules/genquery/GenQueryOutputStream.java:156 GenQueryOutputStream.SimpleResult
    SimpleResult { data: Vec<u8> },
    /// Used when input GenQuery result is in compressed format and output should be
    /// decompressed.
    // Java: com/google/devtools/build/lib/rules/genquery/GenQueryOutputStream.java:186 GenQueryOutputStream.CompressedResultWithDecompressedOutput
    CompressedResultWithDecompressedOutput { compressed_data: Vec<u8>, size: i32 },
}

impl GenQueryResult {
    /// Returns the query output as a byte string.
    // Java: com/google/devtools/build/lib/rules/genquery/GenQueryOutputStream.java:56 GenQueryOutputStream.GenQueryResult.getBytes
    pub fn get_bytes(&self) -> io::Result<Vec<u8>> {
        match self {
            GenQueryResult::SimpleResult { data } => Ok(data.clone()),
            GenQueryResult::CompressedResultWithDecompressedOutput {
                compressed_data,
                size,
            } => {
                let mut out = Vec::with_capacity((*size).max(0) as usize);
                flate2::read::GzDecoder::new(&compressed_data[..]).read_to_end(&mut out)?;
                Ok(out)
            }
        }
    }

    /// Adds the query output to the supplied fingerprint. Equivalent to
    /// `fingerprint.addBytes(genQueryResult.getBytes())`, but potentially more efficient.
    // Java: com/google/devtools/build/lib/rules/genquery/GenQueryOutputStream.java:65 GenQueryOutputStream.GenQueryResult.fingerprint
    pub fn fingerprint(&self, fingerprint: &mut Fingerprint) {
        match self {
            GenQueryResult::SimpleResult { data } => {
                fingerprint.add_bytes(data);
            }
            GenQueryResult::CompressedResultWithDecompressedOutput {
                compressed_data, ..
            } => {
                let mut gzip_in = flate2::read::GzDecoder::new(&compressed_data[..]);
                let mut chunk = [0u8; 4092];
                loop {
                    match gzip_in.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(bytes_read) => {
                            fingerprint.add_bytes(&chunk[..bytes_read]);
                        }
                        // Unexpected, everything should be in memory!
                        Err(e) => panic!("IllegalStateException: Unexpected IOException: {e}"),
                    }
                }
            }
        }
    }

    /// Returns the size of the output. This must be a constant-time operation for all
    /// implementations.
    // Java: com/google/devtools/build/lib/rules/genquery/GenQueryOutputStream.java:71 GenQueryOutputStream.GenQueryResult.size
    pub fn size(&self) -> i32 {
        match self {
            GenQueryResult::SimpleResult { data } => data.len() as i32,
            GenQueryResult::CompressedResultWithDecompressedOutput { size, .. } => *size,
        }
    }

    /// Writes the query output to the provided output stream.
    // Java: com/google/devtools/build/lib/rules/genquery/GenQueryOutputStream.java:77 GenQueryOutputStream.GenQueryResult.writeTo
    pub fn write_to(&self, out: &mut dyn Write) -> io::Result<()> {
        match self {
            GenQueryResult::SimpleResult { data } => out.write_all(data),
            GenQueryResult::CompressedResultWithDecompressedOutput {
                compressed_data, ..
            } => {
                let mut gzip_in = flate2::read::GzDecoder::new(&compressed_data[..]);
                io::copy(&mut gzip_in, out)?;
                Ok(())
            }
        }
    }
}

/// The stream `out` writes to.
enum Out {
    Plain,
    Gzip(GzEncoder<Vec<u8>>),
}

/// `OutputStream` implementation optimized for `GenQuery` by (optionally) compressing query
/// results on the fly. Produces [`GenQueryResult`]s.
// Java: com/google/devtools/build/lib/rules/genquery/GenQueryOutputStream.java:38 GenQueryOutputStream
pub struct GenQueryOutputStream {
    bytes_written: i32,
    output_was_compressed: bool,
    closed: bool,
    /// Java `bytesOut` while `out` is `bytesOut` itself.
    bytes_out: Vec<u8>,
    out: Out,
    compressed_output_requested: bool,
}

impl GenQueryOutputStream {
    // Java: com/google/devtools/build/lib/rules/genquery/GenQueryOutputStream.java:89 GenQueryOutputStream.<init>
    pub fn new(compressed_output_requested: bool) -> GenQueryOutputStream {
        if compressed_output_requested {
            GenQueryOutputStream {
                bytes_written: 0,
                output_was_compressed: true,
                closed: false,
                bytes_out: Vec::new(),
                out: Out::Gzip(gzip_output_stream(Vec::new())),
                compressed_output_requested,
            }
        } else {
            GenQueryOutputStream {
                bytes_written: 0,
                output_was_compressed: false,
                closed: false,
                bytes_out: Vec::new(),
                out: Out::Plain,
                compressed_output_requested,
            }
        }
    }

    fn out_write(&mut self, bytes: &[u8]) -> io::Result<()> {
        match &mut self.out {
            Out::Plain => {
                self.bytes_out.extend_from_slice(bytes);
                Ok(())
            }
            Out::Gzip(g) => g.write_all(bytes),
        }
    }

    // Java: com/google/devtools/build/lib/rules/genquery/GenQueryOutputStream.java:120 GenQueryOutputStream.close
    pub fn close(&mut self) -> io::Result<()> {
        if let Out::Gzip(g) = std::mem::replace(&mut self.out, Out::Plain) {
            self.bytes_out = g.finish()?;
        }
        self.closed = true;
        Ok(())
    }

    // Java: com/google/devtools/build/lib/rules/genquery/GenQueryOutputStream.java:125 GenQueryOutputStream.getResult
    pub fn get_result(&self) -> GenQueryResult {
        blaze_util::check_state!(self.closed, "Must be closed");
        if !self.output_was_compressed || self.compressed_output_requested {
            GenQueryResult::SimpleResult {
                data: self.bytes_out.clone(),
            }
        } else {
            GenQueryResult::CompressedResultWithDecompressedOutput {
                compressed_data: self.bytes_out.clone(),
                size: self.bytes_written,
            }
        }
    }

    // Java: com/google/devtools/build/lib/rules/genquery/GenQueryOutputStream.java:132 GenQueryOutputStream.maybeStartCompression
    fn maybe_start_compression(&mut self, additional_bytes: i32) -> io::Result<()> {
        if self.output_was_compressed {
            return Ok(());
        }

        if !self.compressed_output_requested
            && self.bytes_written.wrapping_add(additional_bytes) < COMPRESSION_THRESHOLD
        {
            return Ok(());
        }

        let mut gzip_out = gzip_output_stream(Vec::new());
        gzip_out.write_all(&std::mem::take(&mut self.bytes_out))?;
        self.out = Out::Gzip(gzip_out);
        self.output_was_compressed = true;
        Ok(())
    }
}

impl Write for GenQueryOutputStream {
    // Java: com/google/devtools/build/lib/rules/genquery/GenQueryOutputStream.java:101 GenQueryOutputStream.write
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let len = bytes.len() as i32;
        self.maybe_start_compression(len)?;
        self.out_write(bytes)?;
        self.bytes_written = self.bytes_written.wrapping_add(len);
        Ok(bytes.len())
    }

    // Java: com/google/devtools/build/lib/rules/genquery/GenQueryOutputStream.java:115 GenQueryOutputStream.flush
    fn flush(&mut self) -> io::Result<()> {
        match &mut self.out {
            Out::Plain => Ok(()),
            // `GZIPOutputStream(out, size)` has `syncFlush == false`: `DeflaterOutputStream.flush`
            // only flushes `out` and writes no deflate block (flate2's `flush` would write a
            // sync-flush block, so the bytes would differ).
            // JDK: java.base/java/util/zip/DeflaterOutputStream.java DeflaterOutputStream.flush (syncFlush)
            Out::Gzip(g) => g.get_mut().flush(),
        }
    }
}

#[cfg(test)]
mod tests {
    //! Port of src/test/java/com/google/devtools/build/lib/rules/genquery/GenQueryOutputStreamTest.java
    //! (`@TestParameter boolean outputCompressed`: each test runs with both values).
    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    enum ResultClass {
        SimpleResult,
        CompressedResultWithDecompressedOutput,
    }

    fn class_of(result: &GenQueryResult) -> ResultClass {
        match result {
            GenQueryResult::SimpleResult { .. } => ResultClass::SimpleResult,
            GenQueryResult::CompressedResultWithDecompressedOutput { .. } => {
                ResultClass::CompressedResultWithDecompressedOutput
            }
        }
    }

    // Java: src/test/java/com/google/devtools/build/lib/rules/genquery/GenQueryOutputStreamTest.java:38 GenQueryOutputStreamTest.testSmallOutputMultibyteWrite
    #[test]
    fn test_small_output_multibyte_write() {
        for output_compressed in [false, true] {
            run_multibyte_write_test(
                &"xyz".repeat(10_000),
                ResultClass::SimpleResult,
                output_compressed,
            );
        }
    }

    // Java: src/test/java/com/google/devtools/build/lib/rules/genquery/GenQueryOutputStreamTest.java:44 GenQueryOutputStreamTest.testBigOutputMultibyteWrite
    #[test]
    fn test_big_output_multibyte_write() {
        for output_compressed in [false, true] {
            run_multibyte_write_test(
                &"xyz".repeat(1_000_000),
                if output_compressed {
                    ResultClass::SimpleResult
                } else {
                    ResultClass::CompressedResultWithDecompressedOutput
                },
                output_compressed,
            );
        }
    }

    // Java: src/test/java/com/google/devtools/build/lib/rules/genquery/GenQueryOutputStreamTest.java:54 GenQueryOutputStreamTest.testSmallOutputSingleByteWrites
    #[test]
    fn test_small_output_single_byte_writes() {
        for output_compressed in [false, true] {
            run_single_byte_write_test(
                &"xyz".repeat(10_000),
                ResultClass::SimpleResult,
                output_compressed,
            );
        }
    }

    // Java: src/test/java/com/google/devtools/build/lib/rules/genquery/GenQueryOutputStreamTest.java:60 GenQueryOutputStreamTest.testBigOutputSingleByteWrites
    #[test]
    fn test_big_output_single_byte_writes() {
        for output_compressed in [false, true] {
            run_single_byte_write_test(
                &"xyz".repeat(1_000_000),
                if output_compressed {
                    ResultClass::SimpleResult
                } else {
                    ResultClass::CompressedResultWithDecompressedOutput
                },
                output_compressed,
            );
        }
    }

    /// Not a Java test: `flush()` between writes leaves the compressed bytes as they are
    /// (`GZIPOutputStream` without `syncFlush` writes no flush block), and the stream starts with
    /// Java's header.
    #[test]
    fn flush_does_not_change_compressed_bytes() {
        let data = "abcdefgh".repeat(5_000);
        let compressed = |flush: bool| {
            let mut s = GenQueryOutputStream::new(true);
            for chunk in data.as_bytes().chunks(777) {
                s.write_all(chunk).unwrap();
                if flush {
                    s.flush().unwrap();
                }
            }
            s.close().unwrap();
            match s.get_result() {
                GenQueryResult::SimpleResult { data } => data,
                other => panic!("{:?}", class_of(&other)),
            }
        };
        let plain = compressed(false);
        assert_eq!(compressed(true), plain);
        assert_eq!(plain[..10], [0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff]);
    }

    fn run_multibyte_write_test(data: &str, result_class: ResultClass, output_compressed: bool) {
        let mut under_test = GenQueryOutputStream::new(output_compressed);
        under_test.write_all(data.as_bytes()).unwrap();
        under_test.close().unwrap();

        verify_gen_query_result(
            &under_test.get_result(),
            data,
            result_class,
            output_compressed,
        );
    }

    fn run_single_byte_write_test(data: &str, result_class: ResultClass, output_compressed: bool) {
        let mut under_test = GenQueryOutputStream::new(output_compressed);
        for b in data.as_bytes() {
            under_test.write_all(std::slice::from_ref(b)).unwrap();
        }
        under_test.close().unwrap();

        verify_gen_query_result(
            &under_test.get_result(),
            data,
            result_class,
            output_compressed,
        );
    }

    fn verify_gen_query_result(
        result: &GenQueryResult,
        data: &str,
        result_class: ResultClass,
        output_compressed: bool,
    ) {
        assert_eq!(class_of(result), result_class);

        if output_compressed {
            // If result is actually compressed, also compress input data so that it is comparable
            // to what is outputted from GenQueryResult.
            let mut gzip_data_out = gzip_output_stream(Vec::new());
            gzip_data_out.write_all(data.as_bytes()).unwrap();
            let data_compressed = gzip_data_out.finish().unwrap();

            assert_eq!(result.get_bytes().unwrap(), data_compressed);

            let mut actual_fingerprint = Fingerprint::new();
            result.fingerprint(&mut actual_fingerprint);
            let mut expect_fingerprint = Fingerprint::new();
            expect_fingerprint.add_bytes(&data_compressed);
            assert_eq!(
                actual_fingerprint.hex_digest_and_reset(),
                expect_fingerprint.hex_digest_and_reset()
            );

            let mut bytes_out = Vec::new();
            result.write_to(&mut bytes_out).unwrap();
            assert_eq!(bytes_out, data_compressed);
        } else {
            assert_eq!(result.get_bytes().unwrap(), data.as_bytes());
            assert_eq!(result.size() as usize, data.len());

            let mut actual_fingerprint = Fingerprint::new();
            result.fingerprint(&mut actual_fingerprint);
            let mut expect_fingerprint = Fingerprint::new();
            expect_fingerprint.add_bytes(data.as_bytes());
            assert_eq!(
                actual_fingerprint.hex_digest_and_reset(),
                expect_fingerprint.hex_digest_and_reset()
            );

            let mut bytes_out = Vec::new();
            result.write_to(&mut bytes_out).unwrap();
            assert_eq!(bytes_out, data.as_bytes());
        }
    }
}
