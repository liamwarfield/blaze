//! Port of `src/test/java/com/google/devtools/build/lib/remote/RemoteRetrierTest.java`: tests
//! for `RemoteRetrier`.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_remote::common::errors;
use blaze_remote::remote_retrier::{ExponentialBackoff, RandomSource, RemoteRetrier};
use blaze_remote::retrier::{
    ALLOW_ALL_CALLS, Backoff, BackoffSupplier, ResultClassifier, ResultClassifierResult,
    RetryScheduler, Sleeper, TokioRetryScheduler,
};
use parking_lot::Mutex;

/// `Mockito.mock(RemoteRetrierTest.Foo.class)` with the stubbing each test sets up: the answers
/// are given in order, the last one repeats.
struct FooMock {
    calls: AtomicI32,
    answers: Vec<fn() -> io::Result<String>>,
}

impl FooMock {
    fn new(answers: Vec<fn() -> io::Result<String>>) -> FooMock {
        FooMock {
            calls: AtomicI32::new(0),
            answers,
        }
    }

    fn foo(&self) -> io::Result<String> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) as usize;
        let answer = self.answers[call.min(self.answers.len() - 1)];
        answer()
    }

    /// `Mockito.verify(fooMock, Mockito.times(n)).foo()`.
    fn times(&self) -> i32 {
        self.calls.load(Ordering::SeqCst)
    }
}

/// `.thenReturn("bla")`.
fn return_bla() -> io::Result<String> {
    Ok("bla".to_string())
}

/// `.thenThrow(Status.Code.UNKNOWN.toStatus().asRuntimeException())`.
fn throw_unknown() -> io::Result<String> {
    Err(errors::status_to_io_error(tonic::Status::new(
        tonic::Code::Unknown,
        "",
    )))
}

/// `assertThrows(StatusRuntimeException.class, ...)`.
fn assert_status_runtime_exception(result: io::Result<String>) {
    let e = result.unwrap_err();
    assert!(errors::downcast_ref::<errors::StatusRuntimeException>(&e).is_some());
}

/// `Mockito.mock(Sleeper.class)`, recording the calls.
#[derive(Default)]
struct SleeperMock {
    sleeps: Mutex<Vec<i64>>,
}

impl Sleeper for SleeperMock {
    fn sleep(&self, millis: i64) -> io::Result<()> {
        self.sleeps.lock().push(millis);
        Ok(())
    }
}

impl SleeperMock {
    /// `Mockito.verify(sleeper, Mockito.times(n)).sleep(millis)`.
    fn times(&self, millis: i64) -> usize {
        self.sleeps.lock().iter().filter(|m| **m == millis).count()
    }
}

/// `retryService`.
fn retry_service() -> Arc<dyn RetryScheduler> {
    Arc::new(TokioRetryScheduler)
}

/// `(e) -> Result.<result>`.
fn classifier(result: ResultClassifierResult) -> Arc<dyn ResultClassifier> {
    Arc::new(move |_e: &io::Error| result)
}

/// `new Exception()`.
fn exception() -> io::Error {
    errors::io_exception("")
}

#[test]
fn test_exponential_backoff() {
    let e = exception();
    let backoff: Box<dyn Backoff> = Box::new(ExponentialBackoff::new(
        Duration::from_secs(1),
        Duration::from_secs(10),
        2.0,
        0.0,
        6,
    ));
    assert_eq!(backoff.next_delay_millis(&e), 1000);
    assert_eq!(backoff.next_delay_millis(&e), 2000);
    assert_eq!(backoff.next_delay_millis(&e), 4000);
    assert_eq!(backoff.next_delay_millis(&e), 8000);
    assert_eq!(backoff.next_delay_millis(&e), 10000);
    assert_eq!(backoff.next_delay_millis(&e), 10000);
    assert!(backoff.next_delay_millis(&e) < 0);
}

#[test]
fn test_exponential_backoff_jittered() {
    let e = exception();
    let backoff: Box<dyn Backoff> = Box::new(ExponentialBackoff::new(
        Duration::from_secs(1),
        Duration::from_secs(10),
        2.0,
        0.1,
        6,
    ));
    assert!((900..1100).contains(&backoff.next_delay_millis(&e)));
    assert!((1800..2200).contains(&backoff.next_delay_millis(&e)));
    assert!((3600..4400).contains(&backoff.next_delay_millis(&e)));
    assert!((7200..8800).contains(&backoff.next_delay_millis(&e)));
    assert!((9000..11000).contains(&backoff.next_delay_millis(&e)));
    assert!((9000..11000).contains(&backoff.next_delay_millis(&e)));
    assert!(backoff.next_delay_millis(&e) < 0);
}

