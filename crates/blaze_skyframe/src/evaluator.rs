//! The evaluator: ports of `MemoizingEvaluator` / `AbstractInMemoryMemoizingEvaluator`,
//! `ParallelEvaluator` and `AbstractParallelEvaluator`, reshaped for blocking evaluation (see the
//! crate docs).

use crate::environment::{BubbleErrorInfo, EnvState, Environment};
use crate::evaluation_result::EvaluationResult;
use crate::simple_cycle_detector::SimpleCycleDetector;
use crate::{
    ComputeError, ComputeResult, ErrorInfo, GroupedDeps, Interrupted, Key, SkyFunction,
    SkyFunctionException, SkyFunctionName, SkyKey, SkyValue, typed,
};
use blaze_events::{Event, EventHandler, EventKind, ExtendedEventHandler, Postable};
use blaze_shell::ThreadInterrupt;
use blaze_util::{check_argument, check_state};
use parking_lot::{Condvar, Mutex};
use std::any::Any;
use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
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
}

/// A value that represents "error transience", i.e. anything which may have caused an unexpected
/// failure.
// Java: com/google/devtools/build/skyframe/ErrorTransienceValue.java:22 ErrorTransienceValue
#[derive(Debug)]
pub struct ErrorTransienceValue;

/// `ErrorTransienceValue.KEY` as a [`SkyKey`].
pub static ERROR_TRANSIENCE_KEY: LazyLock<SkyKey> =
    LazyLock::new(|| SkyKey::new(ErrorTransienceKey));

/// How a node finished in one evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DoneKind {
    /// Java: the node is done in the graph (value and/or error). Memoized.
    Committed,
    /// Java: an entry of `bubbleErrorInfo`, the error `ParallelEvaluator.bubbleErrorUp` built for
    /// a node on the bubbling path. It is never written to the graph: such entries live only in
    /// the map that `bubble_error_up` returns.
    Bubbled,
    /// Java: the node was still in flight when the evaluation aborted (no keep_going error,
    /// catastrophe, crash), or it found a dep in error without keep_going and so cannot finish.
    /// Parents see it as not done. Dropped after the evaluation.
    Aborted,
    /// Java: the evaluation was interrupted. Dropped after the evaluation.
    Interrupted,
    /// Java: the node is in or above a cycle and never finished. The cycle detector commits it
    /// (keep_going) or it is dropped after the evaluation.
    InCycle,
}

/// A finished node: Java's `NodeEntry` once done (`ValueWithMetadata` plus direct deps).
pub(crate) struct DoneEntry {
    pub(crate) kind: DoneKind,
    pub(crate) value: Option<SkyValue>,
    pub(crate) error: Option<Arc<ErrorInfo>>,
    pub(crate) deps: GroupedDeps,
    /// For a node that is not done: its function returned null because of a dep that was not
    /// available. Java then registers the node as a reverse dep of the deps it declared
    /// (`enqueueChild`, or the edge to the child in error) and the node waits. False when the
    /// function threw an exception that was ignored (not the first error), was interrupted, or
    /// never ran: Java registers nothing for those.
    pub(crate) awaits_deps: bool,
}

impl DoneEntry {
    pub(crate) fn undone(kind: DoneKind, deps: GroupedDeps) -> DoneEntry {
        DoneEntry {
            kind,
            value: None,
            error: None,
            deps,
            awaits_deps: false,
        }
    }

    /// An undone node whose function returned null (see `awaits_deps`).
    fn awaiting_deps(kind: DoneKind, deps: GroupedDeps) -> DoneEntry {
        DoneEntry {
            awaits_deps: true,
            ..DoneEntry::undone(kind, deps)
        }
    }

    /// Java: `NodeEntry.isDone()` (committed to the graph).
    pub(crate) fn is_done(&self) -> bool {
        self.kind == DoneKind::Committed
    }

    /// Java: the dependency was not done when requested (`NULL_MARKER`).
    pub(crate) fn is_undone_view(&self) -> bool {
        matches!(
            self.kind,
            DoneKind::Aborted | DoneKind::Interrupted | DoneKind::InCycle
        )
    }
}

impl fmt::Debug for DoneEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeEntry")
            .field("state", &self.kind)
            .field("hasValue", &self.value.is_some())
            .field("error", &self.error)
            .field("directDeps", &self.deps)
            .finish()
    }
}

enum NodeState {
    InProgress { owner: ThreadId },
    Done(Arc<DoneEntry>),
}

pub(crate) struct Node {
    state: Mutex<NodeState>,
    done: Condvar,
    /// Committed nodes that depend on this one, in commit order.
    rdeps: Mutex<Vec<SkyKey>>,
    /// Nodes that requested this one during the current evaluation while it was not done, in
    /// request order. Taken (and so emptied) when the node commits. Lock order: `state`, then
    /// this.
    ///
    /// The flag is true for a request that closed a dependency cycle through this node (the
    /// requester is below this node and got "not done" back at once).
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:314 NodeEntry.getInProgressReverseDeps
    in_progress_rdeps: Mutex<Vec<(SkyKey, bool)>>,
}

impl Node {
    fn new(state: NodeState) -> Arc<Node> {
        Arc::new(Node {
            state: Mutex::new(state),
            done: Condvar::new(),
            rdeps: Mutex::new(vec![]),
            in_progress_rdeps: Mutex::new(vec![]),
        })
    }

