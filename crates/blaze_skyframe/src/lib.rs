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
//! * `get_value_or_throw::<K, E>` returns `Ok(Err(e))` when the dependency failed with an `E`
//!   and the evaluation is keep_going or the function is being re-run in error bubbling, like
//!   `getValueOrThrow(key, E.class)`. Without keep_going, in normal evaluation, a dependency in
//!   error is `Ok(Ok(None))` and marks values missing, as in Java.
//!   PORT: Java matches `E` and its subclasses (`throwIfInstanceOf`); the downcast here matches
//!   the one concrete type `E`. For several types or a Java class hierarchy use
//!   `get_value_or_throw_with`, whose closure inspects the exception.
//! * `get_values_and_exceptions` evaluates a batch in parallel and returns a
//!   [`SkyframeLookupResult`] (`get`, `get_or_throw`), like Java. As in Java, where the result is
//!   the environment itself, it reads every dependency the environment has requested, by this
//!   call or any other.
//! * `values_missing()` reports whether any requested dependency was unavailable.
//! * Every dependency request returns `Err(Interrupted)` once the evaluator is interrupted
//!   (Java's `InterruptedException`); `?` propagates it through [`ComputeError`].
//!
//! PORT: dependencies are evaluated in request order, one request at a time: a request returns
//! when its deps have been evaluated as far as they can be. Java collects every dep a run
//! requests and evaluates none of them before the function returns. The difference shows when a
//! function goes on after a missing dep: without keep_going, Java's function that requests a dep
//! not yet built and then one whose error is cached fails on the cached error and the first dep
//! is never evaluated; here the first dep is evaluated (and may fail first). Likewise a function
//! that throws although a dep is missing: Java commits that exception without evaluating the
//! dep; here the dep was evaluated by the request.
//!
//! Each `Evaluator::evaluate(roots, ctx)` call takes its own [`EvaluationContext`]
//! (`keep_going`, parallelism, event handler), like `MemoizingEvaluator.evaluate`. Committed values
//! and errors are memoized in the graph. Nodes Java would not have committed (still in flight when
//! a no-keep-going evaluation aborted, on the error-bubbling path, in or above a cycle that the
//! cycle detector did not reach, interrupted) stay in the graph, not done, until the next
//! `evaluate` deletes them with their reverse transitive closure, as Java does.
//!
//! ## Incremental evaluation
//!
//! The graph is Java's: each node is an `InMemoryNodeEntry` (`IncrementalInMemoryNodeEntry`, or
//! `NonIncrementalInMemoryNodeEntry` with [`EvaluatorBuilder::keep_edges`]`(false)`) driven with
//! Java's calls in Java's order, under graph versions (one per `evaluate`). Between evaluations a
//! caller records changes, applied at the start of the next `evaluate`
//! (`AbstractInMemoryMemoizingEvaluator.evaluate`): [`Evaluator::inject`] (an injected value
//! equal to the node's, by [`Key::values_equal`], changes nothing), [`Evaluator::invalidate`],
//! [`Evaluator::invalidate_transient_errors`], [`Evaluator::delete`],
//! [`Evaluator::delete_dirty`]. Changed nodes are marked changed and their reverse transitive
//! closure dirty; a dirty node that is requested checks the dep groups of its last build in order
//! and is marked clean if none changed, else re-runs its function; a node that re-evaluates to an
//! equal value keeps its version (change pruning). A progress receiver
//! ([`EvaluatorBuilder::progress_receiver`]) is told what Java tells it, and with a storing
//! [`EventFilter`] ([`EvaluatorBuilder::event_filter`]) nodes store their events, which are
//! replayed when a node is reused ([`Evaluator::emitted_event_state`]).
//!
//! PORT: the blocking engine evaluates a dep where Java enqueues it; a node whose dep does not
//! finish in this evaluation (abort, cycle, interrupt) ends this evaluation "undone", its entry
//! still evaluating, which is the in-flight node Java leaves behind. See
//! `docs/skyframe-incremental.d/engine.md` for the mapping.

