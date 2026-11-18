//! Port of `com.google.devtools.build.lib.remote.circuitbreaker`.

pub mod circuit_breaker_factory;
pub mod failure_circuit_breaker;

pub use circuit_breaker_factory::{
    DEFAULT_MIN_CALL_COUNT_TO_COMPUTE_FAILURE_RATE, DEFAULT_MIN_FAIL_COUNT_TO_COMPUTE_FAILURE_RATE,
    create_circuit_breaker,
};
pub use failure_circuit_breaker::FailureCircuitBreaker;
