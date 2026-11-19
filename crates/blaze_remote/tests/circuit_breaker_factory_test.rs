//! Port of `src/test/java/com/google/devtools/build/lib/remote/circuitbreaker/CircuitBreakerFactoryTest.java`:
//! tests for `CircuitBreakerFactory`.

use std::sync::Arc;

use blaze_optdefs::misc::remote_options::{CircuitBreakerStrategy, RemoteOptions};
use blaze_remote::circuitbreaker::{
    DEFAULT_MIN_FAIL_COUNT_TO_COMPUTE_FAILURE_RATE, create_circuit_breaker,
};
use blaze_remote::retrier::{ALLOW_ALL_CALLS, CircuitBreakerState};

#[test]
// Java mutates the defaults instance in place.
#[allow(clippy::field_reassign_with_default)]
fn test_create_circuit_breaker_failure_strategy() {
    let mut remote_options = RemoteOptions::default();
    remote_options.circuit_breaker_strategy = Some(CircuitBreakerStrategy::FAILURE);

    // PORT: `isInstanceOf(FailureCircuitBreaker.class)`. A `dyn CircuitBreaker` has no
    // `instanceof`: the breaker is not the shared ALLOW_ALL_CALLS instance (the only other value
    // the factory returns) and it trips the way a FailureCircuitBreaker with the default options
    // (threshold 10%) does once the minimal number of failures was recorded.
    let circuit_breaker = create_circuit_breaker(&remote_options);
    assert!(!Arc::ptr_eq(&circuit_breaker, &ALLOW_ALL_CALLS));
    for _ in 0..DEFAULT_MIN_FAIL_COUNT_TO_COMPUTE_FAILURE_RATE {
        assert_eq!(circuit_breaker.state(), CircuitBreakerState::AcceptCalls);
        circuit_breaker.record_failure();
    }
    assert_eq!(circuit_breaker.state(), CircuitBreakerState::RejectCalls);
}

#[test]
fn test_create_circuit_breaker_null_strategy() {
    let remote_options = RemoteOptions::default();
    assert!(Arc::ptr_eq(
        &create_circuit_breaker(&remote_options),
        &ALLOW_ALL_CALLS
    ));
}
