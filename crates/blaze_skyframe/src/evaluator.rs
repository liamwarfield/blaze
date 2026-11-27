//! The evaluator: ports of `MemoizingEvaluator` / `AbstractInMemoryMemoizingEvaluator`,
//! `ParallelEvaluator` and `AbstractParallelEvaluator`, reshaped for blocking evaluation (see the
//! crate docs).

use crate::differencer::{
    Delta, Diff, Differencer, Injectable, RecordingDifferencer, SequencedRecordingDifferencer,
};
use crate::environment::{BubbleErrorInfo, EnvState, Environment, PendingDep};
use crate::evaluation_result::EvaluationResult;
use crate::event_filter::EventFilter;
use crate::graph_inconsistency_receiver::{GraphInconsistencyReceiver, Inconsistency};
#[cfg(test)]
use crate::graph_listener::Context;
use crate::in_memory_graph::InMemoryGraphImpl;
pub(crate) use crate::in_memory_graph::{DoneEntry, DoneKind, Node};
use crate::invalidation::{DeletingInvalidationState, EagerInvalidator, InvalidationState};
use crate::node_entry::{DependencyState, DirtyType, LifecycleState, NodeEntry};
use crate::progress::{
    DirtyAndInflightTrackingProgressReceiver, EvaluationProgressReceiver, EvaluationState,
    InflightTrackingProgressReceiver, NodeState,
};
use crate::reset::Reset;
use crate::reverse_deps_utility::java_list;
use crate::simple_cycle_detector::SimpleCycleDetector;
use crate::value_with_metadata::{StoredReportable, ValueWithMetadata};
use crate::version::{IntVersion, Version};
use crate::{
    ComputeError, ComputeResult, ErrorInfo, GroupedDeps, Interrupted, Key, SkyFunction,
    SkyFunctionException, SkyFunctionName, SkyKey, SkyValue, typed,
};
use blaze_collect::{NestedSet, NestedSetNode, NestedSetVisitor, NestedSetVisitorVisitedState};
use blaze_events::{Event, EventHandler, EventKind, ExtendedEventHandler, Postable, Reportable};
use blaze_shell::ThreadInterrupt;
use blaze_util::profiler::{Profiler, ProfilerTask};
use blaze_util::{check_argument, check_state};
use indexmap::{IndexMap, IndexSet};
use parking_lot::{Condvar, Mutex};
use std::any::Any;
use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::fmt::Write as _;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};
use std::thread::{Scope, ScopedJoinHandle, ThreadId};
use std::time::Duration;

thread_local! {
    /// Number of keys this thread is computing inline (its compute stack depth).
    static DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// The key of the [`ErrorTransienceValue`] node: every node with a directly transient error
/// depends on it, and [`Evaluator::invalidate_transient_errors`] re-evaluates its dependents.
// Java: com/google/devtools/build/skyframe/ErrorTransienceValue.java:27 ErrorTransienceValue.KEY
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ErrorTransienceKey;

impl fmt::Debug for ErrorTransienceKey {
    // Java: com/google/devtools/build/skyframe/ErrorTransienceValue.java:40 KEY.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ErrorTransienceValue.KEY")
    }
}

impl Key for ErrorTransienceKey {
    type Value = ErrorTransienceValue;
    // Java: com/google/devtools/build/skyframe/ErrorTransienceValue.java:24 FUNCTION_NAME
    const FUNCTION: SkyFunctionName = SkyFunctionName("ERROR_TRANSIENCE");
    // Java: com/google/devtools/build/skyframe/ErrorTransienceValue.java:25 SkyFunctionName.createNonHermetic("ERROR_TRANSIENCE")
    const HERMETICITY: crate::FunctionHermeticity = crate::FunctionHermeticity::Nonhermetic;
}

/// A value that represents "error transience", i.e. anything which may have caused an unexpected
/// failure.
// Java: com/google/devtools/build/skyframe/ErrorTransienceValue.java:22 ErrorTransienceValue
#[derive(Debug)]
pub struct ErrorTransienceValue;

/// `ErrorTransienceValue.KEY` as a [`SkyKey`].
pub static ERROR_TRANSIENCE_KEY: LazyLock<SkyKey> =
    LazyLock::new(|| SkyKey::new(ErrorTransienceKey));

/// Counting semaphore for compute permits.
struct Permits {
    available: Mutex<usize>,
    cv: Condvar,
}

impl Permits {
    fn acquire(&self) {
        let mut n = self.available.lock();
        while *n == 0 {
            self.cv.wait(&mut n);
        }
        *n -= 1;
    }
    fn release(&self) {
        *self.available.lock() += 1;
        self.cv.notify_one();
    }
}

#[derive(Default)]
struct WaitGraph {
    /// Keys each thread is computing, outermost first.
    stacks: HashMap<ThreadId, Vec<SkyKey>>,
    /// Waits of each thread, innermost last. `(frames, keys)`: the wait was made while the
    /// thread's stack had `frames` entries, so it blocks the keys at stack index `< frames`
    /// (the frame that waited and its ancestors), not frames pushed later.
    waiting: HashMap<ThreadId, Vec<(usize, Vec<SkyKey>)>>,
}

/// Keeps track of nested set visitation and warnings for purposes of deduplication.
///
/// PORT: Java's `seenNodes` holds raw `NestedSet` nodes (`Object[]` by identity, elements by
/// `equals`); here they are [`NestedSetNode`]s, which compare the same way.
// Java: com/google/devtools/build/skyframe/EmittedEventState.java:29 EmittedEventState
#[derive(Default)]
pub struct EmittedEventState {
    seen_nodes: Mutex<HashSet<NestedSetNode<StoredReportable>>>,
    seen_warnings: Mutex<HashSet<Event>>,
}

impl EmittedEventState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Clears the seen nodes and warnings.
    // Java: com/google/devtools/build/skyframe/EmittedEventState.java:35 EmittedEventState.clear
    pub fn clear(&self) {
        self.seen_nodes.lock().clear();
        self.seen_warnings.lock().clear();
    }

    // Java: com/google/devtools/build/skyframe/EmittedEventState.java:44 EmittedEventState.needToVisit
    pub(crate) fn need_to_visit(&self, node: &NestedSetNode<StoredReportable>) -> bool {
        self.seen_nodes.lock().insert(node.clone())
    }

    /// Returns `true` if the given warning was not seen before.
    // Java: com/google/devtools/build/skyframe/EmittedEventState.java:46 EmittedEventState.addWarning
    pub(crate) fn add_warning(&self, warning: &Event) -> bool {
        check_argument!(
            warning.get_kind() == EventKind::Warning,
            "Not a warning: {:?}",
            warning
        );
        self.seen_warnings.lock().insert(warning.clone())
    }
}

impl NestedSetVisitorVisitedState<StoredReportable> for &EmittedEventState {
    fn need_to_visit(&mut self, node: &NestedSetNode<StoredReportable>) -> bool {
        EmittedEventState::need_to_visit(self, node)
    }
}

struct NoopEventHandler;

impl EventHandler for NoopEventHandler {
    fn handle(&self, _event: &Event) {}
    fn as_extended_event_handler(&self) -> Option<&dyn ExtendedEventHandler> {
        Some(self)
    }
}

impl ExtendedEventHandler for NoopEventHandler {
    fn post(&self, _obj: Arc<dyn Postable>) {}
}

/// Settings for one evaluation.
// Java: com/google/devtools/build/skyframe/EvaluationContext.java:30 EvaluationContext
#[derive(Clone)]
pub struct EvaluationContext {
    pub keep_going: bool,
    pub parallelism: usize,
    /// Where events reported through `Environment::listener` go.
    pub event_handler: Arc<dyn ExtendedEventHandler>,
    /// Java default true. When false, cycles are reported without details.
    pub store_exact_cycles: bool,
    /// Java default true. When false Java uses `ShortCircuitingCycleDetector` (not ported).
    pub detect_cycles: bool,
    /// PORT: Blaze only. Stack size of evaluator threads. Starlark evaluation is stack hungry;
    /// the memory is reserved, not committed.
    pub thread_stack_size: usize,
    /// PORT: Blaze only. Cap on live evaluator threads in the process. A batch request beyond it
    /// is evaluated inline, sequentially.
    pub max_threads: usize,
    /// PORT: Blaze only. A key that would be computed deeper than this many inline frames on one
    /// thread is computed on a fresh thread.
    pub max_inline_depth: usize,
}

impl Default for EvaluationContext {
    fn default() -> Self {
        EvaluationContext {
            keep_going: false,
            parallelism: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4),
            event_handler: Arc::new(NoopEventHandler),
            store_exact_cycles: true,
            detect_cycles: true,
            thread_stack_size: 256 << 20,
            max_threads: 512,
            max_inline_depth: 32,
        }
    }
}

impl fmt::Debug for EvaluationContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EvaluationContext")
            .field("keep_going", &self.keep_going)
            .field("parallelism", &self.parallelism)
            .field("store_exact_cycles", &self.store_exact_cycles)
            .field("detect_cycles", &self.detect_cycles)
            .field("thread_stack_size", &self.thread_stack_size)
            .field("max_threads", &self.max_threads)
            .field("max_inline_depth", &self.max_inline_depth)
            .finish_non_exhaustive()
    }
}

/// The state `AbstractInMemoryMemoizingEvaluator` keeps between evaluations: the differencer and
/// what is to be deleted, dirtied and injected at the start of the next one.
// Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:60 AbstractInMemoryMemoizingEvaluator
struct IncrementalState {
    // State related to invalidation and deletion.
    values_to_delete: IndexSet<SkyKey>,
    values_to_dirty: IndexSet<SkyKey>,
    values_to_inject: IndexMap<SkyKey, Delta>,
    deleter_state: DeletingInvalidationState,
    differencer: SequencedRecordingDifferencer,
    // Values that the caller explicitly specified are assumed to be changed -- they will be
    // re-evaluated even if none of their children are changed.
    invalidator_state: InvalidationState,
    /// Null until the first incremental evaluation completes. Always null when not keeping
    /// edges.
    last_graph_version: Option<IntVersion>,
}

/// The memoizing evaluator: functions and the graph.
// Java: com/google/devtools/build/skyframe/InMemoryMemoizingEvaluator.java:38 InMemoryMemoizingEvaluator
pub struct Evaluator {
    functions: HashMap<SkyFunctionName, Arc<dyn SkyFunction>>,
    pub(crate) graph: InMemoryGraphImpl,
    /// Whether to store edges in the graph. Can be false to save memory, in which case
    /// incremental builds are not possible, and all evaluations will be at `Version.constant`.
    pub(crate) keep_edges: bool,
    pub(crate) progress_receiver: DirtyAndInflightTrackingProgressReceiver,
    /// The statistics of `InMemoryMemoizingEvaluator.ProgressReceiver` (kept across
    /// [`EvaluatorBuilder::build_reusing`], as the Java evaluator's receiver).
    evaluation_stats: Arc<crate::evaluation_stats::EvaluationStatsCounters>,
    /// Whether the caller gave a progress receiver (`progress_receiver` then forwards to it).
    has_user_progress_receiver: bool,
    event_filter: Arc<dyn EventFilter>,
    /// Java: `ParallelEvaluatorContext.getGraphInconsistencyReceiver()` (the command's receiver,
    /// `SequencedSkyframeExecutor.inconsistencyReceiver`'s delegate).
    graph_inconsistency_receiver: Arc<dyn GraphInconsistencyReceiver>,
    incremental: Mutex<IncrementalState>,
    waits: Mutex<WaitGraph>,
    client: Arc<dyn Any + Send + Sync>,
    computed: AtomicU64,
    /// The keys computed while [`Evaluator::start_recording_computed_keys`] is on.
    recorded_computed_keys: Mutex<Option<Vec<SkyKey>>>,
    evaluating: AtomicBool,
    interrupted: Arc<AtomicBool>,
    /// Java: `AbstractQueueVisitor.jobs`: the threads running a function, with the number of
    /// functions each runs (a thread computes dependencies inline), for
    /// [`Evaluator::interrupt_in_flight_tasks`].
    jobs: Mutex<HashMap<ThreadId, (ThreadInterrupt, usize)>>,
    live_threads: AtomicUsize,
    peak_threads: AtomicUsize,
    pub(crate) emitted_event_state: EmittedEventState,
    /// Test hook: `Some(n)` lets `n` more evaluator threads start, then every spawn fails.
    #[cfg(test)]
    pub(crate) spawns_before_failure_for_testing: Mutex<Option<usize>>,
    /// Test hook: error bubbling picks the alphabetically first reverse dep, as Java tests get
    /// with `DeterministicHelper.DeterministicProcessableGraph`.
    #[cfg(test)]
    pub(crate) deterministic_for_testing: AtomicBool,
}

/// Builder for an [`Evaluator`].
#[derive(Clone)]
pub struct EvaluatorBuilder {
    functions: HashMap<SkyFunctionName, Arc<dyn SkyFunction>>,
    injected: Vec<(SkyKey, SkyValue)>,
    client: Arc<dyn Any + Send + Sync>,
    keep_edges: bool,
    progress_receiver: Option<Arc<dyn EvaluationProgressReceiver>>,
    event_filter: Arc<dyn EventFilter>,
    graph_inconsistency_receiver: Arc<dyn GraphInconsistencyReceiver>,
}

impl Default for EvaluatorBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl EvaluatorBuilder {
    pub fn new() -> Self {
        EvaluatorBuilder {
            functions: HashMap::new(),
            injected: vec![],
            client: Arc::new(()),
            keep_edges: true,
            progress_receiver: None,
            event_filter: Arc::new(<dyn EventFilter>::NO_STORAGE),
            graph_inconsistency_receiver: Arc::new(crate::graph_inconsistency_receiver::Throwing),
        }
    }
    /// The receiver of graph inconsistencies (Java's `GraphInconsistencyReceiver`, default
    /// `THROWING`). Java's `SequencedSkyframeExecutor` picks one per command
    /// (`getGraphInconsistencyReceiverForCommand`); Blaze builds an evaluator per command.
    pub fn graph_inconsistency_receiver(
        mut self,
        receiver: Arc<dyn GraphInconsistencyReceiver>,
    ) -> Self {
        self.graph_inconsistency_receiver = receiver;
        self
    }
    /// Registers the function for `name`. Panics on duplicates.
    pub fn function(mut self, name: SkyFunctionName, f: impl SkyFunction) -> Self {
        let prev = self.functions.insert(name, Arc::new(f));
        assert!(prev.is_none(), "duplicate SkyFunction {name}");
        self
    }
    /// Registers a typed function for key type `K`.
    pub fn typed_function<K, F>(self, f: F) -> Self
    where
        K: Key,
        F: Fn(&K, &mut Environment<'_>) -> Result<Option<K::Value>, ComputeError>
            + Send
            + Sync
            + 'static,
    {
        self.function(K::FUNCTION, typed::<K, F>(f))
    }
    /// Injects a precomputed value. Java: `PrecomputedValue` / `Injectable.inject`.
    ///
    /// PORT: the value is in the graph when [`EvaluatorBuilder::build`] returns; Java records it
    /// in the differencer and injects it at the start of the first `evaluate`.
    pub fn inject<K: Key>(mut self, key: K, value: K::Value) -> Self {
        self.injected.push((SkyKey::new(key), Arc::new(value)));
        self
    }

    /// Whether [`EvaluatorBuilder::inject`] was given a value for `key`. Lets a crate inject the
    /// default of a precomputed value it reads that another crate may already have injected
    /// (Java injects every precomputed value of the server's modules, so it is always present).
    pub fn has_injected(&self, key: &SkyKey) -> bool {
        self.injected.iter().any(|(k, _)| k == key)
    }
    /// Sets the client context functions can read with [`Environment::client`].
    pub fn client<T: Any + Send + Sync>(mut self, client: Arc<T>) -> Self {
        self.client = client;
        self
    }
    /// Whether to store edges in the graph (Java's `keepEdges`, default true). Without edges no
    /// incremental evaluation is possible: every evaluation is at `Version.constant()`.
    pub fn keep_edges(mut self, keep_edges: bool) -> Self {
        self.keep_edges = keep_edges;
        self
    }
    /// The receiver of evaluation progress (Java's `EvaluationProgressReceiver`, default
    /// `NULL`). The evaluator wraps it in a `DirtyAndInflightTrackingProgressReceiver`.
    pub fn progress_receiver(mut self, receiver: Arc<dyn EvaluationProgressReceiver>) -> Self {
        self.progress_receiver = Some(receiver);
        self
    }
    /// Which events nodes store for replay (Java's `EventFilter`). The default,
    /// `EventFilter::NO_STORAGE`, stores none: a node's events are reported when it commits (or
    /// is re-run in error bubbling) and never replayed.
    pub fn event_filter(mut self, event_filter: Arc<dyn EventFilter>) -> Self {
        self.event_filter = event_filter;
        self
    }
    // Java: com/google/devtools/build/skyframe/InMemoryMemoizingEvaluator.java:159 InMemoryMemoizingEvaluator.<init>
    pub fn build(self) -> Evaluator {
        let has_user_progress_receiver = self.progress_receiver.is_some();
        let receiver = self
            .progress_receiver
            .unwrap_or_else(|| Arc::new(crate::progress::NULL));
        let evaluation_stats =
            Arc::new(crate::evaluation_stats::EvaluationStatsCounters::default());
        let receiver: Arc<dyn EvaluationProgressReceiver> =
            Arc::new(crate::evaluation_stats::StatsCollectingProgressReceiver {
                stats: evaluation_stats.clone(),
                inner: receiver,
            });
        let ev = Evaluator {
            evaluation_stats,
            functions: self.functions,
            graph: InMemoryGraphImpl::new(self.keep_edges),
            keep_edges: self.keep_edges,
            // PORT: Java's InMemoryMemoizingEvaluator.ProgressReceiver also collects statistics
            // for postLoggingStats (not ported: logging).
            progress_receiver: DirtyAndInflightTrackingProgressReceiver::new(receiver),
            has_user_progress_receiver,
            event_filter: self.event_filter,
            graph_inconsistency_receiver: self.graph_inconsistency_receiver,
            incremental: Mutex::new(IncrementalState {
                values_to_delete: IndexSet::new(),
                values_to_dirty: IndexSet::new(),
                values_to_inject: IndexMap::new(),
                deleter_state: DeletingInvalidationState::new(),
                differencer: SequencedRecordingDifferencer::new(),
                invalidator_state: InvalidationState::dirtying(),
                last_graph_version: None,
            }),
            waits: Mutex::new(WaitGraph::default()),
            client: self.client,
            computed: AtomicU64::new(0),
            recorded_computed_keys: Mutex::new(None),
            evaluating: AtomicBool::new(false),
            interrupted: Arc::new(AtomicBool::new(false)),
            jobs: Mutex::new(HashMap::new()),
            live_threads: AtomicUsize::new(0),
            peak_threads: AtomicUsize::new(0),
            emitted_event_state: EmittedEventState::default(),
            #[cfg(test)]
            spawns_before_failure_for_testing: Mutex::new(None),
            #[cfg(test)]
            deterministic_for_testing: AtomicBool::new(false),
        };
        // PORT: Java's differencer would inject these at the start of the first evaluation, at
        // its graph version; they are injected now, at that version.
        if !self.injected.is_empty() {
            let first_version = ev.get_next_graph_version(None);
            let map: IndexMap<SkyKey, Delta> = self
                .injected
                .into_iter()
                .map(|(k, v)| (k, Delta::just_new(v)))
                .collect();
            ev.inject_values_into_graph(&map, first_version);
        }
        ev
    }

    /// Builds an evaluator with this builder's functions, client, progress receiver and event
    /// filter over the graph of `previous`: its nodes, versions, pending invalidations, dirty and
    /// in-flight sets and emitted event state. The values given to [`EvaluatorBuilder::inject`]
    /// are recorded in the differencer and applied at the start of the next `evaluate`, where an
    /// injection of a value equal to the node's is pruned (Java: `PrecomputedValue.set` on the
    /// executor's `injectable()` once per command, `pruneInjectedValues`).
    ///
    /// PORT: Blaze-only. Java's `SkyframeExecutor` outlives a command: its functions read the
    /// command's inputs through precomputed values and cells. Blaze builds the functions of an
    /// executor per command (they capture the command's inputs), so the graph of a kept
    /// evaluator moves into the next command's evaluator. Whoever calls this has proved that the
    /// new functions compute, for every node it keeps, what the old ones computed, or invalidated
    /// or deleted the nodes for which that does not hold (blaze_sky_top `incremental`).
    pub fn build_reusing(self, previous: Evaluator) -> Evaluator {
        check_state!(
            !previous.evaluating.load(Ordering::SeqCst),
            "the previous evaluator is evaluating"
        );
        check_argument!(
            previous.keep_edges == self.keep_edges,
            "keep_edges differs: {} -> {}",
            previous.keep_edges,
            self.keep_edges
        );
        let has_user_progress_receiver = self.progress_receiver.is_some();
        let receiver = self
            .progress_receiver
            .unwrap_or_else(|| Arc::new(crate::progress::NULL));
        let Evaluator {
            graph,
            keep_edges,
            progress_receiver,
            evaluation_stats,
            incremental,
            emitted_event_state,
            ..
        } = previous;
        let receiver: Arc<dyn EvaluationProgressReceiver> =
            Arc::new(crate::evaluation_stats::StatsCollectingProgressReceiver {
                stats: evaluation_stats.clone(),
                inner: receiver,
            });
        let ev = Evaluator {
            evaluation_stats,
            functions: self.functions,
            graph,
            keep_edges,
            progress_receiver: progress_receiver.with_receiver(receiver),
            has_user_progress_receiver,
            event_filter: self.event_filter,
            graph_inconsistency_receiver: self.graph_inconsistency_receiver,
            incremental,
            waits: Mutex::new(WaitGraph::default()),
            client: self.client,
            computed: AtomicU64::new(0),
            recorded_computed_keys: Mutex::new(None),
            evaluating: AtomicBool::new(false),
            interrupted: Arc::new(AtomicBool::new(false)),
            jobs: Mutex::new(HashMap::new()),
            live_threads: AtomicUsize::new(0),
            peak_threads: AtomicUsize::new(0),
            emitted_event_state,
            #[cfg(test)]
            spawns_before_failure_for_testing: Mutex::new(None),
            #[cfg(test)]
            deterministic_for_testing: AtomicBool::new(false),
        };
        {
            let mut inc = ev.incremental.lock();
            for (k, v) in self.injected {
                inc.differencer.inject(k, Delta::just_new(v));
            }
        }
        ev
    }
}

impl Evaluator {
    /// Number of functions computed so far.
    /// `postLoggingStats(eventHandler)`: the `SkyframeGraphStatsEvent` (graph size and the
    /// evaluation statistics since the last call, which resets them). Returned instead of
    /// posted: the caller posts it where Java's `SkyframeExecutor.notifyCommandComplete` does.
    // Java: com/google/devtools/build/skyframe/InMemoryMemoizingEvaluator.java:184 InMemoryMemoizingEvaluator.postLoggingStats
    pub fn post_logging_stats(&self) -> crate::evaluation_stats::SkyframeGraphStatsEvent {
        let evaluation_stats = self.evaluation_stats.aggregate_and_reset();
        crate::evaluation_stats::SkyframeGraphStatsEvent {
            // graph.valuesSize(): nodeMap.size()
            graph_size: self.graph.get_all_node_entries().len() as i32,
            evaluation_stats,
        }
    }

