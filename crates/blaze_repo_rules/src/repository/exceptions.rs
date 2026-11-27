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
use std::io;

use blaze_repo_fetch::errors::IoException;
use blaze_repo_fetch::errors::RepositoryFunctionCause as FetchRepositoryFunctionCause;
use blaze_repo_fetch::errors::RepositoryFunctionException as FetchRepositoryFunctionException;
use blaze_sky_loading::io::IOException;
use blaze_sky_loading::io::IOExceptionKind;
use blaze_skyframe::ComputeError;
use blaze_skyframe::SkyFunctionException;
use blaze_skyframe::Transience;
use blaze_starlark::EvalError;
use blaze_vfs::FileSymlinkLoopException;
use blaze_vfs::NotASymlinkException;

/// `java.io.IOException`.
const IO_EXCEPTION: &str = "java.io.IOException";
/// `java.io.FileNotFoundException`.
const FILE_NOT_FOUND_EXCEPTION: &str = "java.io.FileNotFoundException";
/// `FileAccessException`.
// Java: com/google/devtools/build/lib/vfs/FileAccessException.java:30 FileAccessException
const FILE_ACCESS_EXCEPTION: &str = "com.google.devtools.build.lib.vfs.FileAccessException";
/// `InvalidArgumentIOException`.
// Java: com/google/devtools/build/lib/unix/InvalidArgumentIOException.java:23 InvalidArgumentIOException
const INVALID_ARGUMENT_IO_EXCEPTION: &str =
    "com.google.devtools.build.lib.unix.InvalidArgumentIOException";
/// `FileSymlinkLoopException`.
// Java: com/google/devtools/build/lib/vfs/FileSymlinkLoopException.java:20 FileSymlinkLoopException
const FILE_SYMLINK_LOOP_EXCEPTION: &str =
    "com.google.devtools.build.lib.vfs.FileSymlinkLoopException";
/// `FileSystem.NotASymlinkException` (`Class.getName()` of a nested class).
// Java: com/google/devtools/build/lib/vfs/FileSystem.java:76 FileSystem.NotASymlinkException
const NOT_A_SYMLINK_EXCEPTION: &str =
    "com.google.devtools.build.lib.vfs.FileSystem$NotASymlinkException";

// Java: com/google/devtools/build/lib/vfs/FileSystem.java:63 FileSystem.ERR_PERMISSION_DENIED
const ERR_PERMISSION_DENIED: &str = " (Permission denied)";
/// `strerror(EINVAL)` as the native layer appends it.
const ERR_INVALID_ARGUMENT: &str = " (Invalid argument)";

/// The Java class of [`blaze_sky_loading`]'s [`IOException`].
///
/// PORT: that type does not record `java.io` subclasses such as `FileNotFoundException`; those
/// print as `java.io.IOException`.
fn io_exception_class_name(e: &IOException) -> &'static str {
    match e.kind() {
        IOExceptionKind::Plain => IO_EXCEPTION,
        IOExceptionKind::InconsistentFilesystem => {
            "com.google.devtools.build.lib.io.InconsistentFilesystemException"
        }
        IOExceptionKind::FileSymlinkCycle { .. } => {
            "com.google.devtools.build.lib.io.FileSymlinkCycleException"
        }
        IOExceptionKind::FileSymlinkInfiniteExpansion { .. } => {
            "com.google.devtools.build.lib.io.FileSymlinkInfiniteExpansionException"
        }
        IOExceptionKind::Detailed(_) => "com.google.devtools.build.lib.vfs.DetailedIOException",
    }
}

