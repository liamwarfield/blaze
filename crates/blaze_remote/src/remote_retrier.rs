//! Port of `lib/remote/RemoteRetrier.java`.

use std::cell::Cell;
use std::hash::{BuildHasher, Hasher};
use std::io;
use std::ops::Deref;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use blaze_optdefs::misc::remote_options::RemoteOptions;
use parking_lot::Mutex;

use crate::execution_status_exception;
use crate::remote_retrier_utils;
use crate::retrier::{
    Backoff, BackoffSupplier, CircuitBreaker, RETRIES_DISABLED, ResultClassifier,
    ResultClassifierResult, Retrier, RetryScheduler, Sleeper,
};

/// The status of the `StatusRuntimeException` in the cause chain of `e`, if any.
///
/// PORT: `ExecutionStatusException` is a `StatusRuntimeException` in Java, hence
/// `execution_status_exception::status_from_throwable`.
// Java: com/google/devtools/build/lib/remote/RemoteRetrier.java:36 RemoteRetrier.fromException
fn from_exception(e: &io::Error) -> Option<&tonic::Status> {
    execution_status_exception::status_from_throwable(e)
}

// Java: com/google/devtools/build/lib/remote/RemoteRetrier.java:46 RemoteRetrier.EXPERIMENTAL_GRPC_RESULT_CLASSIFIER
fn experimental_grpc_result_classifier(e: &io::Error) -> ResultClassifierResult {
    let Some(s) = from_exception(e) else {
        // It's not a gRPC error.
        return ResultClassifierResult::PermanentFailure;
    };
    match s.code() {
        tonic::Code::Cancelled => {
            if !blaze_shell::thread_interrupt::is_current_interrupted() {
                ResultClassifierResult::TransientFailure
            } else {
                ResultClassifierResult::PermanentFailure
            }
        }
        tonic::Code::NotFound | tonic::Code::AlreadyExists => ResultClassifierResult::Success,
        tonic::Code::Unknown
        | tonic::Code::DeadlineExceeded
        | tonic::Code::Aborted
        | tonic::Code::Internal
        | tonic::Code::Unavailable
        | tonic::Code::ResourceExhausted => ResultClassifierResult::TransientFailure,
        _ => ResultClassifierResult::PermanentFailure,
    }
}

/// A ResultClassifier suitable to be used by ExperimentalGrpcRemoteExecutor.
// Java: com/google/devtools/build/lib/remote/RemoteRetrier.java:46 RemoteRetrier.EXPERIMENTAL_GRPC_RESULT_CLASSIFIER
pub static EXPERIMENTAL_GRPC_RESULT_CLASSIFIER: LazyLock<Arc<dyn ResultClassifier>> =
    LazyLock::new(|| -> Arc<dyn ResultClassifier> {
        Arc::new(experimental_grpc_result_classifier)
    });

// Java: com/google/devtools/build/lib/remote/RemoteRetrier.java:66 RemoteRetrier.GRPC_RESULT_CLASSIFIER
fn grpc_result_classifier(e: &io::Error) -> ResultClassifierResult {
    // A WaitExecution call in GrpcRemoteExecutor may fail with a NOT_FOUND error.
    // That means the Operation was lost on the server, and we will retry to Execute.
    if remote_retrier_utils::caused_by_status(e, tonic::Code::NotFound) {
        ResultClassifierResult::TransientFailure
    } else {
        EXPERIMENTAL_GRPC_RESULT_CLASSIFIER.test(e)
    }
}

/// A ResultClassifier suitable to be used by GrpcRemoteExecutor.
// Java: com/google/devtools/build/lib/remote/RemoteRetrier.java:66 RemoteRetrier.GRPC_RESULT_CLASSIFIER
pub static GRPC_RESULT_CLASSIFIER: LazyLock<Arc<dyn ResultClassifier>> =
    LazyLock::new(|| -> Arc<dyn ResultClassifier> { Arc::new(grpc_result_classifier) });

/// Specific retry logic for remote execution/caching.
///
/// PORT: `extends Retrier` is a wrapper that derefs to [`Retrier`].
// Java: com/google/devtools/build/lib/remote/RemoteRetrier.java:33 RemoteRetrier
#[derive(Clone)]
pub struct RemoteRetrier {
    retrier: Retrier,
}

impl Deref for RemoteRetrier {
    type Target = Retrier;

    fn deref(&self) -> &Retrier {
        &self.retrier
    }
}

