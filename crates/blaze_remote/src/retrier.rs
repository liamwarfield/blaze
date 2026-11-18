//! Port of `lib/remote/Retrier.java`.
//!
//! PORT: Java's `Exception` is `std::io::Error` here (docs/remote.md "Exceptions"); an
//! `InterruptedException` is an error of kind `Interrupted`. Java objects that are shared and
//! mutated through references ([`Backoff`], [`CircuitBreaker`]) take `&self` and keep their state
//! behind a lock, because the retrier and the call it retries hold the same backoff (see
//! `RemoteRetrier.ProgressiveBackoff.reset()`).

use std::fmt;
use std::future::Future;
use std::io;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::common::errors;

/// A backoff strategy.
// Java: com/google/devtools/build/lib/remote/Retrier.java:45 Retrier.Backoff
pub trait Backoff: Send + Sync {
    /// Returns the next delay in milliseconds, or a value less than `0` if we should stop
    /// retrying.
    fn next_delay_millis(&self, e: &io::Error) -> i64;

    /// Returns the number of calls to [`Backoff::next_delay_millis`] thus far, not counting any
    /// calls that returned less than `0`.
    fn get_retry_attempts(&self) -> i32;
}

/// `Supplier<Backoff>`.
pub type BackoffSupplier = Arc<dyn Fn() -> Arc<dyn Backoff> + Send + Sync>;

/// The state of the circuit breaker.
// Java: com/google/devtools/build/lib/remote/Retrier.java:77 Retrier.CircuitBreaker.State
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CircuitBreakerState {
    /// `ACCEPT_CALLS`: calls are executed and retried in case of failure.
    ///
    /// The circuit breaker can transition into state [`CircuitBreakerState::RejectCalls`].
    AcceptCalls,

    /// `TRIAL_CALL`: a call is executed and not retried in case of failure.
    ///
    /// The circuit breaker can transition into any state.
    TrialCall,

    /// `REJECT_CALLS`: all calls are rejected.
    ///
    /// The circuit breaker can transition into state [`CircuitBreakerState::TrialCall`].
    RejectCalls,
}

/// The circuit breaker allows to reject execution when failure rates are high.
///
/// The initial state of a circuit breaker is the [`CircuitBreakerState::AcceptCalls`]. Calls are
/// executed and retried in this state. However, if error rates are high a circuit breaker can
/// choose to transition into [`CircuitBreakerState::RejectCalls`]. In this state any calls are
/// rejected with a [`CircuitBreakerException`] immediately. A circuit breaker in state
/// [`CircuitBreakerState::RejectCalls`] can periodically return a `TRIAL_CALL` state, in which
/// case a call will be executed once and in case of success the circuit breaker may return to
/// state `ACCEPT_CALLS`.
///
/// A circuit breaker implementation must be thread-safe.
///
/// See <https://martinfowler.com/bliki/CircuitBreaker.html>.
// Java: com/google/devtools/build/lib/remote/Retrier.java:74 Retrier.CircuitBreaker
pub trait CircuitBreaker: Send + Sync {
    /// Returns the current [`CircuitBreakerState`] of the circuit breaker.
    fn state(&self) -> CircuitBreakerState;

    /// Called after an execution failed.
    fn record_failure(&self);

    /// Called after an execution succeeded.
    fn record_success(&self);
}

/// Thrown if the call was stopped by a circuit breaker.
// Java: com/google/devtools/build/lib/remote/Retrier.java:111 Retrier.CircuitBreakerException
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CircuitBreakerException {
    _private: (),
}

impl CircuitBreakerException {
    // Java: com/google/devtools/build/lib/remote/Retrier.java:112 Retrier.CircuitBreakerException.<init>
    fn new() -> CircuitBreakerException {
        CircuitBreakerException { _private: () }
    }
}

impl fmt::Display for CircuitBreakerException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Call not executed due to a high failure rate.")
    }
}

impl std::error::Error for CircuitBreakerException {}