    /// Java: `addReverseDepAndCheckIfDone(parent)` on a node that is not done.
    fn add_in_progress_rdep(&self, requester: Option<&SkyKey>) {
        if let Some(requester) = requester {
            self.in_progress_rdeps
                .lock()
                .push((requester.clone(), false));
        }
    }

    /// Marks the registration of `requester` as one that closed a cycle through this node.
    fn mark_cycle_rdep(&self, requester: Option<&SkyKey>) {
        if let Some(requester) = requester
            && let Some(rdep) = self
                .in_progress_rdeps
                .lock()
                .iter_mut()
                .rev()
                .find(|(k, _)| k == requester)
        {
            rdep.1 = true;
        }
    }

    /// Java: `NodeEntry.getInProgressReverseDeps` of a node that is not done.
    fn get_in_progress_reverse_deps(&self) -> Vec<SkyKey> {
        self.in_progress_rdeps
            .lock()
            .iter()
            .map(|(k, _)| k.clone())
            .collect()
    }

    pub(crate) fn done_entry(&self) -> Option<Arc<DoneEntry>> {
        match &*self.state.lock() {
            NodeState::Done(e) => Some(e.clone()),
            NodeState::InProgress { .. } => None,
        }
    }

    pub(crate) fn rdeps(&self) -> Vec<SkyKey> {
        self.rdeps.lock().clone()
    }
}

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

const SHARDS: usize = 64;

struct Graph {
    shards: Vec<Mutex<HashMap<SkyKey, Arc<Node>>>>,
}

impl Graph {
    fn new() -> Self {
        Graph {
            shards: (0..SHARDS).map(|_| Mutex::new(HashMap::new())).collect(),
        }
    }
    fn shard(&self, key: &SkyKey) -> &Mutex<HashMap<SkyKey, Arc<Node>>> {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut h);
        &self.shards[(h.finish() as usize) % SHARDS]
    }
    /// Returns the node and whether this call created it (and so must compute it).
    fn get_or_claim(&self, key: &SkyKey) -> (Arc<Node>, bool) {
        let mut shard = self.shard(key).lock();
        if let Some(n) = shard.get(key) {
            return (n.clone(), false);
        }
        let node = Node::new(NodeState::InProgress {
            owner: std::thread::current().id(),
        });
        shard.insert(key.clone(), node.clone());
        (node, true)
    }
    fn get(&self, key: &SkyKey) -> Option<Arc<Node>> {
        self.shard(key).lock().get(key).cloned()
    }
    fn insert(&self, key: SkyKey, node: Arc<Node>) {
        self.shard(&key).lock().insert(key, node);
    }
    fn remove(&self, key: &SkyKey) -> Option<Arc<Node>> {
        self.shard(key).lock().remove(key)
    }
    fn all_keys(&self) -> Vec<SkyKey> {
        self.shards
            .iter()
            .flat_map(|s| s.lock().keys().cloned().collect::<Vec<_>>())
            .collect()
    }
}

/// Keeps track of warnings for purposes of deduplication.
///
/// PORT: Java also tracks visited event `NestedSet` nodes for replay; events are not stored on
/// nodes here (one build per process), so only the warning set remains.
// Java: com/google/devtools/build/skyframe/EmittedEventState.java:29 EmittedEventState
#[derive(Default)]
pub(crate) struct EmittedEventState {
    seen_warnings: Mutex<HashSet<Event>>,
}

impl EmittedEventState {
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

/// The memoizing evaluator: functions and the graph.
// Java: com/google/devtools/build/skyframe/InMemoryMemoizingEvaluator.java:38 InMemoryMemoizingEvaluator
pub struct Evaluator {
    functions: HashMap<SkyFunctionName, Arc<dyn SkyFunction>>,
    graph: Graph,
    waits: Mutex<WaitGraph>,
    client: Arc<dyn Any + Send + Sync>,
    computed: AtomicU64,
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
pub struct EvaluatorBuilder {
    functions: HashMap<SkyFunctionName, Arc<dyn SkyFunction>>,
    injected: Vec<(SkyKey, SkyValue)>,
    client: Arc<dyn Any + Send + Sync>,
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
        }
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
    pub fn inject<K: Key>(mut self, key: K, value: K::Value) -> Self {
        self.injected.push((SkyKey::new(key), Arc::new(value)));
        self
    }
    /// Sets the client context functions can read with [`Environment::client`].
    pub fn client<T: Any + Send + Sync>(mut self, client: Arc<T>) -> Self {
        self.client = client;
        self
    }
    pub fn build(self) -> Evaluator {
        let ev = Evaluator {
            functions: self.functions,
            graph: Graph::new(),
            waits: Mutex::new(WaitGraph::default()),
            client: self.client,
            computed: AtomicU64::new(0),
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
        // Java: ParallelEvaluator.injectErrorTransienceValue
        ev.insert_value(ERROR_TRANSIENCE_KEY.clone(), Arc::new(ErrorTransienceValue));
        for (k, v) in self.injected {
            ev.insert_value(k, v);
        }
        ev
    }
}

impl Evaluator {
    /// Number of functions computed so far.
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

