//! `RepositoryFunctionException` and `AlreadyReportedRepositoryAccessException`.
//!
//! Java: com/google/devtools/build/lib/bazel/repository/RepositoryFunctionException.java and
//! com/google/devtools/build/lib/skyframe/AlreadyReportedException.java.
//!
//! PORT: Java's `RepositoryFunctionException` is a `SkyFunctionException` subclass. Here it is a
//! plain value ([`RepositoryFunctionException`]) that converts into a
//! [`SkyFunctionException`] whose cause is the wrapped exception itself
//! ([`IOException`], [`EvalError`] or [`AlreadyReportedRepositoryAccessException`]): consumers of
//! REPOSITORY_DIRECTORY catch those cause types with `get_value_or_throw` (Java
//! `PackageLookupFunction` catches `IOException`, `EvalException` and
//! `AlreadyReportedException`).

use std::fmt;

use blaze_sky_loading::io::IOException;
use blaze_sky_loading::io::IOExceptionKind;
use blaze_skyframe::ComputeError;
use blaze_skyframe::SkyFunctionException;
use blaze_skyframe::Transience;
use blaze_starlark::EvalError;

/// Java `Throwable.toString()` of an `IOException`: `<class name>: <message>`.
///
/// PORT: `blaze_sky_loading`'s [`IOException`] does not record `java.io` subclasses such as
/// `FileNotFoundException`; those print as `java.io.IOException`.
fn io_exception_to_java_string(e: &IOException) -> String {
    let class = match e.kind() {
        IOExceptionKind::Plain => "java.io.IOException",
        IOExceptionKind::InconsistentFilesystem => {
            "com.google.devtools.build.lib.io.InconsistentFilesystemException"
        }
        IOExceptionKind::FileSymlinkCycle { .. } => {
            "com.google.devtools.build.lib.io.FileSymlinkCycleException"
        }
        IOExceptionKind::FileSymlinkInfiniteExpansion { .. } => {
            "com.google.devtools.build.lib.io.FileSymlinkInfiniteExpansionException"
        }
    };
    format!("{}: {}", class, e.get_message())
}

/// Encapsulates the exceptions that arise when accessing a repository. Error reporting should ONLY
/// be handled in `RepositoryFetchFunction`.
///
/// `Display` is Java `getMessage()`: the message of the wrapped exception.
///
/// Thrown through Skyframe as its superclass, `blaze_sky_loading`'s
/// [`AlreadyReportedException`](blaze_sky_loading::exceptions::AlreadyReportedException)
/// (keeping this class name), so that `PackageLookupFunction` catches it.
// Java: com/google/devtools/build/lib/bazel/repository/RepositoryFunctionException.java:51 RepositoryFunctionException.AlreadyReportedRepositoryAccessException
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlreadyReportedRepositoryAccessException {
    message: String,
}

impl AlreadyReportedRepositoryAccessException {
    /// `new AlreadyReportedRepositoryAccessException(ioException)`: `super(e.getMessage(),
    /// e.getCause())`.
    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryFunctionException.java:52 RepositoryFunctionException.AlreadyReportedRepositoryAccessException.<init>
    pub fn from_io(e: &IOException) -> Self {
        AlreadyReportedRepositoryAccessException {
            message: e.get_message().to_owned(),
        }
    }

    /// `new AlreadyReportedRepositoryAccessException(evalException)`.
    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryFunctionException.java:52 RepositoryFunctionException.AlreadyReportedRepositoryAccessException.<init>
    pub fn from_eval(e: &EvalError) -> Self {
        AlreadyReportedRepositoryAccessException {
            message: e.message.clone(),
        }
    }

    /// `new AlreadyReportedRepositoryAccessException(e)` for an `EvalException` without a call
    /// stack (e.g. thrown by `Dict.cast` or `close()`), given its message.
    pub fn from_eval_message(message: impl Into<String>) -> Self {
        AlreadyReportedRepositoryAccessException {
            message: message.into(),
        }
    }

    /// Java `getMessage()`.
    pub fn get_message(&self) -> &str {
        &self.message
    }

    /// Java `Throwable.toString()`.
    pub fn to_java_string(&self) -> String {
        format!(
            "com.google.devtools.build.lib.bazel.repository.RepositoryFunctionException$AlreadyReportedRepositoryAccessException: {}",
            self.message
        )
    }
}

impl fmt::Display for AlreadyReportedRepositoryAccessException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AlreadyReportedRepositoryAccessException {}

/// Java: `AlreadyReportedRepositoryAccessException extends AlreadyReportedException`.
impl From<AlreadyReportedRepositoryAccessException>
    for blaze_sky_loading::exceptions::AlreadyReportedException
{
    fn from(e: AlreadyReportedRepositoryAccessException) -> Self {
        blaze_sky_loading::exceptions::AlreadyReportedException::of_subclass(
            e.message,
            "com.google.devtools.build.lib.bazel.repository.RepositoryFunctionException$AlreadyReportedRepositoryAccessException",
        )
    }
}

