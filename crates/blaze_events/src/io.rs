//! The subset of `lib/util/io` (and `lib/util/AnsiStrippingOutputStream`) the event system
//! writes through: `OutputStream`, `OutErr`, `RecordingOutErr` and the ANSI-stripping stream
//! that implements `--color=no`.
//!
//! PORT: these belong to Bazel's `lib/util/io` package. They live here until a `util/io` crate
//! exists; move them there (keeping these signatures) when it does.

use std::fmt;
use std::io::{self, Write};
use std::sync::{Arc, LazyLock};

use parking_lot::Mutex;

/// Port of `java.io.OutputStream` as a shared, internally synchronized byte sink.
///
/// PORT: Java streams are shared by reference between threads and handlers, so the methods take
/// `&self`.
pub trait OutputStream: Send + Sync {
    /// `write(byte[], int, int)`.
    fn write(&self, bytes: &[u8]) -> io::Result<()>;

    fn flush(&self) -> io::Result<()> {
        Ok(())
    }

    fn close(&self) -> io::Result<()> {
        Ok(())
    }
}

/// Port of `java.io.ByteArrayOutputStream`.
#[derive(Default)]
pub struct ByteArrayOutputStream {
    buf: Mutex<Vec<u8>>,
}

impl ByteArrayOutputStream {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn size(&self) -> usize {
        self.buf.lock().len()
    }

    pub fn reset(&self) {
        self.buf.lock().clear();
    }

    pub fn to_byte_array(&self) -> Vec<u8> {
        self.buf.lock().clone()
    }

    /// `writeTo(OutputStream)`.
    pub fn write_to(&self, out: &dyn OutputStream) -> io::Result<()> {
        let bytes = self.buf.lock().clone();
        out.write(&bytes)
    }
}

impl OutputStream for ByteArrayOutputStream {
    fn write(&self, bytes: &[u8]) -> io::Result<()> {
        self.buf.lock().extend_from_slice(bytes);
        Ok(())
    }
}

/// An [`OutputStream`] over any `std::io::Write` (e.g. the process's stdout or stderr).
pub struct WriteOutputStream<W: Write + Send> {
    inner: Mutex<W>,
}

impl<W: Write + Send> WriteOutputStream<W> {
    pub fn new(inner: W) -> Self {
        WriteOutputStream {
            inner: Mutex::new(inner),
        }
    }
}

impl<W: Write + Send> OutputStream for WriteOutputStream<W> {
    fn write(&self, bytes: &[u8]) -> io::Result<()> {
        self.inner.lock().write_all(bytes)
    }

    fn flush(&self) -> io::Result<()> {
        self.inner.lock().flush()
    }
}

/// A pair of output streams to be used for redirecting the output and error streams of a
/// subprocess. Port of `OutErr`.
// Java: com/google/devtools/build/lib/util/io/OutErr.java:27 OutErr
#[derive(Clone)]
pub struct OutErr {
    out: Arc<dyn OutputStream>,
    err: Arc<dyn OutputStream>,
}