/// The Java class of the exception Bazel's native file system layer throws for `e`, an error of
/// a `blaze_vfs` operation other than opening a stream.
///
/// The native layer picks the class from `errno`: `ENOENT` is `FileNotFoundException`, `EACCES`
/// `FileAccessException`, `EINVAL` `InvalidArgumentIOException`, `ELOOP`
/// `FileSymlinkLoopException`, `ETIMEDOUT` `SocketTimeoutException`, and the rest `IOException`.
///
/// PORT: `blaze_vfs` keeps the `io::ErrorKind` and the `<path> (<strerror>)` message but not
/// `errno`, so `EACCES` and `EINVAL` are recognised by the `strerror` suffix (as
/// `DiskBackedFileSystem.getInputStream` itself does for "Permission denied"). `EFAULT`,
/// `ENOSYS` and `ENOTSUP` are unchecked exceptions in Java (a crash); they print as
/// `java.io.IOException` here.
// Java: (native) src/main/native/unix_jni.cc:81 PostException
fn vfs_exception_class_name(e: &io::Error) -> &'static str {
    if FileSymlinkLoopException::is(e) {
        return FILE_SYMLINK_LOOP_EXCEPTION;
    }
    if NotASymlinkException::is(e) {
        return NOT_A_SYMLINK_EXCEPTION;
    }
    let message = e.to_string();
    match e.kind() {
        io::ErrorKind::NotFound => FILE_NOT_FOUND_EXCEPTION,
        io::ErrorKind::PermissionDenied if message.ends_with(ERR_PERMISSION_DENIED) => {
            FILE_ACCESS_EXCEPTION
        }
        io::ErrorKind::InvalidInput if message.ends_with(ERR_INVALID_ARGUMENT) => {
            INVALID_ARGUMENT_IO_EXCEPTION
        }
        io::ErrorKind::TimedOut => "java.net.SocketTimeoutException",
        _ => IO_EXCEPTION,
    }
}

/// A Java checked exception reduced to what `Throwable.toString()` and `getMessage()` print: its
/// class and its message.
///
/// The text that Starlark code and the user see for a failed `repository_ctx` / `module_ctx`
/// method depends on the *class* of the Java exception:
///
/// - `throw new RepositoryFunctionException(cause, transience)`: `SkyFunctionException(cause)`
///   calls `Exception(Throwable cause)`, so the message is `cause.toString()`:
///   `java.io.IOException: <message>`, `java.io.FileNotFoundException: <message>`, ... but only
///   `<message>` for an `EvalException` without a call stack, whose `toString()` is
///   `getMessageWithStack()`.
/// - a checked exception escaping a `@StarlarkMethod` becomes `new EvalException(e)`, whose
///   message is `e.getMessage()` (`e.toString()` when the message is null).
///
/// PORT: the Rust error types standing for Java's `IOException`s do not all record the Java class
/// (an `std::io::Error` from `blaze_vfs` has an `ErrorKind` and a message). The conversions into
/// this type are the one place that decides which class name a failure prints.
// Java: com/google/devtools/build/skyframe/SkyFunctionException.java:64 SkyFunctionException.<init>
// Java: net/starlark/java/eval/MethodDescriptor.java:295 MethodDescriptor.call
// Java: net/starlark/java/eval/EvalException.java:89 EvalException.getCauseMessage
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JavaThrowable {
    /// `getClass().getName()`; `None` for an `EvalException` without a call stack (its
    /// `toString()` is the message alone).
    class_name: Option<&'static str>,
    /// `getMessage()`; `None` is Java `null`.
    message: Option<String>,
}

impl JavaThrowable {
    /// `new IOException(message)` (also `new IOException(message, cause)`: the cause does not
    /// print).
    pub fn io_exception(message: impl Into<String>) -> Self {
        JavaThrowable {
            class_name: Some(IO_EXCEPTION),
            message: Some(message.into()),
        }
    }

    /// An exception of the class with the given `Class.getName()`.
    pub fn of_class(class_name: &'static str, message: Option<String>) -> Self {
        JavaThrowable {
            class_name: Some(class_name),
            message,
        }
    }

    /// `Starlark.errorf(...)` / `new EvalException(message)`: an `EvalException` that has not
    /// been thrown through a Starlark call, so it has no call stack.
    // Java: net/starlark/java/eval/EvalException.java:135 EvalException.toString
    pub fn eval_exception(message: impl Into<String>) -> Self {
        JavaThrowable {
            class_name: None,
            message: Some(message.into()),
        }
    }

    /// The exception `FileSystem.getInputStream` / `getOutputStream` throws when opening fails:
    /// `java.io.FileInputStream` and `FileOutputStream` throw `FileNotFoundException` "if opening
    /// fails for any reason, including permissions", which Bazel replaces by a
    /// `FileAccessException` when the message ends with " (Permission denied)".
    // Java: com/google/devtools/build/lib/vfs/DiskBackedFileSystem.java:64 DiskBackedFileSystem.getInputStream
    // Java: com/google/devtools/build/lib/vfs/DiskBackedFileSystem.java:90 DiskBackedFileSystem.getOutputStream
    pub fn from_stream_open(e: &io::Error) -> Self {
        let message = e.to_string();
        let class_name = if message.ends_with(ERR_PERMISSION_DENIED) {
            FILE_ACCESS_EXCEPTION
        } else {
            FILE_NOT_FOUND_EXCEPTION
        };
        JavaThrowable {
            class_name: Some(class_name),
            message: Some(message),
        }
    }

