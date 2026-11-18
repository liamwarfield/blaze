//! Blaze's evaluation engine: a port of Bazel's Skyframe (`com.google.devtools.build.skyframe`)
//! shaped so that `SkyFunction`s port line by line.
//!
//! # Model
//!
//! Bazel's evaluator restarts a function whenever `env.getValue` returns null for a dependency
//! that is not computed yet. Blaze instead *blocks*: `Environment::get_value` computes a missing
//! dependency (inline on this thread, on a helper thread, or by waiting for the thread already
//! computing it) and then returns it. The Java protocol is kept so ported code reads the same:
//!
//! * `get_value` returns `Ok(None)` only when the dependency finished in error, is part of (or
//!   depends on) a cycle, or was abandoned because the evaluation is aborting. As in Java, the
//!   function should then return `Ok(None)` and the engine decides the node's error from its
//!   children (`ErrorInfo::from_child_errors`).
//! * `get_value_or_throw::<K, E>` returns `Ok(Err(e))` when the dependency failed with an `E`,
//!   exactly like `getValueOrThrow(key, E.class)`.
//! * `get_values_and_exceptions` evaluates a batch in parallel and returns a
//!   [`SkyframeLookupResult`] (`get`, `get_or_throw`), like Java.
//! * `values_missing()` reports whether any requested dependency was unavailable.
//! * Every dependency request returns `Err(Interrupted)` once the evaluator is interrupted
//!   (Java's `InterruptedException`); `?` propagates it through [`ComputeError`].
//!
//! Each `Evaluator::evaluate(roots, ctx)` call takes its own [`EvaluationContext`]
//! (`keep_going`, parallelism, event handler), like `MemoizingEvaluator.evaluate`. Results that
//! Java would not have committed to the graph (nodes still in flight when a no-keep-going
//! evaluation aborted, nodes on the error-bubbling path, nodes in or above a cycle that the cycle
//! detector did not reach, interrupted nodes) are dropped at the end of the call and re-evaluated
//! by the next call. Committed values and errors are memoized for the life of the evaluator.
//!
//! A single build per process is assumed (no invalidation of changed files, no change pruning,
//! no versions). `// PORT: skipped, incremental` marks the Java features this leaves out.
//!
//! ## Errors
//!
//! * keep_going: every node is computed as far as possible. A function that throws while a
//!   requested dependency is missing has its exception ignored, as in Java, and gets its
//!   children's error.
//! * no keep_going: the first exception thrown stops new work (`preventNewEvaluations`). Later
//!   independent exceptions are dropped, like Java. Parents waiting on the failed node see its
//!   error (Java's error bubbling, done during normal compute here) and may wrap it. The result
//!   then holds the committed error(s) plus exactly one bubbled root error (the first such root in
//!   request order); roots that only saw the abort have neither a value nor an error
//!   (`ParallelEvaluator.constructResult`).
//! * Catastrophic exceptions (`SkyFunctionException::is_catastrophic`) stop a keep_going
//!   evaluation the same way and set `EvaluationResult::catastrophe`.
//!
//! ## Cycles
//!
//! A cycle cannot be found by waiting for the evaluation to stall, because callers block. Each
//! thread records the stack of keys it computes inline and the keys it waits for. Before waiting,
//! a thread walks this wait-for graph under one lock; if the walk reaches itself, the request
//! returns "not done" (`Ok(None)`, values missing) instead of deadlocking, and every node above
//! finishes "in cycle". After the evaluation, Java's `SimpleCycleDetector` runs over the
//! dependencies each node recorded in request order (Java's `GroupedDeps`), from each top-level
//! key, so the reported [`CycleInfo`]s do not depend on which thread closed the loop.
//!
//! ## Threads
//!
//! At most `parallelism` threads compute at once (a permit pool). A thread that blocks on another
//! thread's computation, or on a batch it handed out, gives its permit back while it waits.
//! `get_values_and_exceptions` with several unstarted keys spreads them over helper threads, up
//! to a global cap on live threads (`EvaluationContext::max_threads`); beyond it the batch is
//! evaluated inline. A key whose inline computation would be deeper than
//! `EvaluationContext::max_inline_depth` frames on the current thread is computed on a fresh
//! thread with a large reserved stack (`thread_stack_size`).
//!
//! ## Rules for ported SkyFunctions
//!
//! * Never hold a lock across `get_value` / `get_values_and_exceptions`: the call may compute
//!   arbitrary other nodes on this thread or block on other threads.
//! * Only immutable, `Send + Sync` values (frozen Starlark values) cross nodes.

#![allow(clippy::too_many_arguments, clippy::new_without_default)]

mod cycle_info;
mod environment;
mod error_info;
mod evaluation_result;
mod evaluator;
mod grouped_deps;
mod simple_cycle_detector;
mod walkable_graph;