    /// Evaluates `roots` (in parallel) and returns their values and errors.
    // Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:118 AbstractInMemoryMemoizingEvaluator.evaluate
    pub fn evaluate(
        &self,
        roots: &[SkyKey],
        ctx: &EvaluationContext,
    ) -> Result<EvaluationResult, Interrupted> {
        // Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:227 setAndCheckEvaluateState
        check_state!(
            !self.evaluating.swap(true, Ordering::SeqCst),
            "Re-entrant evaluation for request: {:?}",
            roots
        );
        struct ResetEvaluating<'a>(&'a AtomicBool);
        impl Drop for ResetEvaluating<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _reset = ResetEvaluating(&self.evaluating);
        // PORT: incremental bookkeeping (graph versions, diffs, deleting in-flight nodes) is
        // replaced by dropping uncommitted nodes at the end of each call (Run::cleanup).
        if self.interrupted.load(Ordering::SeqCst) {
            return Err(Interrupted);
        }

        // Java: com/google/devtools/build/skyframe/ParallelEvaluator.java:641 ParallelEvaluator.eval
        let mut seen = HashSet::new();
        let roots: Vec<SkyKey> = roots
            .iter()
            .filter(|k| seen.insert((*k).clone()))
            .cloned()
            .collect();
        let run = Run::new(self, ctx);
        // Whatever way this call ends (result, interrupt, crash), the nodes Java would not have
        // committed leave the graph.
        struct Cleanup<'a, 'e>(&'a Run<'e>);
        impl Drop for Cleanup<'_, '_> {
            fn drop(&mut self) {
                self.0.cleanup();
            }
        }
        let _cleanup = Cleanup(&run);

        // Optimization: if all required node values are already present in the cache, return them
        // directly without launching the heavy machinery, spawning threads, etc.
        if roots.iter().all(|k| self.is_done(k)) {
            // Note that the 'catastrophe' parameter doesn't really matter here (it's only used
            // for checking).
            return Ok(run.construct_result(&roots, None, /* catastrophe= */ false));
        }

        if !ctx.keep_going {
            let cached_error_keys: Vec<SkyKey> = roots
                .iter()
                .filter(|k| {
                    self.done_entry(k)
                        .is_some_and(|e| e.is_done() && e.error.is_some())
                })
                .cloned()
                .collect();
            // Errors, even cached ones, should halt evaluations not in keepGoing mode.
            if !cached_error_keys.is_empty() {
                return Ok(run.construct_result(
                    &cached_error_keys,
                    None,
                    /* catastrophe= */ false,
                ));
            }
        }

        run.evaluate_roots(&roots);

        // Java: com/google/devtools/build/skyframe/ParallelEvaluator.java:206 ParallelEvaluator.waitForCompletionAndConstructResult
        run.propagate_crash_if_any();
        if run.saw_interrupt.load(Ordering::SeqCst) || self.interrupted.load(Ordering::SeqCst) {
            return Err(Interrupted);
        }
        let mut bubble_error_info: Option<BubbleErrorInfo> = None;
        let mut catastrophe = false;
        let failure = run.failure.lock().take();
        if let Some(e) = failure {
            bubble_error_info = run.bubble_error_up_on_evaluator_thread(&e, &roots);
            run.propagate_crash_if_any();
            if ctx.keep_going {
                check_state!(
                    e.error_info.is_catastrophic(),
                    "Scheduler exception only thrown for catastrophe in keep_going evaluation: \
                     {:?}",
                    e
                );
                catastrophe = true;
            }
        }
        Ok(run.construct_result(&roots, bubble_error_info.as_ref(), catastrophe))
    }

    /// Returns the value of a done node (also one that has an error with a value).
    // Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:272 AbstractInMemoryMemoizingEvaluator.getExistingValue
    pub fn get_existing_value(&self, key: &SkyKey) -> Option<SkyValue> {
        self.committed_entry(key).and_then(|e| e.value.clone())
    }

    // Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:281 AbstractInMemoryMemoizingEvaluator.getExistingErrorForTesting
    pub fn get_existing_error_for_testing(&self, key: &SkyKey) -> Option<ErrorInfo> {
        self.committed_entry(key)
            .and_then(|e| e.error.as_deref().cloned())
    }

    /// All keys in the graph.
    // Java: com/google/devtools/build/skyframe/WalkableGraph.java:106 WalkableGraph.getAllKeysForTesting
    pub fn get_all_keys_for_testing(&self) -> Vec<SkyKey> {
        self.graph.all_keys()
    }

    /// Injects a value between evaluations, invalidating the reverse transitive closure of an
    /// existing node for the key. Java: `Injectable.inject` applied at the next `evaluate`.
    ///
    /// PORT: Java skips an injection whose value equals the existing one
    /// (`pruneInjectedValues`); values here have no equality, so dependents are always
    /// re-evaluated.
    pub fn inject<K: Key>(&self, key: K, value: K::Value) {
        check_state!(
            !self.evaluating.load(Ordering::SeqCst),
            "inject during evaluation"
        );
        let key = SkyKey::new(key);
        if self.graph.get(&key).is_some() {
            self.delete_reverse_transitive_closure(vec![key.clone()]);
        }
        self.insert_value(key, Arc::new(value));
    }