    /// The `IOException` a read from or write to an open stream throws (`java.io.IOException`
    /// from `FileInputStream.read` / `FileOutputStream.write`, whose message is
    /// `strerror(errno)`).
    pub fn from_stream_io(e: &io::Error) -> Self {
        // std prints an OS error as "<strerror> (os error <errno>)".
        let text = e.to_string();
        let message = match text.rfind(" (os error ") {
            Some(i) if e.raw_os_error().is_some() => text[..i].to_owned(),
            _ => text,
        };
        JavaThrowable::io_exception(message)
    }

    /// Java `getMessage()`, with Java's `"null"` string conversion for a missing message (as in
    /// `"...: " + e.getMessage()`).
    pub fn get_message(&self) -> String {
        self.message.clone().unwrap_or_else(|| "null".to_owned())
    }

    /// Java `Throwable.toString()`: `<class name>: <message>`, or the class name alone when the
    /// message is null; for an `EvalException` without a call stack, the message.
    // Java: net/starlark/java/eval/EvalException.java:135 EvalException.toString
    pub fn to_java_string(&self) -> String {
        match (self.class_name, &self.message) {
            (Some(class_name), Some(message)) => format!("{}: {}", class_name, message),
            (Some(class_name), None) => class_name.to_owned(),
            (None, message) => message.clone().unwrap_or_default(),
        }
    }
}

impl From<&IoException> for JavaThrowable {
    /// A `blaze_repo_fetch` exception: it records its Java class.
    fn from(e: &IoException) -> Self {
        JavaThrowable {
            class_name: Some(e.kind.java_class_name()),
            message: e.message.clone(),
        }
    }
}

impl From<IoException> for JavaThrowable {
    fn from(e: IoException) -> Self {
        JavaThrowable::from(&e)
    }
}

impl From<&IOException> for JavaThrowable {
    /// A `blaze_sky_loading` exception.
    fn from(e: &IOException) -> Self {
        JavaThrowable {
            class_name: Some(io_exception_class_name(e)),
            message: Some(e.get_message().to_owned()),
        }
    }
}

impl From<&io::Error> for JavaThrowable {
    /// An `IOException` thrown by a file system operation other than opening a stream (see
    /// [`JavaThrowable::from_stream_open`]).
    fn from(e: &io::Error) -> Self {
        // An IoException tunnelled through std::io (blaze_repo_fetch `IoException::into_io_error`)
        // keeps its class.
        if let Some(inner) = e.get_ref().and_then(|i| i.downcast_ref::<IoException>()) {
            return JavaThrowable::from(inner);
        }
        JavaThrowable {
            class_name: Some(vfs_exception_class_name(e)),
            message: Some(e.to_string()),
        }
    }
}

impl From<io::Error> for JavaThrowable {
    fn from(e: io::Error) -> Self {
        JavaThrowable::from(&e)
    }
}

impl From<&FetchRepositoryFunctionException> for JavaThrowable {
    /// The cause of a `RepositoryFunctionException` thrown by the fetch machinery (the
    /// decompressors): that exception's message is its cause's `toString()`.
    fn from(e: &FetchRepositoryFunctionException) -> Self {
        match &e.cause {
            FetchRepositoryFunctionCause::Io(io) => JavaThrowable::from(io),
            FetchRepositoryFunctionCause::Eval(message) => {
                JavaThrowable::eval_exception(message.clone())
            }
        }
    }
}

impl From<FetchRepositoryFunctionException> for JavaThrowable {
    fn from(e: FetchRepositoryFunctionException) -> Self {
        JavaThrowable::from(&e)
    }
}