impl OutErr {
    // Java: com/google/devtools/build/lib/util/io/OutErr.java:32 OutErr.SYSTEM_OUT_ERR
    pub fn system_out_err() -> &'static OutErr {
        static SYSTEM_OUT_ERR: LazyLock<OutErr> = LazyLock::new(|| {
            OutErr::create(
                Arc::new(WriteOutputStream::new(io::stdout())),
                Arc::new(WriteOutputStream::new(io::stderr())),
            )
        });
        &SYSTEM_OUT_ERR
    }

    // Java: com/google/devtools/build/lib/util/io/OutErr.java:35 OutErr.create
    pub fn create(out: Arc<dyn OutputStream>, err: Arc<dyn OutputStream>) -> OutErr {
        OutErr { out, err }
    }

    // Java: com/google/devtools/build/lib/util/io/OutErr.java:45 OutErr.close
    pub fn close(&self) -> io::Result<()> {
        // Ensure that we close both out and err even if one throws.
        let r = self.out.close();
        if !std::ptr::addr_eq(Arc::as_ptr(&self.out), Arc::as_ptr(&self.err)) {
            self.err.close()?;
        }
        r
    }

    // Java: com/google/devtools/build/lib/util/io/OutErr.java:114 OutErr.getOutputStream
    pub fn get_output_stream(&self) -> &Arc<dyn OutputStream> {
        &self.out
    }

    // Java: com/google/devtools/build/lib/util/io/OutErr.java:118 OutErr.getErrorStream
    pub fn get_error_stream(&self) -> &Arc<dyn OutputStream> {
        &self.err
    }

    /// Writes the specified string to the output stream, and flushes.
    // Java: com/google/devtools/build/lib/util/io/OutErr.java:123 OutErr.printOut
    pub fn print_out(&self, s: &str) {
        // PORT: PrintWriter swallows IOExceptions; so do we. StringEncoding identity.
        let _ = self.out.write(s.as_bytes());
        let _ = self.out.flush();
    }

    // Java: com/google/devtools/build/lib/util/io/OutErr.java:129 OutErr.printOutLn
    pub fn print_out_ln(&self, s: &str) {
        self.print_out(&format!("{s}\n"));
    }

    /// Writes the specified string to the error stream, and flushes.
    // Java: com/google/devtools/build/lib/util/io/OutErr.java:134 OutErr.printErr
    pub fn print_err(&self, s: &str) {
        let _ = self.err.write(s.as_bytes());
        let _ = self.err.flush();
    }

    // Java: com/google/devtools/build/lib/util/io/OutErr.java:140 OutErr.printErrLn
    pub fn print_err_ln(&self, s: &str) {
        self.print_err(&format!("{s}\n"));
    }
}

/// An [`OutErr`] that captures all out / err output and makes it available as ISO-8859-1
/// strings. Useful for implementing test cases that assert particular output. Port of
/// `RecordingOutErr`.
// Java: com/google/devtools/build/lib/util/io/RecordingOutErr.java:24 RecordingOutErr
#[derive(Clone)]
pub struct RecordingOutErr {
    out: Arc<ByteArrayOutputStream>,
    err: Arc<ByteArrayOutputStream>,
    out_err: OutErr,
}

impl Default for RecordingOutErr {
    fn default() -> Self {
        Self::new()
    }
}

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

impl RecordingOutErr {
    // Java: com/google/devtools/build/lib/util/io/RecordingOutErr.java:26 RecordingOutErr.RecordingOutErr
    pub fn new() -> Self {
        let out = Arc::new(ByteArrayOutputStream::new());
        let err = Arc::new(ByteArrayOutputStream::new());
        RecordingOutErr {
            out_err: OutErr::create(out.clone(), err.clone()),
            out,
            err,
        }
    }

    /// The recording pair as a plain [`OutErr`].
    pub fn out_err(&self) -> &OutErr {
        &self.out_err
    }

    /// Reset the captured content; that is, reset the out / err buffers.
    // Java: com/google/devtools/build/lib/util/io/RecordingOutErr.java:37 RecordingOutErr.reset
    pub fn reset(&self) {
        self.out.reset();
        self.err.reset();
    }

    /// Interprets the captured out content as an ISO-8859-1 encoded string.
    // Java: com/google/devtools/build/lib/util/io/RecordingOutErr.java:46 RecordingOutErr.outAsLatin1
    pub fn out_as_latin1(&self) -> String {
        latin1(&self.out.to_byte_array())
    }

    /// Interprets the captured err content as an ISO-8859-1 encoded string.
    // Java: com/google/devtools/build/lib/util/io/RecordingOutErr.java:58 RecordingOutErr.errAsLatin1
    pub fn err_as_latin1(&self) -> String {
        latin1(&self.err.to_byte_array())
    }

    /// Returns true if any output was recorded.
    // Java: com/google/devtools/build/lib/util/io/RecordingOutErr.java:69 RecordingOutErr.hasRecordedOutput
    pub fn has_recorded_output(&self) -> bool {
        self.out.size() > 0 || self.err.size() > 0
    }

    pub fn get_output_stream(&self) -> &Arc<ByteArrayOutputStream> {
        &self.out
    }

    pub fn get_error_stream(&self) -> &Arc<ByteArrayOutputStream> {
        &self.err
    }
}

// Java: com/google/devtools/build/lib/util/io/RecordingOutErr.java:74 RecordingOutErr.toString
impl fmt::Display for RecordingOutErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let out = self.out_as_latin1();
        let err = self.err_as_latin1();
        if !out.is_empty() {
            writeln!(f, "stdout: {out}")?;
        }
        if !err.is_empty() {
            write!(f, "stderr: {err}")?;
        }
        Ok(())
    }
}

