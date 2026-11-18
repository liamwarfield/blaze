//! Port of `com.google.devtools.build.lib.shell.InputStreamSink`.

use std::io::{self, Read, Write};
use std::sync::Arc;

use parking_lot::Mutex;

/// A thread-safe `java.io.OutputStream`: Java callers hand `Command` streams that "must be
/// thread-safe"; in Rust that is a shared, locked writer. The same stream may be passed for both
/// stdout and stderr.
pub type OutputStream = Arc<Mutex<dyn Write + Send>>;

/// A `java.io.InputStream` owned by the reader.
pub type InputStream = Box<dyn Read + Send>;

/// A sink task (a Java `Runnable` that tunnels `IOException`s): runs until end-of-file.
pub type RunnableSink = Box<dyn FnOnce() -> io::Result<()> + Send>;

/// Provides sinks for input streams. Continuously read an input stream until the end-of-file is
/// encountered. The stream may be redirected to an `OutputStream`, or discarded.
// Java: com/google/devtools/build/lib/shell/InputStreamSink.java:34 InputStreamSink
pub struct InputStreamSink;

fn read_retrying(input: &mut dyn Read, buffer: &mut [u8]) -> io::Result<usize> {
    loop {
        match input.read(buffer) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            r => return r,
        }
    }
}

impl InputStreamSink {
    /// Creates a sink which consumes the provided input stream, discarding its contents.
    // Java: com/google/devtools/build/lib/shell/InputStreamSink.java:111 InputStreamSink.newRunnableSink
    pub fn new_runnable_sink_discarding(mut input: InputStream) -> RunnableSink {
        // Java: com/google/devtools/build/lib/shell/InputStreamSink.java:58 NullSink.run
        // PORT: Java first tries InputStream.skip; reading into a scratch buffer has the same
        // observable effect.
        Box::new(move || {
            let mut discard = [0u8; 4096];
            while read_retrying(&mut *input, &mut discard)? != 0 {}
            Ok(())
        })
    }

    /// Creates a sink which copies everything from `input` to `out`. `out` will be written to and
    /// flushed after each read from `input`. However, `out` will not be closed.
    // Java: com/google/devtools/build/lib/shell/InputStreamSink.java:123 InputStreamSink.newRunnableSink
    pub fn new_runnable_sink(mut input: InputStream, out: OutputStream) -> RunnableSink {
        // Java: com/google/devtools/build/lib/shell/InputStreamSink.java:93 CopySink.run
        Box::new(move || {
            let mut buffer = [0u8; 2048];
            loop {
                let bytes_read = read_retrying(&mut *input, &mut buffer)?;
                if bytes_read == 0 {
                    return Ok(());
                }
                let mut out = out.lock();
                out.write_all(&buffer[..bytes_read])?;
                out.flush()?;
            }
        })
    }
}