    /// Removes the evaluation statistics (`postLoggingStats`) gathered for `function` since the
    /// last [`Evaluator::post_logging_stats`].
    ///
    /// PORT: not a Bazel method. For a Blaze-only function that computes on Skyframe what Java
    /// computes outside it (cquery's CQUERY_TRANSITION_RESOLVER, decisions.md), so that the
    /// per-function statistics are Java's.
    pub fn remove_evaluation_stats_of(&self, function: crate::SkyFunctionName) {
        self.evaluation_stats.remove_function(function);
    }

    /// Starts recording the keys of the functions computed from now on (each `compute` call),
    /// replacing an earlier recording.
    ///
    /// PORT: not a Bazel method, like [`Evaluator::remove_evaluation_stats_of`]: cquery's
    /// CQUERY_TRANSITION_RESOLVER removes what its evaluation computed that Java computes outside
    /// Skyframe.
    pub fn start_recording_computed_keys(&self) {
        *self.recorded_computed_keys.lock() = Some(Vec::new());
    }

    /// Stops recording and returns the keys computed since
    /// [`Evaluator::start_recording_computed_keys`], in order, a key once per `compute` call.
    pub fn take_recorded_computed_keys(&self) -> Vec<SkyKey> {
        self.recorded_computed_keys
            .lock()
            .take()
            .unwrap_or_default()
    }

    /// A copy of the evaluation statistics gathered since the last
    /// [`Evaluator::post_logging_stats`], for [`Evaluator::restore_evaluation_stats`].
    ///
    /// PORT: not a Bazel method (see [`Evaluator::start_recording_computed_keys`]).
    pub fn evaluation_stats_snapshot(&self) -> crate::evaluation_stats::EvaluationStatsSnapshot {
        self.evaluation_stats.snapshot()
    }

    /// Puts back the evaluation statistics of `snapshot`, forgetting what was counted since.
    pub fn restore_evaluation_stats(
        &self,
        snapshot: crate::evaluation_stats::EvaluationStatsSnapshot,
    ) {
        self.evaluation_stats.restore(snapshot);
    }

    pub fn computed_count(&self) -> u64 {
        self.computed.load(Ordering::Relaxed)
    }

    /// Interrupts the current and every later evaluation until [`Evaluator::clear_interrupt`]:
    /// dependency requests return `Err(Interrupted)` and `evaluate` returns `Err(Interrupted)`.
    /// The threads running functions are interrupted as well (their
    /// `blaze_shell::ThreadInterrupt`), so blocking work such as a spawn waiting for its child
    /// stops too.
    /// Java: `Thread.interrupt()` of the thread calling `evaluate`, which then leaves
    /// `awaitQuiescence` and interrupts the workers (`AbstractQueueVisitor.awaitTermination`).
    pub fn interrupt(&self) {
        self.interrupted.store(true, Ordering::SeqCst);
        self.interrupt_in_flight_tasks();
    }

    /// Java: `Thread.interrupted()` (clears the interrupt status).
    pub fn clear_interrupt(&self) {
        self.interrupted.store(false, Ordering::SeqCst);
    }

    /// The interrupt flag, for a signal handler to set. Setting it has the effect of
    /// [`Evaluator::interrupt`]: a running evaluation notices it within
    /// [`IN_FLIGHT_INTERRUPT_POLL`] and interrupts its in-flight work.
    pub fn interrupt_flag(&self) -> Arc<AtomicBool> {
        self.interrupted.clone()
    }

    /// Number of live evaluator threads.
    pub fn live_threads(&self) -> usize {
        self.live_threads.load(Ordering::SeqCst)
    }

    /// The highest number of live evaluator threads seen so far.
    pub fn peak_threads(&self) -> usize {
        self.peak_threads.load(Ordering::SeqCst)
    }

    pub(crate) fn client(&self) -> &Arc<dyn Any + Send + Sync> {
        &self.client
    }

    pub(crate) fn event_filter(&self) -> &dyn EventFilter {
        &*self.event_filter
    }

    /// The state of the replayed and reported events: clear it to have stored events replayed
    /// again (Java tests: `tester.resetPlayedEvents()`).
    // Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:88 AbstractInMemoryMemoizingEvaluator.emittedEventState
    pub fn emitted_event_state(&self) -> &EmittedEventState {
        &self.emitted_event_state
    }

    // Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:495 AbstractInMemoryMemoizingEvaluator.getNextGraphVersion
    fn get_next_graph_version(&self, last_graph_version: Option<IntVersion>) -> Version {
        if !self.keep_edges {
            Version::constant()
        } else {
            match last_graph_version {
                None => IntVersion::of(0).into(),
                Some(v) => v.next().into(),
            }
        }
    }

    /// Evaluates `roots` (in parallel) and returns their values and errors.
    // Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:118 AbstractInMemoryMemoizingEvaluator.evaluate
    pub fn evaluate(
        &self,
        roots: &[SkyKey],
        ctx: &EvaluationContext,
    ) -> Result<EvaluationResult, Interrupted> {
        // NOTE: Performance critical code. See bug "Null build performance parity".
        let graph_version = self.get_next_graph_version(self.incremental.lock().last_graph_version);
        // Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:227 setAndCheckEvaluateState
        check_state!(
            !self.evaluating.swap(true, Ordering::SeqCst),
            "Re-entrant evaluation for request: {:?}",
            roots
        );
        struct Finally<'a> {
            ev: &'a Evaluator,
            graph_version: Version,
        }
        impl Drop for Finally<'_> {
            fn drop(&mut self) {
                if self.ev.keep_edges {
                    let Version::Int(v) = self.graph_version else {
                        unreachable!("graph versions are IntVersions when keeping edges")
                    };
                    self.ev.incremental.lock().last_graph_version = Some(v);
                }
                self.ev.evaluating.store(false, Ordering::SeqCst);
            }
        }
        let _finally = Finally {
            ev: self,
            graph_version,
        };

        // PORT: Skyfocus (rememberTopLevelEvaluations) is not ported.
        {
            let mut inc = self.incremental.lock();
            let inc = &mut *inc;
            // Mark for removal any nodes from the previous evaluation that were still inflight
            // or were rewound but did not complete successfully. When the invalidator runs, it
            // will delete the reverse transitive closure.
            inc.values_to_delete
                .extend(self.progress_receiver.get_and_clear_inflight_keys());
            inc.values_to_delete.extend(
                self.progress_receiver
                    .get_and_clear_unsuccessfully_rewound_keys(),
            );
            // The RecordingDifferencer implementation is not quite working as it should be at
            // this point. It clears the internal data structures after getDiff is called and will
            // not return diffs for historical versions. This makes the following code sensitive
            // to interrupts. Ideally we would simply not update lastGraphVersion if an interrupt
            // occurs.
            let diff = inc
                .differencer
                .get_diff(inc.last_graph_version.map(Version::from), graph_version);
            if !diff.is_empty()
                || !inc.values_to_inject.is_empty()
                || !inc.values_to_delete.is_empty()
            {
                for (k, d) in diff.changed_keys_with_new_values() {
                    inc.values_to_inject.insert(k.clone(), d.clone());
                }
                inc.values_to_dirty
                    .extend(diff.changed_keys_without_new_values().iter().cloned());
                self.prune_injected_values(inc);
                let inject_keys: Vec<SkyKey> = inc.values_to_inject.keys().cloned().collect();
                inc.values_to_dirty.extend(inject_keys);

                self.perform_invalidation(inc);
                // Java's invalidators wait for their visitor (`awaitQuiescence`), which throws
                // `InterruptedException` for a pending interrupt; `injectValues` is then not
                // reached and the values to inject are kept for the next evaluation.
                // PORT: invalidation here is sequential and runs to completion first (decision
                // 9.5); the interrupt is seen when it returns.
                // Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:481 AbstractInMemoryMemoizingEvaluator.performInvalidation
                if self.interrupted.load(Ordering::SeqCst) {
                    return Err(Interrupted);
                }
                self.inject_values(inc, graph_version);
            }
        }
        // Java has no interrupt check between here and `ParallelEvaluator.eval`: when every root
        // is done (or a cached error stops a no-keep-going evaluation) the result is returned
        // even with an interrupt pending; otherwise the evaluation sees the interrupt
        // (`wait_for_completion_and_construct_result`).
        // Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:156 AbstractInMemoryMemoizingEvaluator.evaluate
        let _c = Profiler::instance().profile(ProfilerTask::INFO, "ParallelEvaluator.eval");
        // Java: com/google/devtools/build/skyframe/ParallelEvaluator.java:641 ParallelEvaluator.eval
        let run = Run::new(self, ctx, graph_version);
        run.eval(roots)
    }

    /// Removes entries in `values_to_inject` whose values are equal to the present values in the
    /// graph.
    // Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:432 AbstractInMemoryMemoizingEvaluator.pruneInjectedValues
    fn prune_injected_values(&self, inc: &mut IncrementalState) {
        let mut to_remove: Vec<SkyKey> = vec![];
        for (key, delta) in &inc.values_to_inject {
            let new_value = &delta.new_value;
            let Some(prev_entry) = self.graph.get_if_present(key) else {
                continue;
            };
            let state = prev_entry.lock();
            if !state.entry.is_done() {
                continue;
            }
            // PORT: max transitive source versions are not ported; both are null.
            let new_mtsv = delta.new_max_transitive_source_version;
            let old_mtsv = state.entry.get_max_transitive_source_version();
            if self.keep_edges {
                if !state.entry.has_at_least_one_dep() {
                    // Java: newValue.equals(prevEntry.getValue()). PORT: SkyValue equality is the
                    // key's (Key::values_equal, docs/decisions.md D10).
                    let equal = state
                        .entry
                        .get_value()
                        .is_some_and(|prev| key.argument().dyn_values_equal(&prev, new_value));
                    if equal
                        && !inc.values_to_dirty.contains(key)
                        && !inc.values_to_delete.contains(key)
                        && new_mtsv == old_mtsv
                    {
                        to_remove.push(key.clone());
                    }
                } else {
                    // Rare situation of an injected dep that depends on another node. Usually the
                    // dep is the error transience node. When working with external repositories,
                    // it can also be an external workspace file. Don't bother injecting it, just
                    // invalidate it. We'll wastefully evaluate the node freshly during
                    // evaluation, but this happens very rarely.
                    inc.values_to_dirty.insert(key.clone());
                    to_remove.push(key.clone());
                }
            } else {
                // No incrementality. Just delete the old value from the graph. The new value is
                // about to be injected.
                drop(state);
                self.graph.remove(key);
            }
        }
        for key in to_remove {
            inc.values_to_inject.shift_remove(&key);
        }
    }

    /// Injects values in `values_to_inject` into the graph.
    // Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:468 AbstractInMemoryMemoizingEvaluator.injectValues
    fn inject_values(&self, inc: &mut IncrementalState, version: Version) {
        if inc.values_to_inject.is_empty() {
            return;
        }
        self.inject_values_into_graph(&inc.values_to_inject, version);
        // Start with a new map to avoid bloat since clear() does not downsize the map.
        inc.values_to_inject = IndexMap::new();
    }

    // Java: com/google/devtools/build/skyframe/ParallelEvaluator.java:599 ParallelEvaluator.injectValues
    fn inject_values_into_graph(&self, injection_map: &IndexMap<SkyKey, Delta>, version: Version) {
        let prev_node_entries = self.graph.create_if_absent_batch(injection_map.keys());
        for ((key, delta), node) in injection_map.iter().zip(prev_node_entries) {
            let value = delta.new_value.clone();
            {
                let mut state = node.lock();
                let prev_entry = &mut state.entry;
                let new_state = prev_entry.add_reverse_dep_and_check_if_done(None);
                check_state!(
                    new_state != DependencyState::AlreadyEvaluating,
                    "{:?} {:?}",
                    key,
                    prev_entry
                );
                if prev_entry.is_dirty() {
                    // Get the node in the state where it is able to accept a value.
                    check_state!(
                        new_state == DependencyState::NeedsScheduling,
                        "{:?} {:?}",
                        key,
                        prev_entry
                    );
                    // If there was a node in the graph before, check that the previous node has
                    // no dependencies. Overwriting a value with deps with an injected value
                    // (which is by definition deps-free) needs a little additional bookkeeping
                    // (removing reverse deps from the dependencies), but more importantly it's
                    // something that we want to avoid, because it indicates confusion of input
                    // values and derived values.
                    check_state!(
                        prev_entry.no_deps_last_build(),
                        "existing entry for {:?} has deps: {:?}",
                        key,
                        prev_entry
                    );
                }
                prev_entry.mark_rebuilding();
                let max_transitive_source_version = delta.new_max_transitive_source_version;
                prev_entry.set_value(
                    ValueWithMetadata::normal(
                        Some(value),
                        None,
                        crate::value_with_metadata::no_events(),
                    ),
                    version,
                    max_transitive_source_version,
                );
                state.refresh_snapshot();
                state.outcome = state.snapshot.clone();
            }
            // Now that this key's injected value is set, it is no longer dirty.
            self.progress_receiver.injected(key);
        }
    }

    // Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:480 AbstractInMemoryMemoizingEvaluator.performInvalidation
    fn perform_invalidation(&self, inc: &mut IncrementalState) {
        let values_to_delete = std::mem::take(&mut inc.values_to_delete);
        EagerInvalidator::delete(
            &self.graph,
            values_to_delete,
            &self.progress_receiver,
            &mut inc.deleter_state,
            self.keep_edges,
        );
        // Note that clearing the valuesToDelete would not do an internal resizing. Therefore, if
        // any build has a large set of dirty values, subsequent operations (even clearing) will
        // be slower. Instead, just start afresh with a new LinkedHashSet.
        let values_to_dirty = std::mem::take(&mut inc.values_to_dirty);
        EagerInvalidator::invalidate(
            &self.graph,
            values_to_dirty,
            &self.progress_receiver,
            &mut inc.invalidator_state,
        );
    }

    /// Marks for deletion, at the start of the next evaluation, every node for which `predicate`
    /// is true (it gets the key and the node's value; `None` for a node in error without a
    /// value) and every node that is not done, with their reverse transitive closure.
    // Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:212 AbstractInMemoryMemoizingEvaluator.delete
    pub fn delete(&self, delete_predicate: impl Fn(&SkyKey, Option<&SkyValue>) -> bool) {
        let mut to_delete: Vec<SkyKey> = vec![];
        self.graph.parallel_for_each(|key, node| {
            let state = node.lock();
            // Java: e.isDirty() || deletePredicate.test(e.getKey(), e.getValue()).
            if state.entry.is_dirty() || delete_predicate(key, state.entry.get_value().as_ref()) {
                to_delete.push(key.clone());
            }
        });
        self.incremental.lock().values_to_delete.extend(to_delete);
    }

    /// Whether an evaluation has run on this graph (Java's `lastGraphVersion != null`).
    ///
    /// PORT: Java's evaluator lives with the server, so `deleteDirty` always finds one; a Blaze
    /// executor's new graph may have none yet (a command that failed before evaluating).
    pub fn has_evaluated(&self) -> bool {
        self.incremental.lock().last_graph_version.is_some()
    }

    /// Marks for deletion, at the start of the next evaluation, the dirty nodes that were not
    /// evaluated since they were dirtied and whose version is at most the last graph version
    /// minus `version_age_limit`.
    // Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:243 AbstractInMemoryMemoizingEvaluator.deleteDirty
    pub fn delete_dirty(&self, version_age_limit: i64) {
        check_argument!(version_age_limit >= 0, "{}", version_age_limit);
        let mut inc = self.incremental.lock();
        let last = check_not_null!(inc.last_graph_version);
        let threshold: Version = IntVersion::of(last.get_val() - version_age_limit).into();
        let to_delete: Vec<SkyKey> = self
            .progress_receiver
            .get_unenqueued_dirty_keys()
            .into_iter()
            .filter(|sky_key| {
                let node = check_not_null!(self.graph.get_if_present(sky_key), "{:?}", sky_key);
                let state = node.lock();
                check_state!(state.entry.is_dirty(), "{:?}", sky_key);
                state.entry.get_version().at_most(&threshold)
            })
            .collect();
        inc.values_to_delete.extend(to_delete);
    }

    /// Marks `keys` changed: at the start of the next evaluation they and their reverse
    /// transitive closure are dirtied. Java: `RecordingDifferencer.invalidate`.
    // Java: com/google/devtools/build/skyframe/SequencedRecordingDifferencer.java:47 SequencedRecordingDifferencer.invalidate
    pub fn invalidate(&self, keys: impl IntoIterator<Item = SkyKey>) {
        self.incremental.lock().differencer.invalidate(keys);
    }

    /// The values of every node: done values and the last built values of dirty nodes.
    // Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:258 AbstractInMemoryMemoizingEvaluator.getValues
    pub fn get_values(&self) -> HashMap<SkyKey, SkyValue> {
        self.graph.get_values().into_iter().collect()
    }

    /// The values of the done nodes.
    // Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:263 AbstractInMemoryMemoizingEvaluator.getDoneValues
    pub fn get_done_values(&self) -> HashMap<SkyKey, SkyValue> {
        self.graph.get_done_values().into_iter().collect()
    }

    /// Returns the value of a done node (also one that has an error with a value).
    // Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:272 AbstractInMemoryMemoizingEvaluator.getExistingValue
    pub fn get_existing_value(&self, key: &SkyKey) -> Option<SkyValue> {
        self.committed_entry(key).and_then(|e| e.value.clone())
    }

    /// `getInMemoryGraph().getIfPresent(key).toValue()`: the value of a done node, or the last
    /// built value of a dirty one (for the diff checkers that compare with the file system).
    // Java: com/google/devtools/build/skyframe/AbstractInMemoryNodeEntry.java:137 AbstractInMemoryNodeEntry.toValue
    pub fn get_node_value_for_diff(&self, key: &SkyKey) -> Option<SkyValue> {
        self.graph
            .get_if_present(key)
            .and_then(|n| n.lock().entry.to_value())
    }

    /// The keys of every node in the graph (Java `getInMemoryGraph().parallelForEach(entry ->
    /// entry.getKey())`).
    pub fn get_all_keys(&self) -> Vec<SkyKey> {
        self.graph
            .get_all_node_entries()
            .into_iter()
            .map(|(k, _)| k)
            .collect()
    }

    // Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:281 AbstractInMemoryMemoizingEvaluator.getExistingErrorForTesting
    pub fn get_existing_error_for_testing(&self, key: &SkyKey) -> Option<ErrorInfo> {
        self.committed_entry(key)
            .and_then(|e| e.error.as_deref().cloned())
    }

    /// All keys in the graph, done or not.
    // Java: com/google/devtools/build/skyframe/WalkableGraph.java:106 WalkableGraph.getAllKeysForTesting
    pub fn get_all_keys_for_testing(&self) -> Vec<SkyKey> {
        self.graph
            .get_all_node_entries()
            .into_iter()
            .map(|(k, _)| k)
            .collect()
    }

    /// Injects a value: at the start of the next evaluation the node for `key` gets `value` and
    /// the nodes that depend on it are dirtied, unless the value equals the node's
    /// ([`Key::values_equal`]). Java: `Injectable.inject` on the evaluator's differencer.
    // Java: com/google/devtools/build/skyframe/SequencedRecordingDifferencer.java:57 SequencedRecordingDifferencer.inject
    pub fn inject<K: Key>(&self, key: K, value: K::Value) {
        self.inject_dyn(SkyKey::new(key), Arc::new(value));
    }

    /// Untyped [`Evaluator::inject`].
    pub fn inject_dyn(&self, key: SkyKey, value: SkyValue) {
        self.incremental
            .lock()
            .differencer
            .inject(key, Delta::just_new(value));
    }

    /// Re-evaluates, on the next `evaluate`, every node whose error was transient, and their
    /// dependents. Bazel does this at the start of every command.
    // Java: com/google/devtools/build/lib/skyframe/SequencedSkyframeExecutor.java:475 SequencedSkyframeExecutor.invalidateTransientErrors
    // Java: com/google/devtools/build/skyframe/RecordingDifferencer.java:33 RecordingDifferencer.invalidateTransientErrors
    pub fn invalidate_transient_errors(&self) {
        self.incremental
            .lock()
            .differencer
            .invalidate_transient_errors();
    }

    /// The snapshot or this evaluation's outcome of `key`.
    pub(crate) fn done_entry(&self, key: &SkyKey) -> Option<Arc<DoneEntry>> {
        self.graph.get_if_present(key).and_then(|n| n.done_entry())
    }

    /// Java: `evaluatorContext.getGraphInconsistencyReceiver()`.
    pub(crate) fn graph_inconsistency_receiver(&self) -> &dyn GraphInconsistencyReceiver {
        &*self.graph_inconsistency_receiver
    }

    /// The entry of `key` if it is done in the graph (Java `NodeEntry.isDone()`).
    pub(crate) fn committed_entry(&self, key: &SkyKey) -> Option<Arc<DoneEntry>> {
        self.graph.get_if_present(key).and_then(|n| n.snapshot())
    }

    pub(crate) fn node(&self, key: &SkyKey) -> Option<Arc<Node>> {
        self.graph.get_if_present(key)
    }

    /// Test hook: with [`Evaluator::deterministic_for_testing`], the reverse deps that `setValue`
    /// and `markClean` return are in alphabetical order, as Java tests get them from
    /// `DeterministicHelper.DeterministicNodeEntry`.
    // Java: src/test/java/com/google/devtools/build/skyframe/DeterministicHelper.java:146 DeterministicNodeEntry.setValue
    #[cfg(test)]
    pub(crate) fn deterministic_rdeps(&self, rdeps: IndexSet<SkyKey>) -> IndexSet<SkyKey> {
        if !self.deterministic_for_testing.load(Ordering::SeqCst) {
            return rdeps;
        }
        // Java: DeterministicHelper.ALPHABETICAL_SKYKEY_COMPARATOR
        let mut rdeps = rdeps;
        rdeps.sort_by(|a, b| {
            (format!("{:?}", a.argument()), a.function_name().0)
                .cmp(&(format!("{:?}", b.argument()), b.function_name().0))
        });
        rdeps
    }

    /// Test hook: [`Evaluator::deterministic_rdeps`] for `markClean`.
    // Java: src/test/java/com/google/devtools/build/skyframe/DeterministicHelper.java:155 DeterministicNodeEntry.markClean
    #[cfg(test)]
    pub(crate) fn deterministic_mark_clean(
        &self,
        r: crate::node_entry::NodeValueAndRdepsToSignal,
    ) -> crate::node_entry::NodeValueAndRdepsToSignal {
        crate::node_entry::NodeValueAndRdepsToSignal::new(
            r.get_value().clone(),
            self.deterministic_rdeps(r.get_rdeps_to_signal().clone()),
        )
    }

    fn is_done(&self, key: &SkyKey) -> bool {
        self.committed_entry(key).is_some()
    }

    /// Reserves up to `wanted` thread slots under `cap`; returns how many were reserved.
    fn reserve_threads(&self, wanted: usize, cap: usize) -> usize {
        let mut cur = self.live_threads.load(Ordering::SeqCst);
        loop {
            let take = wanted.min(cap.saturating_sub(cur));
            if take == 0 {
                return 0;
            }
            match self.live_threads.compare_exchange(
                cur,
                cur + take,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => {
                    self.peak_threads.fetch_max(cur + take, Ordering::SeqCst);
                    return take;
                }
                Err(actual) => cur = actual,
            }
        }
    }

    /// Reserves `n` slots regardless of the cap (threads the engine cannot do without).
    fn force_reserve_threads(&self, n: usize) {
        let now = self.live_threads.fetch_add(n, Ordering::SeqCst) + n;
        self.peak_threads.fetch_max(now, Ordering::SeqCst);
    }

    fn release_threads(&self, n: usize) {
        self.live_threads.fetch_sub(n, Ordering::SeqCst);
    }

    fn unregister_thread(&self) {
        let mut w = self.waits.lock();
        let id = std::thread::current().id();
        w.stacks.remove(&id);
        w.waiting.remove(&id);
    }

    /// Records the current thread as running a function until the guard is dropped.
    // Java: com/google/devtools/build/lib/concurrent/AbstractQueueVisitor.java:446 AbstractQueueVisitor.addJob
    fn add_job(&self) -> JobGuard<'_> {
        let id = std::thread::current().id();
        self.jobs
            .lock()
            .entry(id)
            .or_insert_with(|| (ThreadInterrupt::current(), 0))
            .1 += 1;
        JobGuard { ev: self, id }
    }

    /// Interrupts every thread that runs a function, except the current one.
    // Java: com/google/devtools/build/lib/concurrent/AbstractQueueVisitor.java:643 AbstractQueueVisitor.interruptInFlightTasks
    fn interrupt_in_flight_tasks(&self) {
        let this_thread = std::thread::current().id();
        let threads: Vec<ThreadInterrupt> = self
            .jobs
            .lock()
            .iter()
            .filter(|(id, _)| **id != this_thread)
            .map(|(_, (thread, _))| thread.clone())
            .collect();
        // Outside the lock: an interrupt runs the thread's listeners.
        for thread in threads {
            thread.interrupt();
        }
    }
}

