//! The subset of `lib/util/io` (and `lib/util/AnsiStrippingOutputStream`) the event system
//! writes through: `OutputStream`, `OutErr`, `RecordingOutErr` and the ANSI-stripping stream
//! that implements `--color=no`.
//!
//! PORT: these belong to Bazel's `lib/util/io` package. They live here until a `util/io` crate
//! exists; move them there (keeping these signatures) when it does.

use std::fmt;
use std::io::{self, Write};
use std::sync::{Arc, LazyLock, OnceLock};

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

/// Called when a write to this process's stdout or stderr through [`OutErr::system_out_err`]
/// fails with EPIPE (set once, by the binary: the client's `signal_handler::note_broken_pipe`).
static BROKEN_PIPE_LISTENER: OnceLock<fn()> = OnceLock::new();

/// Sets the function called when a write to this process's stdout or stderr through
/// [`OutErr::system_out_err`] fails with EPIPE.
///
/// PORT: in Bazel the server's output goes over the command stream (`RpcOutputStream`) to the
/// client, whose `fwrite` to its stdout or stderr gets EPIPE and SIGPIPE; the client's handler
/// records the signal and the client ends by it (`SignalHandler::PropagateSignalOrExit`). With
/// the server in the client's process these streams are the client's stdout and stderr, so
/// their EPIPE is reported to the client here.
pub fn set_broken_pipe_listener(listener: fn()) {
    let _ = BROKEN_PIPE_LISTENER.set(listener);
}

/// This process's stdout or stderr: a write or flush that fails with EPIPE calls the
/// [`set_broken_pipe_listener`] listener and returns the error unchanged.
struct StdOutErrWriter<W: Write>(W);

impl<W: Write> StdOutErrWriter<W> {
    fn note<T>(r: io::Result<T>) -> io::Result<T> {
        if let Err(e) = &r
            && e.kind() == io::ErrorKind::BrokenPipe
            && let Some(listener) = BROKEN_PIPE_LISTENER.get()
        {
            listener();
        }
        r
    }
}

impl<W: Write> Write for StdOutErrWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Self::note(self.0.write(buf))
    }

    fn flush(&mut self) -> io::Result<()> {
        Self::note(self.0.flush())
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
                Arc::new(WriteOutputStream::new(StdOutErrWriter(io::stdout()))),
                Arc::new(WriteOutputStream::new(StdOutErrWriter(io::stderr()))),
            )
        });
        &SYSTEM_OUT_ERR
    }

    // Java: com/google/devtools/build/lib/util/io/OutErr.java:35 OutErr.create
    pub fn create(out: Arc<dyn OutputStream>, err: Arc<dyn OutputStream>) -> OutErr {
        OutErr { out, err }
    }

    /// Returns a [`SystemPatcher`] that uses this instance's out and err streams.
    // Java: com/google/devtools/build/lib/util/io/OutErr.java:57 OutErr.getSystemPatcher
    pub fn get_system_patcher(&self) -> SystemPatcher {
        SystemPatcher::new(self.out.clone(), self.err.clone())
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
        // PORT: PrintWriter swallows IOExceptions; so do we. `new PrintWriter(out, true)` encodes
        // with the default charset, ISO-8859-1.
        let _ = self
            .out
            .write(&blaze_util::string_encoding::latin1_encode_lossy(s));
        let _ = self.out.flush();
    }

    // Java: com/google/devtools/build/lib/util/io/OutErr.java:129 OutErr.printOutLn
    pub fn print_out_ln(&self, s: &str) {
        self.print_out(&format!("{s}\n"));
    }

    /// Writes the specified string to the error stream, and flushes.
    // Java: com/google/devtools/build/lib/util/io/OutErr.java:134 OutErr.printErr
    pub fn print_err(&self, s: &str) {
        let _ = self
            .err
            .write(&blaze_util::string_encoding::latin1_encode_lossy(s));
        let _ = self.err.flush();
    }

    // Java: com/google/devtools/build/lib/util/io/OutErr.java:140 OutErr.printErrLn
    pub fn print_err_ln(&self, s: &str) {
        self.print_err(&format!("{s}\n"));
    }
}