/// Not in the Java test: the jitter arithmetic with the randomness pinned to the two ends of
/// `nextDouble(2.0)`'s range, `jitter * (r - 1)` being -10% and (just under) +10%.
#[test]
fn exponential_backoff_jitter_with_pinned_randomness() {
    struct Fixed(f64);
    impl RandomSource for Fixed {
        fn next_double(&self, bound: f64) -> f64 {
            assert_eq!(bound, 2.0);
            self.0
        }
    }
    let e = exception();
    let low = ExponentialBackoff::with_random(
        Duration::from_secs(1),
        Duration::from_secs(10),
        2.0,
        0.1,
        6,
        Arc::new(Fixed(0.0)),
    );
    // (long) (1000 * (1 + 0.1 * (0.0 - 1)))
    assert_eq!(low.next_delay_millis(&e), 900);
    assert_eq!(low.next_delay_millis(&e), 1800);
    let mid = ExponentialBackoff::with_random(
        Duration::from_secs(1),
        Duration::from_secs(10),
        2.0,
        0.1,
        6,
        Arc::new(Fixed(1.0)),
    );
    assert_eq!(mid.next_delay_millis(&e), 1000);
    assert_eq!(mid.next_delay_millis(&e), 2000);
    assert_eq!(mid.get_retry_attempts(), 2);
}

#[test]
// Java mutates the defaults instance in place.
#[allow(clippy::field_reassign_with_default)]
fn test_no_retries() {
    let mut options = RemoteOptions::default();
    options.remote_max_retry_attempts = 0;

    let retrier = RemoteRetrier::new(
        &options,
        classifier(ResultClassifierResult::TransientFailure),
        retry_service(),
        ALLOW_ALL_CALLS.clone(),
    );
    let answers: Vec<fn() -> io::Result<String>> = vec![return_bla, throw_unknown];
    let foo_mock = FooMock::new(answers);
    assert_eq!(retrier.execute(|| foo_mock.foo()).unwrap(), "bla");
    assert_status_runtime_exception(retrier.execute(|| foo_mock.foo()));
    assert_eq!(foo_mock.times(), 2);
}

#[test]
fn test_non_retriable_error() {
    let s: BackoffSupplier = Arc::new(|| -> Arc<dyn Backoff> {
        Arc::new(ExponentialBackoff::new(
            Duration::from_secs(1),
            Duration::from_secs(10),
            2.0,
            0.0,
            2,
        ))
    });
    let retrier = RemoteRetrier::with_sleeper(
        s,
        classifier(ResultClassifierResult::PermanentFailure),
        retry_service(),
        ALLOW_ALL_CALLS.clone(),
        Arc::new(SleeperMock::default()),
    );
    let foo_mock = FooMock::new(vec![throw_unknown]);
    assert_status_runtime_exception(retrier.execute(|| foo_mock.foo()));
    assert_eq!(foo_mock.times(), 1);
}

#[test]
fn test_repeated_retries_reset() {
    let s: BackoffSupplier = Arc::new(|| -> Arc<dyn Backoff> {
        Arc::new(ExponentialBackoff::new(
            Duration::from_secs(1),
            Duration::from_secs(10),
            2.0,
            0.0,
            2,
        ))
    });
    let sleeper = Arc::new(SleeperMock::default());
    let retrier = RemoteRetrier::with_sleeper(
        s,
        classifier(ResultClassifierResult::TransientFailure),
        retry_service(),
        ALLOW_ALL_CALLS.clone(),
        sleeper.clone(),
    );

    let foo_mock = FooMock::new(vec![throw_unknown]);
    assert_status_runtime_exception(retrier.execute(|| foo_mock.foo()));
    assert_status_runtime_exception(retrier.execute(|| foo_mock.foo()));
    assert_eq!(sleeper.times(1000), 2);
    assert_eq!(sleeper.times(2000), 2);
    assert_eq!(foo_mock.times(), 6);
}

#[test]
// Java mutates the defaults instance in place.
#[allow(clippy::field_reassign_with_default)]
fn test_interrupted_exception_is_passed_through() {
    // PORT: `isSameInstanceAs(thrown)`: an `io::Error` has no identity, the instance is
    // recognised by its message.
    let thrown = || io::Error::new(io::ErrorKind::Interrupted, "the thrown instance");

    let mut options = RemoteOptions::default();
    options.remote_max_retry_attempts = 0;
    let retrier = RemoteRetrier::new(
        &options,
        classifier(ResultClassifierResult::TransientFailure),
        retry_service(),
        ALLOW_ALL_CALLS.clone(),
    );
    let expected = retrier
        .execute(|| -> io::Result<()> { Err(thrown()) })
        .unwrap_err();
    assert!(errors::is_interrupted(&expected));
    assert_eq!(expected.to_string(), "the thrown instance");
}

/// Not in RemoteRetrierTest.java (review 2026-11-19 R4): CANCELLED is transient unless the
/// classifying thread is interrupted (`RemoteRetrier.EXPERIMENTAL_GRPC_RESULT_CLASSIFIER`).
#[test]
fn cancelled_is_permanent_on_an_interrupted_thread() {
    use blaze_remote::remote_retrier::EXPERIMENTAL_GRPC_RESULT_CLASSIFIER;
    let cancelled = errors::status_to_io_error(tonic::Status::cancelled(""));
    assert_eq!(
        EXPERIMENTAL_GRPC_RESULT_CLASSIFIER.test(&cancelled),
        ResultClassifierResult::TransientFailure
    );
    blaze_shell::thread_interrupt::interrupt_current();
    assert_eq!(
        EXPERIMENTAL_GRPC_RESULT_CLASSIFIER.test(&cancelled),
        ResultClassifierResult::PermanentFailure
    );
    assert!(blaze_shell::thread_interrupt::interrupted());
}