/// Removes a job recorded by [`Evaluator::add_job`].
struct JobGuard<'e> {
    ev: &'e Evaluator,
    id: ThreadId,
}

impl Drop for JobGuard<'_> {
    // Java: com/google/devtools/build/lib/concurrent/AbstractQueueVisitor.java:450 AbstractQueueVisitor.removeJob
    fn drop(&mut self) {
        let mut jobs = self.ev.jobs.lock();
        if let Some(entry) = jobs.get_mut(&self.id) {
            entry.1 -= 1;
            if entry.1 == 0 {
                jobs.remove(&self.id);
            }
        }
    }
}

/// How often the thread that called [`Evaluator::evaluate`] checks the interrupt flag that
/// [`Evaluator::interrupt_flag`] hands out while the evaluation runs.
///
/// PORT: Java's evaluating thread waits in `awaitQuiescence` and is woken by its own
/// `Thread.interrupt()`; the flag here is a plain `AtomicBool` that a signal handler may set, so
/// the waiting thread polls it.
pub const IN_FLIGHT_INTERRUPT_POLL: Duration = Duration::from_millis(20);

/// What a function returned, after interrupts are sorted out.
enum Ret {
    Value(SkyValue),
    Null,
    Threw(SkyFunctionException),
    /// The function stopped with `Interrupted` because the evaluation is aborting (Java
    /// interrupts in-flight work once it fails fast).
    Abandoned,
}

/// Java: `AbstractParallelEvaluator.DirtyOutcome`, with the outcome of a node that is
/// `ALREADY_PROCESSED` (done, or not finishing in this evaluation).
enum DirtyOutcome {
    AlreadyProcessed(Arc<DoneEntry>),
    NeedsEvaluation,
}

/// The node that requests a dep: its key, its node, and the deps of its last build that it has
/// not re-registered for yet (Java's `oldDeps`: a request for one of them re-registers with
/// `checkIfDoneForDirtyReverseDep`).
pub(crate) struct Requester<'a> {
    pub(crate) key: &'a SkyKey,
    pub(crate) node: &'a Arc<Node>,
    pub(crate) old_deps: &'a IndexSet<SkyKey>,
}

/// State of one `evaluate` call. Java: `ParallelEvaluatorContext` plus the visitor's
/// error/abort state.
pub(crate) struct Run<'e> {
    pub(crate) ev: &'e Evaluator,
    pub(crate) ctx: &'e EvaluationContext,
    /// Java: `ParallelEvaluatorContext.getGraphVersion()`.
    pub(crate) graph_version: Version,
    permits: Permits,
    /// Java: `NodeEntryVisitor.preventNewEvaluations` (first error without keep_going, or a
    /// catastrophe, or a crash).
    pub(crate) aborting: AtomicBool,
    saw_interrupt: AtomicBool,
    /// The in-flight work was interrupted because the evaluation stops (first error without
    /// keep_going, catastrophe, crash).
    jobs_stopped: AtomicBool,
    /// The in-flight work was interrupted because the evaluation was interrupted.
    interrupt_delivered: AtomicBool,
    /// Root workers still running, for the evaluating thread to wait on.
    running_root_workers: Mutex<usize>,
    root_workers_done: Condvar,
    /// The deps that were not done when a node that asked for them committed (a function that
    /// threw although a dep was missing). See [`Run::never_evaluated`].
    undone_deps_of_committed: Mutex<Vec<SkyKey>>,
    crash: Mutex<Option<Box<dyn Any + Send>>>,
    /// The failure that stopped the evaluation: Java's first `SchedulerException.ofError`, which
    /// `waitForCompletion` rethrows. Lock order: this, then a node's lock.
    failure: Mutex<Option<SchedulerError>>,
}

/// Java: `SchedulerException.ofError(errorInfo, failedValue, rdepsToBubbleUpTo)`.
// Java: com/google/devtools/build/skyframe/SchedulerException.java:21 SchedulerException
#[derive(Debug)]
struct SchedulerError {
    error_info: Arc<ErrorInfo>,
    failed_value: SkyKey,
    rdeps_to_bubble_up_to: Vec<SkyKey>,
}

/// What a dependency request found.
pub(crate) struct Requested {
    pub(crate) entry: Arc<DoneEntry>,
    /// The node was done when it was asked for (Java: `graph.get` during `compute` returns its
    /// value, or `addReverseDepAndCheckIfDone` returns `DONE`). False when the request computed
    /// it or waited for it (Java: `compute` got null, returned null, and the node waits to be
    /// restarted when the dep is done).
    pub(crate) was_done: bool,
}

/// Java's `checkState` for a Guava lenient format with more arguments than `%s` placeholders:
/// the extra ones are appended in square brackets.
fn lenient_extra(extra: &[&dyn fmt::Debug]) -> String {
    let parts: Vec<String> = extra.iter().map(|a| format!("{a:?}")).collect();
    format!(" [{}]", parts.join(", "))
}

impl<'e> Run<'e> {
    fn new(ev: &'e Evaluator, ctx: &'e EvaluationContext, graph_version: Version) -> Self {
        Run {
            ev,
            ctx,
            graph_version,
            permits: Permits {
                available: Mutex::new(ctx.parallelism.max(1)),
                cv: Condvar::new(),
            },
            aborting: AtomicBool::new(false),
            saw_interrupt: AtomicBool::new(false),
            jobs_stopped: AtomicBool::new(false),
            interrupt_delivered: AtomicBool::new(false),
            running_root_workers: Mutex::new(0),
            root_workers_done: Condvar::new(),
            undone_deps_of_committed: Mutex::new(vec![]),
            crash: Mutex::new(None),
            failure: Mutex::new(None),
        }
    }

    fn progress(&self) -> &DirtyAndInflightTrackingProgressReceiver {
        &self.ev.progress_receiver
    }

    /// Evaluates a set of values.
    // Java: com/google/devtools/build/skyframe/ParallelEvaluator.java:641 ParallelEvaluator.eval
    fn eval(&self, roots: &[SkyKey]) -> Result<EvaluationResult, Interrupted> {
        let mut seen = HashSet::new();
        let roots: Vec<SkyKey> = roots
            .iter()
            .filter(|k| seen.insert((*k).clone()))
            .cloned()
            .collect();
        // Java: graph.getBatch(null, Reason.PRE_OR_POST_EVALUATION, skyKeySet)
        #[cfg(test)]
        for key in &roots {
            self.ev.graph.listeners_for_testing.notify(
                key,
                crate::graph_listener::EventType::GetBatch,
                crate::graph_listener::Order::Before,
                Context::Reason(crate::graph_listener::Reason::PreOrPostEvaluation),
            );
        }
        let done_before_evaluation: Vec<Option<Arc<DoneEntry>>> =
            roots.iter().map(|k| self.ev.committed_entry(k)).collect();

        // Optimization: if all required node values are already present in the cache, return them
        // directly without launching the heavy machinery, spawning threads, etc.
        // Inform progressReceiver that these nodes are done to be consistent with the main code
        // path.
        if done_before_evaluation.iter().all(Option::is_some) {
            for (key, entry) in roots.iter().zip(&done_before_evaluation) {
                self.inform_progress_receiver_that_value_is_done(
                    key,
                    entry.as_ref().expect("all done"),
                );
            }
            // Note that the 'catastrophe' parameter doesn't really matter here (it's only used
            // for checking).
            return Ok(self.construct_result(&roots, None, /* catastrophe= */ false));
        }

        // PORT: Java collects the cached error keys in a HashSet; they are kept in the order of
        // the roots.
        let mut cached_error_keys: Vec<SkyKey> = vec![];
        if !self.ctx.keep_going {
            for (key, entry) in roots.iter().zip(&done_before_evaluation) {
                if let Some(entry) = entry
                    && entry.error.is_some()
                {
                    self.inform_progress_receiver_that_value_is_done(key, entry);
                    cached_error_keys.push(key.clone());
                }
            }
        }
        // Errors, even cached ones, should halt evaluations not in keepGoing mode.
        if !cached_error_keys.is_empty() {
            // Note that the 'catastrophe' parameter doesn't really matter here (it's only used
            // for checking).
            return Ok(self.construct_result(
                &cached_error_keys,
                None,
                /* catastrophe= */ false,
            ));
        }

        // PORT: SkyKeyComputeState is not ported (unnecessaryTemporaryStateDropperReceiver).
        // Java: com/google/devtools/build/skyframe/ParallelEvaluator.java:692 ParallelEvaluator.eval
        let _c = Profiler::instance()
            .profile(ProfilerTask::SKYFRAME_EVAL, "Parallel Evaluator evaluation");
        self.do_mutating_evaluation(&roots, &done_before_evaluation)
    }

    // Java: com/google/devtools/build/skyframe/ParallelEvaluator.java:135 ParallelEvaluator.doMutatingEvaluation
    fn do_mutating_evaluation(
        &self,
        roots: &[SkyKey],
        done_before_evaluation: &[Option<Arc<DoneEntry>>],
    ) -> Result<EvaluationResult, Interrupted> {
        self.inject_error_transience_value();
        self.evaluate_roots(roots, done_before_evaluation);
        self.wait_for_completion_and_construct_result(roots)
    }

    // Java: com/google/devtools/build/skyframe/ParallelEvaluator.java:187 ParallelEvaluator.injectErrorTransienceValue
    fn inject_error_transience_value(&self) {
        // We unconditionally add the ErrorTransienceValue here, to ensure that it will be
        // created, and in the graph, by the time that it is needed. Creating it on demand in a
        // parallel context sets up a race condition, because there is no way to atomically create
        // a node and set its value.
        let error_transience_entry = self.ev.graph.create_if_absent(&ERROR_TRANSIENCE_KEY);
        let done = error_transience_entry.lock().entry.is_done();
        if !done {
            let map: IndexMap<SkyKey, Delta> = IndexMap::from([(
                ERROR_TRANSIENCE_KEY.clone(),
                Delta::just_new(Arc::new(ErrorTransienceValue)),
            )]);
            self.ev.inject_values_into_graph(&map, self.graph_version);
        }
    }

    // Java: com/google/devtools/build/skyframe/ParallelEvaluator.java:206 ParallelEvaluator.waitForCompletionAndConstructResult
    fn wait_for_completion_and_construct_result(
        &self,
        roots: &[SkyKey],
    ) -> Result<EvaluationResult, Interrupted> {
        self.propagate_crash_if_any();
        if self.saw_interrupt.load(Ordering::SeqCst) || self.ev.interrupted.load(Ordering::SeqCst) {
            return Err(Interrupted);
        }
        let mut bubble_error_info: Option<BubbleErrorInfo> = None;
        let mut catastrophe = false;
        let failure = self.failure.lock().take();
        if let Some(e) = failure {
            bubble_error_info = self.bubble_error_up_on_evaluator_thread(&e, roots);
            self.propagate_crash_if_any();
            if self.ctx.keep_going {
                check_state!(
                    e.error_info.is_catastrophic(),
                    "Scheduler exception only thrown for catastrophe in keep_going evaluation: \
                     {:?}",
                    e
                );
                catastrophe = true;
            }
        }
        Ok(self.construct_result(roots, bubble_error_info.as_ref(), catastrophe))
    }

    // Java: com/google/devtools/build/skyframe/ParallelEvaluator.java:96 ParallelEvaluator.informProgressReceiverThatValueIsDone
    fn inform_progress_receiver_that_value_is_done(&self, key: &SkyKey, entry: &DoneEntry) {
        check_state!(entry.is_done(), "{:?}", entry);
        let value = entry.value.as_ref();
        let value_version = entry.version;
        check_state!(
            value_version.at_most(&self.graph_version),
            "{:?} should be at most {:?} in the version partial ordering",
            value_version,
            self.graph_version
        );

        self.replay(&entry.get_events());
        let error = entry.error.as_deref();

        // For most nodes we do not inform the progress receiver if they were already done when
        // we retrieve them, but top-level nodes are presumably of more interest.
        // If valueVersion is not equal to graphVersion, it must be less than it (by the
        // Preconditions check above), and so the node is clean.
        let changed = value_version == self.graph_version;
        self.progress().evaluated(
            key,
            EvaluationState::get(value, changed),
            if changed { value } else { None },
            if changed { error } else { None },
            None,
        );
    }