    /// Re-evaluates, on the next `evaluate`, every node whose error was transient, and their
    /// dependents. Bazel does this at the start of every command.
    // Java: com/google/devtools/build/lib/skyframe/SequencedSkyframeExecutor.java:475 SequencedSkyframeExecutor.invalidateTransientErrors
    pub fn invalidate_transient_errors(&self) {
        check_state!(
            !self.evaluating.load(Ordering::SeqCst),
            "invalidate during evaluation"
        );
        let Some(node) = self.graph.get(&ERROR_TRANSIENCE_KEY) else {
            return;
        };
        let rdeps = std::mem::take(&mut *node.rdeps.lock());
        self.delete_reverse_transitive_closure(rdeps);
    }

    fn insert_value(&self, key: SkyKey, value: SkyValue) {
        let entry = Arc::new(DoneEntry {
            kind: DoneKind::Committed,
            value: Some(value),
            error: None,
            deps: GroupedDeps::new(),
            awaits_deps: false,
        });
        self.graph.insert(key, Node::new(NodeState::Done(entry)));
    }

    /// Deletes `start` and every node that transitively depends on them.
    // PORT: Java dirties these nodes (InvalidatingNodeVisitor); without versions or change
    // pruning, deleting them is equivalent.
    fn delete_reverse_transitive_closure(&self, start: Vec<SkyKey>) {
        let mut doomed: HashSet<SkyKey> = HashSet::new();
        let mut stack = start;
        while let Some(k) = stack.pop() {
            if !doomed.insert(k.clone()) {
                continue;
            }
            if let Some(node) = self.graph.get(&k) {
                stack.extend(node.rdeps());
            }
        }
        for k in &doomed {
            let Some(node) = self.graph.remove(k) else {
                continue;
            };
            if let Some(entry) = node.done_entry() {
                for dep in entry.deps.get_all_elements_as_iterable() {
                    if doomed.contains(dep) {
                        continue;
                    }
                    if let Some(dep_node) = self.graph.get(dep) {
                        dep_node.rdeps.lock().retain(|r| r != k);
                    }
                }
            }
        }
    }

    /// The finished entry of `key`, whatever its kind.
    pub(crate) fn done_entry(&self, key: &SkyKey) -> Option<Arc<DoneEntry>> {
        self.graph.get(key).and_then(|n| n.done_entry())
    }

    /// The entry of `key` if it is done in the graph (Java `NodeEntry.isDone()`).
    pub(crate) fn committed_entry(&self, key: &SkyKey) -> Option<Arc<DoneEntry>> {
        self.done_entry(key).filter(|e| e.is_done())
    }

    pub(crate) fn node(&self, key: &SkyKey) -> Option<Arc<Node>> {
        self.graph.get(key)
    }

    fn is_done(&self, key: &SkyKey) -> bool {
        self.committed_entry(key).is_some()
    }

