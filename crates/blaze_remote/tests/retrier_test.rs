// Options and protos are filled in field by field, as the Java tests set them.
#![allow(clippy::field_reassign_with_default)]
//! Port of `src/test/java/com/google/devtools/build/lib/remote/RetrierTest.java`: tests for
//! `Retrier`.
//!
//! Java's `Exception("call failed")` is `errors::io_exception("call failed")`; a
//! `ListenableFuture` is awaited with `runtime::get_from_future`.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use blaze_remote::common::errors;
use blaze_remote::remote_retrier::EXPERIMENTAL_GRPC_RESULT_CLASSIFIER;
use blaze_remote::retrier::{
    Backoff, BackoffSupplier, CircuitBreaker, CircuitBreakerException, CircuitBreakerState,
    ResultClassifier, ResultClassifierResult, Retrier, RetryScheduler, TokioRetryScheduler,
    ZeroBackoff,
};
use blaze_remote::runtime::get_from_future;
use parking_lot::Mutex;

/// `@Mock private CircuitBreaker alwaysOpen` with `when(alwaysOpen.state()).thenReturn(
/// State.ACCEPT_CALLS)`; counts the calls `verify` asks about.
#[derive(Default)]
struct AlwaysOpen {
    record_failure_calls: AtomicI32,
    record_success_calls: AtomicI32,
}

impl CircuitBreaker for AlwaysOpen {
    fn state(&self) -> CircuitBreakerState {
        CircuitBreakerState::AcceptCalls
    }

    fn record_failure(&self) {
        self.record_failure_calls.fetch_add(1, Ordering::SeqCst);
    }

    fn record_success(&self) {
        self.record_success_calls.fetch_add(1, Ordering::SeqCst);
    }
}

/// `RETRY_ALL`.
fn retry_all() -> Arc<dyn ResultClassifier> {
    Arc::new(|_e: &io::Error| ResultClassifierResult::TransientFailure)
}

/// `RETRY_NONE`.
fn retry_none() -> Arc<dyn ResultClassifier> {
    Arc::new(|_e: &io::Error| ResultClassifierResult::Success)
}

/// `retryService`.
fn retry_service() -> Arc<dyn RetryScheduler> {
    Arc::new(TokioRetryScheduler)
}

/// `() -> new ZeroBackoff(maxRetries)`.
fn zero_backoff_supplier(max_retries: i32) -> BackoffSupplier {
    Arc::new(move || -> Arc<dyn Backoff> { Arc::new(ZeroBackoff::new(max_retries)) })
}