//! ## Errors
//!
//! * keep_going: every node is computed as far as possible. A function that throws while a
//!   requested dependency is missing has its exception ignored, as in Java, and gets its
//!   children's error.
//! * no keep_going: the first exception thrown is committed and stops new work
//!   (`preventNewEvaluations`). Later exceptions are dropped, like Java. No function sees a
//!   dependency's exception in normal evaluation: a function that asked for a dependency in error
//!   gets `None`, returns `Ok(None)`, and its node stays unfinished.
//! * Catastrophic exceptions (`SkyFunctionException::is_catastrophic`) stop a keep_going
//!   evaluation the same way and set `EvaluationResult::catastrophe`.
//!
//! ### Error bubbling
//!
//! After an evaluation has failed that way, and every evaluator thread has stopped, the engine
//! does what `ParallelEvaluator.bubbleErrorUp` does: it walks from the failed node up to a
//! top-level node, one requester per level (the first that registered), and runs each one's
//! function once more with [`Environment::in_error_bubbling`] true. In that run
//!
//! * a dependency request computes nothing: a dep is the error built for it on the way up, or
//!   what is done in the graph, or missing;
//! * `get_value_or_throw` hands the function its dependencies' exceptions, so it can throw a
//!   better one (which replaces the error on the way up) or return anything (ignored);
//! * [`Environment::check_interrupted`] fails once a dependency in error was requested (Java sets
//!   the thread's interrupt bit);
//! * the events the function stores are reported; events of the normal run that ended with a
//!   missing dependency were dropped.
//!
//! So a function on that path runs twice: once in normal evaluation (it must return `Ok(None)`
//! when values are missing, as in Java) and once in error bubbling. The errors built on the way
//! are returned for the top-level nodes on the path and never written to the graph. If the walk
//! comes back to a node it has passed (a cycle), it is abandoned and the cycle detector reports
//! the cycle instead.
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
//! thread with a large reserved stack (`thread_stack_size`). If the OS refuses a thread, helpers
//! are done without; a thread the engine cannot do without crashes the evaluation (Java:
//! OutOfMemoryError), it does not hang it.
//!
//! A panic in a function crashes the evaluation: `evaluate` panics with Java's "Unrecoverable
//! error while evaluating node '...' (requested by nodes ...)" followed by "Caused by: " and the
//! function's panic message.
//!
//! ## Not ported
//!
//! Engine API of `com.google.devtools.build.skyframe` that has no port here. Where a Java method
//! has a Rust counterpart to hang it on, calling it fails with `unported!`.
//!
//! * `SkyFunction.Environment.getState` / `SkyKeyComputeState` ([`Environment::get_state`]).
//! * `Environment.registerDependencies` ([`Environment::register_dependencies`]).
//! * `SkyframeLookupResult.queryDep` ([`SkyframeLookupResult::query_dep`]).
//! * `Environment.dependOnFuture` and external deps ([`Environment::depend_on_future`]).
//! * `SkyFunction.Reset`, node rewinding (`DirtyType.REWIND`), partial reevaluation.
//! * The per-key keep_going predicate (`ParallelEvaluatorContext.keepGoing(SkyKey)`): keep_going
//!   is one flag per evaluation.
//! * `ShortCircuitingCycleDetector` (`EvaluationContext::detect_cycles == false`).
//! * `GraphInconsistencyReceiver`: an inconsistency panics with the text of Java's default
//!   (`THROWING`).
//! * Max transitive source versions, Skyfocus, graph statistics (`postLoggingStats`), key
//!   interning.
//! * Interrupting invalidation: the invalidators run to completion; `evaluate` then sees the
//!   interrupt.

//! ## Rules for ported SkyFunctions
//!
//! * Never hold a lock across `get_value` / `get_values_and_exceptions`: the call may compute
//!   arbitrary other nodes on this thread or block on other threads.
//! * Only immutable, `Send + Sync` values (frozen Starlark values) cross nodes.
//! * A function can run twice for one node (normal evaluation, then error bubbling). Request
//!   dependencies before doing work that must not be repeated, as the Java function does.

#![allow(clippy::too_many_arguments, clippy::new_without_default)]

/// Port of Guava `Preconditions.checkNotNull`: the value, or a panic with Java's
/// `NullPointerException` (and message, when Java gives one).
macro_rules! check_not_null {
    ($e:expr) => {
        match $e {
            Some(v) => v,
            None => panic!("NullPointerException"),
        }
    };
    ($e:expr, $($arg:tt)+) => {
        match $e {
            Some(v) => v,
            None => panic!("NullPointerException: {}", format_args!($($arg)+)),
        }
    };
}

/// Test hook: tells the graph's injected listeners (`graph_listener`, Java's `NotifyingHelper`)
/// about a call on a node entry. Expands to nothing outside `cfg(test)`. Use it as a statement:
/// `notify_graph_listener!(graph, key, EventType, Order, context)`.
macro_rules! notify_graph_listener {
    ($graph:expr, $key:expr, $event_type:ident, $order:ident, $context:expr) => {
        #[cfg(test)]
        $graph.listeners_for_testing.notify(
            $key,
            crate::graph_listener::EventType::$event_type,
            crate::graph_listener::Order::$order,
            $context,
        );
    };
}

