//! Port of `lib/remote/ExecuteRetrier.java`.

use std::io;
use std::ops::Deref;
use std::sync::Arc;

use blaze_protos::google::protobuf::{Any, Duration as ProtoDuration};
use parking_lot::Mutex;
use prost::Message as _;

use crate::common::bulk_transfer_exception::BulkTransferException;
use crate::execution_status_exception;
use crate::reapi::rpc::{PreconditionFailure, RetryInfo, Status};
use crate::remote_retrier::RemoteRetrier;
use crate::retrier::{
    Backoff, CircuitBreaker, RETRIES_DISABLED, ResultClassifierResult, RetryScheduler,
};

const VIOLATION_TYPE_MISSING: &str = "MISSING";

/// `Any.is(clazz)`: whether the type name of the `Any`'s type URL is `full_name`.
// Java: com/google/protobuf/Any.java Any.is
fn any_is(detail: &Any, full_name: &str) -> bool {
    // Any.getTypeNameFromTypeUrl
    let type_name = match detail.type_url.rfind('/') {
        None => "",
        Some(pos) => &detail.type_url[pos + 1..],
    };
    type_name == full_name
}

/// `StatusProto.fromThrowable(e)`: the `com.google.rpc.Status` of the first gRPC status
/// exception in the cause chain of `e`, or `None` if there is none.
///
/// PORT: grpc-java keeps the status proto in the exception's trailers
/// (`grpc-status-details-bin`); tonic keeps the same bytes in `tonic::Status::details()` (see
/// `execution_status_exception::to_status_runtime_exception`). `ExecutionStatusException` is a
/// `StatusRuntimeException` in Java, hence `execution_status_exception::status_from_throwable`.
// Java: io/grpc/protobuf/StatusProto.java StatusProto.fromThrowable
fn status_proto_from_throwable(e: &io::Error) -> Option<Status> {
    let status = execution_status_exception::status_from_throwable(e)?;
    // StatusProto.fromStatusAndTrailers
    if !status.details().is_empty() {
        let status_proto = match Status::decode(status.details()) {
            Ok(status_proto) => status_proto,
            // Metadata.get: the marshaller of STATUS_DETAILS_KEY throws.
            Err(invalid) => panic!("IllegalArgumentException: {invalid}"),
        };
        blaze_util::check_argument!(
            status.code() as i32 == status_proto.code,
            "com.google.rpc.Status code must match gRPC status code"
        );
        return Some(status_proto);
    }
    Some(Status {
        code: status.code() as i32,
        message: status.message().to_string(),
        details: Vec::new(),
    })
}

/// `Durations.toMillis(duration)`.
// Java: com/google/protobuf/util/Durations.java Durations.toMillis
fn durations_to_millis(duration: &ProtoDuration) -> i64 {
    // Durations.checkValid
    let seconds = duration.seconds;
    let nanos = duration.nanos;
    let mut valid = true;
    if !(-315_576_000_000..=315_576_000_000).contains(&seconds) {
        valid = false;
    }
    if !(-999_999_999..1_000_000_000).contains(&nanos) {
        valid = false;
    }
    if (seconds < 0 || nanos < 0) && (seconds > 0 || nanos > 0) {
        valid = false;
    }
    if !valid {
        panic!(
            "IllegalArgumentException: Duration is not valid. See proto definition for valid values. Seconds ({seconds}) must be in range [-315,576,000,000, +315,576,000,000]. Nanos ({nanos}) must be in range [-999,999,999, +999,999,999]. Nanos must have the same sign as seconds"
        );
    }
    // checkedAdd(checkedMultiply(seconds, MILLIS_PER_SECOND), nanos / NANOS_PER_MILLISECOND):
    // cannot overflow for a valid duration.
    seconds * 1000 + (nanos / 1_000_000) as i64
}

// Java: com/google/devtools/build/lib/remote/ExecuteRetrier.java:40 ExecuteRetrier.RetryInfoBackoff
struct RetryInfoBackoff {
    max_retry_attempts: i32,
    retries: Mutex<i32>,
}

impl RetryInfoBackoff {
    // Java: com/google/devtools/build/lib/remote/ExecuteRetrier.java:44 ExecuteRetrier.RetryInfoBackoff.<init>
    fn new(max_retry_attempts: i32) -> RetryInfoBackoff {
        RetryInfoBackoff {
            max_retry_attempts,
            retries: Mutex::new(0),
        }
    }

    // Java: com/google/devtools/build/lib/remote/ExecuteRetrier.java:58 ExecuteRetrier.RetryInfoBackoff.getRetryInfo
    fn get_retry_info(&self, e: &io::Error) -> RetryInfo {
        let mut retry_info = RetryInfo::default();
        let status = status_proto_from_throwable(e);
        if let Some(status) = status {
            for detail in &status.details {
                if any_is(detail, "google.rpc.RetryInfo") {
                    match RetryInfo::decode(detail.value.as_slice()) {
                        Ok(unpacked) => retry_info = unpacked,
                        Err(_proto_ex) => {
                            // really shouldn't happen, ignore
                        }
                    }
                }
            }
        }
        retry_info
    }
}