/// `java.lang.System.out` and `System.err`: the streams that code which writes "to the
/// process's stdout or stderr" uses. Unlike [`OutErr::system_out_err`] (Java's
/// `OutErr.SYSTEM_OUT_ERR`, created once from the streams the JVM started with), these are
/// switched by [`SystemPatcher`] while a command runs (`BlazeCommandDispatcher.execExclusively`),
/// so that their output reaches the command's client through its reporter.
///
/// When nothing is patched they are the process's own stdout and stderr, the same streams as
/// [`OutErr::system_out_err`].
// Java: java/lang/System.java System.out, System.err (JDK)
struct SystemStreams {
    out: Arc<dyn OutputStream>,
    err: Arc<dyn OutputStream>,
}

static SYSTEM_STREAMS: LazyLock<parking_lot::RwLock<SystemStreams>> = LazyLock::new(|| {
    let system_out_err = OutErr::system_out_err();
    parking_lot::RwLock::new(SystemStreams {
        out: system_out_err.get_output_stream().clone(),
        err: system_out_err.get_error_stream().clone(),
    })
});

/// `System.out`: the current standard output stream of this process (see [`SystemPatcher`]).
// Java: java/lang/System.java System.out (JDK)
pub fn system_out() -> Arc<dyn OutputStream> {
    SYSTEM_STREAMS.read().out.clone()
}

/// `System.err`: the current standard error stream of this process (see [`SystemPatcher`]).
// Java: java/lang/System.java System.err (JDK)
pub fn system_err() -> Arc<dyn OutputStream> {
    SYSTEM_STREAMS.read().err.clone()
}

/// `System.err.println(s)`: `s` encoded with the server's default charset (ISO-8859-1, so an
/// internal string leaves as its bytes), then the line separator. A `PrintStream` swallows
/// `IOException`s; so does this.
///
/// PORT: `PrintStream.println(String)` of a `PrintStream` subclass (the patched stream,
/// `SwitchingPrintStream`) writes the string and the newline as two writes; the JVM's own
/// `System.err` writes them as one. Both are two writes here; the bytes are the same.
// Java: java/io/PrintStream.java PrintStream.println (JDK)
pub fn system_err_println(s: &str) {
    let err = system_err();
    let _ = err.write(&blaze_util::string_encoding::latin1_encode_lossy(s));
    let _ = err.write(b"\n");
}

// Java: java/lang/System.java System.setOut (JDK)
fn system_set_out(out: Arc<dyn OutputStream>) {
    SYSTEM_STREAMS.write().out = out;
}

// Java: java/lang/System.java System.setErr (JDK)
fn system_set_err(err: Arc<dyn OutputStream>) {
    SYSTEM_STREAMS.write().err = err;
}

/// Temporarily patches [`system_out`] and [`system_err`] (`System.out`, `System.err`) with
/// custom streams.
///
/// [`SystemPatcher::start`] is called to signal the beginning of the scope of the patch.
/// [`SystemPatcher::close`] ends the scope of the patch, returning `System.out` and `System.err`
/// to what they were when this instance was instantiated.
// Java: com/google/devtools/build/lib/util/io/OutErr.java:68 OutErr.SystemPatcher
pub struct SystemPatcher {
    saved_out: Arc<dyn OutputStream>,
    saved_err: Arc<dyn OutputStream>,
    out_patch: Arc<SwitchingPrintStream>,
    err_patch: Arc<SwitchingPrintStream>,
}

impl SystemPatcher {
    // Java: com/google/devtools/build/lib/util/io/OutErr.java:74 SystemPatcher.<init>
    fn new(override_out: Arc<dyn OutputStream>, override_err: Arc<dyn OutputStream>) -> Self {
        SystemPatcher {
            saved_out: system_out(),
            saved_err: system_err(),
            out_patch: Arc::new(SwitchingPrintStream::new(override_out)),
            err_patch: Arc::new(SwitchingPrintStream::new(override_err)),
        }
    }

    // Java: com/google/devtools/build/lib/util/io/OutErr.java:81 SystemPatcher.start
    pub fn start(&self) {
        system_set_out(self.out_patch.clone());
        system_set_err(self.err_patch.clone());
    }

    // Java: com/google/devtools/build/lib/util/io/OutErr.java:87 SystemPatcher.close
    pub fn close(&self) {
        system_set_out(self.saved_out.clone());
        system_set_err(self.saved_err.clone());
        self.out_patch.switch_back_to(self.saved_out.clone());
        self.err_patch.switch_back_to(self.saved_err.clone());
    }
}

