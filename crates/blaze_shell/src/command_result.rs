//! Port of `com.google.devtools.build.lib.shell.CommandResult`.

use std::fmt;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;

use crate::termination_status::TerminationStatus;

/// Port of the `java.io.ByteArrayOutputStream` values a [`CommandResult`] carries, including the
/// two sentinels `CommandResult.EMPTY_OUTPUT` and `CommandResult.NO_OUTPUT_COLLECTED`.
///
/// A `Buffer` is shared: the accumulating consumer keeps writing into it (Java shares the stream
/// object the same way).
#[derive(Clone)]
pub enum ByteArrayOutputStream {
    /// A real, growable buffer.
    Buffer(Arc<Mutex<Vec<u8>>>),
    /// `CommandResult.EMPTY_OUTPUT`: `toByteArray()` is empty.
    EmptyOutput,
    /// `CommandResult.NO_OUTPUT_COLLECTED`: `toByteArray()` throws.
    NoOutputCollected,
}

impl ByteArrayOutputStream {
    /// `new ByteArrayOutputStream()`.
    pub fn new() -> ByteArrayOutputStream {
        ByteArrayOutputStream::Buffer(Arc::new(Mutex::new(Vec::new())))
    }

    /// `toByteArray()`.
    // Java: com/google/devtools/build/lib/shell/CommandResult.java:70 CommandResult.NO_OUTPUT_COLLECTED
    pub fn to_byte_array(&self) -> Vec<u8> {
        match self {
            ByteArrayOutputStream::Buffer(b) => b.lock().clone(),
            ByteArrayOutputStream::EmptyOutput => Vec::new(),
            ByteArrayOutputStream::NoOutputCollected => {
                panic!("IllegalStateException: Output was not collected")
            }
        }
    }

    /// Whether this is the `NO_OUTPUT_COLLECTED` sentinel (Java compares by identity).
    pub fn is_no_output_collected(&self) -> bool {
        matches!(self, ByteArrayOutputStream::NoOutputCollected)
    }

    /// `write(byte[], int, int)`.
    pub fn write_bytes(&self, bytes: &[u8]) {
        match self {
            ByteArrayOutputStream::Buffer(b) => b.lock().extend_from_slice(bytes),
            // The sentinels are never written to in Bazel.
            _ => unreachable!("write to a CommandResult sentinel stream"),
        }
    }
}

impl Default for ByteArrayOutputStream {
    fn default() -> Self {
        ByteArrayOutputStream::new()
    }
}

impl io::Write for ByteArrayOutputStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.write_bytes(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl fmt::Display for ByteArrayOutputStream {
    /// `ByteArrayOutputStream.toString()`: the bytes decoded with the platform charset (UTF-8).
    /// The sentinels override only `toByteArray`, so their `toString()` is empty.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ByteArrayOutputStream::Buffer(b) => f.write_str(&String::from_utf8_lossy(&b.lock())),
            _ => Ok(()),
        }
    }
}

impl fmt::Debug for ByteArrayOutputStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ByteArrayOutputStream::Buffer(b) => write!(f, "Buffer({} bytes)", b.lock().len()),
            ByteArrayOutputStream::EmptyOutput => f.write_str("EMPTY_OUTPUT"),
            ByteArrayOutputStream::NoOutputCollected => f.write_str("NO_OUTPUT_COLLECTED"),
        }
    }
}

/// Encapsulates the results of a command execution, including exit status and output to stdout
/// and stderr.
// Java: com/google/devtools/build/lib/shell/CommandResult.java:41 CommandResult
#[derive(Clone, Debug)]
pub struct CommandResult {
    stdout_stream: ByteArrayOutputStream,
    stderr_stream: ByteArrayOutputStream,
    termination_status: TerminationStatus,
    wall_execution_time: Option<Duration>,
    user_execution_time: Option<Duration>,
    system_execution_time: Option<Duration>,
}

impl CommandResult {
    pub const EMPTY_OUTPUT: ByteArrayOutputStream = ByteArrayOutputStream::EmptyOutput;
    pub const NO_OUTPUT_COLLECTED: ByteArrayOutputStream = ByteArrayOutputStream::NoOutputCollected;

    /// Returns the stdout stream.
    pub fn stdout_stream(&self) -> &ByteArrayOutputStream {
        &self.stdout_stream
    }

    /// Returns the stderr stream.
    pub fn stderr_stream(&self) -> &ByteArrayOutputStream {
        &self.stderr_stream
    }

    /// Returns the termination status of the subprocess.
    pub fn termination_status(&self) -> &TerminationStatus {
        &self.termination_status
    }