impl RemoteRetrier {
    // Java: com/google/devtools/build/lib/remote/RemoteRetrier.java:75 RemoteRetrier.<init>
    pub fn new(
        options: &RemoteOptions,
        result_classifier: Arc<dyn ResultClassifier>,
        retry_scheduler: Arc<dyn RetryScheduler>,
        circuit_breaker: Arc<dyn CircuitBreaker>,
    ) -> RemoteRetrier {
        // PORT: the Java lambdas capture `options`; the two options `new
        // ExponentialBackoff(options)` reads are copied instead.
        let remote_retry_max_delay = options.remote_retry_max_delay;
        let remote_max_retry_attempts = options.remote_max_retry_attempts;
        let backoff: BackoffSupplier = if remote_max_retry_attempts > 0 {
            Arc::new(move || -> Arc<dyn Backoff> {
                Arc::new(ExponentialBackoff::from_option_values(
                    remote_retry_max_delay,
                    remote_max_retry_attempts,
                ))
            })
        } else {
            Arc::new(|| -> Arc<dyn Backoff> { RETRIES_DISABLED.clone() })
        };
        RemoteRetrier::with_backoff(backoff, result_classifier, retry_scheduler, circuit_breaker)
    }

    // Java: com/google/devtools/build/lib/remote/RemoteRetrier.java:89 RemoteRetrier.<init>
    pub fn with_backoff(
        backoff: BackoffSupplier,
        result_classifier: Arc<dyn ResultClassifier>,
        retry_scheduler: Arc<dyn RetryScheduler>,
        circuit_breaker: Arc<dyn CircuitBreaker>,
    ) -> RemoteRetrier {
        RemoteRetrier {
            retrier: Retrier::new(backoff, result_classifier, retry_scheduler, circuit_breaker),
        }
    }

    /// The `@VisibleForTesting` constructor with an explicit [`Sleeper`].
    // Java: com/google/devtools/build/lib/remote/RemoteRetrier.java:98 RemoteRetrier.<init>
    pub fn with_sleeper(
        backoff: BackoffSupplier,
        result_classifier: Arc<dyn ResultClassifier>,
        retry_scheduler: Arc<dyn RetryScheduler>,
        circuit_breaker: Arc<dyn CircuitBreaker>,
        sleeper: Arc<dyn Sleeper>,
    ) -> RemoteRetrier {
        RemoteRetrier {
            retrier: Retrier::with_sleeper(
                backoff,
                result_classifier,
                retry_scheduler,
                circuit_breaker,
                sleeper,
            ),
        }
    }

    /// Execute a callable with retries.
    // Java: com/google/devtools/build/lib/remote/RemoteRetrier.java:109 RemoteRetrier.execute
    pub fn execute<T>(&self, call: impl FnMut() -> io::Result<T>) -> io::Result<T> {
        let backoff = self.new_backoff();
        self.retrier.execute_with_backoff(call, &*backoff)
    }
}

/// `ThreadLocalRandom.current().nextDouble(bound)`, injectable so that tests can pin the jitter
/// of [`ExponentialBackoff`].
pub trait RandomSource: Send + Sync {
    /// A pseudorandom value between zero (inclusive) and `bound` (exclusive).
    fn next_double(&self, bound: f64) -> f64;
}

/// `ThreadLocalRandom.current()`.
// Java: java/util/concurrent/ThreadLocalRandom.java ThreadLocalRandom
#[derive(Debug, Clone, Copy, Default)]
pub struct ThreadLocalRandom;

thread_local! {
    // PORT: the JDK seeds each thread from a process-wide seeder; here from the standard
    // library's randomly keyed hasher.
    static THREAD_LOCAL_RANDOM_SEED: Cell<u64> = Cell::new(
        std::collections::hash_map::RandomState::new().build_hasher().finish(),
    );
}

impl ThreadLocalRandom {
    /// The seed increment.
    // Java: java/util/concurrent/ThreadLocalRandom.java ThreadLocalRandom.GAMMA
    const GAMMA: u64 = 0x9e3779b97f4a7c15;

    // Java: java/util/concurrent/ThreadLocalRandom.java ThreadLocalRandom.mix64
    fn mix64(mut z: u64) -> u64 {
        z = (z ^ (z >> 33)).wrapping_mul(0xff51afd7ed558ccd);
        z = (z ^ (z >> 33)).wrapping_mul(0xc4ceb9fe1a85ec53);
        z ^ (z >> 33)
    }

    // Java: java/util/concurrent/ThreadLocalRandom.java ThreadLocalRandom.nextSeed
    fn next_seed() -> u64 {
        THREAD_LOCAL_RANDOM_SEED.with(|seed| {
            let r = seed.get().wrapping_add(ThreadLocalRandom::GAMMA);
            seed.set(r);
            r
        })
    }
}