impl From<CircuitBreakerException> for io::Error {
    fn from(e: CircuitBreakerException) -> io::Error {
        io::Error::other(e)
    }
}

/// The result of a call execution.
// Java: com/google/devtools/build/lib/remote/Retrier.java:122 Retrier.ResultClassifier.Result
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResultClassifierResult {
    /// `SUCCESS`: a call is executed successfully.
    Success,

    /// `TRANSIENT_FAILURE`: a call execution is failed with retriable error.
    TransientFailure,

    /// `PERMANENT_FAILURE`: a call execution is failed with permanent error.
    PermanentFailure,
}

/// Determines whether the result of a call is success, retriable failure or permanent failure.
///
/// PORT: a `@FunctionalInterface`; every `Fn(&io::Error) -> ResultClassifierResult` is one
/// (annotate the closure parameter: `|e: &io::Error| ...`).
// Java: com/google/devtools/build/lib/remote/Retrier.java:119 Retrier.ResultClassifier
pub trait ResultClassifier: Send + Sync {
    /// Returns the [`ResultClassifierResult`] of the call execution.
    fn test(&self, e: &io::Error) -> ResultClassifierResult;
}

impl<F> ResultClassifier for F
where
    F: Fn(&io::Error) -> ResultClassifierResult + Send + Sync,
{
    fn test(&self, e: &io::Error) -> ResultClassifierResult {
        self(e)
    }
}

/// [`Sleeper::sleep`] is called to pause between synchronous retries ([`Retrier::execute`]).
///
/// PORT: `throws InterruptedException` is an `Err` of kind `Interrupted`. Every
/// `Fn(i64) -> io::Result<()>` is a sleeper.
// Java: com/google/devtools/build/lib/remote/Retrier.java:141 Retrier.Sleeper
pub trait Sleeper: Send + Sync {
    /// `sleep(long millis)`.
    fn sleep(&self, millis: i64) -> io::Result<()>;
}

impl<F> Sleeper for F
where
    F: Fn(i64) -> io::Result<()> + Send + Sync,
{
    fn sleep(&self, millis: i64) -> io::Result<()> {
        self(millis)
    }
}

/// `MILLISECONDS::sleep`, the sleeper of the public constructor.
// Java: java/util/concurrent/TimeUnit.java TimeUnit.sleep
fn milliseconds_sleep(millis: i64) -> io::Result<()> {
    if millis > 0 {
        std::thread::sleep(Duration::from_millis(millis as u64));
    }
    Ok(())
}

/// `java.util.concurrent.RejectedExecutionException`, as [`RetryScheduler::schedule`] reports an
/// executor that no longer accepts work.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RejectedExecutionException {
    /// `getMessage()`.
    pub message: Option<String>,
}

impl fmt::Display for RejectedExecutionException {
    /// `Throwable.toString()` (what `new IOException(e).getMessage()` is).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.message {
            None => f.write_str("java.util.concurrent.RejectedExecutionException"),
            Some(m) => write!(f, "java.util.concurrent.RejectedExecutionException: {m}"),
        }
    }
}

impl std::error::Error for RejectedExecutionException {}

/// The part of `ListeningScheduledExecutorService` the retrier uses: the delay of
/// `Futures.scheduleAsync(() -> executeAsync(call, backoff), waitMillis, MILLISECONDS,
/// retryService)`.
///
/// PORT: Java runs the retried call on the scheduler's thread; here the caller's task awaits
/// the delay and then runs the retry itself.
#[async_trait]
pub trait RetryScheduler: Send + Sync {
    /// Completes once `delay_millis` have passed. An `Err` is the `RejectedExecutionException`
    /// of an executor that was shut down.
    async fn schedule(&self, delay_millis: i64) -> Result<(), RejectedExecutionException>;
}

/// The scheduler of production code: a timer of the remote runtime (`tokio::time::sleep`).
#[derive(Debug, Clone, Copy, Default)]
pub struct TokioRetryScheduler;