    /// Java: `AbstractQueueVisitor.getExceptionLatchForTestingOnly()` is released: the visitor
    /// has caught the exception that stops the evaluation (the failed node is done by then).
    #[cfg(test)]
    pub(crate) fn exception_latch_released_for_testing(&self) -> bool {
        self.failure.lock().is_some() || self.crash.lock().is_some()
    }

    /// Java: `propagateEvaluatorContextCrashIfAny` / the visitor rethrowing a crash.
    fn propagate_crash_if_any(&self) {
        let crash = self.crash.lock().take();
        if let Some(payload) = crash {
            resume_unwind(payload);
        }
    }

    /// Records a crash of the evaluation: stops new work; `evaluate` rethrows the first one.
    fn note_crash(&self, payload: Box<dyn Any + Send>) {
        self.crash.lock().get_or_insert(payload);
        self.aborting.store(true, Ordering::SeqCst);
        self.stop_in_flight_jobs();
    }

    /// Java: `NodeEntryVisitor.preventNewEvaluations()`: true for the caller that set it.
    // Java: com/google/devtools/build/skyframe/NodeEntryVisitor.java:190 NodeEntryVisitor.preventNewEvaluations
    fn prevent_new_evaluations(&self) -> bool {
        self.aborting
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    /// The evaluation stops (`jobsMustBeStopped`): the evaluating thread leaves
    /// `awaitQuiescence` and interrupts the threads that still run functions
    /// (`awaitTermination(interruptWorkers = true)`). Called once `aborting` is set and the
    /// failure is recorded; a function that starts later sees `aborting` (Java:
    /// `blockNewActions`, checked after `addJob`).
    // Java: com/google/devtools/build/lib/concurrent/AbstractQueueVisitor.java:593 AbstractQueueVisitor.reallyAwaitTermination
    fn stop_in_flight_jobs(&self) {
        if !self.jobs_stopped.swap(true, Ordering::SeqCst) {
            self.ev.interrupt_in_flight_tasks();
        }
    }

    /// Starts an evaluator thread. An error (the OS refuses another thread) is returned, never
    /// thrown: a claimed node must be finished on every path, or its waiters sleep forever.
    fn spawn<'scope, T: Send + 'scope>(
        &self,
        scope: &'scope Scope<'scope, '_>,
        f: impl FnOnce() -> T + Send + 'scope,
    ) -> std::io::Result<ScopedJoinHandle<'scope, T>> {
        #[cfg(test)]
        if let Some(left) = &mut *self.ev.spawns_before_failure_for_testing.lock() {
            if *left == 0 {
                return Err(std::io::Error::other("injected spawn failure"));
            }
            *left -= 1;
        }
        self.spawn_builder().spawn_scoped(scope, f)
    }

    fn parallelism(&self) -> usize {
        self.ctx.parallelism.max(1)
    }

    fn spawn_builder(&self) -> std::thread::Builder {
        std::thread::Builder::new()
            .name("skyframe-evaluator".to_string())
            .stack_size(self.ctx.thread_stack_size)
    }

    /// Evaluates the roots on up to `parallelism` worker threads. A root that is done is reported
    /// to the progress receiver if it was done before the evaluation (Java's loop over the roots
    /// in `doMutatingEvaluation`).
    fn evaluate_roots(&self, roots: &[SkyKey], done_before_evaluation: &[Option<Arc<DoneEntry>>]) {
        let workers = roots.len().min(self.parallelism()).max(1);
        self.ev.force_reserve_threads(workers);
        let queue = Mutex::new(
            roots
                .iter()
                .cloned()
                .zip(done_before_evaluation.iter().map(Option::is_some))
                .collect::<VecDeque<_>>(),
        );
        /// Counts a root worker as finished however it ends.
        struct RootWorker<'a, 'e>(&'a Run<'e>);
        impl Drop for RootWorker<'_, '_> {
            fn drop(&mut self) {
                let mut running = self.0.running_root_workers.lock();
                *running -= 1;
                if *running == 0 {
                    self.0.root_workers_done.notify_all();
                }
            }
        }
        std::thread::scope(|scope| {
            let mut handles = vec![];
            for _ in 0..workers {
                let queue = &queue;
                *self.running_root_workers.lock() += 1;
                let spawned = self.spawn(scope, move || {
                    let _done = RootWorker(self);
                    self.permits.acquire();
                    loop {
                        let Some((k, done_before)) = queue.lock().pop_front() else {
                            break;
                        };
                        let r = self.eval_dep(&k, None);
                        // Scheduling above starts concurrent evaluation, so a key that is a
                        // dependency of an earlier key may have become done while this loop was
                        // running. Avoid reporting it twice.
                        if r.was_done && r.entry.is_done() && done_before {
                            self.inform_progress_receiver_that_value_is_done(&k, &r.entry);
                        }
                    }
                    self.ev.unregister_thread();
                    self.permits.release();
                });
                match spawned {
                    Ok(h) => handles.push(h),
                    Err(e) => {
                        *self.running_root_workers.lock() -= 1;
                        // The workers that did start drain the queue. With none, nothing can be
                        // evaluated: crash, as Java does when it cannot create a thread.
                        if handles.is_empty() {
                            self.note_crash(Box::new(format!(
                                "OutOfMemoryError: unable to create evaluator thread: {e}"
                            )));
                        }
                        break;
                    }
                }
            }
            // Java: ParallelEvaluator waits in awaitQuiescence; its interrupt makes it interrupt
            // the workers (AbstractQueueVisitor.awaitQuiescence / awaitTermination).
            let mut running = self.running_root_workers.lock();
            while *running > 0 {
                self.root_workers_done
                    .wait_for(&mut running, IN_FLIGHT_INTERRUPT_POLL);
                if *running > 0
                    && self.ev.interrupted.load(Ordering::SeqCst)
                    && !self.interrupt_delivered.swap(true, Ordering::SeqCst)
                {
                    drop(running);
                    self.ev.interrupt_in_flight_tasks();
                    running = self.running_root_workers.lock();
                }
            }
            drop(running);
            for h in handles {
                if let Err(p) = h.join() {
                    resume_unwind(p);
                }
            }
        });
        self.ev.release_threads(workers);
    }

    /// Computes or waits for `key` on the current thread (which holds a permit), and tells
    /// whether the key was done when it was asked for. `requester` is the node whose function
    /// asked for `key` (`None` for a top-level key): it is registered as a reverse dep of `key`
    /// (Java: `enqueueChild`), and signals itself if `key` is done (Java: `signalDep` on
    /// `DONE`).
    // Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:171 Evaluate.enqueueChild
    pub(crate) fn eval_dep(&self, key: &SkyKey, requester: Option<&Requester<'_>>) -> Requested {
        let node = match requester {
            // A dep of the requester's last build must be in the graph (deletion takes the
            // reverse transitive closure). Reached from the dirty check (Java's
            // `handleKnownChildrenForDirtyNode`); an old dep re-requested by the function was
            // already looked up in `Environment::lookup`, which fails with Java's commit-time
            // text.
            // Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:394 Evaluate.handleKnownChildrenForDirtyNode
            Some(r) if r.old_deps.contains(key) => {
                check_not_null!(
                    self.ev.graph.get_if_present(key),
                    "Dirty parent had missing child (child={:?}, parent={:?} {:?})",
                    key,
                    r.key,
                    r.node.lock().entry
                )
            }
            _ => self.ev.graph.create_if_absent(key),
        };
        // Test hook: the event of the call below (Java's NotifyingNodeEntry).
        #[cfg(test)]
        let (listened_call, listened_context) = match requester {
            Some(r) if r.old_deps.contains(key) => (
                crate::graph_listener::EventType::CheckIfDone,
                Context::Key(r.key.clone()),
            ),
            Some(r) => (
                crate::graph_listener::EventType::AddReverseDep,
                Context::Key(r.key.clone()),
            ),
            None => (
                crate::graph_listener::EventType::AddReverseDep,
                Context::Null,
            ),
        };
        #[cfg(test)]
        self.ev.graph.listeners_for_testing.notify(
            key,
            listened_call,
            crate::graph_listener::Order::Before,
            listened_context.clone(),
        );
        let mut state = node.lock();
        let dependency_state = match requester {
            None => state.entry.add_reverse_dep_and_check_if_done(None),
            Some(r) if r.old_deps.contains(key) => state
                .entry
                .check_if_done_for_dirty_reverse_dep(r.key.clone()),
            Some(r) => state
                .entry
                .add_reverse_dep_and_check_if_done(Some(r.key.clone())),
        };
        // Test hook: the AFTER event, once the node's lock is released.
        #[cfg(test)]
        let notify_after = || {
            self.ev.graph.listeners_for_testing.notify(
                key,
                listened_call,
                crate::graph_listener::Order::After,
                listened_context.clone(),
            )
        };
        match dependency_state {
            DependencyState::Done => {
                let snapshot = state.snapshot.clone().expect("done entry has a snapshot");
                drop(state);
                #[cfg(test)]
                notify_after();
                if let Some(r) = requester {
                    notify_graph_listener!(
                        self.ev.graph,
                        r.key,
                        Signal,
                        Before,
                        Context::Key(key.clone())
                    );
                    let mut parent = r.node.lock();
                    parent.entry.signal_dep(snapshot.version, Some(key));
                    drop(parent);
                    notify_graph_listener!(
                        self.ev.graph,
                        r.key,
                        Signal,
                        After,
                        Context::Key(key.clone())
                    );
                }
                Requested {
                    entry: snapshot,
                    was_done: true,
                }
            }
            DependencyState::NeedsScheduling => {
                state.owner = Some(std::thread::current().id());
                state.outcome = None;
                // PORT: blocking engine. Like `outcome`, the requesters that closed a cycle
                // through the node belong to one evaluation: a node committed by the cycle
                // detector stays in the graph, and when it is evaluated again (dirty or
                // changed) its old cycle requesters must not be dropped from the reverse deps
                // its error bubbles up to (`rdeps_to_bubble_up_to` below).
                state.cycle_rdeps.clear();
                drop(state);
                #[cfg(test)]
                notify_after();
                self.enqueue_evaluation(key);
                let entry = if DEPTH.with(Cell::get) >= self.ctx.max_inline_depth {
                    self.compute_on_fresh_thread(key, &node)
                } else {
                    self.compute_claimed(key, &node)
                };
                Requested {
                    entry,
                    was_done: false,
                }
            }
            DependencyState::AlreadyEvaluating => {
                if let Some(outcome) = state.outcome.clone() {
                    drop(state);
                    #[cfg(test)]
                    notify_after();
                    // It ended in this evaluation without finishing.
                    return Requested {
                        entry: outcome,
                        was_done: false,
                    };
                }
                check_state!(
                    state.owner.is_some(),
                    "node in flight from an earlier evaluation: {:?} {:?}",
                    key,
                    state.entry
                );
                drop(state);
                #[cfg(test)]
                notify_after();
                Requested {
                    entry: self.wait_for(key, &node, requester.map(|r| r.key)),
                    was_done: false,
                }
            }
        }
    }

    /// Java: `NodeEntryVisitor.enqueueEvaluation`'s progress bookkeeping.
    // Java: com/google/devtools/build/skyframe/NodeEntryVisitor.java:233 NodeEntryVisitor.innerEnqueueEvaluation
    fn enqueue_evaluation(&self, key: &SkyKey) {
        if self.aborting.load(Ordering::SeqCst) {
            // If an error happens in nokeep_going mode, we still want to mark these nodes as
            // inflight, otherwise cleanup will not happen properly.
            self.progress().enqueue_after_error(key);
            return;
        }
        self.progress().enqueueing(key);
    }

    /// Computes a key this thread claimed on a new thread, so that inline recursion depth stays
    /// bounded. The permit this thread holds passes to the new thread while this one waits.
    fn compute_on_fresh_thread(&self, key: &SkyKey, node: &Arc<Node>) -> Arc<DoneEntry> {
        let me = std::thread::current().id();
        {
            let mut w = self.ev.waits.lock();
            let frames = w.stacks.get(&me).map_or(0, Vec::len);
            w.waiting
                .entry(me)
                .or_default()
                .push((frames, vec![key.clone()]));
        }
        self.ev.force_reserve_threads(1);
        let r = std::thread::scope(|scope| {
            let spawned = self.spawn(scope, || {
                let r = self.compute_claimed(key, node);
                self.ev.unregister_thread();
                r
            });
            match spawned {
                Ok(h) => h.join().unwrap_or_else(|p| resume_unwind(p)),
                Err(e) => {
                    // The key is claimed: finish it, or its waiters never wake. Java dies of an
                    // OutOfMemoryError ("unable to create native thread") here; crash likewise.
                    self.note_crash(Box::new(format!(
                        "OutOfMemoryError: unable to create evaluator thread for {key:?}: {e}"
                    )));
                    let outcome = Arc::new(DoneEntry::undone(DoneKind::Aborted));
                    node.publish_outcome(outcome.clone());
                    outcome
                }
            }
        });
        self.ev.release_threads(1);
        if let Some(v) = self.ev.waits.lock().waiting.get_mut(&me) {
            v.pop();
        }
        r
    }

    fn compute_claimed(&self, key: &SkyKey, node: &Arc<Node>) -> Arc<DoneEntry> {
        let me = std::thread::current().id();
        {
            let mut w = self.ev.waits.lock();
            // The claiming thread may have handed the key to this thread (depth guard).
            node.lock().owner = Some(me);
            w.stacks.entry(me).or_default().push(key.clone());
        }
        // Java: WrappedRunnable.run: addJob, then blockNewActions, so that a thread racing to
        // interrupt the in-flight work either finds this job or this job sees the abort.
        let job = self.ev.add_job();
        let outcome: Arc<DoneEntry> = if self.ev.interrupted.load(Ordering::SeqCst) {
            self.saw_interrupt.store(true, Ordering::SeqCst);
            Arc::new(DoneEntry::undone(DoneKind::Interrupted))
        } else if self.aborting.load(Ordering::SeqCst) {
            // Java: no new evaluations start once the visitor prevents them.
            Arc::new(DoneEntry::undone(DoneKind::Aborted))
        } else {
            DEPTH.with(|d| d.set(d.get() + 1));
            let r = catch_unwind(AssertUnwindSafe(|| self.evaluate_node(key, node)));
            DEPTH.with(|d| d.set(d.get() - 1));
            match r {
                Ok(e) => e,
                Err(payload) => {
                    // Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:548 Evaluate.run
                    // Programmer error (most likely NPE or a failed precondition in a
                    // SkyFunction). Output some context together with the exception.
                    // Stop new work, let waiters finish, and rethrow from `evaluate`.
                    let reverse_deps: Vec<SkyKey> = {
                        let state = node.lock();
                        if state.entry.is_done() {
                            vec![]
                        } else {
                            state
                                .entry
                                .get_in_progress_reverse_deps()
                                .into_iter()
                                .collect()
                        }
                    };
                    let msg = prepare_crash_message(key, &reverse_deps);
                    self.note_crash(wrap_crash(msg, payload));
                    Arc::new(DoneEntry::undone(DoneKind::Aborted))
                }
            }
        };
        drop(job);
        if let Some(s) = self.ev.waits.lock().stacks.get_mut(&me) {
            s.pop();
        }
        node.publish_outcome(outcome.clone());
        outcome
    }

    fn function_for(&self, key: &SkyKey, node: &Node) -> Arc<dyn SkyFunction> {
        let function_name = key.function_name();
        // Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:457
        let Some(function) = self.ev.functions.get(&function_name).cloned() else {
            let entry = format!("{:?}", node.lock().entry);
            panic!(
                "NullPointerException: Unable to find SkyFunction '{function_name}' for node with \
                 key {key:?}, {entry}"
            );
        };
        function
    }

    /// An action that evaluates a value: the owning thread's run of a node it claimed.
    ///
    /// PORT: blocking engine. Where Java re-enqueues the node (`enqueueEvaluation(skyKey, null)`
    /// after a `Reset`, an undone previously requested dep, or an undone dep found while
    /// registering the deps of a done function), the claiming thread runs it again here.
    // Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:415 Evaluate.run
    fn evaluate_node(&self, key: &SkyKey, node: &Arc<Node>) -> Arc<DoneEntry> {
        loop {
            if let Some(outcome) = self.evaluate_node_once(key, node) {
                return outcome;
            }
            if self.ev.interrupted.load(Ordering::SeqCst) {
                self.saw_interrupt.store(true, Ordering::SeqCst);
                return Arc::new(DoneEntry::undone(DoneKind::Interrupted));
            }
            if self.aborting.load(Ordering::SeqCst) {
                // Java: the visitor accepts no new evaluation once it prevents them; the node
                // stays in flight.
                self.enqueue_evaluation(key);
                return Arc::new(DoneEntry::undone(DoneKind::Aborted));
            }
            // Java: evaluatorContext.getVisitor().enqueueEvaluation(skyKey, null).
            self.enqueue_evaluation(key);
        }
    }

    /// One run of [`Run::evaluate_node`]: `None` when the node must run again.
    // Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:415 Evaluate.run
    fn evaluate_node_once(&self, key: &SkyKey, node: &Arc<Node>) -> Option<Arc<DoneEntry>> {
        // PORT: Java's `!nodeEntry.isReadyToEvaluate()` check is for partial reevaluation (not
        // ported): a node is claimed here once, ready. The test hook still sees the call.
        notify_graph_listener!(self.ev.graph, key, IsReady, Before, Context::Entry);
        let dirty_outcome = {
            let _state = StateGuard::new(self.progress(), key, NodeState::CheckDirty);
            let outcome = self.maybe_handle_dirty_node(key, node);
            if let DirtyOutcome::AlreadyProcessed(_) = &outcome
                && self.get_lifecycle_state(key, node) == LifecycleState::Done
            {
                self.progress().change_pruned(key);
            }
            outcome
        };
        if let DirtyOutcome::AlreadyProcessed(outcome) = dirty_outcome {
            return Some(outcome);
        }

        // PORT: Java keeps the deps registered before a reset (`getResetDirectDeps`) apart from
        // `oldDeps` and unites them where it registers new deps (`alreadyRegisteredDeps`:
        // `enqueueChild` with `depAlreadyExists`, `maybeHandleRegisteringNewlyDiscoveredDeps
        // ForDoneEntry`) and where it commits (`depsToRemove`); the environment's `old_deps`
        // serve exactly those, so they hold both. (Java's child-error edge of a no-keep-going
        // failure is added with `addReverseDepAndCheckIfDone` for a reset dep; here with
        // `checkIfDoneForDirtyReverseDep`, as for an old dep.)
        let old_deps: IndexSet<SkyKey> = {
            let mut state = node.lock();
            let mut old_deps = state.entry.get_all_remaining_dirty_direct_deps();
            old_deps.extend(state.entry.get_reset_direct_deps());
            old_deps
        };
        let function = self.function_for(key, node);
        let mut env = {
            let _state = StateGuard::new(self.progress(), key, NodeState::InitializingEnvironment);
            let previously_requested_deps = {
                let mut state = node.lock();
                let deps = state.entry.get_temporary_direct_deps();
                if deps.is_empty() {
                    GroupedDeps::new()
                } else {
                    deps.clone()
                }
            };
            match Environment::create(
                self,
                key,
                node,
                &*function,
                previously_requested_deps,
                old_deps,
            ) {
                Ok(env) => env,
                Err(_undone_previously_requested_deps) => {
                    // Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:448 Evaluate.run
                    self.handle_undone_previously_requested_dep(key, node);
                    return None;
                }
            }
        };
        self.ev.computed.fetch_add(1, Ordering::Relaxed);
        if let Some(recorded) = &mut *self.ev.recorded_computed_keys.lock() {
            recorded.push(key.clone());
        }
        let r = {
            // Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:466 Evaluate.run
            let _s = Profiler::instance().profile(ProfilerTask::SKYFUNCTION, key.function_name().0);
            let _state = StateGuard::new(self.progress(), key, NodeState::Compute);
            function.compute(key, &mut env)
        };

        // Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:569 Evaluate.run
        if let Ok(Some(value)) = &r
            && let Some(reset) = value.downcast_ref::<Reset>()
        {
            if node.lock().entry.has_unsignaled_deps() {
                // PORT: Java's case is a partial reevaluation (not ported). Here a dep the
                // function waited for ended undone (the evaluation stops): the node stays in
                // flight, as Java's does when the visitor accepts no new evaluation.
                return Some(Arc::new(DoneEntry::undone(DoneKind::Aborted)));
            }
            // PORT: the deps the function requested since it last blocked (`pending`) were never
            // registered; like Java's newly requested deps of a run that returns a Reset, they
            // are dropped.
            drop(env);
            self.dirty_rewind_graph_and_reset_entry(key, node, reset);
            // PORT: stateCache.invalidate (SkyKeyComputeState) and cancelExternalDeps
            // (dependOnFuture) are not ported.
            return None;
        }

        if node.lock().restart_requested {
            // A dep registered while the function ran was found undone (rewound); the thread that
            // registered it evaluated it, and it signalled this node. The run is discarded and the
            // function runs again (Java: the node is not committed, and is re-enqueued when the
            // dep signals it). The deps requested since the function last blocked are registered
            // first, as Java registers a function's new deps before it finds an undone one.
            let (mut st, _values_missing, _listener) = env.into_parts();
            let requester = Requester {
                key,
                node,
                old_deps: &st.old_deps,
            };
            self.register_pending_deps(&requester, &mut st.pending, &mut st.pending_group_sizes);
            node.lock().restart_requested = false;
            return None;
        }

        let outcome = self.resolve(key, node, env, r);
        let mut state = node.lock();
        if state.restart_requested && !state.entry.is_done() {
            // An undone dep found while the function's deps were registered at commit
            // (`maybeHandleRegisteringNewlyDiscoveredDepsForDoneEntry`): nothing was committed.
            state.restart_requested = false;
            return None;
        }
        state.restart_requested = false;
        Some(outcome)
    }