/// Java `EvalException.toString()`: `getMessageWithStack()`, the message with the call stack when
/// the exception has one (it was thrown through a Starlark call) and the message alone otherwise.
// Java: net/starlark/java/eval/EvalException.java:135 EvalException.toString
// Java: net/starlark/java/eval/EvalException.java:143 EvalException.getMessageWithStack
fn eval_exception_to_java_string(e: &EvalError) -> String {
    if e.callstack.is_empty() {
        // Java: `callstack == null`.
        e.message.clone()
    } else {
        e.message_with_stack()
    }
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
            RepositoryFunctionExceptionCause::Io(e) => JavaThrowable::from(e).to_java_string(),
            RepositoryFunctionExceptionCause::Eval(e) => eval_exception_to_java_string(e),
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
    /// The Java class of an `IOException` cause thrown by the file system, when it is a
    /// subclass (`FileNotFoundException`, `FileAccessException`, ...).
    ///
    /// PORT: `blaze_sky_loading`'s [`IOException`], the type Skyframe consumers catch, does not
    /// record `java.io` subclasses; [`RepositoryFunctionException::from_io_error`] keeps the
    /// class here so that `getMessage()` (`cause.toString()`) prints it.
    io_cause_class_name: Option<&'static str>,
}

impl RepositoryFunctionException {
    /// Error reading or writing to the filesystem.
    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryFunctionException.java:34 RepositoryFunctionException.<init>
    pub fn io(cause: IOException, transience: Transience) -> Self {
        RepositoryFunctionException {
            cause: RepositoryFunctionExceptionCause::Io(cause),
            transience,
            io_cause_class_name: None,
        }
    }

    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryFunctionException.java:38 RepositoryFunctionException.<init>
    pub fn eval(cause: EvalError, transience: Transience) -> Self {
        RepositoryFunctionException {
            cause: RepositoryFunctionExceptionCause::Eval(cause),
            transience,
            io_cause_class_name: None,
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
            io_cause_class_name: None,
        }
    }

    /// `new RepositoryFunctionException(new IOException(message), transience)`.
    pub fn io_message(message: impl Into<String>, transience: Transience) -> Self {
        Self::io(IOException::new(message), transience)
    }