#[async_trait]
impl RetryScheduler for TokioRetryScheduler {
    async fn schedule(&self, delay_millis: i64) -> Result<(), RejectedExecutionException> {
        // A non-positive delay is scheduled for immediate execution.
        tokio::time::sleep(Duration::from_millis(delay_millis.max(0) as u64)).await;
        Ok(())
    }
}

// Java: com/google/devtools/build/lib/remote/Retrier.java:146 Retrier.ALLOW_ALL_CALLS
struct AllowAllCalls;

impl CircuitBreaker for AllowAllCalls {
    fn state(&self) -> CircuitBreakerState {
        CircuitBreakerState::AcceptCalls
    }

    fn record_failure(&self) {}

    fn record_success(&self) {}
}

/// Disables circuit breaking.
///
/// PORT: one shared instance (`ALLOW_ALL_CALLS.clone()`), so `Arc::ptr_eq` is Java's `==`.
// Java: com/google/devtools/build/lib/remote/Retrier.java:146 Retrier.ALLOW_ALL_CALLS
pub static ALLOW_ALL_CALLS: LazyLock<Arc<dyn CircuitBreaker>> =
    LazyLock::new(|| -> Arc<dyn CircuitBreaker> { Arc::new(AllowAllCalls) });

// Java: com/google/devtools/build/lib/remote/Retrier.java:161 Retrier.RETRIES_DISABLED
struct RetriesDisabled;

impl Backoff for RetriesDisabled {
    fn next_delay_millis(&self, _e: &io::Error) -> i64 {
        -1
    }

    fn get_retry_attempts(&self) -> i32 {
        0
    }
}

/// Disables retries.
///
/// PORT: one shared instance (`RETRIES_DISABLED.clone()`), so `Arc::ptr_eq` is Java's `==`.
// Java: com/google/devtools/build/lib/remote/Retrier.java:161 Retrier.RETRIES_DISABLED
pub static RETRIES_DISABLED: LazyLock<Arc<dyn Backoff>> =
    LazyLock::new(|| -> Arc<dyn Backoff> { Arc::new(RetriesDisabled) });

/// No backoff.
// Java: com/google/devtools/build/lib/remote/Retrier.java:175 Retrier.ZeroBackoff
#[derive(Debug)]
pub struct ZeroBackoff {
    max_retries: i32,
    retries: Mutex<i32>,
}

impl ZeroBackoff {
    // Java: com/google/devtools/build/lib/remote/Retrier.java:180 Retrier.ZeroBackoff.<init>
    pub fn new(max_retries: i32) -> ZeroBackoff {
        ZeroBackoff {
            max_retries,
            retries: Mutex::new(0),
        }
    }
}

impl Backoff for ZeroBackoff {
    // Java: com/google/devtools/build/lib/remote/Retrier.java:185 Retrier.ZeroBackoff.nextDelayMillis
    fn next_delay_millis(&self, _e: &io::Error) -> i64 {
        let mut retries = self.retries.lock();
        if *retries >= self.max_retries {
            return -1;
        }
        *retries += 1;
        0
    }

    // Java: com/google/devtools/build/lib/remote/Retrier.java:194 Retrier.ZeroBackoff.getRetryAttempts
    fn get_retry_attempts(&self) -> i32 {
        *self.retries.lock()
    }
}

/// Supports retrying the execution of a callable in case of failure.
///
/// The errors that are retried are configurable via a [`ResultClassifier`]. The delay between
/// executions is specified by a [`Backoff`]. Additionally, the retrier supports circuit breaking
/// to stop execution in case of high failure rates.
///
/// Thread-safe. PORT: cloning shares the collaborators (a Java reference to the same retrier).
// Java: com/google/devtools/build/lib/remote/Retrier.java:42 Retrier
#[derive(Clone)]
pub struct Retrier {
    backoff_supplier: BackoffSupplier,
    result_classifier: Arc<dyn ResultClassifier>,
    circuit_breaker: Arc<dyn CircuitBreaker>,
    retry_service: Arc<dyn RetryScheduler>,
    sleeper: Arc<dyn Sleeper>,
}