impl RandomSource for ThreadLocalRandom {
    // Java: java/util/concurrent/ThreadLocalRandom.java ThreadLocalRandom.nextDouble(double)
    fn next_double(&self, bound: f64) -> f64 {
        // `!(bound > 0.0)` also rejects NaN.
        blaze_util::check_argument!(bound > 0.0, "bound must be positive");
        // (mix64(nextSeed()) >>> 11) * DOUBLE_UNIT * bound, with DOUBLE_UNIT = 0x1.0p-53
        let result = (ThreadLocalRandom::mix64(ThreadLocalRandom::next_seed()) >> 11) as f64
            * (1.0 / (1u64 << 53) as f64)
            * bound;
        if result < bound {
            result
        } else {
            // correct for rounding: Double.longBitsToDouble(Double.doubleToLongBits(bound) - 1)
            f64::from_bits(bound.to_bits() - 1)
        }
    }
}

/// The mutable fields of [`ExponentialBackoff`].
#[derive(Debug)]
struct ExponentialBackoffState {
    next_delay_millis: i64,
    attempts: i32,
}

/// Backoff strategy that backs off exponentially.
// Java: com/google/devtools/build/lib/remote/RemoteRetrier.java:115 RemoteRetrier.ExponentialBackoff
pub struct ExponentialBackoff {
    max_millis: i64,
    state: Mutex<ExponentialBackoffState>,
    multiplier: f64,
    jitter: f64,
    max_attempts: i32,
    // PORT: `ThreadLocalRandom.current()` in Java.
    random: Arc<dyn RandomSource>,
}

impl ExponentialBackoff {
    /// Creates a Backoff supplier for an optionally jittered exponential backoff. The supplier
    /// is ThreadSafe (non-synchronized calls to get() are fine), but the returned Backoff is not.
    ///
    /// * `initial`: the initial backoff duration.
    /// * `max`: the maximum backoff duration.
    /// * `multiplier`: the amount the backoff should increase in each iteration. Must be >1.
    /// * `jitter`: the amount the backoff should be randomly varied (0-1), with 0 providing no
    ///   jitter, and 1 providing a duration that is 0-200% of the non-jittered duration.
    /// * `max_attempts`: maximal times to attempt a retry 0 means no retries.
    // Java: com/google/devtools/build/lib/remote/RemoteRetrier.java:135 RemoteRetrier.ExponentialBackoff.<init>
    pub fn new(
        initial: Duration,
        max: Duration,
        multiplier: f64,
        jitter: f64,
        max_attempts: i32,
    ) -> ExponentialBackoff {
        ExponentialBackoff::with_random(
            initial,
            max,
            multiplier,
            jitter,
            max_attempts,
            Arc::new(ThreadLocalRandom),
        )
    }

    /// [`ExponentialBackoff::new`] with the source of the jitter's randomness (tests).
    // Java: com/google/devtools/build/lib/remote/RemoteRetrier.java:135 RemoteRetrier.ExponentialBackoff.<init>
    pub fn with_random(
        initial: Duration,
        max: Duration,
        multiplier: f64,
        jitter: f64,
        max_attempts: i32,
        random: Arc<dyn RandomSource>,
    ) -> ExponentialBackoff {
        blaze_util::check_argument!(multiplier > 1.0, "multipler must be > 1");
        blaze_util::check_argument!(
            (0.0..=1.0).contains(&jitter),
            "jitter must be in the range (0, 1)"
        );
        blaze_util::check_argument!(max_attempts >= 0, "maxAttempts must be >= 0");
        let next_delay_millis = initial.as_millis() as i64;
        let max_millis = std::cmp::max(max.as_millis() as i64, next_delay_millis);
        ExponentialBackoff {
            max_millis,
            state: Mutex::new(ExponentialBackoffState {
                next_delay_millis,
                attempts: 0,
            }),
            multiplier,
            jitter,
            max_attempts,
            random,
        }
    }

    // Java: com/google/devtools/build/lib/remote/RemoteRetrier.java:147 RemoteRetrier.ExponentialBackoff.<init>
    pub fn from_options(options: &RemoteOptions) -> ExponentialBackoff {
        ExponentialBackoff::from_option_values(
            options.remote_retry_max_delay,
            options.remote_max_retry_attempts,
        )
    }

