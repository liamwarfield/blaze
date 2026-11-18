//! Port of `packages/Globber.java`.

/// An opaque token for fetching the result of a glob computation (Java `Globber.Token`).
// Java: com/google/devtools/build/lib/packages/Globber.java:22 Globber.Token
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlobberToken(pub u64);

/// The kind of globbing operation (Java `Globber.Operation`).
// Java: com/google/devtools/build/lib/packages/Globber.java:25 Globber.Operation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GlobberOperation {
    /// Return only files.
    Files,
    /// Return files and directories, but not sub-packages.
    FilesAndDirs,
    /// Return only sub-packages.
    Subpackages,
}

/// An invalid glob pattern (Java `Globber.BadGlobException`).
// Java: com/google/devtools/build/lib/packages/Globber.java:35 Globber.BadGlobException
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct BadGlobException(pub String);

/// Errors of [`Globber::fetch_unsorted`] (Java: BadGlobException, IOException,
/// InterruptedException).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GlobberError {
    #[error("{0}")]
    BadGlob(#[from] BadGlobException),
    #[error("{0}")]
    Io(String),
    #[error("interrupted")]
    Interrupted,
}

/// Runs glob computations for `glob()` and `subpackages()` (Java `Globber`).
// Java: com/google/devtools/build/lib/packages/Globber.java:20 Globber
pub trait Globber: Send + Sync {
    /// Starts the given glob computation and returns a token for fetching the result.
    // Java: com/google/devtools/build/lib/packages/Globber.java:47 Globber.runAsync
    fn run_async(
        &self,
        includes: &[String],
        excludes: &[String],
        operation: GlobberOperation,
        allow_empty: bool,
    ) -> Result<GlobberToken, GlobberError>;

    /// Fetches the result of a started computation, in arbitrary order.
    // Java: com/google/devtools/build/lib/packages/Globber.java:55 Globber.fetchUnsorted
    fn fetch_unsorted(&self, token: GlobberToken) -> Result<Vec<String>, GlobberError>;

    // Java: com/google/devtools/build/lib/packages/Globber.java:59 Globber.onInterrupt
    fn on_interrupt(&self);

    // Java: com/google/devtools/build/lib/packages/Globber.java:62 Globber.onCompletion
    fn on_completion(&self);
}