impl Retrier {
    // Java: com/google/devtools/build/lib/remote/Retrier.java:205 Retrier.<init>
    pub fn new(
        backoff_supplier: BackoffSupplier,
        result_classifier: Arc<dyn ResultClassifier>,
        retry_scheduler: Arc<dyn RetryScheduler>,
        circuit_breaker: Arc<dyn CircuitBreaker>,
    ) -> Retrier {
        Retrier::with_sleeper(
            backoff_supplier,
            result_classifier,
            retry_scheduler,
            circuit_breaker,
            Arc::new(milliseconds_sleep),
        )
    }

    /// The `@VisibleForTesting` constructor with an explicit [`Sleeper`].
    // Java: com/google/devtools/build/lib/remote/Retrier.java:214 Retrier.<init>
    pub fn with_sleeper(
        backoff_supplier: BackoffSupplier,
        result_classifier: Arc<dyn ResultClassifier>,
        retry_service: Arc<dyn RetryScheduler>,
        circuit_breaker: Arc<dyn CircuitBreaker>,
        sleeper: Arc<dyn Sleeper>,
    ) -> Retrier {
        Retrier {
            backoff_supplier,
            result_classifier,
            circuit_breaker,
            retry_service,
            sleeper,
        }
    }

    /// Execute a callable (`RetryableCallable`), retrying execution in case of transient failure
    /// and returning the result in case of success.
    // Java: com/google/devtools/build/lib/remote/Retrier.java:238 Retrier.execute
    pub fn execute<T>(&self, call: impl FnMut() -> io::Result<T>) -> io::Result<T> {
        let backoff = self.new_backoff();
        self.execute_with_backoff(call, &*backoff)
    }

    /// Execute a callable (`RetryableCallable`), retrying execution in case of transient failure
    /// and returning the result in case of success with given [`Backoff`].
    ///
    /// `InterruptedException` is not retried.
    ///
    /// Fails with the error of `call` if it didn't succeed within the framework specified by
    /// `backoff_supplier` and `result_classifier`; with a [`CircuitBreakerException`] in case a
    /// call was rejected because the circuit breaker tripped; with an interrupted error if the
    /// `call` fails with one.
    ///
    /// PORT: Java's `E extends Exception` type parameter is `io::Error` (the callable's checked
    /// exceptions all travel as `io::Error` in this crate).
    // Java: com/google/devtools/build/lib/remote/Retrier.java:257 Retrier.execute
    pub fn execute_with_backoff<T>(
        &self,
        mut call: impl FnMut() -> io::Result<T>,
        backoff: &dyn Backoff,
    ) -> io::Result<T> {
        loop {
            let circuit_state = self.circuit_breaker.state();
            if CircuitBreakerState::RejectCalls == circuit_state {
                return Err(CircuitBreakerException::new().into());
            }
            // PORT: `if (Thread.interrupted()) throw new InterruptedException()` has no
            // counterpart: Blaze has no thread interruption flag (crate::runtime), an interrupted
            // build surfaces as an `Interrupted` error from `call`.
            let e = match call() {
                Ok(r) => {
                    self.circuit_breaker.record_success();
                    return Ok(r);
                }
                Err(e) => e,
            };
            // catch (InterruptedException e) { throw e; }
            if errors::is_interrupted(&e) {
                return Err(e);
            }
            let r = self.result_classifier.test(&e);
            if r == ResultClassifierResult::Success {
                self.circuit_breaker.record_success();
            } else {
                self.circuit_breaker.record_failure();
            }
            if r != ResultClassifierResult::TransientFailure
                || circuit_state == CircuitBreakerState::TrialCall
            {
                return Err(e);
            }
            let delay_millis = backoff.next_delay_millis(&e);
            if delay_millis < 0 {
                return Err(e);
            }
            self.sleeper.sleep(delay_millis)?;
        }
    }