    pub fn wall_execution_time(&self) -> Option<Duration> {
        self.wall_execution_time
    }

    pub fn user_execution_time(&self) -> Option<Duration> {
        self.user_execution_time
    }

    pub fn system_execution_time(&self) -> Option<Duration> {
        self.system_execution_time
    }

    /// Returns the stdout as a byte array. Panics (Java `IllegalStateException`) if output was
    /// not collected.
    // Java: com/google/devtools/build/lib/shell/CommandResult.java:86 CommandResult.getStdout
    pub fn get_stdout(&self) -> Vec<u8> {
        self.stdout_stream.to_byte_array()
    }

    /// Returns the stderr as a byte array. Panics (Java `IllegalStateException`) if output was
    /// not collected.
    // Java: com/google/devtools/build/lib/shell/CommandResult.java:97 CommandResult.getStderr
    pub fn get_stderr(&self) -> Vec<u8> {
        self.stderr_stream.to_byte_array()
    }

    // Java: com/google/devtools/build/lib/shell/CommandResult.java:101 CommandResult.logThis
    pub(crate) fn log_this(&self) {
        // PORT: skipped, FINER-level java.log logging.
    }

    /// Returns a new builder.
    // Java: com/google/devtools/build/lib/shell/CommandResult.java:112 CommandResult.builder
    pub fn builder() -> CommandResultBuilder {
        CommandResultBuilder::default()
    }
}

/// A builder for [`CommandResult`]s (an `@AutoBuilder` in Java).
// Java: com/google/devtools/build/lib/shell/CommandResult.java:118 CommandResult.Builder
#[derive(Clone, Debug, Default)]
pub struct CommandResultBuilder {
    stdout_stream: Option<ByteArrayOutputStream>,
    stderr_stream: Option<ByteArrayOutputStream>,
    termination_status: Option<TerminationStatus>,
    wall_execution_time: Option<Duration>,
    user_execution_time: Option<Duration>,
    system_execution_time: Option<Duration>,
}

impl CommandResultBuilder {
    pub fn set_stdout_stream(&mut self, stdout: ByteArrayOutputStream) -> &mut Self {
        self.stdout_stream = Some(stdout);
        self
    }

    pub fn set_stderr_stream(&mut self, stderr: ByteArrayOutputStream) -> &mut Self {
        self.stderr_stream = Some(stderr);
        self
    }

    pub fn set_termination_status(&mut self, termination_status: TerminationStatus) -> &mut Self {
        self.termination_status = Some(termination_status);
        self
    }

    pub fn set_wall_execution_time(&mut self, d: Duration) -> &mut Self {
        self.wall_execution_time = Some(d);
        self
    }

    /// Sets or clears the wall execution time.
    pub fn set_wall_execution_time_opt(&mut self, d: Option<Duration>) -> &mut Self {
        self.wall_execution_time = d;
        self
    }

    pub fn set_user_execution_time(&mut self, d: Duration) -> &mut Self {
        self.user_execution_time = Some(d);
        self
    }

    /// Sets or clears the user execution time.
    pub fn set_user_execution_time_opt(&mut self, d: Option<Duration>) -> &mut Self {
        self.user_execution_time = d;
        self
    }

    pub fn set_system_execution_time(&mut self, d: Duration) -> &mut Self {
        self.system_execution_time = Some(d);
        self
    }

    /// Sets or clears the system execution time.
    pub fn set_system_execution_time_opt(&mut self, d: Option<Duration>) -> &mut Self {
        self.system_execution_time = d;
        self
    }

    /// Builds a [`CommandResult`]. Panics like AutoValue's generated `build()` when a required
    /// property is missing.
    pub fn build(&self) -> CommandResult {
        if self.stdout_stream.is_none()
            || self.stderr_stream.is_none()
            || self.termination_status.is_none()
        {
            let mut missing = String::new();
            if self.stdout_stream.is_none() {
                missing.push_str(" stdoutStream");
            }
            if self.stderr_stream.is_none() {
                missing.push_str(" stderrStream");
            }
            if self.termination_status.is_none() {
                missing.push_str(" terminationStatus");
            }
            panic!("IllegalStateException: Missing required properties:{missing}");
        }
        CommandResult {
            stdout_stream: self.stdout_stream.clone().unwrap(),
            stderr_stream: self.stderr_stream.clone().unwrap(),
            termination_status: self.termination_status.clone().unwrap(),
            wall_execution_time: self.wall_execution_time,
            user_execution_time: self.user_execution_time,
            system_execution_time: self.system_execution_time,
        }
    }
}
