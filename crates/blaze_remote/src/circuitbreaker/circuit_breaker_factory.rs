//! Port of `lib/remote/circuitbreaker/CircuitBreakerFactory.java`: factory for
//! [`CircuitBreaker`].

use std::sync::Arc;

use blaze_optdefs::misc::remote_options::{CircuitBreakerStrategy, RemoteOptions};

use super::failure_circuit_breaker::FailureCircuitBreaker;
use crate::retrier::{ALLOW_ALL_CALLS, CircuitBreaker};

// Java: com/google/devtools/build/lib/remote/circuitbreaker/CircuitBreakerFactory.java:21 CircuitBreakerFactory.DEFAULT_MIN_CALL_COUNT_TO_COMPUTE_FAILURE_RATE
pub const DEFAULT_MIN_CALL_COUNT_TO_COMPUTE_FAILURE_RATE: i32 = 100;
// Java: com/google/devtools/build/lib/remote/circuitbreaker/CircuitBreakerFactory.java:22 CircuitBreakerFactory.DEFAULT_MIN_FAIL_COUNT_TO_COMPUTE_FAILURE_RATE
pub const DEFAULT_MIN_FAIL_COUNT_TO_COMPUTE_FAILURE_RATE: i32 = 12;

/// Creates the instance of the [`CircuitBreaker`] as per the strategy defined in
/// [`RemoteOptions`]. In case of undefined strategy defaults to [`ALLOW_ALL_CALLS`]
/// implementation.
///
/// * `remote_options`: the configuration for the CircuitBreaker implementation.
///
/// Returns an instance of CircuitBreaker.
// Java: com/google/devtools/build/lib/remote/circuitbreaker/CircuitBreakerFactory.java:34 CircuitBreakerFactory.createCircuitBreaker
pub fn create_circuit_breaker(remote_options: &RemoteOptions) -> Arc<dyn CircuitBreaker> {
    if remote_options.circuit_breaker_strategy == Some(CircuitBreakerStrategy::FAILURE) {
        return Arc::new(FailureCircuitBreaker::new(
            remote_options.remote_failure_rate_threshold,
            // (int) remoteOptions.remoteFailureWindowInterval.toMillis()
            remote_options.remote_failure_window_interval.as_millis() as i64 as i32,
        ));
    }
    ALLOW_ALL_CALLS.clone()
}