    fn register_rdeps(&self, key: &SkyKey, deps: &GroupedDeps) {
        for dep in deps.get_all_elements_as_iterable() {
            if let Some(n) = self.graph.get(dep) {
                n.rdeps.lock().push(key.clone());
            }
        }
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

/// State of one `evaluate` call. Java: `ParallelEvaluatorContext` plus the visitor's
/// error/abort state.
pub(crate) struct Run<'e> {
    pub(crate) ev: &'e Evaluator,
    pub(crate) ctx: &'e EvaluationContext,
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
    uncommitted: Mutex<Vec<SkyKey>>,
    /// The deps that were not done when a node that asked for them committed (a function that
    /// threw although a dep was missing). See [`Run::never_evaluated`].
    undone_deps_of_committed: Mutex<Vec<SkyKey>>,
    crash: Mutex<Option<Box<dyn Any + Send>>>,
    /// The failure that stopped the evaluation: Java's first `SchedulerException.ofError`, which
    /// `waitForCompletion` rethrows. Lock order: this, then a node's `state`.
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
    /// value). False when the request computed it or waited for it (Java: `compute` got null,
    /// returned null, and the node waits to be restarted when the dep is done).
    pub(crate) was_done: bool,
}

/// How a function's node finishes.
struct Resolved {
    entry: DoneEntry,
    /// Java: `shouldFailFast` after the error was committed: throw
    /// `SchedulerException.ofError(errorInfo, skyKey, rdepsToBubbleUpTo)`.
    fail_fast: bool,
}

impl From<DoneEntry> for Resolved {
    fn from(entry: DoneEntry) -> Self {
        Resolved {
            entry,
            fail_fast: false,
        }
    }
}

impl<'e> Run<'e> {
    fn new(ev: &'e Evaluator, ctx: &'e EvaluationContext) -> Self {
        Run {
            ev,
            ctx,
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
            uncommitted: Mutex::new(vec![]),
            undone_deps_of_committed: Mutex::new(vec![]),
            crash: Mutex::new(None),
            failure: Mutex::new(None),
        }
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

    /// Evaluates the roots on up to `parallelism` worker threads.
    fn evaluate_roots(&self, roots: &[SkyKey]) {
        let workers = roots.len().min(self.parallelism()).max(1);
        self.ev.force_reserve_threads(workers);
        let queue = Mutex::new(roots.iter().cloned().collect::<VecDeque<_>>());
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
                        let Some(k) = queue.lock().pop_front() else {
                            break;
                        };
                        self.eval_key(&k, None);
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

    /// Computes or waits for `key` on the current thread (which holds a permit). `requester` is
    /// the node whose function asked for `key` (`None` for a top-level key): it is registered as
    /// a reverse dep of `key` unless `key` is already done, as Java's `enqueueChild` does.
    pub(crate) fn eval_key(&self, key: &SkyKey, requester: Option<&SkyKey>) -> Arc<DoneEntry> {
        self.eval_dep(key, requester).entry
    }

    /// [`Run::eval_key`], also telling whether the key was done when it was asked for.
    fn eval_dep(&self, key: &SkyKey, requester: Option<&SkyKey>) -> Requested {
        let (node, claimed) = self.ev.graph.get_or_claim(key);
        if claimed {
            node.add_in_progress_rdep(requester);
            let entry = if DEPTH.with(Cell::get) >= self.ctx.max_inline_depth {
                self.compute_on_fresh_thread(key, &node)
            } else {
                self.compute_claimed(key, &node)
            };
            return Requested {
                entry,
                was_done: false,
            };
        }
        {
            let state = node.state.lock();
            if let NodeState::Done(r) = &*state {
                if !r.is_done() {
                    node.add_in_progress_rdep(requester);
                }
                return Requested {
                    entry: r.clone(),
                    was_done: true,
                };
            }
            node.add_in_progress_rdep(requester);
        }
        Requested {
            entry: self.wait_for(key, &node, requester),
            was_done: false,
        }
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
                    self.finish(
                        key,
                        node,
                        DoneEntry::undone(DoneKind::Aborted, GroupedDeps::new()).into(),
                    )
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
            *node.state.lock() = NodeState::InProgress { owner: me };
            w.stacks.entry(me).or_default().push(key.clone());
        }
        // Java: WrappedRunnable.run: addJob, then blockNewActions, so that a thread racing to
        // interrupt the in-flight work either finds this job or this job sees the abort.
        let job = self.ev.add_job();
        let entry: Resolved = if self.ev.interrupted.load(Ordering::SeqCst) {
            self.saw_interrupt.store(true, Ordering::SeqCst);
            DoneEntry::undone(DoneKind::Interrupted, GroupedDeps::new()).into()
        } else if self.aborting.load(Ordering::SeqCst) {
            // Java: no new evaluations start once the visitor prevents them.
            DoneEntry::undone(DoneKind::Aborted, GroupedDeps::new()).into()
        } else {
            DEPTH.with(|d| d.set(d.get() + 1));
            let r = catch_unwind(AssertUnwindSafe(|| self.run_function(key)));
            DEPTH.with(|d| d.set(d.get() - 1));
            match r {
                Ok(e) => e,
                Err(payload) => {
                    // Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:548 Evaluate.run
                    // Programmer error (most likely NPE or a failed precondition in a
                    // SkyFunction). Output some context together with the exception.
                    // Stop new work, let waiters finish, and rethrow from `evaluate`.
                    let reverse_deps = node.get_in_progress_reverse_deps();
                    let msg = prepare_crash_message(key, &reverse_deps);
                    self.note_crash(wrap_crash(msg, payload));
                    DoneEntry::undone(DoneKind::Aborted, GroupedDeps::new()).into()
                }
            }
        };
        drop(job);
        if let Some(s) = self.ev.waits.lock().stacks.get_mut(&me) {
            s.pop();
        }
        self.finish(key, node, entry)
    }

    fn function_for(&self, key: &SkyKey, node_state: &str) -> Arc<dyn SkyFunction> {
        let function_name = key.function_name();
        // Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:457
        // PORT: Java also prints the NodeEntry's toString; the node here is simply in progress.
        let Some(function) = self.ev.functions.get(&function_name).cloned() else {
            panic!(
                "NullPointerException: Unable to find SkyFunction '{function_name}' for node with \
                 key {key:?}, NodeEntry{{state={node_state}}}"
            );
        };
        function
    }

    // Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:415 Evaluate.run
    fn run_function(&self, key: &SkyKey) -> Resolved {
        let function = self.function_for(key, "EVALUATING");
        self.ev.computed.fetch_add(1, Ordering::Relaxed);
        let mut env = Environment::new(self, key, &*function);
        let r = function.compute(key, &mut env);
        self.resolve(key, env, r)
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
            return Arc::new(DoneEntry::undone(DoneKind::Aborted, GroupedDeps::new()));
        }
        requested.entry
    }