    /// `new ExponentialBackoff(options)` from the two options it reads.
    // Java: com/google/devtools/build/lib/remote/RemoteRetrier.java:147 RemoteRetrier.ExponentialBackoff.<init>
    fn from_option_values(
        remote_retry_max_delay: Duration,
        remote_max_retry_attempts: i32,
    ) -> ExponentialBackoff {
        ExponentialBackoff::new(
            /* initial= */ Duration::from_millis(100),
            /* max= */ remote_retry_max_delay,
            /* multiplier= */ 2.0,
            /* jitter= */ 0.1,
            remote_max_retry_attempts,
        )
    }
}

impl Backoff for ExponentialBackoff {
    // Java: com/google/devtools/build/lib/remote/RemoteRetrier.java:157 RemoteRetrier.ExponentialBackoff.nextDelayMillis
    fn next_delay_millis(&self, _e: &io::Error) -> i64 {
        let mut state = self.state.lock();
        if state.attempts == self.max_attempts {
            return -1;
        }
        state.attempts += 1;
        let jitter_ratio = self.jitter * (self.random.next_double(2.0) - 1.0);
        // PORT: `as i64` is Java's `(long)` narrowing of a double (saturating, NaN is 0).
        let result = (state.next_delay_millis as f64 * (1.0 + jitter_ratio)) as i64;
        // Advance current by the non-jittered result.
        state.next_delay_millis = (state.next_delay_millis as f64 * self.multiplier) as i64;
        if state.next_delay_millis > self.max_millis {
            state.next_delay_millis = self.max_millis;
        }
        result
    }

    // Java: com/google/devtools/build/lib/remote/RemoteRetrier.java:173 RemoteRetrier.ExponentialBackoff.getRetryAttempts
    fn get_retry_attempts(&self) -> i32 {
        self.state.lock().attempts
    }
}

/// The mutable fields of [`ProgressiveBackoff`].
struct ProgressiveBackoffState {
    current_backoff: Option<Arc<dyn Backoff>>,
    retries: i32,
}

/// `RemoteRetrier.ProgressiveBackoff`: a resettable [`Backoff`] for progressive reads and writes
/// (see [`ProgressiveBackoff::new`]).
// Java: com/google/devtools/build/lib/remote/RemoteRetrier.java:178 RemoteRetrier.ProgressiveBackoff
pub struct ProgressiveBackoff {
    backoff_supplier: BackoffSupplier,
    state: Mutex<ProgressiveBackoffState>,
}

impl ProgressiveBackoff {
    /// Creates a resettable Backoff for progressive reads. After a reset, the nextDelay returned
    /// indicates an immediate retry. Initially and after indicating an immediate retry, a
    /// delegate is generated to provide nextDelay until reset.
    ///
    /// * `backoff_supplier`: delegate Backoff generator
    // Java: com/google/devtools/build/lib/remote/RemoteRetrier.java:191 RemoteRetrier.ProgressiveBackoff.<init>
    pub fn new(backoff_supplier: BackoffSupplier) -> ProgressiveBackoff {
        let current_backoff = (backoff_supplier)();
        ProgressiveBackoff {
            backoff_supplier,
            state: Mutex::new(ProgressiveBackoffState {
                current_backoff: Some(current_backoff),
                retries: 0,
            }),
        }
    }

    // Java: com/google/devtools/build/lib/remote/RemoteRetrier.java:196 RemoteRetrier.ProgressiveBackoff.reset
    pub fn reset(&self) {
        let mut state = self.state.lock();
        if let Some(current_backoff) = state.current_backoff.take() {
            state.retries += current_backoff.get_retry_attempts();
        }
        // currentBackoff = null (already taken)
    }
}

impl Backoff for ProgressiveBackoff {
    // Java: com/google/devtools/build/lib/remote/RemoteRetrier.java:204 RemoteRetrier.ProgressiveBackoff.nextDelayMillis
    fn next_delay_millis(&self, e: &io::Error) -> i64 {
        let mut state = self.state.lock();
        let current_backoff = state.current_backoff.clone();
        match current_backoff {
            None => {
                state.current_backoff = Some((self.backoff_supplier)());
                state.retries += 1;
                0
            }
            Some(current_backoff) => current_backoff.next_delay_millis(e),
        }
    }

    // Java: com/google/devtools/build/lib/remote/RemoteRetrier.java:214 RemoteRetrier.ProgressiveBackoff.getRetryAttempts
    fn get_retry_attempts(&self) -> i32 {
        let state = self.state.lock();
        let mut retry_attempts = state.retries;
        if let Some(current_backoff) = &state.current_backoff {
            retry_attempts += current_backoff.get_retry_attempts();
        }
        retry_attempts
    }
}
