//! Java's unchecked exceptions (`RuntimeException`s) that a credential helper's answer can raise
//! inside `CredentialHelperCredentials.getRequestMetadata`: `ArithmeticException` from
//! `Duration.toNanos()` (an `expires` centuries ahead) and `IllegalArgumentException` from
//! `ImmutableMap.Builder.buildOrThrow()` (a header name given twice).
//!
//! In Java they propagate out of `getRequestMetadata`. On the downloader path nothing catches
//! them (`HttpConnectorMultiplexer` catches `IOException` only) and Bazel crashes. On the gRPC
//! path `com.google.auth.Credentials.blockingGetToCallback` catches every `Throwable` and hands it
//! to `GoogleAuthLibraryCallCredentials`' callback, which fails the call with
//! `Status.UNAUTHENTICATED.withDescription("Failed computing credential metadata").withCause(e)`
//! (checked in the oracle's `A-server.jar`: google-auth `Credentials.blockingGetToCallback` has a
//! `Throwable` handler calling `onFailure`; grpc-auth `GoogleAuthLibraryCallCredentials$1.onFailure`
//! builds that status, or `UNAVAILABLE` "Credentials failed to obtain metadata" for a
//! `Retryable` exception, which these are not).
//!
//! PORT: [`UncheckedException::throw`] panics with an [`UncheckedException`] payload, and the gRPC
//! path catches it with [`catch_unchecked`]. A panic message would be printed by the panic hook
//! on the thread that panics, before anyone can catch it, and the `IllegalArgumentException`
//! text holds header values; so a panic hook installed on the first throw prints the exception
//! (`thread '...' panicked at ...` then `toString()`, as an uncaught Java exception shows it)
//! only when no [`catch_unchecked`] is active on the thread, and prints nothing while one is.

use std::cell::Cell;
use std::fmt;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Once;

/// An unchecked Java exception: its class and message.
#[derive(Clone, PartialEq, Eq)]
pub struct UncheckedException {
    class_name: &'static str,
    message: String,
}

impl fmt::Debug for UncheckedException {
    /// The class only: the message of an `IllegalArgumentException` may hold header values.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UncheckedException")
            .field("class_name", &self.class_name)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for UncheckedException {
    /// `Throwable.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.class_name, self.message)
    }
}

impl std::error::Error for UncheckedException {}

thread_local! {
    /// The number of active [`catch_unchecked`] calls on this thread.
    static CATCHING: Cell<usize> = const { Cell::new(0) };
}

static INSTALL_HOOK: Once = Once::new();

fn install_hook() {
    INSTALL_HOOK.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let Some(e) = info.payload().downcast_ref::<UncheckedException>() else {
                return previous(info);
            };
            if CATCHING.with(Cell::get) > 0 {
                return;
            }
            let thread = std::thread::current();
            let location = info
                .location()
                .map(|l| format!(" at {l}"))
                .unwrap_or_default();
            eprintln!(
                "\nthread '{}' panicked{location}:\n{e}",
                thread.name().unwrap_or("<unnamed>")
            );
        }));
    });
}

impl UncheckedException {
    /// `throw new <class_name>(message)`.
    pub fn throw(class_name: &'static str, message: impl Into<String>) -> ! {
        install_hook();
        panic::panic_any(UncheckedException {
            class_name,
            message: message.into(),
        })
    }

    /// The fully qualified Java class name (`java.lang.ArithmeticException`, ...).
    pub fn get_class_name(&self) -> &'static str {
        self.class_name
    }

    /// `getMessage()`.
    pub fn get_message(&self) -> &str {
        &self.message
    }
}

/// Runs `f`, turning an [`UncheckedException`] it throws into `Err` (Java `catch (Throwable)`
/// around a call that only throws checked exceptions as values). Any other panic continues.
pub fn catch_unchecked<R>(f: impl FnOnce() -> R) -> Result<R, UncheckedException> {
    CATCHING.with(|c| c.set(c.get() + 1));
    let result = panic::catch_unwind(AssertUnwindSafe(f));
    CATCHING.with(|c| c.set(c.get() - 1));
    match result {
        Ok(r) => Ok(r),
        Err(payload) => match payload.downcast::<UncheckedException>() {
            Ok(e) => Err(*e),
            Err(payload) => panic::resume_unwind(payload),
        },
    }
}