    /// If a previously requested dep is no longer done, restart this node from scratch.
    // Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:795 Evaluate.handleUndonePreviouslyRequestedDep
    fn handle_undone_previously_requested_dep(&self, key: &SkyKey, node: &Arc<Node>) {
        // PORT: stateCache.invalidate (SkyKeyComputeState) is not ported.
        self.reset_entry(key, node);
    }

    /// Resets `entry`, and the other nodes specified by `restart.rewind_graph()` will be marked
    /// changed via postorder DFS.
    ///
    /// `restart.rewind_graph()` must contain `key`.
    ///
    /// Nodes must be marked changed via postorder DFS. To see why, suppose we have this graph:
    ///
    /// ```text
    ///   FailedNode   SomeOtherRdepOfR1
    ///       |       /
    ///       |  -----
    ///       | /
    ///       R1
    ///       |
    ///       R2
    /// ```
    ///
    /// Suppose FailedNode (FN) fails and requires that R1 and R2 must be dirtied and run again.
    /// Suppose they aren't dirtied via postorder DFS, so R1 is dirtied first.
    ///
    /// Then, the evaluation thread working on dirtying these nodes is suspended.
    ///
    /// On a separate evaluation thread, SomeOtherRdepOfR1 requests R1. R1 is scheduled for
    /// evaluation, checks its dep R2, and because R2 is done, R1 completes without scheduling R2
    /// for evaluation.
    ///
    /// Then, the evaluation thread working on dirtying these nodes continues its work. It dirties
    /// R2 and schedules FN for evaluation.
    ///
    /// When FN next evaluates, it requests R1, and because R1 is done, R2 is not scheduled for
    /// evaluation, contrary to FN's expectations.
    // Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:872 AbstractParallelEvaluator.dirtyRewindGraphAndResetEntry
    fn dirty_rewind_graph_and_reset_entry(&self, key: &SkyKey, node: &Arc<Node>, restart: &Reset) {
        let rewind_graph = restart.rewind_graph();
        check_state!(
            rewind_graph.contains(key),
            "Rewind graph missing evaluating key {:?}: {:?}",
            key,
            rewind_graph
        );

        let children_to_restart: Vec<SkyKey> = rewind_graph
            .depth_first_post_order(key)
            .into_iter()
            .filter(|k| k != key)
            .collect();
        if !children_to_restart.is_empty() {
            self.ev
                .graph_inconsistency_receiver()
                .note_inconsistency_and_maybe_throw(
                    key,
                    Some(&children_to_restart),
                    Inconsistency::ParentForceRebuildOfChild,
                );

            for child_to_restart in &children_to_restart {
                let child_entry = check_not_null!(
                    self.ev.graph.get_if_present(child_to_restart),
                    "Missing child for rewinding: {:?} (parent={:?})",
                    child_to_restart,
                    key
                );
                let marked = {
                    let mut state = child_entry.lock();
                    let marked = state.entry.mark_dirty(DirtyType::Rewind);
                    state.refresh_snapshot();
                    marked
                };
                if marked.is_some() {
                    self.progress().dirtied(child_to_restart, DirtyType::Rewind);
                }
            }
        }

        self.reset_entry(key, node);
    }

    // Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:915 AbstractParallelEvaluator.resetEntry
    fn reset_entry(&self, key: &SkyKey, node: &Arc<Node>) {
        self.ev
            .graph_inconsistency_receiver()
            .note_inconsistency_and_maybe_throw(key, None, Inconsistency::ResetRequested);
        node.lock().entry.reset_evaluation_from_scratch();
    }

    /// `nodeEntry.getLifecycleState()`, with the test hook's events.
    #[cfg_attr(not(test), allow(unused_variables))]
    fn get_lifecycle_state(&self, sky_key: &SkyKey, node: &Node) -> LifecycleState {
        notify_graph_listener!(
            self.ev.graph,
            sky_key,
            GetLifecycleState,
            Before,
            Context::Entry
        );
        let lifecycle_state = node.lock().entry.get_lifecycle_state();
        notify_graph_listener!(
            self.ev.graph,
            sky_key,
            GetLifecycleState,
            After,
            Context::LifecycleState(lifecycle_state)
        );
        lifecycle_state
    }

    /// Returns true if this depGroup consists of the error transience value and the error
    /// transience value is newer than the entry, meaning that the entry must be re-evaluated.
    // Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:221 Evaluate.invalidatedByErrorTransience
    fn invalidated_by_error_transience(
        &self,
        dep_group: &[SkyKey],
        entry_version: Version,
    ) -> bool {
        dep_group.len() == 1
            && dep_group[0] == *ERROR_TRANSIENCE_KEY
            && !self
                .ev
                .graph
                .get_if_present(&ERROR_TRANSIENCE_KEY)
                .expect("ErrorTransienceValue.KEY is in the graph")
                .lock()
                .entry
                .get_version()
                .at_most(&entry_version)
    }

    /// PORT: where Java enqueues the deps of a group that are not done and returns (the node is
    /// re-enqueued when the last one signals), the group is evaluated here with the code path of
    /// a function's requests; a dep that is not done afterwards never will be in this
    /// evaluation, and the node ends with that dep's kind (still dirty, in flight).
    // Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:233 Evaluate.maybeHandleDirtyNode
    fn maybe_handle_dirty_node(&self, sky_key: &SkyKey, node: &Arc<Node>) -> DirtyOutcome {
        while self.get_lifecycle_state(sky_key, node) == LifecycleState::CheckDependencies {
            // Evaluating a dirty node for the first time, and checking its children to see if any
            // of them have changed. Note that there must be dirty children for this to happen.

            // Check the children group by group -- we don't want to evaluate a value that is no
            // longer needed because an earlier dependency changed. For example, //foo:foo depends
            // on target //bar:bar and is built. Then foo/BUILD is modified to remove the
            // dependence on bar, and bar/BUILD is deleted. Reloading //bar:bar would incorrectly
            // throw an exception. To avoid this, we must reload foo/BUILD first, at which point
            // we will discover that it has changed, and re-evaluate target //foo:foo from
            // scratch. On the other hand, when an action requests all of its inputs, we can
            // safely check all of them in parallel on a subsequent build. So we allow checking an
            // entire group in parallel here, if the node builder requested a group last build.
            // Note: every dep returned here must either have this node re-registered for it
            // (using checkIfDoneForDirtyReverseDep) and be registered as a direct dep of this
            // node, or have its reverse dep on this node removed. Failing to do either one of
            // these would result in a graph inconsistency, where the child had a reverse dep on
            // this node, but this node had no kind of dependency on the child.
            let (direct_deps_to_check, entry_version) = {
                let mut state = node.lock();
                let deps = state.entry.get_next_dirty_direct_deps();
                (deps, state.entry.get_version())
            };

            if self.invalidated_by_error_transience(&direct_deps_to_check, entry_version) {
                // If this dep is the ErrorTransienceValue and the ErrorTransienceValue has been
                // updated then we need to force a rebuild. We would like to just signal the entry
                // as usual, but we can't, because then the ErrorTransienceValue would remain as a
                // dep, which would be incorrect if, for instance, the value re-evaluated to a
                // non-error.
                node.lock().entry.force_rebuild();
                notify_graph_listener!(
                    self.ev.graph,
                    &ERROR_TRANSIENCE_KEY,
                    RemoveReverseDep,
                    Before,
                    Context::Key(sky_key.clone())
                );
                self.ev
                    .graph
                    .get_if_present(&ERROR_TRANSIENCE_KEY)
                    .expect("ErrorTransienceValue.KEY is in the graph")
                    .lock()
                    .entry
                    .remove_reverse_dep(sky_key.clone());
                notify_graph_listener!(
                    self.ev.graph,
                    &ERROR_TRANSIENCE_KEY,
                    RemoveReverseDep,
                    After,
                    Context::Key(sky_key.clone())
                );
                return DirtyOutcome::NeedsEvaluation;
            }
            if !self.ctx.keep_going {
                // This check ensures that we maintain the invariant that if a node with an error
                // is reached during a no-keep-going build, none of its currently building parents
                // finishes building. If the child isn't done building yet, it will detect on its
                // own that it has an error (see the VERIFIED_CLEAN case below). On the other
                // hand, if it is done, then it is the parent's responsibility to notice that,
                // which we do here. We check the deps for errors so that we don't continue
                // building this node if it has a child error.
                let entries_to_check: Vec<Arc<Node>> = direct_deps_to_check
                    .iter()
                    .map(|k| {
                        self.ev
                            .graph
                            .get_if_present(k)
                            .unwrap_or_else(|| panic!("NullPointerException: {k:?}"))
                    })
                    .collect();
                for (key_to_check, node_entry_to_check) in
                    direct_deps_to_check.iter().zip(&entries_to_check)
                {
                    let maybe_error_info = {
                        let state = node_entry_to_check.lock();
                        // Java: getValueMaybeWithMetadata() (null: not done), then
                        // ValueWithMetadata.getMaybeErrorInfo.
                        match state.entry.get_value_maybe_with_metadata() {
                            Some(ValueWithMetadata::ErrorInfoValue { error_info, .. }) => {
                                error_info.clone()
                            }
                            _ => continue,
                        }
                    };
                    // This child has an error. We add a dep from this node to it and throw an
                    // exception coming from it.
                    node.lock()
                        .entry
                        .add_singleton_temporary_direct_dep(key_to_check.clone());
                    notify_graph_listener!(
                        self.ev.graph,
                        key_to_check,
                        CheckIfDone,
                        Before,
                        Context::Key(sky_key.clone())
                    );
                    node_entry_to_check
                        .lock()
                        .entry
                        .check_if_done_for_dirty_reverse_dep(sky_key.clone());
                    notify_graph_listener!(
                        self.ev.graph,
                        key_to_check,
                        CheckIfDone,
                        After,
                        Context::Key(sky_key.clone())
                    );
                    // Perform the necessary bookkeeping for any deps that are not being used.
                    for (dep_key, dep_node) in direct_deps_to_check.iter().zip(&entries_to_check) {
                        if dep_key != key_to_check {
                            notify_graph_listener!(
                                self.ev.graph,
                                dep_key,
                                RemoveReverseDep,
                                Before,
                                Context::Key(sky_key.clone())
                            );
                            dep_node.lock().entry.remove_reverse_dep(sky_key.clone());
                            notify_graph_listener!(
                                self.ev.graph,
                                dep_key,
                                RemoveReverseDep,
                                After,
                                Context::Key(sky_key.clone())
                            );
                        }
                    }
                    if !self.prevent_new_evaluations() {
                        // An error was already thrown in the evaluator. Don't do anything here.
                        return DirtyOutcome::AlreadyProcessed(Arc::new(DoneEntry::undone(
                            DoneKind::Aborted,
                        )));
                    }
                    // Java: throw SchedulerException.ofError(maybeErrorInfo, keyToCheck,
                    // ImmutableSet.of(skyKey)).
                    self.failure.lock().get_or_insert_with(|| SchedulerError {
                        error_info: maybe_error_info,
                        failed_value: key_to_check.clone(),
                        rdeps_to_bubble_up_to: vec![sky_key.clone()],
                    });
                    self.stop_in_flight_jobs();
                    return DirtyOutcome::AlreadyProcessed(Arc::new(DoneEntry::awaiting_deps(
                        DoneKind::Aborted,
                    )));
                }
            }
            // It is safe to add these deps back to the node -- even if one of them has changed,
            // the contract of pruning is that the node will request these deps again when it
            // rebuilds. We must add these deps before enqueuing them, so that the node knows that
            // it depends on them. If one of these deps is the error transience node, the check we
            // did above in #invalidatedByErrorTransience means that the error transience node is
            // not newer than this node, so we are going to mark it clean (since the error
            // transience node is always the last dep).
            node.lock()
                .entry
                .add_temporary_direct_dep_group(direct_deps_to_check.clone());
            // PORT: graph.analyzeDepsDoneness is DepsReport.NO_INFORMATION for the in-memory
            // graph: no dep is signalled with the minimal version.
            // Java: handleKnownChildrenForDirtyNode (enqueueChild with depAlreadyExists = true).
            let all_old: IndexSet<SkyKey> = direct_deps_to_check.iter().cloned().collect();
            let requester = Requester {
                key: sky_key,
                node,
                old_deps: &all_old,
            };
            let results = self.eval_keys(&direct_deps_to_check, Some(&requester));
            let mut kind: Option<DoneKind> = None;
            for r in results {
                let entry = self.entry_seen_in_normal_evaluation(r);
                if entry.is_undone_view() {
                    kind = Some(match (kind, entry.kind) {
                        (Some(DoneKind::Interrupted), _) | (_, DoneKind::Interrupted) => {
                            DoneKind::Interrupted
                        }
                        (Some(DoneKind::Aborted), _) | (_, DoneKind::Aborted) => DoneKind::Aborted,
                        _ => DoneKind::InCycle,
                    });
                }
            }
            if kind == Some(DoneKind::Interrupted) {
                self.saw_interrupt.store(true, Ordering::SeqCst);
                return DirtyOutcome::AlreadyProcessed(Arc::new(DoneEntry::undone(
                    DoneKind::Interrupted,
                )));
            }
            if let Some(kind) = kind {
                // The node waits for a dep that will not signal in this evaluation.
                return DirtyOutcome::AlreadyProcessed(Arc::new(DoneEntry::awaiting_deps(kind)));
            }
            if self.aborting.load(Ordering::SeqCst) {
                return DirtyOutcome::AlreadyProcessed(Arc::new(DoneEntry::awaiting_deps(
                    DoneKind::Aborted,
                )));
            }
            // If we're here, then we may proceed to the rest of the method and continue
            // processing the node intra-thread.
            notify_graph_listener!(self.ev.graph, sky_key, IsReady, Before, Context::Entry);
            let ready = node.lock().entry.is_ready_to_evaluate();
            check_state!(
                ready,
                "Dirty node not ready after its deps signalled: {:?} {:?}",
                sky_key,
                node.lock().entry
            );
        }
        let lifecycle_state = self.get_lifecycle_state(sky_key, node);
        match lifecycle_state {
            LifecycleState::VerifiedClean => {
                // No child has a changed value. This node can be marked done and its parents
                // signaled without any re-evaluation.
                notify_graph_listener!(self.ev.graph, sky_key, MarkClean, Before, Context::Entry);
                let (node_value_and_rdeps, error, version) = {
                    let mut state = node.lock();
                    let r = state.entry.mark_clean();
                    state.refresh_snapshot();
                    let error = state.entry.get_error_info().cloned();
                    (r, error, state.entry.get_version())
                };
                notify_graph_listener!(self.ev.graph, sky_key, MarkClean, After, Context::Entry);
                #[cfg(test)]
                let node_value_and_rdeps = self.ev.deterministic_mark_clean(node_value_and_rdeps);
                let r_deps_to_signal = node_value_and_rdeps.get_rdeps_to_signal();
                let value_maybe_with_metadata = node_value_and_rdeps.get_value();
                // Replay events once change-pruned.
                self.replay(&ValueWithMetadata::get_events(value_maybe_with_metadata));
                // Tell the receiver that the value was not actually changed this run.
                self.progress().evaluated(
                    sky_key,
                    EvaluationState::get(value_maybe_with_metadata.get_value(), false),
                    None,
                    None,
                    None,
                );
                let snapshot = node.snapshot().expect("marked clean");
                if !self.ctx.keep_going && error.is_some() {
                    if !self.prevent_new_evaluations() {
                        return DirtyOutcome::AlreadyProcessed(snapshot);
                    }
                    // Java: throw SchedulerException.ofError(nodeEntry.getErrorInfo(), skyKey,
                    // rDepsToSignal).
                    self.failure.lock().get_or_insert_with(|| SchedulerError {
                        error_info: snapshot.error.clone().expect("checked above"),
                        failed_value: sky_key.clone(),
                        rdeps_to_bubble_up_to: r_deps_to_signal.iter().cloned().collect(),
                    });
                    self.stop_in_flight_jobs();
                    return DirtyOutcome::AlreadyProcessed(snapshot);
                }
                self.signal_parents_and_enqueue_if_ready(sky_key, r_deps_to_signal, version);
                DirtyOutcome::AlreadyProcessed(snapshot)
            }
            LifecycleState::NeedsRebuilding => {
                node.lock().entry.mark_rebuilding();
                DirtyOutcome::NeedsEvaluation
            }
            LifecycleState::Rebuilding => DirtyOutcome::NeedsEvaluation,
            _ => {
                let entry = format!("{:?}", node.lock().entry);
                panic!("IllegalStateException: key: {sky_key:?}, entry: {entry}")
            }
        }
    }

    /// What a dependency request shows to a function in normal evaluation.
    ///
    /// PORT: a dep that was not done when it was asked for, and is done now that the evaluation
    /// has failed, looks not done. In Java the function got null for it and returned null; its
    /// node waits to be restarted when the dep is done, and once the evaluation has failed the
    /// visitor accepts no new work (`preventNewEvaluations`), so it never is: the node stays
    /// unfinished whatever the dep became. Here the function was blocked in the request and
    /// resumes its only run, so the result is withheld from that run. In particular the parents
    /// that were waiting for the node whose exception stopped the evaluation do not see that
    /// error in normal evaluation (with keep_going they otherwise would); they are re-run in
    /// error bubbling.
    ///
    /// A dep that was done when asked for is shown as it is: Java's running function reads it
    /// the same way ("there is a race-like opportunity for the error to be used, either by an
    /// in-flight computation or by a future computation").
    pub(crate) fn entry_seen_in_normal_evaluation(&self, requested: Requested) -> Arc<DoneEntry> {
        if !requested.was_done && requested.entry.is_done() && self.aborting.load(Ordering::SeqCst)
        {
            return Arc::new(DoneEntry::undone(DoneKind::Aborted));
        }
        requested.entry
    }

    /// Registers the deps requested by the function that have not been registered yet (Java:
    /// `env.addTemporaryDirectDepsTo(entry)` and `enqueueChild` for each, or
    /// `maybeHandleRegisteringNewlyDiscoveredDepsForDoneEntry`): adds their groups to the
    /// entry's temporary direct deps, registers the node as a reverse dep of each (they are all
    /// done) and signals the node with each one's version. Returns whether the last signal made
    /// the node ready (Java: `DEP_DONE_SELF_SIGNALLED`).
    ///
    /// PORT: Java registers a function's new deps when the function returns (null, a value or an
    /// error); a request that blocks registers them before it waits (see
    /// [`Environment`]'s requests), so this registers only those requested since then.
    pub(crate) fn register_pending_deps(
        &self,
        requester: &Requester<'_>,
        pending: &mut Vec<PendingDep>,
        pending_group_sizes: &mut Vec<usize>,
    ) -> bool {
        if pending.is_empty() {
            return false;
        }
        {
            let mut state = requester.node.lock();
            let mut it = pending.iter();
            for &size in pending_group_sizes.iter() {
                let group: Vec<SkyKey> = it.by_ref().take(size).map(|p| p.key.clone()).collect();
                state.entry.add_temporary_direct_dep_group(group);
            }
        }
        let self_signalled = self.register_done_deps(requester, pending);
        pending.clear();
        pending_group_sizes.clear();
        self_signalled
    }