// Java: com/google/devtools/build/lib/util/AnsiStrippingOutputStream.java:28 AnsiStrippingOutputStream.State
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Normal,
    AfterEscape,
    Parameter,
}

// Java: com/google/devtools/build/lib/util/AnsiStrippingOutputStream.java:37 AnsiStrippingOutputStream.ESCAPE_BUFFER_LENGTH
const ESCAPE_BUFFER_LENGTH: usize = 128;

struct StripState {
    escape_code_buffer: Vec<u8>,
    state: State,
}

/// A pass-thru [`OutputStream`] that strips ANSI control codes. Port of
/// `AnsiStrippingOutputStream`; Bazel wraps the UI's streams in it for `--color=no`.
// Java: com/google/devtools/build/lib/util/AnsiStrippingOutputStream.java:23 AnsiStrippingOutputStream
pub struct AnsiStrippingOutputStream {
    // The idea is straightforward: the regexp for ANSI control codes is
    // \x1b\[[;0-9]*[a-zA-Z] . Implementing it as a stream is a little ugly,
    // though.
    output: Arc<dyn OutputStream>,
    state: Mutex<StripState>,
}

impl AnsiStrippingOutputStream {
    // Java: com/google/devtools/build/lib/util/AnsiStrippingOutputStream.java:43 AnsiStrippingOutputStream.AnsiStrippingOutputStream
    pub fn new(output: Arc<dyn OutputStream>) -> Self {
        AnsiStrippingOutputStream {
            output,
            state: Mutex::new(StripState {
                escape_code_buffer: Vec::with_capacity(ESCAPE_BUFFER_LENGTH),
                state: State::Normal,
            }),
        }
    }
}

impl StripState {
    // Java: com/google/devtools/build/lib/util/AnsiStrippingOutputStream.java:90 AnsiStrippingOutputStream.processByte
    fn process_byte(&mut self, b: u8, output_buffer: &mut Vec<u8>) {
        match self.state {
            State::Normal => {
                blaze_util::check_state!(self.escape_code_buffer.is_empty());
                if b == 0x1b {
                    self.state = State::AfterEscape;
                    self.add_byte_to_escape_buffer(b, output_buffer);
                } else {
                    output_buffer.push(b);
                }
            }
            State::AfterEscape => {
                if b == b'[' {
                    self.state = State::Parameter;
                    self.add_byte_to_escape_buffer(b, output_buffer);
                } else if b == 0x1b {
                    self.dump_escape_buffer(output_buffer);
                    self.state = State::AfterEscape;
                    self.add_byte_to_escape_buffer(b, output_buffer);
                } else {
                    self.dump_escape_buffer(output_buffer);
                    output_buffer.push(b);
                    self.state = State::Normal;
                }
            }
            State::Parameter => {
                if b.is_ascii_digit() || b == b';' {
                    // Parameter continues
                    self.add_byte_to_escape_buffer(b, output_buffer);
                } else if b.is_ascii_alphabetic() {
                    // Found a control sequence, discard it and revert to normal state
                    self.escape_code_buffer.clear();
                    self.state = State::Normal;
                } else if b == 0x1b {
                    // Another escape sequence begins immediately after, and this is
                    // an illegal escape sequence
                    self.dump_escape_buffer(output_buffer);
                    self.state = State::AfterEscape;
                    self.add_byte_to_escape_buffer(b, output_buffer);
                } else {
                    // Illegal control sequence, output it
                    // PORT: as in Java, the byte that ended the illegal sequence is dropped.
                    self.dump_escape_buffer(output_buffer);
                    self.state = State::Normal;
                }
            }
        }
    }

    // Java: com/google/devtools/build/lib/util/AnsiStrippingOutputStream.java:142 AnsiStrippingOutputStream.addByteToEscapeBuffer
    fn add_byte_to_escape_buffer(&mut self, b: u8, output_buffer: &mut Vec<u8>) {
        self.escape_code_buffer.push(b);
        if self.escape_code_buffer.len() == ESCAPE_BUFFER_LENGTH {
            // Buffer full. Assume that no sane code emits an ANSI control code this
            // long and revert to normal state.
            self.dump_escape_buffer(output_buffer);
            self.state = State::Normal;
        }
    }