/// Starts by streaming to `override`, then switches back to `saved`.
///
/// The switching strategy is used to guard against memory leaks. For example, if `override` is
/// passed directly to `System.setErr`, anyone may retain a reference to it via `System.err`.
/// Instead, they will get a reference to this class, which frees up `override` in
/// `switch_back_to`.
///
/// A `PrintStream` with `autoFlush`: every write is followed by a flush, and `IOException`s are
/// swallowed (`PrintStream.checkError` is not ported: nothing reads it).
// Java: com/google/devtools/build/lib/util/io/OutErr.java:103 OutErr.SwitchingPrintStream
struct SwitchingPrintStream {
    out: Mutex<Arc<dyn OutputStream>>,
}

impl SwitchingPrintStream {
    // Java: com/google/devtools/build/lib/util/io/OutErr.java:105 SwitchingPrintStream.<init>
    fn new(override_: Arc<dyn OutputStream>) -> Self {
        SwitchingPrintStream {
            out: Mutex::new(override_),
        }
    }

    // Java: com/google/devtools/build/lib/util/io/OutErr.java:109 SwitchingPrintStream.switchBackTo
    fn switch_back_to(&self, saved: Arc<dyn OutputStream>) {
        *self.out.lock() = saved;
    }

    fn current(&self) -> Arc<dyn OutputStream> {
        // PORT: PrintStream writes while holding its own monitor; the stream is read under the
        // lock and written outside it, so that a write that comes back to this stream (a reporter
        // handler writing to System.err) cannot deadlock (Java's monitor is reentrant).
        self.out.lock().clone()
    }
}

impl OutputStream for SwitchingPrintStream {
    // Java: java/io/PrintStream.java PrintStream.write(byte[], int, int) (JDK)
    fn write(&self, bytes: &[u8]) -> io::Result<()> {
        let out = self.current();
        if out.write(bytes).is_ok() {
            // if (autoFlush) out.flush()
            let _ = out.flush();
        }
        Ok(())
    }

    // Java: java/io/PrintStream.java PrintStream.flush (JDK)
    fn flush(&self) -> io::Result<()> {
        let _ = self.current().flush();
        Ok(())
    }

    // Java: java/io/PrintStream.java PrintStream.close (JDK)
    fn close(&self) -> io::Result<()> {
        let _ = self.current().close();
        Ok(())
    }
}

/// The class implements a buffered output stream. By setting up such an output stream, an
/// application can write bytes to the underlying output stream without necessarily causing a
/// call to the underlying system for each byte written. Port of `java.io.BufferedOutputStream`
/// (`BlazeCommandDispatcher.bufferOut` / `bufferErr` wrap the command's streams in it).
///
/// PORT: the JDK's buffer may start smaller and grow up to its size; what reaches the
/// underlying stream (when, and in which pieces) is that of a buffer of the full size.
// Java: java/io/BufferedOutputStream.java BufferedOutputStream (JDK)
pub struct BufferedOutputStream {
    out: Arc<dyn OutputStream>,
    buf: Mutex<Vec<u8>>,
    size: usize,
}

impl BufferedOutputStream {
    /// `DEFAULT_MAX_BUFFER_SIZE`.
    pub const DEFAULT_SIZE: usize = 8192;

    // Java: java/io/BufferedOutputStream.java BufferedOutputStream.<init>(OutputStream) (JDK)
    pub fn new(out: Arc<dyn OutputStream>) -> Self {
        Self::with_size(out, Self::DEFAULT_SIZE)
    }

    // Java: java/io/BufferedOutputStream.java BufferedOutputStream.<init>(OutputStream, int) (JDK)
    pub fn with_size(out: Arc<dyn OutputStream>, size: usize) -> Self {
        blaze_util::check_argument!(size > 0, "Buffer size <= 0");
        BufferedOutputStream {
            out,
            buf: Mutex::new(Vec::with_capacity(size)),
            size,
        }
    }

    /// Flush the internal buffer.
    // Java: java/io/BufferedOutputStream.java BufferedOutputStream.flushBuffer (JDK)
    fn flush_buffer(&self, buf: &mut Vec<u8>) -> io::Result<()> {
        if !buf.is_empty() {
            let r = self.out.write(buf);
            // count = 0 only after a successful write in Java; on failure the bytes stay.
            r?;
            buf.clear();
        }
        Ok(())
    }
}

impl OutputStream for BufferedOutputStream {
    // Java: java/io/BufferedOutputStream.java BufferedOutputStream.write(byte[], int, int) (JDK)
    fn write(&self, b: &[u8]) -> io::Result<()> {
        let mut buf = self.buf.lock();
        if b.len() >= self.size {
            // If the request length exceeds the max size of the output buffer, flush the output
            // buffer and then write the data directly. In this way buffered streams will cascade
            // harmlessly.
            self.flush_buffer(&mut buf)?;
            return self.out.write(b);
        }
        if b.len() > self.size - buf.len() {
            self.flush_buffer(&mut buf)?;
        }
        buf.extend_from_slice(b);
        Ok(())
    }