    /// Registers the node as a reverse dep of each of `deps`, which are done, and signals the
    /// node with each one's version (Java: `enqueueChild` / `maybeHandleUndoneDepForDoneEntry`
    /// on `DONE`). Returns whether the last signal made the node ready.
    pub(crate) fn register_done_deps(
        &self,
        requester: &Requester<'_>,
        deps: &[PendingDep],
    ) -> bool {
        if deps.is_empty() {
            return false;
        }
        let pending = deps;
        let mut versions: Vec<Option<Version>> = Vec::with_capacity(pending.len());
        // Deps found undone, with what registering the node with them answered.
        let mut undone: Vec<(&PendingDep, DependencyState)> = Vec::new();
        for dep in pending.iter() {
            #[cfg(test)]
            let listened_call = if requester.old_deps.contains(&dep.key) {
                crate::graph_listener::EventType::CheckIfDone
            } else {
                crate::graph_listener::EventType::AddReverseDep
            };
            #[cfg(test)]
            self.ev.graph.listeners_for_testing.notify(
                &dep.key,
                listened_call,
                crate::graph_listener::Order::Before,
                Context::Key(requester.key.clone()),
            );
            let mut dep_state = dep.node.lock();
            let tri_state = if requester.old_deps.contains(&dep.key) {
                dep_state
                    .entry
                    .check_if_done_for_dirty_reverse_dep(requester.key.clone())
            } else {
                dep_state
                    .entry
                    .add_reverse_dep_and_check_if_done(Some(requester.key.clone()))
            };
            if tri_state != DependencyState::Done {
                drop(dep_state);
                // Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:1115 AbstractParallelEvaluator.maybeHandleUndoneDepForDoneEntry
                // The dep may have transitioned from done to dirty (rewound) between when the
                // function read it and now. Notify the graph inconsistency receiver so that we
                // can crash if that's unexpected.
                self.ev
                    .graph_inconsistency_receiver()
                    .note_inconsistency_and_maybe_throw(
                        requester.key,
                        Some(std::slice::from_ref(&dep.key)),
                        Inconsistency::BuildingParentFoundUndoneChild,
                    );
                versions.push(None);
                undone.push((dep, tri_state));
                #[cfg(test)]
                self.ev.graph.listeners_for_testing.notify(
                    &dep.key,
                    listened_call,
                    crate::graph_listener::Order::After,
                    Context::Key(requester.key.clone()),
                );
                continue;
            }
            versions.push(Some(dep_state.entry.get_version()));
            drop(dep_state);
            #[cfg(test)]
            self.ev.graph.listeners_for_testing.notify(
                &dep.key,
                listened_call,
                crate::graph_listener::Order::After,
                Context::Key(requester.key.clone()),
            );
        }
        let mut self_signalled = false;
        #[cfg(test)]
        for dep in pending.iter() {
            notify_graph_listener!(
                self.ev.graph,
                requester.key,
                Signal,
                Before,
                Context::Key(dep.key.clone())
            );
        }
        {
            let mut state = requester.node.lock();
            for (dep, version) in pending.iter().zip(versions) {
                if let Some(version) = version {
                    self_signalled = state.entry.signal_dep(version, Some(&dep.key));
                }
            }
        }
        #[cfg(test)]
        for dep in pending.iter() {
            notify_graph_listener!(
                self.ev.graph,
                requester.key,
                Signal,
                After,
                Context::Key(dep.key.clone())
            );
        }
        if !undone.is_empty() {
            // Java: DEP_NOT_DONE. The node is not committed; a dep that needs scheduling is
            // enqueued, and the node runs again when its undone deps signal it.
            // PORT: blocking engine: this thread evaluates (or waits for) each undone dep now,
            // which signals the node when it is done, and the node's thread runs its function
            // again (`NodeState::restart_requested`).
            requester.node.lock().restart_requested = true;
            for (dep, dependency_state) in undone {
                self.eval_undone_dep(&dep.key, &dep.node, dependency_state, requester.key);
            }
            return false;
        }
        self_signalled
    }

    /// Evaluates or waits for `key`, which the node `requester` was just registered with and
    /// which answered `dependency_state` (`NEEDS_SCHEDULING` or `ALREADY_EVALUATING`): the
    /// steps of [`Run::eval_dep`] after the registration.
    // Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:1115 AbstractParallelEvaluator.maybeHandleUndoneDepForDoneEntry
    fn eval_undone_dep(
        &self,
        key: &SkyKey,
        node: &Arc<Node>,
        dependency_state: DependencyState,
        requester: &SkyKey,
    ) -> Arc<DoneEntry> {
        let mut state = node.lock();
        match dependency_state {
            DependencyState::NeedsScheduling => {
                state.owner = Some(std::thread::current().id());
                state.outcome = None;
                state.cycle_rdeps.clear();
                drop(state);
                // Java: evaluatorContext.getVisitor().enqueueEvaluation(depKey, null).
                self.enqueue_evaluation(key);
                if DEPTH.with(Cell::get) >= self.ctx.max_inline_depth {
                    self.compute_on_fresh_thread(key, node)
                } else {
                    self.compute_claimed(key, node)
                }
            }
            DependencyState::AlreadyEvaluating => {
                if let Some(outcome) = state.outcome.clone() {
                    return outcome;
                }
                check_state!(
                    state.owner.is_some(),
                    "node in flight from an earlier evaluation: {:?} {:?}",
                    key,
                    state.entry
                );
                drop(state);
                self.wait_for(key, node, Some(requester))
            }
            DependencyState::Done => unreachable!("an undone dep"),
        }
    }

    /// Removes from the node the deps the function requested that were not done when it read
    /// them (Java: `SkyFunctionEnvironment.removeUndoneNewlyRequestedDeps`).
    ///
    /// PORT: Java never registered such a dep (it removes it before registering); a request here
    /// registered it before it waited (see [`Environment`]'s requests), so the registration is
    /// taken back: the dep leaves the node's temporary direct deps, and the node leaves the dep's
    /// reverse deps (for a dep of the last build, `commitAndGetParents` does that, as in Java).
    /// A dep that is done by now (the evaluation is aborting) signalled the node and stays.
    ///
    /// Returns the deps taken back (for [`Run::never_evaluated`], if the node commits).
    // Java: com/google/devtools/build/skyframe/SkyFunctionEnvironment.java:853 SkyFunctionEnvironment.removeUndoneNewlyRequestedDeps
    fn remove_undone_newly_requested_deps(
        &self,
        sky_key: &SkyKey,
        node: &Arc<Node>,
        st: &mut EnvState,
    ) -> HashSet<SkyKey> {
        let undone: Vec<SkyKey> = {
            let requested = st.requested.lock();
            st.deps
                .get_all_elements_as_iterable()
                .filter(|k| requested.get(*k).is_some_and(|e| e.is_undone_view()))
                .cloned()
                .collect()
        };
        if undone.is_empty() {
            return HashSet::new();
        }
        let mut removed: HashSet<SkyKey> = HashSet::new();
        for dep in &undone {
            let Some(dep_node) = self.ev.graph.get_if_present(dep) else {
                continue;
            };
            let mut dep_state = dep_node.lock();
            if dep_state.entry.is_done() {
                continue;
            }
            if !st.old_deps.contains(dep) {
                dep_state.entry.remove_reverse_dep(sky_key.clone());
            }
            removed.insert(dep.clone());
        }
        node.lock().entry.remove_unfinished_deps(&removed);
        st.deps.remove(&removed);
        removed
    }

    // Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:966 AbstractParallelEvaluator.maybeHandleRegisteringNewlyDiscoveredDepsForDoneEntry
    ///
    /// PORT: Java's first step, `if (!keepGoing) env.removeUndoneNewlyRequestedDeps()`, is
    /// [`Run::remove_undone_newly_requested_deps`], called by the caller first.
    fn maybe_handle_registering_newly_discovered_deps_for_done_entry(
        &self,
        sky_key: &SkyKey,
        node: &Arc<Node>,
        st: &mut EnvState,
    ) -> bool {
        if st.pending.is_empty() {
            return false;
        }
        let requester = Requester {
            key: sky_key,
            node,
            old_deps: &st.old_deps,
        };
        let self_signalled =
            self.register_pending_deps(&requester, &mut st.pending, &mut st.pending_group_sizes);
        let dirty_dep_found = node.lock().restart_requested;
        check_state!(
            self_signalled || dirty_dep_found,
            "{:?} {:?}",
            sky_key,
            node.lock().entry
        );
        !self_signalled
    }

    /// Decides how a node finishes from what its function returned and what it saw of its deps.
    /// Ports the part of `AbstractParallelEvaluator.Evaluate.run` after `compute` returns.
    ///
    /// PORT: where Java enqueues the not-done deps and restarts the function when they are done,
    /// the deps here were computed by the requests themselves. A dep that is still not done
    /// afterwards (the evaluation is aborting, it is in a cycle, it was interrupted) never will
    /// be in this evaluation, and neither will this node: it ends "undone", in flight with the
    /// deps it registered, which is the in-flight `NodeEntry` Java leaves behind for error
    /// bubbling, cycle detection and deletion at the next evaluation.
    // Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:473 Evaluate.run
    fn resolve(
        &self,
        key: &SkyKey,
        node: &Arc<Node>,
        env: Environment<'_>,
        r: ComputeResult,
    ) -> Arc<DoneEntry> {
        let keep_going = self.ctx.keep_going;
        let (mut st, values_missing, listener) = env.into_parts();
        let ret = match r {
            Ok(Some(v)) => Ret::Value(v),
            Ok(None) => Ret::Null,
            Err(ComputeError::Exception(e)) => Ret::Threw(e),
            Err(ComputeError::Interrupted(_)) => {
                if self.ev.interrupted.load(Ordering::SeqCst)
                    || !self.aborting.load(Ordering::SeqCst)
                {
                    self.saw_interrupt.store(true, Ordering::SeqCst);
                    return Arc::new(DoneEntry::undone(DoneKind::Interrupted));
                }
                Ret::Abandoned
            }
        };
        if st.interrupted_dep {
            self.saw_interrupt.store(true, Ordering::SeqCst);
            return Arc::new(DoneEntry::undone(DoneKind::Interrupted));
        }

        // In keep-going mode, we do not let SkyFunctions complete with a thrown error if they
        // have missing deps. Instead, we wait until their deps are done and restart the
        // SkyFunction, so we can have a definitive error and definitive graph structure, thus
        // avoiding non-determinism. It's completely reasonable for SkyFunctions to throw eagerly
        // because they do not know if they are in keep-going mode.
        if let Ret::Threw(builder_exception) = &ret
            && (!keep_going || !values_missing)
        {
            let should_fail_fast = !keep_going || builder_exception.is_catastrophic();
            if should_fail_fast && self.aborting.load(Ordering::SeqCst) {
                // PORT: not the first error. Java registers the function's new deps
                // (maybeHandleRegisteringNewlyDiscoveredDepsForDoneEntry, which first drops those
                // that were not done) and then finds that the evaluation already failed. Here
                // the deps that were not done were registered by the requests that waited for
                // them, and the registrations stay: in Java this function would have thrown
                // before such a dep was evaluated, and been the first error; error bubbling goes
                // through this node instead (`Run::bubble_error_up`). The done deps are
                // registered as in Java.
                let requester = Requester {
                    key,
                    node,
                    old_deps: &st.old_deps,
                };
                self.register_pending_deps(
                    &requester,
                    &mut st.pending,
                    &mut st.pending_group_sizes,
                );
                return Arc::new(DoneEntry::undone(DoneKind::Aborted));
            }
            // PORT: `nodeEntry.hasUnsignaledDeps()` is only true with partial reevaluation.
            let taken_back = if !keep_going && values_missing {
                self.remove_undone_newly_requested_deps(key, node, &mut st)
            } else {
                HashSet::new()
            };
            if self
                .maybe_handle_registering_newly_discovered_deps_for_done_entry(key, node, &mut st)
            {
                // A newly requested dep transitioned from done to dirty before this node
                // finished. (Not reached: nothing dirties a node during an evaluation.)
                return Arc::new(DoneEntry::undone(DoneKind::Aborted));
            }
            if should_fail_fast {
                // After we commit this error to the graph but before the doMutatingEvaluation
                // call completes with the error there is a race-like opportunity for the error to
                // be used, either by an in-flight computation or by a future computation.
                if !self.prevent_new_evaluations() {
                    // This is not the first error encountered, so we ignore it so that we can
                    // terminate with the first error.
                    return Arc::new(DoneEntry::undone(DoneKind::Aborted));
                }
            }
            self.undone_deps_of_committed.lock().extend(taken_back);
            // Java: isAnyDirectDepErrorTransitivelyTransient ||
            // isAnyNewlyRequestedDepErrorTransitivelyTransient
            let is_transitively_transient = builder_exception.is_transient()
                || st.requested.lock().values().any(|e| {
                    e.error
                        .as_ref()
                        .is_some_and(|e| e.is_transitively_transient())
                });
            let error_info =
                ErrorInfo::from_exception(builder_exception, is_transitively_transient);
            // The failure is recorded before any waiter can see the node done
            // (`entry_seen_in_normal_evaluation`). Lock order: failure, then nodes.
            let failure = should_fail_fast.then(|| self.failure.lock());
            self.set_error(key, node, &error_info, &st.old_deps);
            let events = listener.report_events_for_commit(node, &st.requested);
            let (rdeps_to_bubble_up_to, snapshot) = self.commit_and_get_parents(
                key,
                node,
                None,
                Some(error_info),
                &[],
                events,
                &st.old_deps,
            );
            if let Some(mut failure) = failure {
                self.signal_parents_on_abort(key, &rdeps_to_bubble_up_to, snapshot.version);
                // PORT: without the requesters whose request closed a cycle through this node.
                // They are below it: in Java they would have been evaluated only after this
                // node's function returned null, and then it could not have committed. (Reached
                // by a function that throws although a dep is missing; Java commits that error
                // without ever evaluating what is below.)
                let cycle_rdeps = node.lock().cycle_rdeps.clone();
                failure.get_or_insert_with(|| SchedulerError {
                    error_info: snapshot.error.clone().expect("fail fast with an error"),
                    failed_value: key.clone(),
                    rdeps_to_bubble_up_to: rdeps_to_bubble_up_to
                        .iter()
                        .filter(|k| !cycle_rdeps.contains(k))
                        .cloned()
                        .collect(),
                });
                drop(failure);
                self.stop_in_flight_jobs();
                return snapshot;
            }
            self.signal_parents_and_enqueue_if_ready(key, &rdeps_to_bubble_up_to, snapshot.version);
            return snapshot;
        }

        if let Ret::Value(value) = ret {
            // PORT: `nodeEntry.hasUnsignaledDeps()` is only true with partial reevaluation.
            check_state!(
                !values_missing,
                "Evaluation of {:?} returned non-null value but requested dependencies that \
                 weren't computed yet (one of {:?}), NodeEntry: {:?}",
                key,
                st.deps,
                node.lock().entry
            );
            let _state = StateGuard::new(self.progress(), key, NodeState::Commit);
            // We don't expect any unfinished deps in a keep-going build. (Values are not missing
            // here, checked above.)
            if self
                .maybe_handle_registering_newly_discovered_deps_for_done_entry(key, node, &mut st)
            {
                // A newly requested dep transitioned from done to dirty before this node
                // finished. This node will be signalled again, and so we should return.
                return Arc::new(DoneEntry::undone(DoneKind::Aborted));
            }
            let events = listener.report_events_for_commit(node, &st.requested);
            let (reverse_deps, snapshot) = self.commit_and_get_parents(
                key,
                node,
                Some(value),
                None,
                &st.child_error_infos,
                events,
                &st.old_deps,
            );
            self.signal_parents_and_enqueue_if_ready(key, &reverse_deps, snapshot.version);
            return snapshot;
        }

        // The function stopped with `Interrupted` because the evaluation is aborting. Java: the
        // InterruptedException leaves the node as it was: nothing is committed, nothing is
        // registered with its deps (SchedulerException.ofInterruption).
        if matches!(ret, Ret::Abandoned) {
            return Arc::new(DoneEntry::undone(DoneKind::Aborted));
        }
        // From here on the function returned null (in keep_going also: threw with deps missing).

        if let Some((child_error_key, child_error_info)) = st.dep_error.clone() {
            check_state!(
                !keep_going,
                "{:?} {:?} {:?}",
                key,
                node.lock().entry,
                child_error_key
            );
            // We encountered a child error in noKeepGoing mode, so we want to fail fast. But we
            // first need to add the edge between the current node and the child error it
            // requested so that error bubbling can occur. Note that this edge will subsequently
            // be removed during graph cleaning (since the current node will never be committed to
            // the graph).
            if let Some(i) = st.pending.iter().position(|p| p.key == child_error_key) {
                // Add this dep if it was just requested. In certain rare race conditions (see
                // MemoizingEvaluatorTest.cachedErrorCausesRestart) this dep may have already been
                // requested.
                let child_error_entry = st.pending[i].node.clone();
                node.lock()
                    .entry
                    .add_singleton_temporary_direct_dep(child_error_key.clone());
                #[cfg(test)]
                let listened_call = if st.old_deps.contains(&child_error_key) {
                    crate::graph_listener::EventType::CheckIfDone
                } else {
                    crate::graph_listener::EventType::AddReverseDep
                };
                #[cfg(test)]
                self.ev.graph.listeners_for_testing.notify(
                    &child_error_key,
                    listened_call,
                    crate::graph_listener::Order::Before,
                    Context::Key(key.clone()),
                );
                let child_error_state = {
                    let mut state = child_error_entry.lock();
                    if st.old_deps.contains(&child_error_key) {
                        state.entry.check_if_done_for_dirty_reverse_dep(key.clone())
                    } else {
                        state
                            .entry
                            .add_reverse_dep_and_check_if_done(Some(key.clone()))
                    }
                };
                #[cfg(test)]
                self.ev.graph.listeners_for_testing.notify(
                    &child_error_key,
                    listened_call,
                    crate::graph_listener::Order::After,
                    Context::Key(key.clone()),
                );
                if child_error_state != DependencyState::Done {
                    // The child in error may have transitioned from done to dirty between when
                    // this node discovered the error and now. Java notifies the graph
                    // inconsistency receiver, THROWING by default.
                    panic!(
                        "IllegalStateException: Unexpected inconsistency: {:?}, [{:?}], \
                         BUILDING_PARENT_FOUND_UNDONE_CHILD",
                        key, child_error_key
                    );
                }
            }
            self.prevent_new_evaluations();
            // Java: throw SchedulerException.ofError(childErrorInfo, childErrorKey,
            // ImmutableSet.of(skyKey)); the visitor keeps the first exception thrown.
            self.failure.lock().get_or_insert_with(|| SchedulerError {
                error_info: child_error_info.clone(),
                failed_value: child_error_key.clone(),
                rdeps_to_bubble_up_to: vec![key.clone()],
            });
            self.stop_in_flight_jobs();
            return Arc::new(DoneEntry::awaiting_deps(DoneKind::Aborted));
        }

        // Add all the newly requested dependencies to the temporary direct deps. Note that
        // newDirectDeps does not contain any elements in common with the already existing
        // temporary direct deps.
        {
            let requester = Requester {
                key,
                node,
                old_deps: &st.old_deps,
            };
            self.register_pending_deps(&requester, &mut st.pending, &mut st.pending_group_sizes);
        }
        if node.lock().restart_requested {
            // A dep found undone (rewound) when registered: the node runs again
            // (`evaluate_node`).
            return Arc::new(DoneEntry::undone(DoneKind::Aborted));
        }

        // Java enqueues the deps that are not done and returns; the node is restarted when they
        // are. Here a dep that is not done after it was requested never will be.
        if st.aborted_dep || st.in_cycle_dep {
            let kind = if st.aborted_dep {
                DoneKind::Aborted
            } else {
                DoneKind::InCycle
            };
            return Arc::new(DoneEntry::awaiting_deps(kind));
        }

        // PORT: every dep the function requested is done; Java restarts the function, which
        // requests no new dep and returns null again (`newDeps.isEmpty()`). The blocking engine
        // does not run it again.
        // If there were no newly requested dependencies, then at least one of them was in error
        // or there is a bug in the SkyFunction implementation. The environment has collected its
        // errors, so we just order it to be built.
        check_state!(
            !st.child_error_infos.is_empty(),
            "Evaluation of SkyKey failed and no dependencies were requested: {:?} {:?}",
            key,
            node.lock().entry
        );
        // If the child error was catastrophic, committing this parent to the graph is not
        // necessary, but since we don't do error bubbling in catastrophes, it doesn't violate any
        // invariants either.
        let events = listener.report_events_for_commit(node, &st.requested);
        let (reverse_deps, snapshot) = self.commit_and_get_parents(
            key,
            node,
            None,
            None,
            &st.child_error_infos,
            events,
            &st.old_deps,
        );
        self.signal_parents_and_enqueue_if_ready(key, &reverse_deps, snapshot.version);
        snapshot
    }