    /// Decides how a node finishes from what its function returned and what it saw of its deps.
    /// Ports the part of `AbstractParallelEvaluator.Evaluate.run` after `compute` returns, and
    /// `SkyFunctionEnvironment.commitAndGetParents`' error construction.
    ///
    /// PORT: where Java enqueues the not-done deps and restarts the function when they are done,
    /// the deps here were computed by the requests themselves. A dep that is still not done
    /// afterwards (the evaluation is aborting, it is in a cycle, it was interrupted) never will
    /// be in this evaluation, and neither will this node: it finishes "undone" with the deps it
    /// declared, which is the in-flight `NodeEntry` Java leaves behind for error bubbling and
    /// cycle detection.
    // Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:473 Evaluate.run
    fn resolve(&self, key: &SkyKey, env: Environment<'_>, r: ComputeResult) -> Resolved {
        let keep_going = self.ctx.keep_going;
        let (st, values_missing, listener) = env.into_parts();
        let ret = match r {
            Ok(Some(v)) => Ret::Value(v),
            Ok(None) => Ret::Null,
            Err(ComputeError::Exception(e)) => Ret::Threw(e),
            Err(ComputeError::Interrupted(_)) => {
                if self.ev.interrupted.load(Ordering::SeqCst)
                    || !self.aborting.load(Ordering::SeqCst)
                {
                    self.saw_interrupt.store(true, Ordering::SeqCst);
                    return DoneEntry::undone(DoneKind::Interrupted, st.deps).into();
                }
                Ret::Abandoned
            }
        };
        if st.interrupted_dep {
            self.saw_interrupt.store(true, Ordering::SeqCst);
            return DoneEntry::undone(DoneKind::Interrupted, st.deps).into();
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
            if should_fail_fast {
                // After we commit this error to the graph but before the doMutatingEvaluation
                // call completes with the error there is a race-like opportunity for the error to
                // be used, either by an in-flight computation or by a future computation.
                // Java: preventNewEvaluations()
                if self
                    .aborting
                    .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                    .is_err()
                {
                    // This is not the first error encountered, so we ignore it so that we can
                    // terminate with the first error.
                    return DoneEntry::undone(DoneKind::Aborted, st.deps).into();
                }
            }
            // Java: isAnyDirectDepErrorTransitivelyTransient ||
            // isAnyNewlyRequestedDepErrorTransitivelyTransient
            let is_transitively_transient = builder_exception.is_transient()
                || st
                    .child_error_infos
                    .iter()
                    .any(|e| e.is_transitively_transient());
            let error_info =
                ErrorInfo::from_exception(builder_exception, is_transitively_transient);
            listener.flush();
            return Resolved {
                entry: self.committed(key, None, Some(error_info), st),
                fail_fast: should_fail_fast,
            };
        }

        if let Ret::Value(value) = ret {
            check_state!(
                !values_missing,
                "Evaluation of {:?} returned non-null value but requested dependencies that \
                 weren't computed yet (one of {:?}), NodeEntry: {:?}",
                key,
                st.deps,
                "EVALUATING"
            );
            // Java: ErrorInfoManager.UseChildErrorInfoIfNecessary.getErrorInfoToUse
            let error_info = if st.child_error_infos.is_empty() {
                None
            } else {
                Some(ErrorInfo::with_value(&ErrorInfo::from_child_errors(
                    key,
                    st.child_error_infos.iter().map(|e| &**e),
                )))
            };
            listener.flush();
            return self.committed(key, Some(value), error_info, st).into();
        }

        // The function stopped with `Interrupted` because the evaluation is aborting. Java: the
        // InterruptedException leaves the node as it was: nothing is committed, nothing is
        // registered with its deps (SchedulerException.ofInterruption).
        if matches!(ret, Ret::Abandoned) {
            return DoneEntry::undone(DoneKind::Aborted, st.deps).into();
        }
        // From here on the function returned null (in keep_going also: threw with deps missing).

        if let Some((child_error_key, child_error_info)) = &st.dep_error {
            check_state!(!keep_going, "{:?} {:?}", key, child_error_key);
            // We encountered a child error in noKeepGoing mode, so we want to fail fast. But we
            // first need to add the edge between the current node and the child error it
            // requested so that error bubbling can occur. Note that this edge will subsequently
            // be removed during graph cleaning (since the current node will never be committed to
            // the graph).
            // PORT: the edge is the dep this run declared; the node finishes undone with it.
            self.aborting.store(true, Ordering::SeqCst);
            // Java: throw SchedulerException.ofError(childErrorInfo, childErrorKey,
            // ImmutableSet.of(skyKey)); the visitor keeps the first exception thrown.
            self.failure.lock().get_or_insert_with(|| SchedulerError {
                error_info: child_error_info.clone(),
                failed_value: child_error_key.clone(),
                rdeps_to_bubble_up_to: vec![key.clone()],
            });
            self.stop_in_flight_jobs();
            return DoneEntry::awaiting_deps(DoneKind::Aborted, st.deps).into();
        }

        // Java enqueues the deps that are not done and returns; the node is restarted when they
        // are. Here a dep that is not done after it was requested never will be.
        if st.aborted_dep || st.in_cycle_dep {
            let kind = if st.aborted_dep {
                DoneKind::Aborted
            } else {
                DoneKind::InCycle
            };
            return DoneEntry::awaiting_deps(kind, st.deps).into();
        }

        // If there were no newly requested dependencies, then at least one of them was in error
        // or there is a bug in the SkyFunction implementation. The environment has collected its
        // errors, so we just order it to be built.
        check_state!(
            !st.child_error_infos.is_empty(),
            "Evaluation of SkyKey failed and no dependencies were requested: {:?} {:?}",
            key,
            st.deps
        );
        // If the child error was catastrophic, committing this parent to the graph is not
        // necessary, but since we don't do error bubbling in catastrophes, it doesn't violate any
        // invariants either.
        let error_info =
            ErrorInfo::from_child_errors(key, st.child_error_infos.iter().map(|e| &**e));
        listener.flush();
        self.committed(key, None, Some(error_info), st).into()
    }