mod abstract_in_memory_node_entry;
mod cycle_info;
mod differencer;
mod dirty_building_state;
mod environment;
mod error_info;
mod evaluation_result;
mod evaluator;
mod event_filter;
mod function_hermeticity;
#[cfg(test)]
mod graph_listener;
mod grouped_deps;
mod in_memory_graph;
mod in_memory_node_entry;
mod incremental_in_memory_node_entry;
mod invalidation;
mod node_entry;
mod non_incremental_in_memory_node_entry;
pub mod evaluation_stats;
pub mod precomputed;
mod progress;
mod reverse_deps_utility;
mod simple_cycle_detector;
pub mod testing;
mod value_with_metadata;
mod version;
mod walkable_graph;

pub use cycle_info::CycleInfo;
pub use environment::{Environment, SkyframeLookupResult, ValueOrThrow};
pub use error_info::ErrorInfo;
pub use evaluation_result::EvaluationResult;
pub use evaluator::{
    ERROR_TRANSIENCE_KEY, EmittedEventState, ErrorTransienceKey, ErrorTransienceValue,
    EvaluationContext, Evaluator, EvaluatorBuilder, IN_FLIGHT_INTERRUPT_POLL,
};
pub use event_filter::EventFilter;
pub use function_hermeticity::FunctionHermeticity;
pub use grouped_deps::GroupedDeps;
pub use node_entry::DirtyType;
pub use progress::{
    CompoundEvaluationProgressReceiver, CompoundEvaluationProgressReceiverBase,
    EvaluationProgressReceiver, EvaluationState, NodeState,
};
pub use version::{IntVersion, NodeVersion, Version};
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
    /// Java: `SkyFunctionName.getHermeticity()`, given where Java creates the function name
    /// (`SkyFunctionName.createHermetic`, `createSemiHermetic`, `createNonHermetic`). Keys that
    /// share a function name declare the same hermeticity (Java interns one name).
    // Java: com/google/devtools/build/skyframe/SkyFunctionName.java:84 SkyFunctionName.getHermeticity
    const HERMETICITY: FunctionHermeticity;

    /// Java: `SkyValue.equals` as used by `DirtyBuildingState.unchangedFromLastBuild` and
    /// `AbstractInMemoryMemoizingEvaluator.pruneInjectedValues`. The default is "not comparable"
    /// (Java identity equality or `NotComparableSkyValue`): never equal.
    ///
    /// Java's default `Object.equals` is identity, so in Java a function that returns the very
    /// same object as last build is change-pruned; a key opts into that here with
    /// `std::ptr::eq(old, new)`. There is no implicit identity shortcut: it would prune values
    /// that Java marks `NotComparableSkyValue`.
    // Java: com/google/devtools/build/skyframe/DirtyBuildingState.java:214 DirtyBuildingState.unchangedFromLastBuild
    fn values_equal(_old: &Self::Value, _new: &Self::Value) -> bool {
        false
    }
}

/// Object-safe view of a [`Key`].
pub trait DynKey: fmt::Debug + Send + Sync + 'static {
    fn function_name(&self) -> SkyFunctionName;
    /// [`Key::HERMETICITY`] of the key's type.
    fn hermeticity(&self) -> FunctionHermeticity;
    fn as_any(&self) -> &dyn Any;
    fn dyn_eq(&self, other: &dyn DynKey) -> bool;
    fn dyn_hash(&self, state: &mut dyn Hasher);
    /// [`Key::values_equal`] on type-erased values: a value of another type than the key's
    /// `Value` is not equal.
    fn dyn_values_equal(&self, old: &SkyValue, new: &SkyValue) -> bool;
}

impl<K: Key> DynKey for K {
    fn function_name(&self) -> SkyFunctionName {
        K::FUNCTION
    }
    fn hermeticity(&self) -> FunctionHermeticity {
        K::HERMETICITY
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
    fn dyn_values_equal(&self, old: &SkyValue, new: &SkyValue) -> bool {
        match (
            old.downcast_ref::<K::Value>(),
            new.downcast_ref::<K::Value>(),
        ) {
            (Some(old), Some(new)) => K::values_equal(old, new),
            _ => false,
        }
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
    /// Java: `skyKey.functionName().getHermeticity()` ([`Key::HERMETICITY`]).
    // Java: com/google/devtools/build/skyframe/SkyFunctionName.java:84 SkyFunctionName.getHermeticity
    pub fn hermeticity(&self) -> FunctionHermeticity {
        self.0.hermeticity()
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