    /// Set this node to be in error. The node's value must not have already been set. However,
    /// all dependencies of this node *must* already have been registered, since this method may
    /// register a dependence on the error transience node, which should always be the last dep.
    // Java: com/google/devtools/build/skyframe/SkyFunctionEnvironment.java:423 SkyFunctionEnvironment.setError
    pub(crate) fn set_error(
        &self,
        sky_key: &SkyKey,
        node: &Arc<Node>,
        error_info: &ErrorInfo,
        old_deps: &IndexSet<SkyKey>,
    ) {
        if error_info.is_directly_transient() {
            let error_transience_node = check_not_null!(
                self.ev.graph.get_if_present(&ERROR_TRANSIENCE_KEY),
                "Null error value? {:?}",
                sky_key
            );
            #[cfg(test)]
            let listened_call = if old_deps.contains(&*ERROR_TRANSIENCE_KEY) {
                crate::graph_listener::EventType::CheckIfDone
            } else {
                crate::graph_listener::EventType::AddReverseDep
            };
            #[cfg(test)]
            self.ev.graph.listeners_for_testing.notify(
                &ERROR_TRANSIENCE_KEY,
                listened_call,
                crate::graph_listener::Order::Before,
                Context::Key(sky_key.clone()),
            );
            let tri_state = {
                let mut state = error_transience_node.lock();
                if old_deps.contains(&*ERROR_TRANSIENCE_KEY) {
                    state
                        .entry
                        .check_if_done_for_dirty_reverse_dep(sky_key.clone())
                } else {
                    state
                        .entry
                        .add_reverse_dep_and_check_if_done(Some(sky_key.clone()))
                }
            };
            #[cfg(test)]
            self.ev.graph.listeners_for_testing.notify(
                &ERROR_TRANSIENCE_KEY,
                listened_call,
                crate::graph_listener::Order::After,
                Context::Key(sky_key.clone()),
            );
            check_state!(
                tri_state == DependencyState::Done,
                "{:?} {:?} {:?}",
                sky_key,
                tri_state,
                error_info
            );
            notify_graph_listener!(
                self.ev.graph,
                sky_key,
                Signal,
                Before,
                Context::Key(ERROR_TRANSIENCE_KEY.clone())
            );
            let mut state = node.lock();
            state
                .entry
                .add_singleton_temporary_direct_dep(ERROR_TRANSIENCE_KEY.clone());
            state
                .entry
                .signal_dep(self.graph_version, Some(&ERROR_TRANSIENCE_KEY));
            drop(state);
            notify_graph_listener!(
                self.ev.graph,
                sky_key,
                Signal,
                After,
                Context::Key(ERROR_TRANSIENCE_KEY.clone())
            );
        }
    }

    /// Applies the change to the graph (mostly) atomically and returns parents to potentially
    /// signal and enqueue, with the node's new snapshot.
    ///
    /// `error_info` is the error set with `setError`, if any; `events` is what
    /// `reportEventsAndGetEventsToStore` returned (the caller calls it first: it reads only the
    /// environment and the node's deps, as `getErrorInfoToUse` does).
    // Java: com/google/devtools/build/skyframe/SkyFunctionEnvironment.java:916 SkyFunctionEnvironment.commitAndGetParents
    pub(crate) fn commit_and_get_parents(
        &self,
        sky_key: &SkyKey,
        node: &Arc<Node>,
        value: Option<SkyValue>,
        error_info: Option<ErrorInfo>,
        child_error_infos: &[Arc<ErrorInfo>],
        events: NestedSet<StoredReportable>,
        old_deps: &IndexSet<SkyKey>,
    ) -> (IndexSet<SkyKey>, Arc<DoneEntry>) {
        // Construct the definitive error info, if there is one.
        let error_info = match error_info {
            Some(e) => Some(e),
            // Java: com/google/devtools/build/skyframe/ErrorInfoManager.java:55 UseChildErrorInfoIfNecessary.getErrorInfoToUse
            None if child_error_infos.is_empty() => None,
            None => {
                let error_info =
                    ErrorInfo::from_child_errors(sky_key, child_error_infos.iter().map(|e| &**e));
                Some(if value.is_some() {
                    ErrorInfo::with_value(&error_info)
                } else {
                    error_info
                })
            }
        };

        // We have the following implications:
        // errorInfo == null => value != null => enqueueParents.
        // All these implications are strict:
        // (1) errorInfo != null && value != null happens for values with recoverable errors.
        // (2) value == null && enqueueParents happens for values that are found to have errors
        // during a --keep_going build.
        let value_with_metadata = match &value {
            None => {
                let error_info = check_not_null!(error_info.clone(), "{:?}", sky_key);
                ValueWithMetadata::error(error_info, events)
            }
            Some(v) => ValueWithMetadata::normal(Some(v.clone()), error_info.clone(), events),
        };
        let (deps_to_remove, deps_for_receiver) = {
            let mut state = node.lock();
            let temporary_direct_deps = state.entry.get_temporary_direct_deps();
            let deps_for_receiver = self
                .ev
                .has_user_progress_receiver
                .then(|| temporary_direct_deps.clone());
            // `old_deps` is the union of the old deps and the reset deps (getResetDirectDeps),
            // taken where the node starts evaluating (see `evaluate_node`).
            let deps_to_remove: Vec<SkyKey> = if old_deps.is_empty() {
                vec![]
            } else {
                // Remove the rdep on this entry for each of 1) its old deps from a prior
                // evaluation that are no longer direct deps and 2) reset deps that were not
                // requested again post-restart.
                let temporary_set = temporary_direct_deps.to_set();
                old_deps
                    .iter()
                    .filter(|k| !temporary_set.contains(*k))
                    .cloned()
                    .collect()
            };
            (deps_to_remove, deps_for_receiver)
        };
        for key in &deps_to_remove {
            let old_dep_entry = check_not_null!(self.ev.graph.get_if_present(key), "{:?}", key);
            notify_graph_listener!(
                self.ev.graph,
                key,
                RemoveReverseDep,
                Before,
                Context::Key(sky_key.clone())
            );
            old_dep_entry
                .lock()
                .entry
                .remove_reverse_dep(sky_key.clone());
            notify_graph_listener!(
                self.ev.graph,
                key,
                RemoveReverseDep,
                After,
                Context::Key(sky_key.clone())
            );
        }

        // If this entry is dirty, setValue may not actually change it, if it determines that the
        // data being written now is the same as the data already present in the entry. We detect
        // this case by comparing versions before and after setting the value.
        notify_graph_listener!(
            self.ev.graph,
            sky_key,
            SetValue,
            Before,
            Context::Value(value.clone())
        );
        let (reverse_deps, snapshot, changed) = {
            let mut state = node.lock();
            let previous_version = state.entry.get_version();
            let reverse_deps = state
                .entry
                .set_value(value_with_metadata, self.graph_version, None);
            let current_version = state.entry.get_version();
            state.refresh_snapshot();
            (
                reverse_deps,
                state
                    .snapshot
                    .clone()
                    .expect("set_value made the node done"),
                current_version != previous_version,
            )
        };
        notify_graph_listener!(
            self.ev.graph,
            sky_key,
            SetValue,
            After,
            Context::Value(value.clone())
        );
        #[cfg(test)]
        let reverse_deps = self.ev.deterministic_rdeps(reverse_deps);

        // Tell the receiver that this value was built. If currentVersion.equals(evaluationVersion),
        // it was evaluated this run, and so was changed. Otherwise, it is less than
        // evaluationVersion, by the Preconditions check above, and was not actually changed this
        // run -- when it was written above, its version stayed below this update's version, so
        // its value remains the same.
        self.progress().evaluated(
            sky_key,
            EvaluationState::get(value.as_ref(), changed),
            if changed { value.as_ref() } else { None },
            if changed { error_info.as_ref() } else { None },
            // PORT: the deps are only copied for a receiver the caller gave; the receiver the
            // evaluator wraps by default ignores them.
            Some(deps_for_receiver.as_ref().unwrap_or(&GroupedDeps::new())),
        );

        (reverse_deps, snapshot)
    }

    /// Java: `ParallelEvaluatorContext.signalParentsAndEnqueueIfReady`.
    ///
    /// PORT: the parents are blocked in their requests for this node and wake up when its
    /// outcome is published (after this), which is Java's re-enqueueing.
    // Java: com/google/devtools/build/skyframe/ParallelEvaluatorContext.java:120 ParallelEvaluatorContext.signalParentsAndEnqueueIfReady
    fn signal_parents_and_enqueue_if_ready(
        &self,
        sky_key: &SkyKey,
        parents: &IndexSet<SkyKey>,
        version: Version,
    ) {
        for parent in parents {
            let entry = check_not_null!(self.ev.graph.get_if_present(parent), "{:?}", parent);
            notify_graph_listener!(
                self.ev.graph,
                parent,
                Signal,
                Before,
                Context::Key(sky_key.clone())
            );
            entry.lock().entry.signal_dep(version, Some(sky_key));
            notify_graph_listener!(
                self.ev.graph,
                parent,
                Signal,
                After,
                Context::Key(sky_key.clone())
            );
        }
    }

    // Java: com/google/devtools/build/skyframe/ParallelEvaluatorContext.java:108 ParallelEvaluatorContext.signalParentsOnAbort
    pub(crate) fn signal_parents_on_abort(
        &self,
        sky_key: &SkyKey,
        parents: &IndexSet<SkyKey>,
        version: Version,
    ) {
        for parent in parents {
            let entry = check_not_null!(self.ev.graph.get_if_present(parent), "{:?}", parent);
            let mut state = entry.lock();
            if !state.entry.is_done() {
                // In cycles, we can have parents that are already done.
                // Test hook: the BEFORE event is emitted under the parent's lock here (the
                // doneness check and the signal are one step).
                notify_graph_listener!(
                    self.ev.graph,
                    parent,
                    Signal,
                    Before,
                    Context::Key(sky_key.clone())
                );
                state.entry.signal_dep(version, Some(sky_key));
                drop(state);
                notify_graph_listener!(
                    self.ev.graph,
                    parent,
                    Signal,
                    After,
                    Context::Key(sky_key.clone())
                );
            }
        }
    }

    /// Java: `replay(transitiveEvents)`: reports the events not reported yet.
    // Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:833 AbstractParallelEvaluator.replay
    pub(crate) fn replay(&self, transitive_events: &NestedSet<StoredReportable>) {
        // Replaying actions is done on a small number of nodes, but potentially over a large
        // dependency graph. Under those conditions, using the regular NestedSet flattening with
        // .toList() is more efficient than using NestedSetVisitor's custom traversal logic.
        if transitive_events.is_empty() {
            return;
        }
        let reporter = &*self.ctx.event_handler;
        let list = transitive_events.to_list();
        NestedSetVisitor::new(
            |e: &StoredReportable| report_stored(e, reporter),
            &self.ev.emitted_event_state,
        )
        .visit_collection(list.iter());
    }

    /// Java: `getReplayingNestedSetEventVisitor().visit(events)`.
    // Java: com/google/devtools/build/skyframe/ParallelEvaluatorContext.java:91 ParallelEvaluatorContext.replayingNestedSetEventVisitor
    pub(crate) fn replay_visit(&self, events: &NestedSet<StoredReportable>) {
        let reporter = &*self.ctx.event_handler;
        NestedSetVisitor::new(
            |e: &StoredReportable| report_stored(e, reporter),
            &self.ev.emitted_event_state,
        )
        .visit(events);
    }

    /// Blocks until `node` has an outcome in this evaluation, detecting cycles first. Releases
    /// the permit while blocked. Returns an "in cycle" entry when waiting would close a cycle.
    fn wait_for(
        &self,
        key: &SkyKey,
        node: &Arc<Node>,
        requester: Option<&SkyKey>,
    ) -> Arc<DoneEntry> {
        let me = std::thread::current().id();
        {
            let mut w = self.ev.waits.lock();
            let frames = w.stacks.get(&me).map_or(0, Vec::len);
            w.waiting
                .entry(me)
                .or_default()
                .push((frames, vec![key.clone()]));
            if self.find_cycle(&w, me) {
                if let Some(v) = w.waiting.get_mut(&me) {
                    v.pop();
                }
                drop(w);
                let mut state = node.lock();
                if let Some(outcome) = &state.outcome {
                    return outcome.clone();
                }
                if !state.entry.is_done() {
                    if let Some(requester) = requester {
                        state.cycle_rdeps.push(requester.clone());
                    }
                    return Arc::new(DoneEntry::undone(DoneKind::InCycle));
                }
                // Done, its owner is signalling its parents: wait for the outcome.
                drop(state);
                let mut w = self.ev.waits.lock();
                let frames = w.stacks.get(&me).map_or(0, Vec::len);
                w.waiting
                    .entry(me)
                    .or_default()
                    .push((frames, vec![key.clone()]));
            }
        }
        self.permits.release();
        let result = node.wait_for_outcome();
        self.permits.acquire();
        if let Some(v) = self.ev.waits.lock().waiting.get_mut(&me) {
            v.pop();
        }
        result
    }

    /// Walks the wait-for graph from `me`. Returns true if the walk returns to `me`.
    fn find_cycle(&self, w: &WaitGraph, me: ThreadId) -> bool {
        // Walk (thread, stack index) states: "the key at this index of this thread's stack
        // cannot finish until the thread's waits that block that index finish".
        fn owner_and_index(ev: &Evaluator, w: &WaitGraph, k: &SkyKey) -> Option<(ThreadId, usize)> {
            let node = ev.graph.get_if_present(k)?;
            let owner = node.lock().owner?;
            let index = w.stacks.get(&owner)?.iter().position(|s| s == k)?;
            Some((owner, index))
        }
        fn dfs(
            ev: &Evaluator,
            w: &WaitGraph,
            t: ThreadId,
            index: usize,
            me: ThreadId,
            seen: &mut HashSet<SkyKey>,
        ) -> bool {
            let Some(waits) = w.waiting.get(&t) else {
                return false;
            };
            for (frames, keys) in waits {
                if *frames <= index {
                    continue; // made by a frame pushed after `index`: does not block it
                }
                for k in keys {
                    let Some((owner, j)) = owner_and_index(ev, w, k) else {
                        continue;
                    };
                    if owner == me {
                        return true;
                    }
                    if seen.insert(k.clone()) && dfs(ev, w, owner, j, me, seen) {
                        return true;
                    }
                }
            }
            false
        }
        let Some(my_stack) = w.stacks.get(&me).filter(|s| !s.is_empty()) else {
            return false; // nothing on this thread can be awaited by others
        };
        dfs(self.ev, w, me, my_stack.len() - 1, me, &mut HashSet::new())
    }

    /// Computes several keys, spreading unstarted ones over helper threads. `requester` as in
    /// [`Run::eval_dep`].
    pub(crate) fn eval_keys(
        &self,
        keys: &[SkyKey],
        requester: Option<&Requester<'_>>,
    ) -> Vec<Requested> {
        if keys.len() <= 1 || self.parallelism() <= 1 {
            return keys.iter().map(|k| self.eval_dep(k, requester)).collect();
        }
        // Already-done keys need no thread.
        let mut results: Vec<Option<Requested>> = keys.iter().map(|_| None).collect();
        let pending: Vec<usize> = (0..keys.len())
            .filter(|&i| !self.ev.is_done(&keys[i]))
            .collect();
        let helper_count = if pending.len() <= 1 {
            0
        } else {
            self.ev.reserve_threads(
                (pending.len() - 1).min(self.parallelism()),
                self.ctx.max_threads,
            )
        };
        if helper_count == 0 {
            // One key, or the thread cap is reached: evaluate inline, in order.
            for (i, r) in results.iter_mut().enumerate() {
                *r = Some(self.eval_dep(&keys[i], requester));
            }
            return results.into_iter().map(|r| r.expect("filled")).collect();
        }
        // Done keys are registered (and signal the requester) on this thread, in order.
        for (i, r) in results.iter_mut().enumerate() {
            if !pending.contains(&i) {
                *r = Some(self.eval_dep(&keys[i], requester));
            }
        }
        // The current thread computes the first pending key itself; helpers and the current
        // thread then take the rest from a queue.
        let me = std::thread::current().id();
        let rest: Vec<SkyKey> = pending[1..].iter().map(|&i| keys[i].clone()).collect();
        let queue = Mutex::new(rest.iter().cloned().enumerate().collect::<VecDeque<_>>());
        let rest_results: Vec<Mutex<Option<Requested>>> =
            rest.iter().map(|_| Mutex::new(None)).collect();
        // The parent depends on every key of the batch from now on: record that before any
        // helper starts, so a helper that asks for a key on this thread's stack sees the cycle.
        // Recording the batch can itself close a cycle (a batch key already in progress that
        // waits on this thread, or is on its stack); a helper waiting on such a key could not
        // see it. Then evaluate the batch inline, where each wait detects its own cycle.
        let closes_cycle = {
            let mut w = self.ev.waits.lock();
            let frames = w.stacks.get(&me).map_or(0, Vec::len);
            w.waiting
                .entry(me)
                .or_default()
                .push((frames, rest.clone()));
            let found = self.find_cycle(&w, me);
            if found && let Some(v) = w.waiting.get_mut(&me) {
                v.pop();
            }
            found
        };
        if closes_cycle {
            self.ev.release_threads(helper_count);
            for i in pending {
                results[i] = Some(self.eval_dep(&keys[i], requester));
            }
            return results.into_iter().map(|r| r.expect("filled")).collect();
        }
        let drain = |queue: &Mutex<VecDeque<(usize, SkyKey)>>| {
            loop {
                let Some((i, k)) = queue.lock().pop_front() else {
                    break;
                };
                let r = self.eval_dep(&k, requester);
                *rest_results[i].lock() = Some(r);
            }
        };
        std::thread::scope(|scope| {
            let mut handles = vec![];
            for _ in 0..helper_count {
                let queue = &queue;
                let drain = &drain;
                let spawned = self.spawn(scope, move || {
                    self.permits.acquire();
                    drain(queue);
                    self.ev.unregister_thread();
                    self.permits.release();
                });
                match spawned {
                    Ok(h) => handles.push(h),
                    // No more threads to be had: this thread drains what the helpers that did
                    // start leave in the queue.
                    Err(_) => break,
                }
            }
            let first = pending[0];
            results[first] = Some(self.eval_dep(&keys[first], requester));
            drain(&queue);
            // Give back the permit while the helpers finish.
            self.permits.release();
            for h in handles {
                if let Err(p) = h.join() {
                    resume_unwind(p);
                }
            }
        });
        self.ev.release_threads(helper_count);
        self.permits.acquire();
        if let Some(v) = self.ev.waits.lock().waiting.get_mut(&me) {
            v.pop();
        }
        for (j, &i) in pending[1..].iter().enumerate() {
            results[i] = rest_results[j].lock().take();
        }
        results.into_iter().map(|r| r.expect("filled")).collect()
    }

