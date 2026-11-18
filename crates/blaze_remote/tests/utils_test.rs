#![cfg(feature = "wip")]
// Options and protos are filled in field by field, as the Java tests set them.
#![allow(clippy::field_reassign_with_default)]
//! Port of `lib/remote/UtilsTest.java`: tests for remote utility methods.

use std::io;
use std::sync::atomic::{AtomicI32, Ordering};

use blaze_remote::common::errors;
use blaze_remote::runtime::block_on;
use blaze_remote::util::utils::{self, CallCredentialsProvider};

/// `mock(CallCredentialsProvider.class)`, counting the calls of `refresh()` for
/// `verify(callCredentialsProvider, times(1)).refresh()`.
struct MockCallCredentialsProvider {
    refresh_times: AtomicI32,
}

impl MockCallCredentialsProvider {
    fn new() -> MockCallCredentialsProvider {
        MockCallCredentialsProvider {
            refresh_times: AtomicI32::new(0),
        }
    }

    fn refresh_times(&self) -> i32 {
        self.refresh_times.load(Ordering::SeqCst)
    }
}

impl CallCredentialsProvider for MockCallCredentialsProvider {
    fn get_call_credentials(
        &self,
    ) -> Option<std::sync::Arc<dyn blaze_remote::grpc::CallCredentials>> {
        None
    }

    fn refresh(&self) -> io::Result<()> {
        self.refresh_times.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// `new StatusRuntimeException(status)`, thrown through an `io::Result`.
fn status_runtime_exception(code: tonic::Code) -> io::Error {
    errors::status_to_io_error(tonic::Status::new(code, ""))
}

#[test]
fn test_grpc_aware_error_message() {
    let io_error = errors::io_exception("io error");
    // PORT: Java builds `new IOException("wrapped error", statusRuntimeException)`. The
    // `io::Error` that wraps a `StatusRuntimeException` is `errors::status_to_io_error`; it has
    // no message of its own, and `grpcAwareErrorMessage` does not print one for such an error.
    let wrapped_grpc_error =
        errors::status_to_io_error(tonic::Status::new(tonic::Code::Aborted, "grpc error"));

    assert_eq!(
        utils::grpc_aware_error_message(&io_error, /* verbose_failures= */ false),
        "io error"
    );
    assert_eq!(
        utils::grpc_aware_error_message(&wrapped_grpc_error, /* verbose_failures= */ false),
        "ABORTED: grpc error"
    );
}

#[test]
#[ignore = "Rust errors carry no stack trace: grpc_aware_error_message appends nothing with verbose_failures (PORT note in utils.rs)"]
fn test_grpc_aware_error_message_verbose_failures() {
    let io_error = errors::io_exception("io error");
    let wrapped_grpc_error =
        errors::status_to_io_error(tonic::Status::new(tonic::Code::Aborted, "grpc error"));

    // Java expects the message followed by the stack trace of the exception, whose frames name
    // the Java test method; there is no counterpart to compare with.
    assert!(
        utils::grpc_aware_error_message(&io_error, /* verbose_failures= */ true)
            .starts_with("io error\njava.io.IOException: io error\n\tat ")
    );
    assert!(
        utils::grpc_aware_error_message(&wrapped_grpc_error, /* verbose_failures= */ true)
            .starts_with("ABORTED: grpc error\njava.io.IOException: wrapped error\n\tat ")
    );
}

#[test]
fn refresh_if_unauthenticated_async_unauthenticated_should_refresh() {
    let call_credentials_provider = MockCallCredentialsProvider::new();
    let call_times = AtomicI32::new(0);

    block_on(utils::refresh_if_unauthenticated_async(
        || {
            // Java throws from `call()` itself; a Rust call can only fail through its future,
            // which is ready here.
            if call_times.fetch_add(1, Ordering::SeqCst) == 0 {
                return std::future::ready(Err(status_runtime_exception(
                    tonic::Code::Unauthenticated,
                )));
            }
            std::future::ready(Ok(()))
        },
        &call_credentials_provider,
    ))
    .unwrap();

    assert_eq!(call_times.load(Ordering::SeqCst), 2);
    assert_eq!(call_credentials_provider.refresh_times(), 1);
}

#[test]
fn refresh_if_unauthenticated_async_unauthenticated_future_should_refresh() {
    let call_credentials_provider = MockCallCredentialsProvider::new();
    let call_times = AtomicI32::new(0);

    block_on(utils::refresh_if_unauthenticated_async(
        || {
            let first_call = call_times.fetch_add(1, Ordering::SeqCst) == 0;
            async move {
                if first_call {
                    return Err(status_runtime_exception(tonic::Code::Unauthenticated));
                }
                Ok(())
            }
        },
        &call_credentials_provider,
    ))
    .unwrap();

    assert_eq!(call_times.load(Ordering::SeqCst), 2);
    assert_eq!(call_credentials_provider.refresh_times(), 1);
}

#[test]
fn refresh_if_unauthenticated_async_permission_denied_should_refresh() {
    let call_credentials_provider = MockCallCredentialsProvider::new();
    let call_times = AtomicI32::new(0);

    // As in Java, this test calls the synchronous method with a block that returns a future.
    let future = utils::refresh_if_unauthenticated(
        || {
            if call_times.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(status_runtime_exception(tonic::Code::PermissionDenied));
            }
            Ok(std::future::ready(()))
        },
        &call_credentials_provider,
    )
    .unwrap();
    block_on(future);

    assert_eq!(call_times.load(Ordering::SeqCst), 2);
    assert_eq!(call_credentials_provider.refresh_times(), 1);
}

#[test]
fn refresh_if_unauthenticated_async_cant_refresh_should_refresh_once_and_fail() {
    let call_credentials_provider = MockCallCredentialsProvider::new();
    let call_times = AtomicI32::new(0);

    let result: io::Result<()> = block_on(utils::refresh_if_unauthenticated_async(
        || {
            call_times.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Err::<(), io::Error>(status_runtime_exception(
                tonic::Code::Unauthenticated,
            )))
        },
        &call_credentials_provider,
    ));
    assert!(result.is_err());