pub use cycle_info::CycleInfo;
pub use environment::{Environment, SkyframeLookupResult, ValueOrThrow};
pub use error_info::ErrorInfo;
pub use evaluation_result::EvaluationResult;
pub use evaluator::{
    ERROR_TRANSIENCE_KEY, ErrorTransienceKey, ErrorTransienceValue, EvaluationContext, Evaluator,
    EvaluatorBuilder,
};
pub use grouped_deps::GroupedDeps;
pub use walkable_graph::WalkableGraph;

use std::any::Any;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

// Java: com/google/devtools/build/skyframe/SkyFunctionName.java:23 SkyFunctionName
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SkyFunctionName(pub &'static str);

impl SkyFunctionName {
    // Java: com/google/devtools/build/skyframe/SkyFunctionName.java:80 SkyFunctionName.getName
    pub fn get_name(&self) -> &'static str {
        self.0
    }
}

impl fmt::Debug for SkyFunctionName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl fmt::Display for SkyFunctionName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// A typed Skyframe key. Implement this for each Java `SkyKey` class.
///
/// `Debug` output is used in cycle and error reports; make it match Java's `toString` where
/// Bazel prints keys.
pub trait Key: fmt::Debug + Eq + Hash + Send + Sync + 'static {
    /// The value type the key's function produces.
    type Value: Send + Sync + 'static;
    /// Java: SkyKey.functionName()
    const FUNCTION: SkyFunctionName;
}

/// Object-safe view of a [`Key`].
pub trait DynKey: fmt::Debug + Send + Sync + 'static {
    fn function_name(&self) -> SkyFunctionName;
    fn as_any(&self) -> &dyn Any;
    fn dyn_eq(&self, other: &dyn DynKey) -> bool;
    fn dyn_hash(&self, state: &mut dyn Hasher);
}

impl<K: Key> DynKey for K {
    fn function_name(&self) -> SkyFunctionName {
        K::FUNCTION
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn dyn_eq(&self, other: &dyn DynKey) -> bool {
        other.as_any().downcast_ref::<K>() == Some(self)
    }
    fn dyn_hash(&self, mut state: &mut dyn Hasher) {
        K::FUNCTION.hash(&mut state);
        self.hash(&mut state);
    }
}

/// A type-erased, cheaply clonable key. Java: `SkyKey`.
#[derive(Clone)]
pub struct SkyKey(Arc<dyn DynKey>);

impl SkyKey {
    pub fn new<K: Key>(key: K) -> Self {
        SkyKey(Arc::new(key))
    }
    // Java: com/google/devtools/build/skyframe/SkyKey.java:42 SkyKey.functionName
    pub fn function_name(&self) -> SkyFunctionName {
        self.0.function_name()
    }
    pub fn downcast<K: Key>(&self) -> Option<&K> {
        self.0.as_any().downcast_ref::<K>()
    }
    // Java: com/google/devtools/build/skyframe/SkyKey.java:44 SkyKey.argument
    pub fn argument(&self) -> &dyn DynKey {
        &*self.0
    }
}

impl PartialEq for SkyKey {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0.dyn_eq(&*other.0)
    }
}
impl Eq for SkyKey {}
impl Hash for SkyKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.dyn_hash(state)
    }
}
impl fmt::Debug for SkyKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&*self.0, f)
    }
}
impl<K: Key> From<K> for SkyKey {
    fn from(k: K) -> Self {
        SkyKey::new(k)
    }
}

/// A computed value. Java: `SkyValue`.
pub type SkyValue = Arc<dyn Any + Send + Sync>;

/// An error a function failed with. Java: the cause wrapped in a `SkyFunctionException`.
pub type SkyError = Arc<dyn std::error::Error + Send + Sync>;

// Java: com/google/devtools/build/skyframe/SkyFunctionException.java:47 SkyFunctionException.Transience
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transience {
    /// An error that may or may not occur again if the node were reevaluated, even when Skyframe
    /// dependencies have not changed. If a node results in a transient error and is needed on a
    /// subsequent MemoizingEvaluator#evaluate call, it will be reevaluated.
    Transient,
    /// An error that is completely deterministic in terms of the node's Skyframe dependencies.
    /// Persistent errors may be cached.
    Persistent,
}

/// Base of exceptions thrown by [`SkyFunction::compute`] on failure.
///
/// Java subclasses override `isCatastrophic()`; here it is the `catastrophic` field, set with
/// [`SkyFunctionException::catastrophic`].
// Java: com/google/devtools/build/skyframe/SkyFunctionException.java:44 SkyFunctionException
#[derive(Clone, Debug)]
pub struct SkyFunctionException {
    pub cause: SkyError,
    pub transience: Transience,
    pub catastrophic: bool,
}