    /// Runs [`Run::bubble_error_up`] on a thread with the evaluator's stack size (functions are
    /// re-run there).
    fn bubble_error_up_on_evaluator_thread(
        &self,
        e: &SchedulerError,
        roots: &[SkyKey],
    ) -> Option<BubbleErrorInfo> {
        self.ev.force_reserve_threads(1);
        let r = std::thread::scope(|scope| {
            match self.spawn(scope, || {
                catch_unwind(AssertUnwindSafe(|| self.bubble_error_up(e, roots)))
            }) {
                Ok(h) => match h.join().unwrap_or_else(|p| resume_unwind(p)) {
                    Ok(bubble_error_info) => bubble_error_info,
                    Err(payload) => {
                        self.note_crash(payload);
                        None
                    }
                },
                Err(err) => {
                    self.note_crash(Box::new(format!(
                        "OutOfMemoryError: unable to create evaluator thread for error bubbling: \
                         {err}"
                    )));
                    None
                }
            }
        });
        self.ev.release_threads(1);
        r
    }

    /// Walk up graph to find a top-level node (without parents) that wanted this failure. Store
    /// the failed nodes along the way in a map, with ErrorInfos that are appropriate for that
    /// layer. Example:
    ///
    /// ```text
    ///                      foo   bar
    ///                        \   /
    ///           unrequested   baz
    ///                     \    |
    ///                      failed-node
    /// ```
    ///
    /// User requests foo, bar. When failed-node fails, we look at its parents. unrequested is not
    /// in-flight, so we replace failed-node by baz and repeat. We look at baz's parents. foo is
    /// in-flight, so we replace baz by foo. Since foo is a top-level node and doesn't have
    /// parents, we then break, since we know a top-level node, foo, that depended on the failed
    /// node.
    ///
    /// There's the potential for a weird "track jump" here in the case:
    ///
    /// ```text
    ///                        foo
    ///                       / \
    ///                   fail1 fail2
    /// ```
    ///
    /// If fail1 and fail2 fail simultaneously, fail2 may start propagating up in the loop below.
    /// However, foo requests fail1 first, and then throws an exception based on that. This is not
    /// incorrect, but may be unexpected.
    ///
    /// Returns a map of errors that have been constructed during the bubbling up, so that the
    /// appropriate error can be returned to the caller, even though that error was not written to
    /// the graph. If a cycle is detected during the bubbling, this method aborts and returns null
    /// so that the normal cycle detection can handle the cycle.
    ///
    /// Note that we are not propagating error to the first top-level node but to the highest one,
    /// because during this process we can add useful information about error from other nodes.
    ///
    /// Every node on this walk but the leaf node is not done, by the following argument: the leaf
    /// node is done, but the parents of it that we consider are in `rdepsToBubbleUpTo`. Each
    /// parent is either (1) a parent that requested the leaf node and found it to be in error,
    /// meaning it is not done, or (2) a parent that had registered a dependency on this leaf node
    /// before it finished building. In the second case, that parent would not have been enqueued,
    /// since we failed fast and prevented all new evaluations. Thus, we will only visit
    /// unfinished parents of the leaf node. For the inductive argument, the only parents we
    /// consider are those that were registered during this build (via
    /// `NodeEntry.getInProgressReverseDeps`). Since we don't allow a node to build with
    /// unfinished deps, those parents cannot have built.
    ///
    /// PORT: runs after every evaluator thread of the normal evaluation has finished, as in Java
    /// (`waitForCompletion`). In case (2) the parent was blocked in its dependency request, not
    /// waiting to be enqueued: its run resumed, found the dep missing and finished undone
    /// (`Run::resolve`), so it is unfinished here as well. The interrupt bit Java sets and clears
    /// around each re-run is per environment (`Environment::check_interrupted`), so there is
    /// nothing to clear or restore.
    // Java: com/google/devtools/build/skyframe/ParallelEvaluator.java:290 ParallelEvaluator.bubbleErrorUp
    fn bubble_error_up(&self, e: &SchedulerError, roots: &[SkyKey]) -> Option<BubbleErrorInfo> {
        let leaf_failure = &e.error_info;
        let rdeps_to_bubble_up_to = &e.rdeps_to_bubble_up_to;
        // PORT: stateCache.invalidateAll() skipped, SkyKeyComputeState is not ported.
        let root_values: HashSet<&SkyKey> = roots.iter().collect();
        let never_evaluated = self.never_evaluated(&root_values);
        let mut error: Arc<ErrorInfo> = leaf_failure.clone();
        let mut error_key = e.failed_value.clone();
        let mut bubble_error_info: BubbleErrorInfo = HashMap::new();
        let mut first_iteration = true;
        loop {
            let error_node = self
                .ev
                .node(&error_key)
                .unwrap_or_else(|| panic!("NullPointerException: {error_key:?}"));
            let (error_entry_done, error_entry_debug, error_entry_snapshot) = {
                let state = error_node.lock();
                (
                    state.entry.is_done(),
                    format!("{:?}", state.entry),
                    state.snapshot.clone(),
                )
            };
            let mut all_reverse_deps: Vec<SkyKey> = if error_entry_done {
                check_state!(
                    first_iteration,
                    "Non-leaf done node reached: {:?} {:?} {:?} {:?} {:?}",
                    error_key,
                    leaf_failure,
                    roots,
                    rdeps_to_bubble_up_to,
                    bubble_error_info
                );
                rdeps_to_bubble_up_to.clone()
            } else {
                check_state!(
                    !first_iteration,
                    "undone first iteration: {:?} {} {:?} {:?} {:?} {:?}",
                    error_key,
                    error_entry_debug,
                    leaf_failure,
                    roots,
                    rdeps_to_bubble_up_to,
                    bubble_error_info
                );
                error_node
                    .lock()
                    .entry
                    .get_in_progress_reverse_deps()
                    .into_iter()
                    .collect()
            };
            // PORT: nodes Java never evaluated registered nothing (`Run::never_evaluated`).
            all_reverse_deps.retain(|parent| !never_evaluated.contains(parent));
            // PORT: Java registers a node as a reverse dep when its function has returned null
            // (`enqueueChild`); a request registers it here, before its function has ended. Only
            // the requesters whose function did return null count: one that then threw (not the
            // first error: ignored) or was interrupted registered nothing in Java.
            #[allow(unused_mut)]
            let mut reverse_deps: Vec<SkyKey> = all_reverse_deps
                .iter()
                .filter(|parent| {
                    self.ev
                        .done_entry(parent)
                        .is_some_and(|e| !e.is_done() && e.awaits_deps)
                })
                .cloned()
                .collect();
            if reverse_deps.is_empty() && !root_values.contains(&error_key) {
                // PORT: a state Java does not reach. The only requesters of this unfinished node
                // threw after it failed under them (in Java a function that throws with a dep
                // missing throws before that dep is evaluated, and is itself the first error).
                // The error has to reach a top-level node: bubble through such a requester, which
                // may throw again in its error-bubbling run.
                reverse_deps = all_reverse_deps;
            }
            #[cfg(test)]
            if self.ev.deterministic_for_testing.load(Ordering::SeqCst) {
                // Java: DeterministicHelper.ALPHABETICAL_SKYKEY_COMPARATOR
                reverse_deps.sort_by_key(|k| (format!("{:?}", k.argument()), k.function_name().0));
            }
            first_iteration = false;
            // We should break from loop only when node doesn't have any parents.
            if reverse_deps.is_empty() {
                check_state!(
                    root_values.contains(&error_key),
                    "Current key {:?} has to be a top-level key: {:?}",
                    error_key,
                    root_values
                );
                if let Some(value_maybe_with_metadata) = &error_entry_snapshot {
                    self.replay(&value_maybe_with_metadata.get_events());
                }
                break;
            }
            let parent = reverse_deps[0].clone();
            if bubble_error_info.contains_key(&parent) {
                // Bubbled into a cycle. Don't try to bubble anything up. Cycle detection will
                // kick in.
                return None;
            }
            let parent_node = self.ev.node(&parent).unwrap_or_else(|| {
                panic!("NullPointerException: parent {parent:?} of {error_key:?} not in graph")
            });
            let (parent_done, parent_entry_debug, parent_declared_dep, previously_requested_deps) = {
                let mut state = parent_node.lock();
                let done = state.entry.is_done();
                let debug = format!("{:?}", state.entry);
                if done {
                    (true, debug, false, GroupedDeps::new())
                } else {
                    let deps = state.entry.get_temporary_direct_deps().clone();
                    (false, debug, deps.contains(&error_key), deps)
                }
            };
            check_state!(
                !parent_done,
                "We cannot bubble into a done node entry: a done node cannot depend on a not-done \
                 node, and the first errorParent was not done: {:?} {} {:?} {} {:?} {:?} {:?} \
                 {:?}",
                error_key,
                error_entry_debug,
                parent,
                parent_entry_debug,
                leaf_failure,
                roots,
                rdeps_to_bubble_up_to,
                bubble_error_info
            );
            // Expected 6 args, but got 8.
            check_state!(
                self.progress().is_inflight(&parent),
                "In-progress reverse deps can only include in-flight nodes: {:?} {} {:?} {} {:?} \
                 {:?}{}",
                error_key,
                error_entry_debug,
                parent,
                parent_entry_debug,
                leaf_failure,
                roots,
                lenient_extra(&[rdeps_to_bubble_up_to, &bubble_error_info])
            );
            // Expected 6 args, but got 8.
            check_state!(
                parent_declared_dep,
                "In-progress reverse deps can only include nodes that have declared a dep: {:?} \
                 {} {:?} {} {:?} {:?}{}",
                error_key,
                error_entry_debug,
                parent,
                parent_entry_debug,
                leaf_failure,
                roots,
                lenient_extra(&[rdeps_to_bubble_up_to, &bubble_error_info])
            );
            let sky_function = self.function_for(&parent, &parent_node);
            {
                let mut state = parent_node.lock();
                if state.entry.is_dirty() {
                    match state.entry.get_lifecycle_state() {
                        LifecycleState::CheckDependencies => {
                            // If this value's child was bubbled up to, it did not signal this
                            // value, and so we must manually make it ready to build.
                            state.entry.signal_dep(self.graph_version, Some(&error_key));
                            // Fall through to NEEDS_REBUILDING, since state is now
                            // NEEDS_REBUILDING.
                            maybe_mark_rebuilding(&mut state.entry);
                        }
                        LifecycleState::NeedsRebuilding => maybe_mark_rebuilding(&mut state.entry),
                        LifecycleState::Rebuilding => {}
                        _ => panic!(
                            "AssertionError: {parent:?} not in valid dirty state: {:?}",
                            state.entry
                        ),
                    }
                }
            }
            error_key = parent;
            let (env_events, value_with_metadata) = {
                let mut env = Environment::create_for_error(
                    self,
                    &error_key,
                    &parent_node,
                    &*sky_function,
                    previously_requested_deps,
                    &bubble_error_info,
                );
                // This build is only to check if the parent node can give us a better error. We
                // don't care about a return value.
                match sky_function.compute(&error_key, &mut env) {
                    Ok(_) => {}
                    Err(ComputeError::Interrupted(_)) => {
                        // Do nothing.
                        // This throw happens if the builder requested the failed node, and then
                        // checked the interrupted state later -- getValueOrThrow sets the
                        // interrupted bit after the failed value is requested, to prevent the
                        // builder from doing too much work.
                    }
                    Err(ComputeError::Exception(builder_exception)) => {
                        error = Arc::new(ErrorInfo::from_exception(
                            &builder_exception,
                            /* is_transitively_transient= */ false,
                        ));
                    }
                }
                // Builder didn't throw its own exception, so just propagate this one up. (Or it
                // did, and `error` is now that exception.)
                let events = env.report_events_and_get_events_to_store_for_error();
                (
                    events.clone(),
                    ValueWithMetadata::error(
                        ErrorInfo::from_child_errors(&error_key, [&*error]),
                        events,
                    ),
                )
            };
            self.replay(&env_events);
            let entry = Arc::new(DoneEntry {
                kind: DoneKind::Bubbled,
                ..DoneEntry::from_value(&value_with_metadata, Version::minimal())
            });
            bubble_error_info.insert(error_key.clone(), entry);
        }
        Some(bubble_error_info)
    }

    /// The unfinished nodes that exist only because a function asked for them and then threw
    /// although they were missing.
    ///
    /// PORT: Java commits the exception of such a function without ever evaluating the missing
    /// deps (`AbstractParallelEvaluator.java:487-546`: the function threw in the run that
    /// requested them, so `enqueueChild` never ran). Those deps, and what only they went on to
    /// request, do not exist in Java, so they are not reverse deps of anything. Here the request
    /// evaluated them before the function threw, and they registered with the nodes they asked
    /// for, among them nodes above the thrower when there is a cycle. Error bubbling walks up
    /// through reverse deps and must not walk down into them: they have no path to a top-level
    /// node but the one through the thrower.
    ///
    /// A node is in the set if it is unfinished, reachable from such a missing dep through the
    /// deps unfinished nodes declared, not a top-level key, and requested by nodes of the set
    /// only (the thrower's own request was taken back when it committed,
    /// `Run::remove_undone_newly_requested_deps`).
    fn never_evaluated(&self, root_values: &HashSet<&SkyKey>) -> HashSet<SkyKey> {
        let mut to_visit: Vec<SkyKey> = std::mem::take(&mut *self.undone_deps_of_committed.lock());
        let mut candidates: HashSet<SkyKey> = HashSet::new();
        while let Some(key) = to_visit.pop() {
            let Some(node) = self.ev.node(&key) else {
                continue;
            };
            let mut state = node.lock();
            if state.entry.is_done() || !candidates.insert(key) {
                continue;
            }
            to_visit.extend(
                state
                    .entry
                    .get_temporary_direct_deps()
                    .get_all_elements_as_iterable()
                    .cloned(),
            );
        }
        // Take out what something else asked for, until nothing changes.
        loop {
            let evaluated: Vec<SkyKey> = candidates
                .iter()
                .filter(|key| {
                    root_values.contains(key)
                        || self.ev.node(key).is_some_and(|node| {
                            let state = node.lock();
                            !state.entry.is_done()
                                && state
                                    .entry
                                    .get_in_progress_reverse_deps()
                                    .iter()
                                    .any(|requester| !candidates.contains(requester))
                        })
                })
                .cloned()
                .collect();
            if evaluated.is_empty() {
                return candidates;
            }
            for key in &evaluated {
                candidates.remove(key);
            }
        }
    }

    /// Constructs an [`EvaluationResult`] from the graph. Looks for cycles if there are
    /// unfinished nodes but no error was already found through bubbling up (as indicated by
    /// `bubble_error_info` being null).
    // Java: com/google/devtools/build/skyframe/ParallelEvaluator.java:506 ParallelEvaluator.constructResult
    fn construct_result(
        &self,
        sky_keys: &[SkyKey],
        bubble_error_info: Option<&BubbleErrorInfo>,
        catastrophe: bool,
    ) -> EvaluationResult {
        let keep_going = self.ctx.keep_going;
        let mut result = EvaluationResult::builder();
        let mut cycle_roots: Vec<SkyKey> = vec![];
        for sky_key in sky_keys {
            check_state!(
                !catastrophe || keep_going,
                "Catastrophe not consistent with keepGoing mode: {:?} {} {:?}",
                sky_key,
                catastrophe,
                bubble_error_info
            );
            // Java: maybeGetValueFromError
            let value_with_metadata = bubble_error_info
                .and_then(|b| b.get(sky_key).cloned())
                .or_else(|| self.ev.committed_entry(sky_key));
            // Cycle checking: if there is a cycle, evaluation cannot progress, therefore, the
            // final values will not be in DONE state when the work runs out.
            let Some(value_with_metadata) = value_with_metadata else {
                // Don't look for cycles if the build failed for a known reason.
                if bubble_error_info.is_none() {
                    cycle_roots.push(sky_key.clone());
                }
                continue;
            };
            let value = &value_with_metadata.value;
            let error_info = value_with_metadata.error.as_deref();
            check_state!(value.is_some() || error_info.is_some(), "{:?}", sky_key);
            if let Some(error_info) = error_info
                && !keep_going
            {
                // value will be null here unless the value was already built on a prior
                // keepGoing build.
                result.add_error(sky_key.clone(), error_info.clone());
                continue;
            }
            match value {
                // Note that we must be in the keepGoing case. Only make this value an error if
                // it doesn't have a value. The error shouldn't matter to the caller since the
                // value succeeded after a fashion.
                None => {
                    result.add_error(sky_key.clone(), error_info.expect("value or error").clone())
                }
                Some(v) => result.add_result(sky_key.clone(), v.clone()),
            }
        }
        if !cycle_roots.is_empty() {
            if self.ctx.detect_cycles {
                SimpleCycleDetector::new(self.ctx.store_exact_cycles).check_for_cycles(
                    &cycle_roots,
                    &mut result,
                    self,
                );
            } else {
                blaze_util::unported!("ShortCircuitingCycleDetector.checkForCycles");
            }
        }
        check_state!(
            !result.is_empty() || sky_keys.is_empty(),
            "No result for keys {:?} ({:?} {})",
            sky_keys,
            bubble_error_info,
            catastrophe
        );
        result.maybe_ensure_catastrophe(catastrophe);
        check_state!(
            bubble_error_info.is_none() || result.has_error(),
            "If an error bubbled up, some top-level node must be in error: {:?} {:?}",
            bubble_error_info,
            sky_keys
        );
        result.build()
    }
}

/// If the entry is dirty and not already rebuilding, puts it in a state so that it can rebuild.
// Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:147 AbstractParallelEvaluator.maybeMarkRebuilding
pub(crate) fn maybe_mark_rebuilding(entry: &mut crate::in_memory_node_entry::InMemoryNodeEntry) {
    if entry.is_dirty() && entry.get_lifecycle_state() != LifecycleState::Rebuilding {
        entry.mark_rebuilding();
    }
}

/// Java's `try { progressReceiver.stateStarting(...); ... } finally { stateEnding(...) }`.
struct StateGuard<'a> {
    receiver: &'a DirtyAndInflightTrackingProgressReceiver,
    key: &'a SkyKey,
    state: NodeState,
}

impl<'a> StateGuard<'a> {
    fn new(
        receiver: &'a DirtyAndInflightTrackingProgressReceiver,
        key: &'a SkyKey,
        state: NodeState,
    ) -> Self {
        receiver.state_starting(key, state);
        StateGuard {
            receiver,
            key,
            state,
        }
    }
}

impl Drop for StateGuard<'_> {
    fn drop(&mut self) {
        self.receiver.state_ending(self.key, self.state);
    }
}

/// Java: `NestedSetEventReceiver.accept`: `event.reportTo(reporter)`.
// Java: com/google/devtools/build/skyframe/ParallelEvaluatorContext.java:203 NestedSetEventReceiver.accept
pub(crate) fn report_stored(event: &StoredReportable, reporter: &dyn ExtendedEventHandler) {
    match event {
        StoredReportable::Event(e) => e.report_to(reporter),
        StoredReportable::Post(p) => p.report_to(reporter),
    }
}

// Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:810 Evaluate.prepareCrashMessage
fn prepare_crash_message(sky_key: &SkyKey, reverse_deps: &[SkyKey]) -> String {
    const MAX_REVERSEDEP_DUMP_LENGTH: usize = 1000;
    let mut reverse_dep_dump = String::new();
    for key in reverse_deps {
        if reverse_dep_dump.len() > MAX_REVERSEDEP_DUMP_LENGTH {
            reverse_dep_dump.push_str(", ...");
            break;
        }
        if !reverse_dep_dump.is_empty() {
            reverse_dep_dump.push_str(", ");
        }
        let _ = write!(reverse_dep_dump, "'{key:?}'");
    }
    format!(
        "Unrecoverable error while evaluating node '{sky_key:?}' (requested by nodes \
         {reverse_dep_dump})"
    )
}

/// Java: `new RuntimeException(msg, re)`, whose stack trace prints `msg`, then "Caused by: " and
/// the original exception.
///
/// PORT: a panic payload that is a message becomes that text. Any other payload (a typed value
/// some caller downcasts) cannot carry a message and is rethrown as it is.
fn wrap_crash(msg: String, payload: Box<dyn Any + Send>) -> Box<dyn Any + Send> {
    let cause: &str = if let Some(s) = payload.downcast_ref::<&'static str>() {
        s
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s
    } else {
        return payload;
    };
    Box::new(format!("{msg}\nCaused by: {cause}"))
}

#[allow(unused_imports)]
use java_list as _;