    /// A node committed to the graph. Deps that were not done when requested are not recorded
    /// (Java: `SkyFunctionEnvironment.removeUndoneNewlyRequestedDeps`), and a directly transient
    /// error adds a dependency on `ErrorTransienceValue.KEY`, always the last dep
    /// (`SkyFunctionEnvironment.setError`).
    ///
    /// PORT: Java registers a node as a reverse dep of the deps it requested only after its
    /// function returns null (`enqueueChild`), so a node that commits (a function that threw with
    /// deps missing) never registered on its undone deps. Here the request itself registered it;
    /// that is taken back.
    fn committed(
        &self,
        key: &SkyKey,
        value: Option<SkyValue>,
        error: Option<ErrorInfo>,
        st: EnvState,
    ) -> DoneEntry {
        let undone: HashSet<SkyKey> = st
            .requested
            .lock()
            .iter()
            .filter(|(_, e)| e.is_undone_view())
            .map(|(k, _)| k.clone())
            .collect();
        if !undone.is_empty() {
            self.undone_deps_of_committed
                .lock()
                .extend(undone.iter().cloned());
        }
        for dep in &undone {
            if let Some(dep_node) = self.ev.node(dep) {
                dep_node.in_progress_rdeps.lock().retain(|(r, _)| r != key);
            }
        }
        let mut deps = st.deps;
        deps.remove(&undone);
        if error.as_ref().is_some_and(|e| e.is_directly_transient()) {
            deps.append_singleton(ERROR_TRANSIENCE_KEY.clone());
        }
        DoneEntry {
            kind: DoneKind::Committed,
            value,
            error: error.map(Arc::new),
            deps,
            awaits_deps: false,
        }
    }

    fn finish(&self, key: &SkyKey, node: &Node, resolved: Resolved) -> Arc<DoneEntry> {
        let entry = Arc::new(resolved.entry);
        if entry.is_done() {
            self.ev.register_rdeps(key, &entry.deps);
        } else {
            self.uncommitted.lock().push(key.clone());
        }
        // Java: `NodeEntry.setValue` returns the reverse deps registered while the node was in
        // progress (`commitAndGetParents`); a parent that asks later finds the node done.
        //
        // PORT: without the requesters whose request closed a cycle through this node. They are
        // below it: in Java they would have been evaluated only after this node's function
        // returned null, and then it could not have committed. (Reached by a function that
        // throws although a dep is missing; Java commits that error without ever evaluating what
        // is below.)
        let set_value = || -> Vec<SkyKey> {
            let mut state = node.state.lock();
            *state = NodeState::Done(entry.clone());
            if entry.is_done() {
                std::mem::take(&mut *node.in_progress_rdeps.lock())
                    .into_iter()
                    .filter(|(_, closed_cycle)| !closed_cycle)
                    .map(|(k, _)| k)
                    .collect()
            } else {
                vec![]
            }
        };
        if resolved.fail_fast {
            // Java: throw SchedulerException.ofError(errorInfo, skyKey, rdepsToBubbleUpTo). The
            // failure is recorded before any waiter can see the node done
            // (`entry_seen_in_normal_evaluation`).
            let mut failure = self.failure.lock();
            let rdeps_to_bubble_up_to = set_value();
            failure.get_or_insert_with(|| SchedulerError {
                error_info: entry.error.clone().expect("fail fast with an error"),
                failed_value: key.clone(),
                rdeps_to_bubble_up_to,
            });
            drop(failure);
            node.done.notify_all();
            self.stop_in_flight_jobs();
            return entry;
        } else {
            set_value();
        }
        node.done.notify_all();
        entry
    }

    /// Commits an error the cycle detector built (Java: `env.setError` +
    /// `commitAndGetParents` in `SimpleCycleDetector`).
    pub(crate) fn commit_cycle_error(&self, key: &SkyKey, error: ErrorInfo, deps: GroupedDeps) {
        let node = self.ev.node(key).expect("node in graph");
        let entry = Arc::new(DoneEntry {
            kind: DoneKind::Committed,
            value: None,
            error: Some(Arc::new(error)),
            deps,
            awaits_deps: false,
        });
        self.ev.register_rdeps(key, &entry.deps);
        let mut state = node.state.lock();
        *state = NodeState::Done(entry);
        node.in_progress_rdeps.lock().clear();
    }