    // Java: java/io/BufferedOutputStream.java BufferedOutputStream.flush (JDK)
    fn flush(&self) -> io::Result<()> {
        let mut buf = self.buf.lock();
        self.flush_buffer(&mut buf)?;
        self.out.flush()
    }

    // Java: java/io/FilterOutputStream.java FilterOutputStream.close (JDK)
    fn close(&self) -> io::Result<()> {
        let flushed = self.flush();
        let closed = self.out.close();
        flushed.and(closed)
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
    blaze_util::string_encoding::latin1_decode(bytes).into_owned()
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

    static BROKEN_PIPES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    fn count_broken_pipe() {
        BROKEN_PIPES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// A write to a pipe whose reader is gone fails with EPIPE (SIGPIPE is ignored in a Rust
    /// process) and reaches the listener; the error is returned unchanged. Other errors do not.
    #[test]
    fn std_out_err_writer_reports_broken_pipe() {
        set_broken_pipe_listener(count_broken_pipe);
        let (reader, writer) = std::io::pipe().unwrap();
        drop(reader);
        let before = BROKEN_PIPES.load(std::sync::atomic::Ordering::SeqCst);
        let stream = WriteOutputStream::new(StdOutErrWriter(writer));
        let e = stream.write(b"x").unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(
            BROKEN_PIPES.load(std::sync::atomic::Ordering::SeqCst),
            before + 1
        );

        struct Failing;
        impl Write for Failing {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::from(io::ErrorKind::PermissionDenied))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let stream = WriteOutputStream::new(StdOutErrWriter(Failing));
        assert!(stream.write(b"x").is_err());
        assert_eq!(
            BROKEN_PIPES.load(std::sync::atomic::Ordering::SeqCst),
            before + 1
        );
    }

    /// `System.out` / `System.err` go to the patcher's streams between `start` and `close`, and
    /// back to the saved ones after; a stream obtained while patched writes to the saved stream
    /// once the patch is closed (`SwitchingPrintStream.switchBackTo`).
    // Java: com/google/devtools/build/lib/util/io/OutErr.java:68 OutErr.SystemPatcher
    #[test]
    fn system_patcher_switches_system_out_and_err() {
        let before_out = system_out();
        let before_err = system_err();
        let recording = RecordingOutErr::new();
        let patcher = recording.out_err().get_system_patcher();
        patcher.start();
        let retained_err = system_err();
        system_out().write(b"to out").unwrap();
        system_err_println("to err \u{e9}");
        patcher.close();
        assert_eq!(recording.out_as_latin1(), "to out");
        assert_eq!(
            recording.get_error_stream().to_byte_array(),
            b"to err \xe9\n"
        );
        assert!(Arc::ptr_eq(&system_out(), &before_out));
        assert!(Arc::ptr_eq(&system_err(), &before_err));
        // The retained patch now writes to the saved stream (here the process's stderr), not to
        // the recording.
        let _ = retained_err.flush();
        assert_eq!(
            recording.get_error_stream().to_byte_array(),
            b"to err \xe9\n"
        );
    }

    /// `BufferedOutputStream`: small writes are collected until the buffer would overflow or the
    /// stream is flushed; a write of at least the buffer's size goes straight through after the
    /// buffer.
    // Java: java/io/BufferedOutputStream.java BufferedOutputStream.write(byte[], int, int) (JDK)
    #[test]
    fn buffered_output_stream_write_granularity() {
        #[derive(Default)]
        struct Writes(Mutex<Vec<Vec<u8>>>);
        impl OutputStream for Writes {
            fn write(&self, bytes: &[u8]) -> io::Result<()> {
                self.0.lock().push(bytes.to_vec());
                Ok(())
            }
        }
        let writes = Arc::new(Writes::default());
        let buffered = BufferedOutputStream::with_size(writes.clone(), 4);
        buffered.write(b"ab").unwrap();
        buffered.write(b"c").unwrap();
        assert!(writes.0.lock().is_empty());
        buffered.write(b"de").unwrap();
        assert_eq!(*writes.0.lock(), vec![b"abc".to_vec()]);
        buffered.write(b"fghij").unwrap();
        assert_eq!(
            *writes.0.lock(),
            vec![b"abc".to_vec(), b"de".to_vec(), b"fghij".to_vec()]
        );
        buffered.write(b"k").unwrap();
        buffered.flush().unwrap();
        assert_eq!(writes.0.lock().last().unwrap(), b"k");
        buffered.flush().unwrap();
        assert_eq!(writes.0.lock().len(), 4);
    }

    mod out_err_test {
        //! Port of `src/test/java/com/google/devtools/build/lib/util/io/OutErrTest.java`.
        use super::*;

        fn set_up() -> (
            Arc<ByteArrayOutputStream>,
            Arc<ByteArrayOutputStream>,
            OutErr,
        ) {
            let out = Arc::new(ByteArrayOutputStream::new());
            let err = Arc::new(ByteArrayOutputStream::new());
            let out_err = OutErr::create(out.clone(), err.clone());
            (out, err, out_err)
        }

        // Java: src/test/java/com/google/devtools/build/lib/util/io/OutErrTest.java:31 OutErrTest.testRetainsOutErr
        #[test]
        fn test_retains_out_err() {
            let (out, err, out_err) = set_up();
            let out: Arc<dyn OutputStream> = out;
            let err: Arc<dyn OutputStream> = err;
            assert!(Arc::ptr_eq(out_err.get_output_stream(), &out));
            assert!(Arc::ptr_eq(out_err.get_error_stream(), &err));
        }

        // Java: src/test/java/com/google/devtools/build/lib/util/io/OutErrTest.java:37 OutErrTest.testPrintsToOut
        #[test]
        fn test_prints_to_out() {
            let (out, _err, out_err) = set_up();
            out_err.print_out("Hello, world.");
            assert_eq!(latin1(&out.to_byte_array()), "Hello, world.");
        }

        // Java: src/test/java/com/google/devtools/build/lib/util/io/OutErrTest.java:43 OutErrTest.testPrintsToErr
        #[test]
        fn test_prints_to_err() {
            let (_out, err, out_err) = set_up();
            out_err.print_err("Hello, moon.");
            assert_eq!(latin1(&err.to_byte_array()), "Hello, moon.");
        }

        // Java: src/test/java/com/google/devtools/build/lib/util/io/OutErrTest.java:49 OutErrTest.testPrintsToOutWithANewline
        #[test]
        fn test_prints_to_out_with_a_newline() {
            let (out, _err, out_err) = set_up();
            out_err.print_out_ln("With a newline.");
            assert_eq!(latin1(&out.to_byte_array()), "With a newline.\n");
        }

        // Java: src/test/java/com/google/devtools/build/lib/util/io/OutErrTest.java:55 OutErrTest.testPrintsToErrWithANewline
        #[test]
        fn test_prints_to_err_with_a_newline() {
            let (_out, err, out_err) = set_up();
            out_err.print_err_ln("With a newline.");
            assert_eq!(latin1(&err.to_byte_array()), "With a newline.\n");
        }

        // Java: src/test/java/com/google/devtools/build/lib/util/io/OutErrTest.java:61 OutErrTest.testPrintsTwoLinesToOut
        #[test]
        fn test_prints_two_lines_to_out() {
            let (out, _err, out_err) = set_up();
            out_err.print_out_ln("line 1");
            out_err.print_out_ln("line 2");
            assert_eq!(latin1(&out.to_byte_array()), "line 1\nline 2\n");
        }

        // Java: src/test/java/com/google/devtools/build/lib/util/io/OutErrTest.java:68 OutErrTest.testPrintsTwoLinesToErr
        #[test]
        fn test_prints_two_lines_to_err() {
            let (_out, err, out_err) = set_up();
            out_err.print_err_ln("line 1");
            out_err.print_err_ln("line 2");
            assert_eq!(latin1(&err.to_byte_array()), "line 1\nline 2\n");
        }
    }

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

    /// `OutErr.printOut` / `printErr` write through a `PrintWriter` with the server's default
    /// charset, ISO-8859-1: an internal string comes out as its bytes (`é` = C3 A9, U+00FF = FF)
    /// and a char above U+00FF as `?`. Oracle: the `RAWPRINT` line of scripts/bytes.
    // Java: com/google/devtools/build/lib/util/io/OutErr.java:123 OutErr.printOut
    #[test]
    fn print_out_and_err_encode_latin1() {
        let recording = RecordingOutErr::new();
        recording
            .out_err()
            .print_out("\u{c3}\u{a9}\u{f0}\u{9f}\u{90}\u{b7}|\u{ff}|\u{20ac}");
        recording.out_err().print_err_ln("\u{c3}\u{a9}");
        assert_eq!(
            recording.get_output_stream().to_byte_array(),
            b"\xc3\xa9\xf0\x9f\x90\xb7|\xff|?"
        );
        assert_eq!(recording.get_error_stream().to_byte_array(), b"\xc3\xa9\n");
        assert_eq!(recording.err_as_latin1(), "\u{c3}\u{a9}\n");
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

/// An `OutErr` that passes all output to the `OutErr`s that subscribed to it (none: like
/// `/dev/null`).
// Java: com/google/devtools/build/lib/util/io/DelegatingOutErr.java:27 DelegatingOutErr
pub struct DelegatingOutErr {
    out: Arc<DelegatingOutputStream>,
    err: Arc<DelegatingOutputStream>,
}

impl Default for DelegatingOutErr {
    fn default() -> Self {
        Self::new()
    }
}

impl DelegatingOutErr {
    /// Create a new instance to which no sinks have subscribed (basically just like a
    /// `/dev/null`).
    // Java: com/google/devtools/build/lib/util/io/DelegatingOutErr.java:33 DelegatingOutErr.<init>
    pub fn new() -> Self {
        DelegatingOutErr {
            out: Arc::new(DelegatingOutputStream::default()),
            err: Arc::new(DelegatingOutputStream::default()),
        }
    }

    /// Add a sink, that is, after calling this method, `out_err_sink` will receive all output /
    /// errors written to this object.
    // Java: com/google/devtools/build/lib/util/io/DelegatingOutErr.java:50 DelegatingOutErr.addSink
    pub fn add_sink(&self, out_err_sink: &OutErr) {
        self.out.add_sink(out_err_sink.get_output_stream().clone());
        self.err.add_sink(out_err_sink.get_error_stream().clone());
    }

    /// Remove the sink, that is, after calling this method, `out_err_sink` will no longer receive
    /// output / errors written to this object.
    // Java: com/google/devtools/build/lib/util/io/DelegatingOutErr.java:59 DelegatingOutErr.removeSink
    pub fn remove_sink(&self, out_err_sink: &OutErr) {
        self.out.remove_sink(out_err_sink.get_output_stream());
        self.err.remove_sink(out_err_sink.get_error_stream());
    }

    /// This object as the `OutErr` it is in Java.
    pub fn as_out_err(&self) -> OutErr {
        OutErr::create(self.out.clone(), self.err.clone())
    }
}

// Java: com/google/devtools/build/lib/util/io/DelegatingOutErr.java:64 DelegatingOutErr.DelegatingOutputStream
#[derive(Default)]
struct DelegatingOutputStream {
    sinks: Mutex<Vec<Arc<dyn OutputStream>>>,
}

impl DelegatingOutputStream {
    // Java: com/google/devtools/build/lib/util/io/DelegatingOutErr.java:68 DelegatingOutputStream.addSink
    fn add_sink(&self, sink: Arc<dyn OutputStream>) {
        self.sinks.lock().push(sink);
    }

    // Java: com/google/devtools/build/lib/util/io/DelegatingOutErr.java:72 DelegatingOutputStream.removeSink
    fn remove_sink(&self, sink: &Arc<dyn OutputStream>) {
        // List.remove(Object): the first equal element (identity for streams).
        let mut sinks = self.sinks.lock();
        if let Some(i) = sinks
            .iter()
            .position(|s| std::ptr::addr_eq(Arc::as_ptr(s), Arc::as_ptr(sink)))
        {
            sinks.remove(i);
        }
    }

    fn sinks(&self) -> Vec<Arc<dyn OutputStream>> {
        self.sinks.lock().clone()
    }
}

impl OutputStream for DelegatingOutputStream {
    // Java: com/google/devtools/build/lib/util/io/DelegatingOutErr.java:83 DelegatingOutputStream.write
    fn write(&self, bytes: &[u8]) -> io::Result<()> {
        for sink in self.sinks() {
            sink.write(bytes)?;
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/util/io/DelegatingOutErr.java:113 DelegatingOutputStream.flush
    fn flush(&self) -> io::Result<()> {
        for sink in self.sinks() {
            sink.flush()?;
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/util/io/DelegatingOutErr.java:97 DelegatingOutputStream.close
    fn close(&self) -> io::Result<()> {
        // Ensure that we close all sinks even if one throws.
        let mut first_exception = None;
        for sink in self.sinks() {
            if let Err(e) = sink.close()
                && first_exception.is_none()
            {
                first_exception = Some(e);
            }
        }
        match first_exception {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}