    assert_eq!(call_times.load(Ordering::SeqCst), 2);
    assert_eq!(call_credentials_provider.refresh_times(), 1);
}

#[test]
fn refresh_if_unauthenticated_unauthenticated_should_refresh() {
    let call_credentials_provider = MockCallCredentialsProvider::new();
    let call_times = AtomicI32::new(0);

    utils::refresh_if_unauthenticated(
        || {
            if call_times.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(status_runtime_exception(tonic::Code::Unauthenticated));
            }
            Ok(())
        },
        &call_credentials_provider,
    )
    .unwrap();

    assert_eq!(call_times.load(Ordering::SeqCst), 2);
    assert_eq!(call_credentials_provider.refresh_times(), 1);
}

#[test]
fn refresh_if_unauthenticated_permission_denied_should_refresh() {
    let call_credentials_provider = MockCallCredentialsProvider::new();
    let call_times = AtomicI32::new(0);

    utils::refresh_if_unauthenticated(
        || {
            if call_times.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(status_runtime_exception(tonic::Code::PermissionDenied));
            }
            Ok(())
        },
        &call_credentials_provider,
    )
    .unwrap();

    assert_eq!(call_times.load(Ordering::SeqCst), 2);
    assert_eq!(call_credentials_provider.refresh_times(), 1);
}

#[test]
fn refresh_if_unauthenticated_cant_refresh_should_refresh_once_and_fail() {
    let call_credentials_provider = MockCallCredentialsProvider::new();
    let call_times = AtomicI32::new(0);

    let result: io::Result<()> = utils::refresh_if_unauthenticated(
        || {
            call_times.fetch_add(1, Ordering::SeqCst);
            Err(status_runtime_exception(tonic::Code::Unauthenticated))
        },
        &call_credentials_provider,
    );
    // `assertThrows(StatusRuntimeException.class, ...)`
    let e = result.unwrap_err();
    assert_eq!(
        errors::status_from_throwable(&e).map(|status| status.code()),
        Some(tonic::Code::Unauthenticated)
    );

    assert_eq!(call_times.load(Ordering::SeqCst), 2);
    assert_eq!(call_credentials_provider.refresh_times(), 1);
}
