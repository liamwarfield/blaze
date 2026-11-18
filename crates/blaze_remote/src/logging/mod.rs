//! Port of `com.google.devtools.build.lib.remote.logging`: the client interceptor behind
//! `--remote_grpc_log` and its per-method handlers.
//!
//! The sink of the log is Java's `AsynchronousMessageOutputStream<LogEntry>` (`lib/util/io`);
//! [`LogEntrySink`] is what the interceptor uses of it.

pub mod execute_handler;
pub mod find_missing_blobs_handler;
pub mod get_action_result_handler;
pub mod get_capabilities_handler;
pub mod logging_handler;
pub mod logging_interceptor;
pub mod query_write_status_handler;
pub mod read_handler;
pub mod splice_blob_handler;
pub mod split_blob_handler;
pub mod update_action_result_handler;
pub mod wait_execution_handler;
pub mod write_handler;

use std::fmt;
use std::io;

use blaze_exec::exec::io::AsynchronousMessageOutputStream;
use blaze_vfs::Path;
use parking_lot::{Mutex, RwLock};

pub use execute_handler::ExecuteHandler;
pub use find_missing_blobs_handler::FindMissingBlobsHandler;
pub use get_action_result_handler::GetActionResultHandler;
pub use get_capabilities_handler::GetCapabilitiesHandler;
pub use logging_handler::{LoggingHandler, RawLoggingHandler, raw};
pub use logging_interceptor::{HandlerSelector, LoggingInterceptor};
pub use query_write_status_handler::QueryWriteStatusHandler;
pub use read_handler::ReadHandler;
pub use splice_blob_handler::SpliceBlobHandler;
pub use split_blob_handler::SplitBlobHandler;
pub use update_action_result_handler::UpdateActionResultHandler;
pub use wait_execution_handler::WaitExecutionHandler;
pub use write_handler::WriteHandler;

use crate::reapi::remote_logging::LogEntry;

/// The `IllegalStateException` of `AsynchronousMessageOutputStream.write` for a write after the
/// stream was closed.
///
/// PORT: Java throws it (a `RuntimeException`) and `LoggingInterceptor` catches it; an unchecked
/// exception that is caught is an `Err` here, not a panic.
// Java: com/google/devtools/build/lib/util/io/AsynchronousMessageOutputStream.java:105 AsynchronousMessageOutputStream.write
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IllegalStateException;

impl fmt::Display for IllegalStateException {
    /// `Throwable.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("java.lang.IllegalStateException")
    }
}

impl std::error::Error for IllegalStateException {}

/// What [`LoggingInterceptor`] uses of `AsynchronousMessageOutputStream<LogEntry>`: an output
/// stream of length-delimited [`LogEntry`] messages. Thread-safe.
// Java: com/google/devtools/build/lib/util/io/AsynchronousMessageOutputStream.java:40 AsynchronousMessageOutputStream
pub trait LogEntrySink: Send + Sync {
    /// Writes a protocol buffer message in the same format as `MessageLite.writeDelimitedTo`.
    ///
    /// The writes land in the output in the same order that they were called. `Err` is the
    /// `RuntimeException` of a write after [`LogEntrySink::close`].
    // Java: com/google/devtools/build/lib/util/io/AsynchronousMessageOutputStream.java:96 AsynchronousMessageOutputStream.write
    fn write(&self, m: &LogEntry) -> Result<(), IllegalStateException>;

    /// Closes the stream and blocks until all pending writes are completed.
    ///
    /// Returns an error if any of the writes or the close itself have failed.
    // Java: com/google/devtools/build/lib/util/io/AsynchronousMessageOutputStream.java:129 AsynchronousMessageOutputStream.close
    fn close(&self) -> io::Result<()>;
}

/// The file-backed sink: `new AsynchronousMessageOutputStream<>(path)`.
///
/// PORT: wraps the port of `AsynchronousMessageOutputStream` in `blaze_exec` (a writer thread
/// that appends the delimited entries in order and flushes on close). That port panics with
/// `IllegalStateException` on a write after close, so this wrapper tracks the closed state and
/// reports it as the `Err` the interceptor expects. Java's window between `closeAsync()` and the
/// completion of the close future, where an entry is queued and never written, also reports
/// `Err` here; in both cases the entry is dropped.
// Java: com/google/devtools/build/lib/util/io/AsynchronousMessageOutputStream.java:40 AsynchronousMessageOutputStream
pub struct FileLogEntrySink {
    stream: AsynchronousMessageOutputStream<LogEntry>,
    /// Whether `close()` was called. Writers hold the read side while they enqueue.
    closed: RwLock<bool>,
}

impl FileLogEntrySink {
    /// `new AsynchronousMessageOutputStream<>(path)`: a 100 kB buffered stream to the file.
    // Java: com/google/devtools/build/lib/util/io/AsynchronousMessageOutputStream.java:52 AsynchronousMessageOutputStream.<init>
    pub fn new(path: &Path) -> io::Result<FileLogEntrySink> {
        Ok(FileLogEntrySink {
            stream: AsynchronousMessageOutputStream::new_for_path(path)?,
            closed: RwLock::new(false),
        })
    }
}

impl LogEntrySink for FileLogEntrySink {
    fn write(&self, m: &LogEntry) -> Result<(), IllegalStateException> {
        let closed = self.closed.read();
        if *closed {
            // Attempted to write after closing.
            return Err(IllegalStateException);
        }
        self.stream.write(m);
        Ok(())
    }

    fn close(&self) -> io::Result<()> {
        *self.closed.write() = true;
        self.stream.close()
    }
}

/// A sink that keeps the entries in memory (tests; Java mocks the stream with Mockito).
#[derive(Debug, Default)]
pub struct InMemoryLogEntrySink {
    entries: Mutex<Vec<LogEntry>>,
    closed: Mutex<bool>,
}

impl InMemoryLogEntrySink {
    /// An empty, open sink.
    pub fn new() -> InMemoryLogEntrySink {
        InMemoryLogEntrySink::default()
    }

    /// The entries written so far, in order.
    pub fn entries(&self) -> Vec<LogEntry> {
        self.entries.lock().clone()
    }

    /// Whether [`LogEntrySink::close`] was called.
    pub fn is_closed(&self) -> bool {
        *self.closed.lock()
    }

    /// The bytes a file-backed sink would hold: every entry as `writeDelimitedTo` writes it.
    pub fn to_delimited_bytes(&self) -> Vec<u8> {
        use blaze_protos::JavaMessage as _;
        let mut out = Vec::new();
        for entry in self.entries.lock().iter() {
            entry.write_delimited_to(&mut out);
        }
        out
    }
}

impl LogEntrySink for InMemoryLogEntrySink {
    fn write(&self, m: &LogEntry) -> Result<(), IllegalStateException> {
        if *self.closed.lock() {
            // Attempted to write after closing.
            return Err(IllegalStateException);
        }
        self.entries.lock().push(m.clone());
        Ok(())
    }

    fn close(&self) -> io::Result<()> {
        *self.closed.lock() = true;
        Ok(())
    }
}