    // Java: com/google/devtools/build/lib/util/AnsiStrippingOutputStream.java:160 AnsiStrippingOutputStream.dumpEscapeBuffer
    fn dump_escape_buffer(&mut self, output_buffer: &mut Vec<u8>) {
        output_buffer.append(&mut self.escape_code_buffer);
    }
}

impl OutputStream for AnsiStrippingOutputStream {
    // Java: com/google/devtools/build/lib/util/AnsiStrippingOutputStream.java:58 AnsiStrippingOutputStream.write
    fn write(&self, b: &[u8]) -> io::Result<()> {
        let mut st = self.state.lock();
        let len = b.len();
        let mut i = 0;
        if st.state == State::Normal {
            // Avoid outputBuffer allocation entirely if that's possible
            while i < len && b[i] != 0x1b {
                i += 1;
            }
            if i == len {
                return self.output.write(b);
            }
        }

        let mut output_buffer = Vec::with_capacity(len + ESCAPE_BUFFER_LENGTH);
        output_buffer.extend_from_slice(&b[..i]);
        for &byte in &b[i..] {
            st.process_byte(byte, &mut output_buffer);
        }
        self.output.write(&output_buffer)
    }

    // Java: com/google/devtools/build/lib/util/AnsiStrippingOutputStream.java:168 AnsiStrippingOutputStream.flush
    fn flush(&self) -> io::Result<()> {
        self.output.flush()
    }

    // Java: com/google/devtools/build/lib/util/AnsiStrippingOutputStream.java:173 AnsiStrippingOutputStream.close
    fn close(&self) -> io::Result<()> {
        self.output.close()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ESCAPE: &str = "\u{1b}[";

    // Java: src/test/java/com/google/devtools/build/lib/util/AnsiStrippingOutputStreamTest.java AnsiStrippingOutputStreamTest.getOutput
    fn get_output(fragments: &[&str]) -> String {
        let output = Arc::new(ByteArrayOutputStream::new());
        let input = AnsiStrippingOutputStream::new(output.clone());
        for fragment in fragments {
            input.write(fragment.as_bytes()).unwrap();
        }
        latin1(&output.to_byte_array())
    }

    // Java: src/test/java/com/google/devtools/build/lib/util/AnsiStrippingOutputStreamTest.java AnsiStrippingOutputStreamTest.doesNotFailHorribly
    #[test]
    fn does_not_fail_horribly() {
        assert_eq!(get_output(&["Love"]), "Love");
    }

    // Java: src/test/java/com/google/devtools/build/lib/util/AnsiStrippingOutputStreamTest.java AnsiStrippingOutputStreamTest.canStripAnsiCode
    #[test]
    fn can_strip_ansi_code() {
        assert_eq!(get_output(&[&format!("{ESCAPE}32mLove{ESCAPE}m")]), "Love");
    }

    // Java: src/test/java/com/google/devtools/build/lib/util/AnsiStrippingOutputStreamTest.java AnsiStrippingOutputStreamTest.recognizesAnsiCodeWhenBrokenUp
    #[test]
    fn recognizes_ansi_code_when_broken_up() {
        assert_eq!(get_output(&["\u{1b}", "[", "mLove"]), "Love");
    }

    // Java: src/test/java/com/google/devtools/build/lib/util/AnsiStrippingOutputStreamTest.java AnsiStrippingOutputStreamTest.handlesOnlyEscCorrectly
    #[test]
    fn handles_only_esc_correctly() {
        assert_eq!(get_output(&["\u{1b}Love"]), "\u{1b}Love");
    }

    // Java: src/test/java/com/google/devtools/build/lib/util/AnsiStrippingOutputStreamTest.java AnsiStrippingOutputStreamTest.handlesEscInPlaceOfControlCharCorrectly
    #[test]
    fn handles_esc_in_place_of_control_char_correctly() {
        assert_eq!(
            get_output(&[&format!("{ESCAPE}31;42{ESCAPE}1mLove")]),
            format!("{ESCAPE}31;42Love")
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/util/AnsiStrippingOutputStreamTest.java AnsiStrippingOutputStreamTest.handlesTwoEscapeSequencesCorrectly
    #[test]
    fn handles_two_escape_sequences_correctly() {
        assert_eq!(get_output(&[&format!("{ESCAPE}32m{ESCAPE}1mLove")]), "Love");
    }
}