    /// Blocks until `node` is done, detecting cycles first. Releases the permit while blocked.
    /// Returns an "in cycle" entry when waiting would close a cycle.
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
                return match &*node.state.lock() {
                    NodeState::Done(r) => r.clone(),
                    NodeState::InProgress { .. } => {
                        node.mark_cycle_rdep(requester);
                        Arc::new(DoneEntry::undone(DoneKind::InCycle, GroupedDeps::new()))
                    }
                };
            }
        }
        self.permits.release();
        let result = {
            let mut state = node.state.lock();
            loop {
                if let NodeState::Done(r) = &*state {
                    break r.clone();
                }
                node.done.wait(&mut state);
            }
        };
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
            let node = ev.graph.get(k)?;
            let owner = match &*node.state.lock() {
                NodeState::InProgress { owner } => *owner,
                NodeState::Done(_) => return None,
            };
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
    /// [`Run::eval_key`].
    pub(crate) fn eval_keys(&self, keys: &[SkyKey], requester: Option<&SkyKey>) -> Vec<Requested> {
        if keys.len() <= 1 || self.parallelism() <= 1 {
            return keys.iter().map(|k| self.eval_dep(k, requester)).collect();
        }
        // Already-done keys need no thread.
        let mut results: Vec<Option<Requested>> = keys
            .iter()
            .map(|k| {
                self.ev.committed_entry(k).map(|entry| Requested {
                    entry,
                    was_done: true,
                })
            })
            .collect();
        let pending: Vec<usize> = (0..keys.len()).filter(|&i| results[i].is_none()).collect();
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
            for i in pending {
                results[i] = Some(self.eval_dep(&keys[i], requester));
            }
            return results.into_iter().map(|r| r.expect("filled")).collect();
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
    /// (`Run::resolve`), so it is unfinished here as well. "Not done" nodes are those that did
    /// not commit; their recorded deps are Java's temporary direct deps. The interrupt bit Java
    /// sets and clears around each re-run is per environment
    /// (`Environment::check_interrupted`), so there is nothing to clear or restore.
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
            let error_entry = error_node
                .done_entry()
                .unwrap_or_else(|| panic!("NullPointerException: {error_key:?}"));
            let mut all_reverse_deps: Vec<SkyKey> = if error_entry.is_done() {
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
                    "undone first iteration: {:?} {:?} {:?} {:?} {:?} {:?}",
                    error_key,
                    error_entry,
                    leaf_failure,
                    roots,
                    rdeps_to_bubble_up_to,
                    bubble_error_info
                );
                error_node.get_in_progress_reverse_deps()
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
                // PORT: replaying the stored events of a done top-level node is skipped (one
                // build per process, events are reported once when the node commits).
                break;
            }
            let parent = reverse_deps[0].clone();
            if bubble_error_info.contains_key(&parent) {
                // Bubbled into a cycle. Don't try to bubble anything up. Cycle detection will
                // kick in.
                return None;
            }
            let parent_entry = self.ev.done_entry(&parent).unwrap_or_else(|| {
                panic!("NullPointerException: parent {parent:?} of {error_key:?} not in graph")
            });
            check_state!(
                !parent_entry.is_done(),
                "We cannot bubble into a done node entry: a done node cannot depend on a not-done \
                 node, and the first errorParent was not done: {:?} {:?} {:?} {:?} {:?} {:?} {:?} \
                 {:?}",
                error_key,
                error_entry,
                parent,
                parent_entry,
                leaf_failure,
                roots,
                rdeps_to_bubble_up_to,
                bubble_error_info
            );
            // PORT: "In-progress reverse deps can only include in-flight nodes" has no
            // counterpart: a node that is not done is in flight.
            check_state!(
                parent_entry.deps.contains(&error_key),
                "In-progress reverse deps can only include nodes that have declared a dep: {:?} \
                 {:?} {:?} {:?} {:?} {:?}",
                error_key,
                error_entry,
                parent,
                parent_entry,
                leaf_failure,
                roots
            );
            let sky_function = self.function_for(&parent, "EVALUATING");
            // PORT: dirty-node handling skipped, incremental.
            error_key = parent;
            let env = {
                let mut env = Environment::create_for_error(
                    self,
                    &error_key,
                    &*sky_function,
                    parent_entry.deps.clone(),
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
                env
            };
            // Builder didn't throw its own exception, so just propagate this one up. (Or it did,
            // and `error` is now that exception.)
            let (_, _, listener) = env.into_parts();
            // Java: env.reportEventsAndGetEventsToStore(parentEntry, /*expectDoneDeps=*/ false)
            listener.flush();
            let value_with_metadata = Arc::new(DoneEntry {
                kind: DoneKind::Bubbled,
                value: None,
                error: Some(Arc::new(ErrorInfo::from_child_errors(
                    &error_key,
                    [&*error],
                ))),
                deps: GroupedDeps::new(),
                awaits_deps: false,
            });
            bubble_error_info.insert(error_key.clone(), value_with_metadata);
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
    /// only (the thrower's own request was taken back when it committed, `Run::committed`).
    fn never_evaluated(&self, root_values: &HashSet<&SkyKey>) -> HashSet<SkyKey> {
        let mut to_visit: Vec<SkyKey> = std::mem::take(&mut *self.undone_deps_of_committed.lock());
        let mut candidates: HashSet<SkyKey> = HashSet::new();
        while let Some(key) = to_visit.pop() {
            let Some(entry) = self.ev.done_entry(&key) else {
                continue;
            };
            if entry.is_done() || !candidates.insert(key) {
                continue;
            }
            to_visit.extend(entry.deps.get_all_elements_as_iterable().cloned());
        }
        // Take out what something else asked for, until nothing changes.
        loop {
            let evaluated: Vec<SkyKey> = candidates
                .iter()
                .filter(|key| {
                    root_values.contains(key)
                        || self.ev.node(key).is_some_and(|node| {
                            node.get_in_progress_reverse_deps()
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

    /// Drops every node that Java would not have committed to the graph.
    fn cleanup(&self) {
        let keys = std::mem::take(&mut *self.uncommitted.lock());
        for key in keys {
            let Some(node) = self.ev.graph.get(&key) else {
                continue;
            };
            let drop_it = node.done_entry().is_some_and(|e| !e.is_done());
            if drop_it {
                self.ev.graph.remove(&key);
            }
        }
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