    /// `new RepositoryFunctionException(e, transience)` for an `IOException` thrown by the file
    /// system (an operation other than opening a stream): the message names the class the
    /// native layer throws for the error.
    pub fn from_io_error(e: &std::io::Error, transience: Transience) -> Self {
        RepositoryFunctionException {
            cause: RepositoryFunctionExceptionCause::Io(IOException::from_io(e)),
            transience,
            io_cause_class_name: JavaThrowable::from(e).class_name,
        }
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
        match (&self.cause, self.io_cause_class_name) {
            (RepositoryFunctionExceptionCause::Io(e), Some(class_name)) => {
                JavaThrowable::of_class(class_name, Some(e.get_message().to_owned()))
                    .to_java_string()
            }
            _ => self.cause.to_java_string(),
        }
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

#[cfg(test)]
mod tests {
    use blaze_repo_fetch::errors::IoExceptionKind;
    use blaze_starlark::error::EvalErrorKind;
    use blaze_starlark::error::StackEntry;
    use blaze_vfs::PathFragment;

    use super::*;

    /// A `blaze_vfs` error: `<path> (<strerror>)` with the kind `std` gives the errno.
    fn vfs_error(kind: io::ErrorKind, message: &str) -> io::Error {
        io::Error::new(kind, message.to_owned())
    }

    /// The class the native layer throws for an errno, and `Throwable.toString()`.
    // Java: (native) src/main/native/unix_jni.cc:81 PostException
    #[test]
    fn file_system_errors_print_the_class_the_native_layer_throws() {
        // ENOENT
        assert_eq!(
            JavaThrowable::from(&vfs_error(
                io::ErrorKind::NotFound,
                "/a/b (No such file or directory)"
            ))
            .to_java_string(),
            "java.io.FileNotFoundException: /a/b (No such file or directory)"
        );
        // EACCES
        assert_eq!(
            JavaThrowable::from(&vfs_error(
                io::ErrorKind::PermissionDenied,
                "/a/b (Permission denied)"
            ))
            .to_java_string(),
            "com.google.devtools.build.lib.vfs.FileAccessException: /a/b (Permission denied)"
        );
        // EPERM is in the default branch.
        assert_eq!(
            JavaThrowable::from(&vfs_error(
                io::ErrorKind::PermissionDenied,
                "/a/b (Operation not permitted)"
            ))
            .to_java_string(),
            "java.io.IOException: /a/b (Operation not permitted)"
        );
        // EINVAL
        assert_eq!(
            JavaThrowable::from(&vfs_error(
                io::ErrorKind::InvalidInput,
                "/a/b (Invalid argument)"
            ))
            .to_java_string(),
            "com.google.devtools.build.lib.unix.InvalidArgumentIOException: /a/b (Invalid \
             argument)"
        );
        // ELOOP
        assert_eq!(
            JavaThrowable::from(&FileSymlinkLoopException::new_error(&PathFragment::create(
                "/a/b"
            )))
            .to_java_string(),
            "com.google.devtools.build.lib.vfs.FileSymlinkLoopException: /a/b (Too many levels \
             of symbolic links)"
        );
        // ENOTEMPTY and every other errno
        assert_eq!(
            JavaThrowable::from(&vfs_error(
                io::ErrorKind::Other,
                "/a/b (Directory not empty)"
            ))
            .to_java_string(),
            "java.io.IOException: /a/b (Directory not empty)"
        );
    }

    /// `FileSystem.NotASymlinkException` is a nested class: `Class.getName()` uses `$`.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:76 FileSystem.NotASymlinkException
    #[test]
    fn not_a_symlink_exception_prints_its_binary_name() {
        assert_eq!(
            JavaThrowable::from(&NotASymlinkException::new_error(&PathFragment::create(
                "/a/b"
            )))
            .to_java_string(),
            "com.google.devtools.build.lib.vfs.FileSystem$NotASymlinkException: /a/b is not a \
             symlink"
        );
    }

    /// `FileInputStream` / `FileOutputStream` throw `FileNotFoundException` whatever the reason;
    /// Bazel turns "Permission denied" into `FileAccessException`.
    // Java: com/google/devtools/build/lib/vfs/DiskBackedFileSystem.java:64 DiskBackedFileSystem.getInputStream
    #[test]
    fn stream_open_errors_are_file_not_found_or_file_access() {
        assert_eq!(
            JavaThrowable::from_stream_open(&vfs_error(
                io::ErrorKind::NotFound,
                "/a/b (No such file or directory)"
            ))
            .to_java_string(),
            "java.io.FileNotFoundException: /a/b (No such file or directory)"
        );
        assert_eq!(
            JavaThrowable::from_stream_open(&vfs_error(
                io::ErrorKind::Other,
                "/a/b (Is a directory)"
            ))
            .to_java_string(),
            "java.io.FileNotFoundException: /a/b (Is a directory)"
        );
        assert_eq!(
            JavaThrowable::from_stream_open(&vfs_error(
                io::ErrorKind::PermissionDenied,
                "/a/b (Permission denied)"
            ))
            .to_java_string(),
            "com.google.devtools.build.lib.vfs.FileAccessException: /a/b (Permission denied)"
        );
    }

    /// A failed read or write on an open stream is a plain `IOException` whose message is the
    /// OS error text (ENOSPC here).
    #[test]
    fn stream_io_errors_are_io_exceptions_with_the_os_error_text() {
        assert_eq!(
            JavaThrowable::from_stream_io(&io::Error::from_raw_os_error(28)).to_java_string(),
            "java.io.IOException: No space left on device"
        );
    }

    /// The downloader's exceptions record their class; it survives a trip through `io::Error`.
    #[test]
    fn fetch_exceptions_keep_their_class() {
        let unknown_host = IoException::of(
            IoExceptionKind::UnknownHostException,
            "Unknown host: example.invalid",
        );
        assert_eq!(
            JavaThrowable::from(&unknown_host).to_java_string(),
            "java.net.UnknownHostException: Unknown host: example.invalid"
        );
        assert_eq!(
            JavaThrowable::from(unknown_host.clone().into_io_error()).to_java_string(),
            "java.net.UnknownHostException: Unknown host: example.invalid"
        );
        // getMessage() is null: toString() is the class name, and string concatenation of the
        // message gives "null".
        let no_message =
            JavaThrowable::from(IoException::without_message(IoExceptionKind::EOFException));
        assert_eq!(no_message.to_java_string(), "java.io.EOFException");
        assert_eq!(no_message.get_message(), "null");
    }

    /// The oracle's line for a failed download (re2 record, `query_deps_label_kind.stderr`):
    /// `java.io.IOException: Error downloading [...`.
    #[test]
    fn new_io_exception_prints_java_io_io_exception() {
        let e =
            JavaThrowable::io_exception("Error downloading [https://a/b] to /c: Unknown host: a");
        assert_eq!(
            e.to_java_string(),
            "java.io.IOException: Error downloading [https://a/b] to /c: Unknown host: a"
        );
        assert_eq!(
            e.get_message(),
            "Error downloading [https://a/b] to /c: Unknown host: a"
        );
    }

    /// `EvalException.toString()` is `getMessageWithStack()`: no class name.
    // Java: net/starlark/java/eval/EvalException.java:135 EvalException.toString
    #[test]
    fn eval_exception_prints_its_message_only() {
        assert_eq!(
            JavaThrowable::eval_exception("Cannot write outside of the repository directory")
                .to_java_string(),
            "Cannot write outside of the repository directory"
        );
        let from_fetch = FetchRepositoryFunctionException::eval(
            "Expected a file with a .zip suffix",
            blaze_repo_fetch::errors::Transience::PERSISTENT,
        );
        assert_eq!(
            JavaThrowable::from(&from_fetch).to_java_string(),
            "Expected a file with a .zip suffix"
        );
        let from_fetch_io = FetchRepositoryFunctionException::io(
            IoException::new("Error extracting /a/b.zip to /a: boom"),
            blaze_repo_fetch::errors::Transience::TRANSIENT,
        );
        assert_eq!(
            JavaThrowable::from(from_fetch_io).to_java_string(),
            "java.io.IOException: Error extracting /a/b.zip to /a: boom"
        );
    }

    /// The message of `RepositoryFunctionException(EvalException)` is the cause's `toString()`:
    /// the message alone without a call stack, `formatCallStack` with one. Neither starts with
    /// the class name.
    // Java: net/starlark/java/eval/EvalException.java:143 EvalException.getMessageWithStack
    // Java: net/starlark/java/eval/EvalException.java:201 EvalException.formatCallStack
    #[test]
    fn repository_function_exception_message_of_an_eval_exception_is_its_to_string() {
        let without_stack = EvalError {
            kind: EvalErrorKind::Eval,
            message: "boom".to_owned(),
            callstack: Vec::new(),
            include_stack: true,
        };
        assert_eq!(
            RepositoryFunctionException::eval(without_stack, Transience::Transient).get_message(),
            "boom"
        );

        let with_stack = EvalError {
            kind: EvalErrorKind::Eval,
            message: "boom".to_owned(),
            callstack: vec![
                StackEntry {
                    name: "<toplevel>".to_owned(),
                    location: Some(blaze_starlark::Location {
                        file: "a.bzl".to_owned(),
                        line: 1,
                        column: 2,
                    }),
                    source_line: None,
                },
                StackEntry {
                    name: "fail".to_owned(),
                    location: None,
                    source_line: None,
                },
            ],
            include_stack: true,
        };
        assert_eq!(
            RepositoryFunctionException::eval(with_stack, Transience::Transient).get_message(),
            "Traceback (most recent call last):\n\tFile \"a.bzl\", line 1, column 2, in \
             <toplevel>\nError in fail: boom"
        );
    }

    /// `RepositoryFunctionException(e)` for a file system exception prints the subclass the
    /// native layer throws (ENOENT: `FileNotFoundException`).
    // Java: (native) src/main/native/unix_jni.cc:81 PostException
    #[test]
    fn repository_function_exception_message_of_a_file_system_exception_has_its_class_name() {
        assert_eq!(
            RepositoryFunctionException::from_io_error(
                &vfs_error(io::ErrorKind::NotFound, "/a/b (No such file or directory)"),
                Transience::Transient
            )
            .get_message(),
            "java.io.FileNotFoundException: /a/b (No such file or directory)"
        );
        assert_eq!(
            RepositoryFunctionException::from_io_error(
                &vfs_error(io::ErrorKind::Other, "/a/b (Directory not empty)"),
                Transience::Transient
            )
            .get_message(),
            "java.io.IOException: /a/b (Directory not empty)"
        );
    }

    /// `RepositoryFunctionException(IOException)`: `java.io.IOException: <message>`.
    #[test]
    fn repository_function_exception_message_of_an_io_exception_has_the_class_name() {
        assert_eq!(
            RepositoryFunctionException::io_message(
                "x must create a directory",
                Transience::Transient
            )
            .get_message(),
            "java.io.IOException: x must create a directory"
        );
    }
}