#[test]
fn retry_should_work_failure() {
    // Test that a call is retried according to the backoff.
    // All calls fail.

    let s = zero_backoff_supplier(/* maxRetries= */ 2);
    let always_open = Arc::new(AlwaysOpen::default());
    let r = Retrier::new(s, retry_all(), retry_service(), always_open.clone());
    let num_calls = AtomicI32::new(0);
    let e = r
        .execute(|| -> io::Result<()> {
            num_calls.fetch_add(1, Ordering::SeqCst);
            Err(errors::io_exception("call failed"))
        })
        .unwrap_err();
    assert_eq!(e.to_string(), "call failed");

    assert_eq!(num_calls.load(Ordering::SeqCst), 3);
    assert_eq!(always_open.record_failure_calls.load(Ordering::SeqCst), 3);
    assert_eq!(always_open.record_success_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn retry_should_work_no_retries_failure() {
    // Test that a non-retriable error is not retried.
    // All calls fail.

    let s = zero_backoff_supplier(/* maxRetries= */ 2);
    let always_open = Arc::new(AlwaysOpen::default());
    let r = Retrier::new(s, retry_none(), retry_service(), always_open.clone());
    let num_calls = AtomicI32::new(0);
    let e = r
        .execute(|| -> io::Result<()> {
            num_calls.fetch_add(1, Ordering::SeqCst);
            Err(errors::io_exception("call failed"))
        })
        .unwrap_err();
    assert_eq!(e.to_string(), "call failed");

    assert_eq!(num_calls.load(Ordering::SeqCst), 1);
    assert_eq!(always_open.record_failure_calls.load(Ordering::SeqCst), 0);
    assert_eq!(always_open.record_success_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn retry_should_work_success() {
    // Test that a call is retried according to the backoff.
    // The last call succeeds.

    let s = zero_backoff_supplier(/* maxRetries= */ 2);
    let always_open = Arc::new(AlwaysOpen::default());
    let r = Retrier::new(s, retry_all(), retry_service(), always_open.clone());
    let num_calls = AtomicI32::new(0);
    let val = r
        .execute(|| -> io::Result<i32> {
            num_calls.fetch_add(1, Ordering::SeqCst);
            if num_calls.load(Ordering::SeqCst) == 3 {
                return Ok(1);
            }
            Err(errors::io_exception("call failed"))
        })
        .unwrap();
    assert_eq!(val, 1);

    assert_eq!(always_open.record_failure_calls.load(Ordering::SeqCst), 2);
    assert_eq!(always_open.record_success_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn nested_retries_should_work() {
    // Test that nested calls using retries compose as expected.

    let s = zero_backoff_supplier(/* maxRetries= */ 1);
    let always_open = Arc::new(AlwaysOpen::default());
    let r = Retrier::new(s, retry_all(), retry_service(), always_open);

    let attempts_lvl0 = AtomicI32::new(0);
    let attempts_lvl1 = AtomicI32::new(0);
    let attempts_lvl2 = AtomicI32::new(0);
    let result = r.execute(|| -> io::Result<()> {
        attempts_lvl0.fetch_add(1, Ordering::SeqCst);
        r.execute(|| -> io::Result<()> {
            attempts_lvl1.fetch_add(1, Ordering::SeqCst);
            r.execute(|| -> io::Result<()> {
                attempts_lvl2.fetch_add(1, Ordering::SeqCst);
                Err(errors::io_exception("failure message"))
            })
        })
    });
    if let Err(e) = result {
        assert_eq!(e.to_string(), "failure message");
        assert_eq!(attempts_lvl0.load(Ordering::SeqCst), 2);
        assert_eq!(attempts_lvl1.load(Ordering::SeqCst), 4);
        assert_eq!(attempts_lvl2.load(Ordering::SeqCst), 8);
    }
}

#[test]
fn circuit_breaker_should_trip() {
    // Test that a circuit breaker can trip.

    let s = zero_backoff_supplier(/* maxRetries= */ 3);
    let cb = Arc::new(TripAfterNCircuitBreaker::new(
        /* maxConsecutiveFailures= */ 2,
    ));
    let r = Retrier::new(s, retry_all(), retry_service(), cb.clone());

    let e = r
        .execute(|| -> io::Result<()> { Err(errors::io_exception("call failed")) })
        .unwrap_err();
    assert!(errors::downcast_ref::<CircuitBreakerException>(&e).is_some());

    assert_eq!(cb.state(), CircuitBreakerState::RejectCalls);
    assert_eq!(cb.consecutive_failures(), 2);
}

#[test]
fn circuit_breaker_can_recover() {
    // Test that a circuit breaker can recover from REJECT_CALLS to ACCEPT_CALLS by
    // utilizing the TRIAL_CALL state.

    let s = zero_backoff_supplier(/* maxRetries= */ 3);
    let cb = Arc::new(TripAfterNCircuitBreaker::new(
        /* maxConsecutiveFailures= */ 2,
    ));
    let r = Retrier::new(s, retry_all(), retry_service(), cb.clone());

    cb.trial_call();

    assert_eq!(cb.state(), CircuitBreakerState::TrialCall);

    let val = r.execute(|| -> io::Result<i32> { Ok(10) }).unwrap();
    assert_eq!(val, 10);
    assert_eq!(cb.state(), CircuitBreakerState::AcceptCalls);
}

#[test]
fn circuit_breaker_half_open_is_not_retried() {
    // Test that a call executed in TRIAL_CALL state is not retried
    // in case of failure.

    let s = zero_backoff_supplier(/* maxRetries= */ 3);
    let cb = Arc::new(TripAfterNCircuitBreaker::new(
        /* maxConsecutiveFailures= */ 2,
    ));
    let r = Retrier::new(s, retry_all(), retry_service(), cb.clone());

    cb.trial_call();

    // Intentionally ignored.
    let _expected = r.execute(|| -> io::Result<()> { Err(errors::io_exception("call failed")) });

    assert_eq!(cb.consecutive_failures(), 1);
}

#[test]
fn interrupts_should_not_be_retried_flag() {
    // Test that a call is not executed / retried if the current thread
    // is interrupted.

    let s = zero_backoff_supplier(/* maxRetries= */ 3);
    let cb = Arc::new(TripAfterNCircuitBreaker::new(
        /* maxConsecutiveFailures= */ 2,
    ));
    let r = Retrier::new(s, retry_all(), retry_service(), cb);

    let num_calls = AtomicI32::new(0);
    blaze_shell::thread_interrupt::interrupt_current();
    let e = r
        .execute(|| -> io::Result<i32> {
            num_calls.fetch_add(1, Ordering::SeqCst);
            Ok(10)
        })
        .unwrap_err();
    assert!(errors::is_interrupted(&e));
    assert_eq!(num_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn interrupts_should_not_be_retried_exception() {
    // Test that a call is not retried if an InterruptedException is thrown.

    let s = zero_backoff_supplier(/* maxRetries= */ 3);
    let cb = Arc::new(TripAfterNCircuitBreaker::new(
        /* maxConsecutiveFailures= */ 2,
    ));
    let r = Retrier::new(s, retry_all(), retry_service(), cb);

    let num_calls = AtomicI32::new(0);
    let e = r
        .execute(|| -> io::Result<()> {
            num_calls.fetch_add(1, Ordering::SeqCst);
            Err(errors::interrupted())
        })
        .unwrap_err();
    assert!(errors::is_interrupted(&e));
    assert_eq!(num_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn async_retry_exhaust_retries() {
    // Test that a call is retried according to the backoff.
    // All calls fail.

    let s = zero_backoff_supplier(/* maxRetries= */ 2);
    let always_open = Arc::new(AlwaysOpen::default());
    let r = Retrier::new(s, retry_all(), retry_service(), always_open);
    let num_calls = AtomicI32::new(0);
    let res = r.execute_async(|| {
        num_calls.fetch_add(1, Ordering::SeqCst);
        async { Err::<(), io::Error>(errors::io_exception("call failed")) }
    });
    let e = get_from_future(res).unwrap_err();
    assert_eq!(num_calls.load(Ordering::SeqCst), 3);
    assert_eq!(e.to_string(), "call failed");
}

#[test]
fn async_retry_non_retriable() {
    // Test that a call is retried according to the backoff.
    // All calls fail.

    let s = zero_backoff_supplier(/* maxRetries= */ 2);
    let always_open = Arc::new(AlwaysOpen::default());
    let r = Retrier::new(s, retry_none(), retry_service(), always_open);
    let num_calls = AtomicI32::new(0);
    let res = r.execute_async(|| {
        num_calls.fetch_add(1, Ordering::SeqCst);
        async { Err::<(), io::Error>(errors::io_exception("call failed")) }
    });
    let e = get_from_future(res).unwrap_err();
    assert_eq!(e.to_string(), "call failed");
    assert_eq!(num_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn async_retry_empty_error() {
    // Test that a call is retried according to the backoff.
    // All calls fail.

    let s = zero_backoff_supplier(/* maxRetries= */ 2);
    let always_open = Arc::new(AlwaysOpen::default());
    let r = Retrier::new(s, retry_none(), retry_service(), always_open);
    let res = r.execute_async(|| async { Err::<(), io::Error>(errors::io_exception("")) });
    let e = get_from_future(res).unwrap_err();
    assert_eq!(e.to_string(), "");
}

/// `throw new StatusRuntimeException(status)` from an `AsyncCallable`, executed and awaited.
fn execute_async_status(r: &Retrier, code: tonic::Code) -> io::Result<()> {
    get_from_future(r.execute_async(|| async move {
        Err::<(), io::Error>(errors::status_to_io_error(tonic::Status::new(code, "")))
    }))
}

#[test]
fn test_circuit_breaker_retriable_failures() {
    let max_retries = 2;
    let s = zero_backoff_supplier(max_retries);
    let retriable_statuses = [
        tonic::Code::Aborted,
        tonic::Code::Unknown,
        tonic::Code::DeadlineExceeded,
    ];
    let successful_statuses = [tonic::Code::NotFound, tonic::Code::AlreadyExists];
    let cb = Arc::new(TripAfterNCircuitBreaker::new(
        retriable_statuses.len() as i32 * (max_retries + 1),
    ));
    let r = Retrier::new(
        s,
        EXPERIMENTAL_GRPC_RESULT_CLASSIFIER.clone(),
        retry_service(),
        cb.clone(),
    );

    let mut expected_consecutive_failures = 0;

    for status in retriable_statuses {
        assert_eq!(cb.state(), CircuitBreakerState::AcceptCalls);
        let res = execute_async_status(&r, status);
        expected_consecutive_failures += max_retries + 1;
        assert!(res.is_err());
        assert_eq!(cb.consecutive_failures(), expected_consecutive_failures);
    }

    assert_eq!(cb.state(), CircuitBreakerState::RejectCalls);
    cb.trial_call();

    for status in successful_statuses {
        let res = execute_async_status(&r, status);
        assert_eq!(cb.consecutive_failures(), 0);
        assert!(res.is_err());
        assert_eq!(cb.state(), CircuitBreakerState::AcceptCalls);
    }
}

#[test]
fn test_circuit_breaker_non_retriable_failures() {
    let s = zero_backoff_supplier(/* maxRetries= */ 2);
    let non_retriable_statuses = [
        tonic::Code::PermissionDenied,
        tonic::Code::Unimplemented,
        tonic::Code::DataLoss,
        tonic::Code::OutOfRange,
    ];
    let successful_statues = [tonic::Code::NotFound, tonic::Code::AlreadyExists];
    let cb = Arc::new(TripAfterNCircuitBreaker::new(
        non_retriable_statuses.len() as i32
    ));
    let r = Retrier::new(
        s,
        EXPERIMENTAL_GRPC_RESULT_CLASSIFIER.clone(),
        retry_service(),
        cb.clone(),
    );

    let mut expected_consecutive_failures = 0;

    for status in non_retriable_statuses {
        let res = execute_async_status(&r, status);
        expected_consecutive_failures += 1;
        assert!(res.is_err());
        assert_eq!(cb.consecutive_failures(), expected_consecutive_failures);
    }

    assert_eq!(cb.state(), CircuitBreakerState::RejectCalls);
    cb.trial_call();

    for status in successful_statues {
        let res = execute_async_status(&r, status);
        assert_eq!(cb.consecutive_failures(), 0);
        assert!(res.is_err());
        assert_eq!(cb.state(), CircuitBreakerState::AcceptCalls);
    }
}

/// The fields of [`TripAfterNCircuitBreaker`] that its `synchronized` methods guard.
struct TripAfterNState {
    state: CircuitBreakerState,
    consecutive_failures: i32,
}

/// Simple circuit breaker that trips after N consecutive failures.
struct TripAfterNCircuitBreaker {
    max_consecutive_failures: i32,
    inner: Mutex<TripAfterNState>,
}

impl TripAfterNCircuitBreaker {
    fn new(max_consecutive_failures: i32) -> TripAfterNCircuitBreaker {
        TripAfterNCircuitBreaker {
            max_consecutive_failures,
            inner: Mutex::new(TripAfterNState {
                state: CircuitBreakerState::AcceptCalls,
                consecutive_failures: 0,
            }),
        }
    }

    /// The `consecutiveFailures` field.
    fn consecutive_failures(&self) -> i32 {
        self.inner.lock().consecutive_failures
    }

    fn trial_call(&self) {
        self.inner.lock().state = CircuitBreakerState::TrialCall;
    }
}

impl CircuitBreaker for TripAfterNCircuitBreaker {
    fn state(&self) -> CircuitBreakerState {
        self.inner.lock().state
    }

    fn record_failure(&self) {
        let mut inner = self.inner.lock();
        inner.consecutive_failures += 1;
        if inner.consecutive_failures >= self.max_consecutive_failures {
            inner.state = CircuitBreakerState::RejectCalls;
        }
    }

    fn record_success(&self) {
        let mut inner = self.inner.lock();
        inner.consecutive_failures = 0;
        inner.state = CircuitBreakerState::AcceptCalls;
    }
}

/// Not in RetrierTest.java (review 2026-11-19 R4): `TimeUnit.MILLISECONDS.sleep` between
/// synchronous retries is interruptible; an interrupt of the sleeping thread ends the retries
/// with an `InterruptedException`.
#[test]
fn interrupt_during_backoff_sleep_ends_the_retries() {
    // A backoff with a long delay, so that the call is still sleeping when it is interrupted.
    struct LongBackoff;
    impl Backoff for LongBackoff {
        fn next_delay_millis(&self, _e: &io::Error) -> i64 {
            60_000
        }
        fn get_retry_attempts(&self) -> i32 {
            0
        }
    }
    let s: BackoffSupplier = Arc::new(|| -> Arc<dyn Backoff> { Arc::new(LongBackoff) });
    let r = Retrier::new(
        s,
        retry_all(),
        retry_service(),
        blaze_remote::retrier::ALLOW_ALL_CALLS.clone(),
    );
    let thread = blaze_shell::thread_interrupt::ThreadInterrupt::current();
    let interrupter = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(200));
        thread.interrupt();
    });

    let num_calls = AtomicI32::new(0);
    let start = std::time::Instant::now();
    let e = r
        .execute(|| -> io::Result<()> {
            num_calls.fetch_add(1, Ordering::SeqCst);
            Err(errors::io_exception("call failed"))
        })
        .unwrap_err();
    interrupter.join().unwrap();

    assert!(errors::is_interrupted(&e), "{e:?}");
    assert_eq!(num_calls.load(Ordering::SeqCst), 1);
    assert!(start.elapsed() < std::time::Duration::from_secs(30));
    // Thread.sleep threw InterruptedException, which cleared the flag.
    assert!(!blaze_shell::thread_interrupt::is_current_interrupted());
}