impl Backoff for RetryInfoBackoff {
    // Java: com/google/devtools/build/lib/remote/ExecuteRetrier.java:49 ExecuteRetrier.RetryInfoBackoff.nextDelayMillis
    fn next_delay_millis(&self, e: &io::Error) -> i64 {
        let mut retries = self.retries.lock();
        if *retries >= self.max_retry_attempts {
            return -1;
        }
        let retry_info = self.get_retry_info(e);
        *retries += 1;
        // retryInfo.getRetryDelay() is the default instance when unset.
        durations_to_millis(&retry_info.retry_delay.unwrap_or_default())
    }

    // Java: com/google/devtools/build/lib/remote/ExecuteRetrier.java:76 ExecuteRetrier.RetryInfoBackoff.getRetryAttempts
    fn get_retry_attempts(&self) -> i32 {
        *self.retries.lock()
    }
}

/// Specific retry logic for execute request with gapi Status.
///
/// PORT: `extends RemoteRetrier` is a wrapper that derefs to [`RemoteRetrier`]; a caller that
/// keeps it in a `RemoteRetrier` field uses [`ExecuteRetrier::into_remote_retrier`].
// Java: com/google/devtools/build/lib/remote/ExecuteRetrier.java:36 ExecuteRetrier
#[derive(Clone)]
pub struct ExecuteRetrier {
    remote_retrier: RemoteRetrier,
}

impl Deref for ExecuteRetrier {
    type Target = RemoteRetrier;

    fn deref(&self) -> &RemoteRetrier {
        &self.remote_retrier
    }
}

impl From<ExecuteRetrier> for RemoteRetrier {
    fn from(e: ExecuteRetrier) -> RemoteRetrier {
        e.remote_retrier
    }
}

impl ExecuteRetrier {
    // Java: com/google/devtools/build/lib/remote/ExecuteRetrier.java:81 ExecuteRetrier.<init>
    pub fn new(
        max_retry_attempts: i32,
        retry_service: Arc<dyn RetryScheduler>,
        circuit_breaker: Arc<dyn CircuitBreaker>,
    ) -> ExecuteRetrier {
        ExecuteRetrier {
            remote_retrier: RemoteRetrier::with_backoff(
                Arc::new(move || -> Arc<dyn Backoff> {
                    if max_retry_attempts > 0 {
                        let backoff: Arc<dyn Backoff> =
                            Arc::new(RetryInfoBackoff::new(max_retry_attempts));
                        backoff
                    } else {
                        RETRIES_DISABLED.clone()
                    }
                }),
                Arc::new(result_classifier),
                retry_service,
                circuit_breaker,
            ),
        }
    }

    /// The upcast to `RemoteRetrier`.
    pub fn into_remote_retrier(self) -> RemoteRetrier {
        self.remote_retrier
    }
}

// Java: com/google/devtools/build/lib/remote/ExecuteRetrier.java:92 ExecuteRetrier.resultClassifier
fn result_classifier(e: &io::Error) -> ResultClassifierResult {
    if BulkTransferException::all_caused_by_cache_not_found_exception_of(e) {
        return ResultClassifierResult::TransientFailure;
    }
    let status = match status_proto_from_throwable(e) {
        Some(status) if !status.details.is_empty() => status,
        _ => return ResultClassifierResult::Success,
    };
    let mut failed_precondition = status.code == tonic::Code::FailedPrecondition as i32;
    for detail in &status.details {
        if any_is(detail, "google.rpc.RetryInfo") {
            // server says we can retry, regardless of other details
            return ResultClassifierResult::TransientFailure;
        } else if failed_precondition {
            if any_is(detail, "google.rpc.PreconditionFailure") {
                match PreconditionFailure::decode(detail.value.as_slice()) {
                    Ok(f) => {
                        if f.violations.is_empty() {
                            failed_precondition = false;
                        }
                        for v in &f.violations {
                            if v.r#type != VIOLATION_TYPE_MISSING {
                                failed_precondition = false;
                            }
                        }
                        // if *all* > 0 precondition failure violations have type MISSING,
                        // failedPrecondition remains true
                    }
                    Err(_proto_ex) => {
                        // really shouldn't happen
                        return ResultClassifierResult::PermanentFailure;
                    }
                }
            } else if !(any_is(detail, "google.rpc.DebugInfo")
                || any_is(detail, "google.rpc.Help")
                || any_is(detail, "google.rpc.LocalizedMessage")
                || any_is(detail, "google.rpc.RequestInfo")
                || any_is(detail, "google.rpc.ResourceInfo"))
            {
                // ignore benign details
                // consider all other details as failures
                failed_precondition = false;
            }
        }
    }
    if failed_precondition {
        ResultClassifierResult::TransientFailure
    } else {
        ResultClassifierResult::PermanentFailure
    }
}
