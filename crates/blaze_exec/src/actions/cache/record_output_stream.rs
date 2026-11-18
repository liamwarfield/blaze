//! Port of `lib/util/io/RecordOutputStream.java`.
//!
//! PORT: belongs to `lib/util/io`; it lives here (its only user is `MapCodec`) until that package
//! is ported.

use std::io::{self, Write};

/// A buffered output stream that only flushes its buffer at record boundaries.
///
/// The [`RecordOutputStream::finish_record`] method marks the current position as the end of a
/// complete record. Whenever a flush occurs (either explicitly via `flush` or implicitly via
/// `write` or `close`), the internal buffer is only flushed up to the last recorded position, with
/// any following bytes remaining in the internal buffer. The internal buffer starts at 4KB but
/// grows to accommodate the largest record seen so far.
///
/// This is intended as a best-effort attempt to prevent incomplete records from being written to
/// disk in the event of an abrupt exit. It isn't completely safe since partial underlying writes
/// are still possible, but experiments suggest that they're very unlikely for small buffer sizes.
// Java: com/google/devtools/build/lib/util/io/RecordOutputStream.java:35 RecordOutputStream
pub struct RecordOutputStream {
    out: Option<Box<dyn Write + Send>>,
    buf: Vec<u8>,
    write_off: usize,
    flush_off: usize,
}

impl RecordOutputStream {
    // Java: com/google/devtools/build/lib/util/io/RecordOutputStream.java:41 RecordOutputStream.<init>
    pub fn new(out: Box<dyn Write + Send>) -> RecordOutputStream {
        RecordOutputStream {
            out: Some(out),
            buf: vec![0u8; 4096],
            write_off: 0,
            flush_off: 0,
        }
    }

    /// Marks the current position as the end of a complete record.
    // Java: com/google/devtools/build/lib/util/io/RecordOutputStream.java:46 RecordOutputStream.finishRecord
    pub fn finish_record(&mut self) {
        self.flush_off = self.write_off;
    }

    /// `write(byte[], int, int)`.
    // Java: com/google/devtools/build/lib/util/io/RecordOutputStream.java:61 RecordOutputStream.write
    pub fn write_bytes(&mut self, b: &[u8]) -> io::Result<()> {
        let len = b.len();
        if len > self.buf.len() - self.write_off {
            // First try to make space by flushing.
            self.flush_records()?;
            if len > self.buf.len() - self.write_off {
                // If the buffer is too small to fit a single record, grow it to the next power of
                // two.
                let new_len = (self.write_off + len).next_power_of_two();
                self.buf.resize(new_len, 0);
            }
        }
        self.buf[self.write_off..self.write_off + len].copy_from_slice(b);
        self.write_off += len;
        Ok(())
    }

    /// `flush()`: writes the complete records to the underlying stream.
    // Java: com/google/devtools/build/lib/util/io/RecordOutputStream.java:75 RecordOutputStream.flush
    pub fn flush_records(&mut self) -> io::Result<()> {
        if self.flush_off > 0 {
            let out = self
                .out
                .as_mut()
                .ok_or_else(|| io::Error::other("Stream closed"))?;
            out.write_all(&self.buf[..self.flush_off])?;
            // TODO(tjgq): Consider using a ring buffer to avoid this copy.
            self.buf.copy_within(self.flush_off..self.write_off, 0);
            self.write_off -= self.flush_off;
            self.flush_off = 0;
        }
        Ok(())
    }

    /// `close()`: flushes the complete records and closes the underlying stream.
    // Java: com/google/devtools/build/lib/util/io/RecordOutputStream.java:86 RecordOutputStream.close
    pub fn close(&mut self) -> io::Result<()> {
        let flushed = self.flush_records();
        let closed = match self.out.take() {
            Some(mut out) => out.flush(),
            None => Ok(()),
        };
        flushed.and(closed)
    }
}

impl Write for RecordOutputStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.write_bytes(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flush_records()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);

    impl Write for Sink {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn only_complete_records_are_written() {
        let sink = Sink::default();
        let mut out = RecordOutputStream::new(Box::new(sink.clone()));
        out.write_bytes(b"abc").unwrap();
        out.finish_record();
        out.write_bytes(b"de").unwrap();
        out.flush_records().unwrap();
        assert_eq!(&*sink.0.lock().unwrap(), b"abc");
        out.close().unwrap();
        assert_eq!(&*sink.0.lock().unwrap(), b"abc");
    }

    #[test]
    fn grows_for_large_records() {
        let sink = Sink::default();
        let mut out = RecordOutputStream::new(Box::new(sink.clone()));
        let big = vec![7u8; 10000];
        out.write_bytes(&big).unwrap();
        out.finish_record();
        out.close().unwrap();
        assert_eq!(sink.0.lock().unwrap().len(), 10000);
    }
}
