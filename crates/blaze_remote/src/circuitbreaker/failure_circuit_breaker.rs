//! Port of `lib/remote/circuitbreaker/FailureCircuitBreaker.java`.

use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

use parking_lot::Mutex;

use super::circuit_breaker_factory::{
    DEFAULT_MIN_CALL_COUNT_TO_COMPUTE_FAILURE_RATE, DEFAULT_MIN_FAIL_COUNT_TO_COMPUTE_FAILURE_RATE,
};
use crate::retrier::{CircuitBreaker, CircuitBreakerState};

/// The [`FailureCircuitBreaker`] implementation of the [`CircuitBreaker`] prevents further calls
/// to a remote cache once the failures rate within a given window exceeds a specified threshold
/// for a build. In the context of Bazel, a new instance of [`CircuitBreaker`] is created for
/// each build. Therefore, if the circuit breaker trips during a build, the remote cache will be
/// disabled for that build. However, it will be enabled again for the next build as a new
/// instance of [`CircuitBreaker`] will be created.
// Java: com/google/devtools/build/lib/remote/circuitbreaker/FailureCircuitBreaker.java:30 FailureCircuitBreaker
#[derive(Debug)]
pub struct FailureCircuitBreaker {
    state: Mutex<CircuitBreakerState>,
    successes: Arc<AtomicI32>,
    failures: Arc<AtomicI32>,
    failure_rate_threshold: i32,
    sliding_window_size: i32,
    min_call_count_to_compute_failure_rate: i32,
    min_fail_count_to_compute_failure_rate: i32,
    // PORT: Java's `scheduledExecutor` (a single-thread scheduled executor when
    // `slidingWindowSize > 0`) is a timer task on the remote runtime, see `schedule_decrement`.
}

impl FailureCircuitBreaker {
    /// Creates a [`FailureCircuitBreaker`].
    ///
    /// * `failure_rate_threshold` is used to set the min percentage of failure required to trip
    ///   the circuit breaker in given time window.
    /// * `sliding_window_size`: the size of the sliding window in milliseconds to calculate the
    ///   number of failures.
    // Java: com/google/devtools/build/lib/remote/circuitbreaker/FailureCircuitBreaker.java:49 FailureCircuitBreaker.<init>
    pub fn new(failure_rate_threshold: i32, sliding_window_size: i32) -> FailureCircuitBreaker {
        FailureCircuitBreaker {
            failures: Arc::new(AtomicI32::new(0)),
            successes: Arc::new(AtomicI32::new(0)),
            failure_rate_threshold,
            sliding_window_size,
            min_call_count_to_compute_failure_rate: DEFAULT_MIN_CALL_COUNT_TO_COMPUTE_FAILURE_RATE,
            min_fail_count_to_compute_failure_rate: DEFAULT_MIN_FAIL_COUNT_TO_COMPUTE_FAILURE_RATE,
            state: Mutex::new(CircuitBreakerState::AcceptCalls),
        }
    }

    /// `scheduledExecutor.schedule(counter::decrementAndGet, slidingWindowSize, MILLISECONDS)`.
    fn schedule_decrement(&self, counter: &Arc<AtomicI32>) {
        let counter = counter.clone();
        let delay = Duration::from_millis(self.sliding_window_size as u64);
        // PORT: the task owns its counter (nothing is borrowed), so it may outlive `self` as the
        // Java executor's queue does.
        crate::runtime::runtime().spawn(async move {
            tokio::time::sleep(delay).await;
            counter.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

impl CircuitBreaker for FailureCircuitBreaker {
    // Java: com/google/devtools/build/lib/remote/circuitbreaker/FailureCircuitBreaker.java:64 FailureCircuitBreaker.state
    fn state(&self) -> CircuitBreakerState {
        *self.state.lock()
    }

    // Java: com/google/devtools/build/lib/remote/circuitbreaker/FailureCircuitBreaker.java:69 FailureCircuitBreaker.recordFailure
    fn record_failure(&self) {
        let failure_count = self.failures.fetch_add(1, Ordering::SeqCst).wrapping_add(1);
        let total_call_count = self
            .successes
            .load(Ordering::SeqCst)
            .wrapping_add(failure_count);
        if self.sliding_window_size > 0 {
            self.schedule_decrement(&self.failures);
        }

        if total_call_count < self.min_call_count_to_compute_failure_rate
            && failure_count < self.min_fail_count_to_compute_failure_rate
        {
            // The remote call count is below the threshold required to calculate the failure rate.
            return;
        }
        let failure_rate = (failure_count as f64 * 100.0) / total_call_count as f64;

        // Since the state can only be changed to the open state, synchronization is not required.
        if failure_rate > self.failure_rate_threshold as f64 {
            *self.state.lock() = CircuitBreakerState::RejectCalls;
        }
    }

    // Java: com/google/devtools/build/lib/remote/circuitbreaker/FailureCircuitBreaker.java:92 FailureCircuitBreaker.recordSuccess
    fn record_success(&self) {
        self.successes.fetch_add(1, Ordering::SeqCst);
        if self.sliding_window_size > 0 {
            self.schedule_decrement(&self.successes);
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}
