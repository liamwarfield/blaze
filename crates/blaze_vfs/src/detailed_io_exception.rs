//! Port of `vfs/DetailedIOException.java`: an `IOException` that includes a `DetailedExitCode`
//! and a `Transience`. Currently only used for `Filesystem` exceptions.
//!
//! A file system returns it inside a `std::io::Error` (`io::Error::other(DetailedIOException ..)`);
//! code that catches `DetailedIOException` in Java finds it with [`DetailedIOException::find`].
//!
//! PORT: the `FailureDetail` is kept reduced (message plus `Filesystem.Code` enum value name);
//! [`DetailedIOException::get_detailed_exit_code`] rebuilds Java's object. `Transience` lives in
//! blaze_skyframe, which this crate does not depend on, so it is a flag here.

use std::error::Error;
use std::fmt;
use std::io;

use blaze_util::DetailedExitCode;

// Java: com/google/devtools/build/lib/vfs/DetailedIOException.java:27 DetailedIOException
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailedIOException {
    message: String,
    /// The `FailureDetails.Filesystem.Code` enum value name (`"REMOTE_FILE_EVICTED"`).
    filesystem_code: &'static str,
    /// `Transience.TRANSIENT` (true) or `Transience.PERSISTENT` (false).
    transient: bool,
}

impl DetailedIOException {
    // Java: com/google/devtools/build/lib/vfs/DetailedIOException.java:32 DetailedIOException.<init>
    pub fn new(
        message: impl Into<String>,
        filesystem_code: &'static str,
        transient: bool,
    ) -> DetailedIOException {
        DetailedIOException {
            message: message.into(),
            filesystem_code,
            transient,
        }
    }

    /// `e instanceof DetailedIOException`, for an exception carried in a `std::io::Error`.
    pub fn find(e: &io::Error) -> Option<&DetailedIOException> {
        e.get_ref()?.downcast_ref::<DetailedIOException>()
    }

    /// Java `Throwable.getMessage()`, also the `FailureDetail` message.
    pub fn get_message(&self) -> &str {
        &self.message
    }

    /// The `Filesystem.Code` enum value name of the `FailureDetail`.
    pub fn get_filesystem_code(&self) -> &'static str {
        self.filesystem_code
    }

    // Java: com/google/devtools/build/lib/vfs/DetailedIOException.java:44 DetailedIOException.getDetailedExitCode
    pub fn get_detailed_exit_code(&self) -> DetailedExitCode {
        DetailedExitCode::of_category_name(
            self.message.clone(),
            "Filesystem",
            Some(self.filesystem_code),
        )
        .unwrap_or_else(|| {
            panic!(
                "IllegalArgumentException: no Filesystem.Code {}",
                self.filesystem_code
            )
        })
    }

    /// `getTransience() == Transience.TRANSIENT`.
    // Java: com/google/devtools/build/lib/vfs/DetailedIOException.java:48 DetailedIOException.getTransience
    pub fn is_transient(&self) -> bool {
        self.transient
    }
}

impl fmt::Display for DetailedIOException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for DetailedIOException {}

impl From<DetailedIOException> for io::Error {
    fn from(e: DetailedIOException) -> io::Error {
        io::Error::other(e)
    }
}
