#![cfg(feature = "wip")]
//! Port of
//! `src/test/java/com/google/devtools/build/lib/remote/circuitbreaker/FailureCircuitBreakerTest.java`.

use std::hash::{BuildHasher, Hasher};
use std::thread;
use std::time::Duration;

use blaze_remote::circuitbreaker::{
    DEFAULT_MIN_CALL_COUNT_TO_COMPUTE_FAILURE_RATE, DEFAULT_MIN_FAIL_COUNT_TO_COMPUTE_FAILURE_RATE,
    FailureCircuitBreaker,
};
use blaze_remote::retrier::{CircuitBreaker, CircuitBreakerState};

/// One of the `Runnable`s of the Java test.
#[derive(Clone, Copy)]
enum Call {
    /// `failureCircuitBreaker::recordFailure`.
    RecordFailure,
    /// `failureCircuitBreaker::recordSuccess`.
    RecordSuccess,
}

/// `Collections.shuffle(list)`.
fn shuffle(list: &mut [Call]) {
    let mut state = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish()
        | 1;
    for i in (1..list.len()).rev() {
        // xorshift64
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let j = (state % (i as u64 + 1)) as usize;
        list.swap(i, j);
    }
}

/// `list.stream().parallel().forEach(Runnable::run)`.
fn run_in_parallel(failure_circuit_breaker: &FailureCircuitBreaker, list: &[Call]) {
    thread::scope(|scope| {
        for chunk in list.chunks(list.len().div_ceil(8)) {
            scope.spawn(move || {
                for call in chunk {
                    match call {
                        Call::RecordFailure => failure_circuit_breaker.record_failure(),
                        Call::RecordSuccess => failure_circuit_breaker.record_success(),
                    }
                }
            });
        }
    });
}

#[test]
fn test_record_failure_circuit_trips() {
    let failure_rate_threshold: i32 = 10;
    let window_interval: i32 = 100;
    let failure_circuit_breaker =
        FailureCircuitBreaker::new(failure_rate_threshold, window_interval);

    let mut list_of_success_and_failure_calls: Vec<Call> = Vec::new();
    for _ in 0..failure_rate_threshold {
        list_of_success_and_failure_calls.push(Call::RecordFailure);
    }

    for _ in 0..failure_rate_threshold * 9 {
        list_of_success_and_failure_calls.push(Call::RecordSuccess);
    }

    shuffle(&mut list_of_success_and_failure_calls);

    // make calls equals to threshold number of not ignored failure calls in parallel.
    run_in_parallel(&failure_circuit_breaker, &list_of_success_and_failure_calls);
    assert_eq!(
        failure_circuit_breaker.state(),
        CircuitBreakerState::AcceptCalls
    );

    // Sleep for windowInterval + 5ms.
    thread::sleep(Duration::from_millis(
        window_interval as u64 + 5, /*to compensate any delay*/
    ));

    // make calls equals to threshold number of not ignored failure calls in parallel.
    run_in_parallel(&failure_circuit_breaker, &list_of_success_and_failure_calls);
    assert_eq!(
        failure_circuit_breaker.state(),
        CircuitBreakerState::AcceptCalls
    );

    // Sleep for less than windowInterval.
    thread::sleep(Duration::from_millis(window_interval as u64 - 5));
    failure_circuit_breaker.record_failure();
    assert_eq!(
        failure_circuit_breaker.state(),
        CircuitBreakerState::RejectCalls
    );
}

#[test]
fn test_record_failure_min_call_criteria_not_met() {
    let failure_rate_threshold: i32 = 0;
    let window_interval: i32 = 100;
    let min_call_to_compute_failure = DEFAULT_MIN_CALL_COUNT_TO_COMPUTE_FAILURE_RATE;
    let failure_circuit_breaker =
        FailureCircuitBreaker::new(failure_rate_threshold, window_interval);

    // make success calls, failure call and number of total calls less than
    // minCallToComputeFailure.
    for _ in 0..min_call_to_compute_failure - 2 {
        failure_circuit_breaker.record_success();
    }
    failure_circuit_breaker.record_failure();
    assert_eq!(
        failure_circuit_breaker.state(),
        CircuitBreakerState::AcceptCalls
    );

    // Sleep for less than windowInterval.
    thread::sleep(Duration::from_millis(window_interval as u64 - 50));
    failure_circuit_breaker.record_failure();
    assert_eq!(
        failure_circuit_breaker.state(),
        CircuitBreakerState::RejectCalls
    );
}

#[test]
fn test_record_failure_min_fail_criteria_not_met() {
    let failure_rate_threshold: i32 = 10;
    let window_interval: i32 = 100;
    let min_fail_to_compute_failure = DEFAULT_MIN_FAIL_COUNT_TO_COMPUTE_FAILURE_RATE;
    let failure_circuit_breaker =
        FailureCircuitBreaker::new(failure_rate_threshold, window_interval);

    // make number of failure calls less than minFailToComputeFailure.
    for _ in 0..min_fail_to_compute_failure - 1 {
        failure_circuit_breaker.record_failure();
    }
    assert_eq!(
        failure_circuit_breaker.state(),
        CircuitBreakerState::AcceptCalls
    );

    // Sleep for less than windowInterval.
    thread::sleep(Duration::from_millis(window_interval as u64 - 50));
    failure_circuit_breaker.record_failure();
    assert_eq!(
        failure_circuit_breaker.state(),
        CircuitBreakerState::RejectCalls
    );
}