    /// Executes an `AsyncCallable`, retrying execution in case of transient failure.
    ///
    /// PORT: `call` is invoked once per attempt and returns the future of that attempt; an
    /// exception thrown by `call.call()` and a failed future are both the `Err` of that future.
    // Java: com/google/devtools/build/lib/remote/Retrier.java:293 Retrier.executeAsync
    pub async fn execute_async<T, F, Fut>(&self, call: F) -> io::Result<T>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = io::Result<T>>,
    {
        let backoff = self.new_backoff();
        self.execute_async_with_backoff(call, &*backoff).await
    }

    /// Executes an `AsyncCallable`, retrying execution in case of transient failure with the
    /// given backoff.
    // Java: com/google/devtools/build/lib/remote/Retrier.java:301 Retrier.executeAsync
    pub async fn execute_async_with_backoff<T, F, Fut>(
        &self,
        mut call: F,
        backoff: &dyn Backoff,
    ) -> io::Result<T>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = io::Result<T>>,
    {
        // PORT: Java re-enters `executeAsync(call, backoff)` from the scheduled retry; each
        // iteration of this loop is one such entry (the circuit breaker is asked again).
        loop {
            let circuit_state = self.circuit_breaker.state();
            if CircuitBreakerState::RejectCalls == circuit_state {
                return Err(CircuitBreakerException::new().into());
            }
            let t = match call().await {
                Ok(f) => {
                    self.circuit_breaker.record_success();
                    return Ok(f);
                }
                Err(t) => t,
            };
            self.on_execute_async_failure(t, backoff, circuit_state)
                .await?;
        }
    }

    /// PORT: `Ok(())` is the moment the scheduled `executeAsync(call, backoff)` starts (the
    /// caller loops); `Err` is the failed future Java returns.
    // Java: com/google/devtools/build/lib/remote/Retrier.java:325 Retrier.onExecuteAsyncFailure
    async fn on_execute_async_failure(
        &self,
        t: io::Error,
        backoff: &dyn Backoff,
        circuit_state: CircuitBreakerState,
    ) -> io::Result<()> {
        let r = self.result_classifier.test(&t);
        if r == ResultClassifierResult::TransientFailure {
            self.circuit_breaker.record_failure();
            if circuit_state == CircuitBreakerState::TrialCall {
                return Err(t);
            }
            let wait_millis = backoff.next_delay_millis(&t);
            if wait_millis >= 0 {
                match self.retry_service.schedule(wait_millis).await {
                    Ok(()) => Ok(()),
                    // May be thrown by .scheduleAsync(...) if i.e. the executor is shutdown.
                    Err(e) => Err(io::Error::other(e)),
                }
            } else {
                Err(t)
            }
        } else {
            if r == ResultClassifierResult::Success {
                self.circuit_breaker.record_success();
            } else {
                self.circuit_breaker.record_failure();
            }
            Err(t)
        }
    }

    // Java: com/google/devtools/build/lib/remote/Retrier.java:355 Retrier.newBackoff
    pub fn new_backoff(&self) -> Arc<dyn Backoff> {
        (self.backoff_supplier)()
    }

    /// `retrier::newBackoff` as a [`BackoffSupplier`] (for `new ProgressiveBackoff(retrier::newBackoff)`).
    pub fn backoff_supplier(&self) -> BackoffSupplier {
        self.backoff_supplier.clone()
    }

    // Java: com/google/devtools/build/lib/remote/Retrier.java:359 Retrier.isRetriable
    pub fn is_retriable(&self, e: &io::Error) -> bool {
        self.result_classifier.test(e) == ResultClassifierResult::TransientFailure
    }

    // Java: com/google/devtools/build/lib/remote/Retrier.java:363 Retrier.getCircuitBreaker
    pub fn get_circuit_breaker(&self) -> &Arc<dyn CircuitBreaker> {
        &self.circuit_breaker
    }
}