impl SkyFunctionException {
    // Java: com/google/devtools/build/skyframe/SkyFunctionException.java:64 SkyFunctionException.<init>
    pub fn new<E: std::error::Error + Send + Sync + 'static>(
        cause: E,
        transience: Transience,
    ) -> Self {
        Self::from_cause(Arc::new(cause), transience)
    }
    /// Wraps an already shared cause (for example a dependency's exception being rethrown).
    pub fn from_cause(cause: SkyError, transience: Transience) -> Self {
        SkyFunctionException {
            cause,
            transience,
            catastrophic: false,
        }
    }
    pub fn persistent<E: std::error::Error + Send + Sync + 'static>(cause: E) -> Self {
        Self::new(cause, Transience::Persistent)
    }
    pub fn transient<E: std::error::Error + Send + Sync + 'static>(cause: E) -> Self {
        Self::new(cause, Transience::Transient)
    }
    /// A subclass whose `isCatastrophic()` returns true: halts the build even with keep_going.
    pub fn catastrophic<E: std::error::Error + Send + Sync + 'static>(
        cause: E,
        transience: Transience,
    ) -> Self {
        SkyFunctionException {
            catastrophic: true,
            ..Self::new(cause, transience)
        }
    }
    // Java: com/google/devtools/build/skyframe/SkyFunctionException.java:70 SkyFunctionException.isTransient
    pub fn is_transient(&self) -> bool {
        self.transience == Transience::Transient
    }
    /// Catastrophic failures halt the build even when in keepGoing mode.
    // Java: com/google/devtools/build/skyframe/SkyFunctionException.java:75 SkyFunctionException.isCatastrophic
    pub fn is_catastrophic(&self) -> bool {
        self.catastrophic
    }
    // Java: com/google/devtools/build/skyframe/SkyFunctionException.java:80 SkyFunctionException.getCause
    pub fn get_cause(&self) -> &SkyError {
        &self.cause
    }
}

/// Java's `InterruptedException`: the evaluation was interrupted (Ctrl-C, or
/// [`Evaluator::interrupt`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Interrupted;

impl fmt::Display for Interrupted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Java: Throwable.toString of a message-less InterruptedException.
        f.write_str("java.lang.InterruptedException")
    }
}
impl std::error::Error for Interrupted {}

/// What `SkyFunction.compute` may throw: `throws SkyFunctionException, InterruptedException`.
///
/// PORT: Java declares two checked exceptions; Rust needs one error type. Both convert with `?`.
#[derive(Clone, Debug)]
pub enum ComputeError {
    Exception(SkyFunctionException),
    Interrupted(Interrupted),
}

impl From<SkyFunctionException> for ComputeError {
    fn from(e: SkyFunctionException) -> Self {
        ComputeError::Exception(e)
    }
}
impl From<Interrupted> for ComputeError {
    fn from(e: Interrupted) -> Self {
        ComputeError::Interrupted(e)
    }
}

/// The result type of a [`SkyFunction`]: `Ok(Some(v))` computed, `Ok(None)` a dependency was
/// unavailable (Java returns null), `Err(e)` the function threw.
pub type ComputeResult = Result<Option<SkyValue>, ComputeError>;

// Java: com/google/devtools/build/skyframe/SkyFunction.java:45 SkyFunction
pub trait SkyFunction: Send + Sync + 'static {
    // Java: com/google/devtools/build/skyframe/SkyFunction.java:81 SkyFunction.compute
    fn compute(&self, key: &SkyKey, env: &mut Environment<'_>) -> ComputeResult;

    /// Extracts a tag (target label) from a SkyKey if it has one. Otherwise return `None`.
    ///
    /// The tag is used for filtering out non-error event messages that do not match
    /// `--output_filter` flag. If a SkyFunction returns `None`, its events are never filtered.
    // Java: com/google/devtools/build/skyframe/SkyFunction.java:92 SkyFunction.extractTag
    fn extract_tag(&self, _key: &SkyKey) -> Option<String> {
        None
    }
}

impl<F> SkyFunction for F
where
    F: Fn(&SkyKey, &mut Environment<'_>) -> ComputeResult + Send + Sync + 'static,
{
    fn compute(&self, key: &SkyKey, env: &mut Environment<'_>) -> ComputeResult {
        self(key, env)
    }
}

/// Wraps a typed function `Fn(&K, &mut Environment) -> Result<Option<K::Value>, ComputeError>`.
pub fn typed<K, F>(f: F) -> impl SkyFunction
where
    K: Key,
    F: Fn(&K, &mut Environment<'_>) -> Result<Option<K::Value>, ComputeError>
        + Send
        + Sync
        + 'static,
{
    move |key: &SkyKey, env: &mut Environment<'_>| {
        let k = key.downcast::<K>().expect("key type matches function name");
        Ok(f(k, env)?.map(|v| Arc::new(v) as SkyValue))
    }
}

#[cfg(test)]
mod tests;