/// The cause of a [`RepositoryFunctionException`] (Java's three constructors).
#[derive(Clone, Debug)]
pub enum RepositoryFunctionExceptionCause {
    /// Error reading or writing to the filesystem.
    Io(IOException),
    /// An `EvalException`.
    Eval(EvalError),
    /// An error that was already reported.
    AlreadyReported(AlreadyReportedRepositoryAccessException),
}

impl RepositoryFunctionExceptionCause {
    /// Java `cause.toString()`.
    fn to_java_string(&self) -> String {
        match self {
            RepositoryFunctionExceptionCause::Io(e) => io_exception_to_java_string(e),
            RepositoryFunctionExceptionCause::Eval(e) => {
                format!("net.starlark.java.eval.EvalException: {}", e.message)
            }
            RepositoryFunctionExceptionCause::AlreadyReported(e) => e.to_java_string(),
        }
    }
}

/// Exception thrown when something goes wrong accessing a remote repository.
///
/// This exception should be used by child classes to limit the types of exceptions
/// `RepositoryFetchFunction` has to know how to catch.
// Java: com/google/devtools/build/lib/bazel/repository/RepositoryFunctionException.java:31 RepositoryFunctionException
#[derive(Clone, Debug)]
pub struct RepositoryFunctionException {
    cause: RepositoryFunctionExceptionCause,
    transience: Transience,
}

impl RepositoryFunctionException {
    /// Error reading or writing to the filesystem.
    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryFunctionException.java:34 RepositoryFunctionException.<init>
    pub fn io(cause: IOException, transience: Transience) -> Self {
        RepositoryFunctionException {
            cause: RepositoryFunctionExceptionCause::Io(cause),
            transience,
        }
    }

    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryFunctionException.java:38 RepositoryFunctionException.<init>
    pub fn eval(cause: EvalError, transience: Transience) -> Self {
        RepositoryFunctionException {
            cause: RepositoryFunctionExceptionCause::Eval(cause),
            transience,
        }
    }

    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryFunctionException.java:42 RepositoryFunctionException.<init>
    pub fn already_reported(
        cause: AlreadyReportedRepositoryAccessException,
        transience: Transience,
    ) -> Self {
        RepositoryFunctionException {
            cause: RepositoryFunctionExceptionCause::AlreadyReported(cause),
            transience,
        }
    }

    /// `new RepositoryFunctionException(new IOException(message), transience)`.
    pub fn io_message(message: impl Into<String>, transience: Transience) -> Self {
        Self::io(IOException::new(message), transience)
    }

    /// `new RepositoryFunctionException(e, transience)` for an `IOException` thrown by the file
    /// system.
    pub fn from_io_error(e: &std::io::Error, transience: Transience) -> Self {
        Self::io(IOException::from_io(e), transience)
    }

    // Java: com/google/devtools/build/skyframe/SkyFunctionException.java:80 SkyFunctionException.getCause
    pub fn get_cause(&self) -> &RepositoryFunctionExceptionCause {
        &self.cause
    }

    // Java: com/google/devtools/build/skyframe/SkyFunctionException.java:70 SkyFunctionException.isTransient
    pub fn is_transient(&self) -> bool {
        self.transience == Transience::Transient
    }

    pub fn transience(&self) -> Transience {
        self.transience
    }

    /// Java `getMessage()`: `SkyFunctionException(cause)` calls `Exception(Throwable cause)`, so
    /// the message is `cause.toString()` (e.g. `java.io.IOException: <message>`).
    pub fn get_message(&self) -> String {
        self.cause.to_java_string()
    }

    /// The exception Skyframe stores: the cause, with this exception's transience.
    pub fn into_sky_function_exception(self) -> SkyFunctionException {
        match self.cause {
            RepositoryFunctionExceptionCause::Io(e) => {
                SkyFunctionException::new(e, self.transience)
            }
            RepositoryFunctionExceptionCause::Eval(e) => {
                SkyFunctionException::new(e, self.transience)
            }
            RepositoryFunctionExceptionCause::AlreadyReported(e) => SkyFunctionException::new(
                blaze_sky_loading::exceptions::AlreadyReportedException::from(e),
                self.transience,
            ),
        }
    }
}

impl fmt::Display for RepositoryFunctionException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.get_message())
    }
}

impl From<RepositoryFunctionException> for SkyFunctionException {
    fn from(e: RepositoryFunctionException) -> Self {
        e.into_sky_function_exception()
    }
}

impl From<RepositoryFunctionException> for ComputeError {
    fn from(e: RepositoryFunctionException) -> Self {
        ComputeError::Exception(e.into_sky_function_exception())
    }
}
