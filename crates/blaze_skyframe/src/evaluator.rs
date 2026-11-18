//! The evaluator: ports of `MemoizingEvaluator` / `AbstractInMemoryMemoizingEvaluator`,
//! `ParallelEvaluator` and `AbstractParallelEvaluator`, reshaped for blocking evaluation (see the
//! crate docs).

use crate::environment::{EnvState, Environment};
use crate::evaluation_result::EvaluationResult;
use crate::simple_cycle_detector::SimpleCycleDetector;
use crate::{
    ComputeError, ComputeResult, ErrorInfo, GroupedDeps, Interrupted, Key, SkyFunction,
    SkyFunctionException, SkyFunctionName, SkyKey, SkyValue, typed,
};
use blaze_events::{Event, EventHandler, EventKind, ExtendedEventHandler, Postable};
use blaze_util::{check_argument, check_state};
use parking_lot::{Condvar, Mutex};
use std::any::Any;
use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};
use std::thread::ThreadId;

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
    /// Java: an error computed by `ParallelEvaluator.bubbleErrorUp`, which is never written to
    /// the graph. Visible to waiting parents, dropped after the evaluation.
    Bubbled,
    /// Java: the node was still in flight when the evaluation aborted (no keep_going error,
    /// catastrophe, crash). Parents see it as not done. Dropped after the evaluation.
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
}

impl DoneEntry {
    fn undone(kind: DoneKind, deps: GroupedDeps) -> DoneEntry {
        DoneEntry {
            kind,
            value: None,
            error: None,
            deps,
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
}

impl Node {
    fn new(state: NodeState) -> Arc<Node> {
        Arc::new(Node {
            state: Mutex::new(state),
            done: Condvar::new(),
            rdeps: Mutex::new(vec![]),
        })
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
    live_threads: AtomicUsize,
    peak_threads: AtomicUsize,
    pub(crate) emitted_event_state: EmittedEventState,
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
            live_threads: AtomicUsize::new(0),
            peak_threads: AtomicUsize::new(0),
            emitted_event_state: EmittedEventState::default(),
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
    /// Java: `Thread.interrupt()` of the thread calling `evaluate`.
    pub fn interrupt(&self) {
        self.interrupted.store(true, Ordering::SeqCst);
    }

    /// Java: `Thread.interrupted()` (clears the interrupt status).
    pub fn clear_interrupt(&self) {
        self.interrupted.store(false, Ordering::SeqCst);
    }

    /// The interrupt flag, for a signal handler to set.
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

        // Optimization: if all required node values are already present in the cache, return them
        // directly without launching the heavy machinery, spawning threads, etc.
        if roots.iter().all(|k| self.is_done(k)) {
            return Ok(run.construct_result(&roots));
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
                return Ok(run.construct_result(&cached_error_keys));
            }
        }

        run.evaluate_roots(&roots);

        let crash = run.crash.lock().take();
        if let Some(payload) = crash {
            run.cleanup();
            resume_unwind(payload);
        }
        if run.saw_interrupt.load(Ordering::SeqCst) || self.interrupted.load(Ordering::SeqCst) {
            run.cleanup();
            return Err(Interrupted);
        }
        let result = run.construct_result(&roots);
        run.cleanup();
        Ok(result)
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
}

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
    catastrophe: AtomicBool,
    saw_interrupt: AtomicBool,
    uncommitted: Mutex<Vec<SkyKey>>,
    crash: Mutex<Option<Box<dyn Any + Send>>>,
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
            catastrophe: AtomicBool::new(false),
            saw_interrupt: AtomicBool::new(false),
            uncommitted: Mutex::new(vec![]),
            crash: Mutex::new(None),
        }
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
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..workers)
                .map(|_| {
                    let queue = &queue;
                    self.spawn_builder()
                        .spawn_scoped(scope, move || {
                            self.permits.acquire();
                            loop {
                                let Some(k) = queue.lock().pop_front() else {
                                    break;
                                };
                                self.eval_key(&k);
                            }
                            self.ev.unregister_thread();
                            self.permits.release();
                        })
                        .expect("spawn evaluator thread")
                })
                .collect();
            for h in handles {
                if let Err(p) = h.join() {
                    resume_unwind(p);
                }
            }
        });
        self.ev.release_threads(workers);
    }

    /// Computes or waits for `key` on the current thread (which holds a permit).
    pub(crate) fn eval_key(&self, key: &SkyKey) -> Arc<DoneEntry> {
        let (node, claimed) = self.ev.graph.get_or_claim(key);
        if claimed {
            if DEPTH.with(Cell::get) >= self.ctx.max_inline_depth {
                return self.compute_on_fresh_thread(key, &node);
            }
            return self.compute_claimed(key, &node);
        }
        if let NodeState::Done(r) = &*node.state.lock() {
            return r.clone();
        }
        self.wait_for(key, &node)
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
            let h = self
                .spawn_builder()
                .spawn_scoped(scope, || {
                    let r = self.compute_claimed(key, node);
                    self.ev.unregister_thread();
                    r
                })
                .expect("spawn evaluator thread");
            h.join().unwrap_or_else(|p| resume_unwind(p))
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
        let entry = if self.ev.interrupted.load(Ordering::SeqCst) {
            self.saw_interrupt.store(true, Ordering::SeqCst);
            DoneEntry::undone(DoneKind::Interrupted, GroupedDeps::new())
        } else if self.aborting.load(Ordering::SeqCst) {
            // Java: no new evaluations start once the visitor prevents them.
            DoneEntry::undone(DoneKind::Aborted, GroupedDeps::new())
        } else {
            DEPTH.with(|d| d.set(d.get() + 1));
            let r = catch_unwind(AssertUnwindSafe(|| self.run_function(key)));
            DEPTH.with(|d| d.set(d.get() - 1));
            match r {
                Ok(e) => e,
                Err(payload) => {
                    // Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:548 Evaluate.run
                    // A RuntimeException in a SkyFunction crashes the evaluation. Stop new work,
                    // let waiters finish, and rethrow from `evaluate`.
                    self.crash.lock().get_or_insert(payload);
                    self.aborting.store(true, Ordering::SeqCst);
                    DoneEntry::undone(DoneKind::Aborted, GroupedDeps::new())
                }
            }
        };
        if let Some(s) = self.ev.waits.lock().stacks.get_mut(&me) {
            s.pop();
        }
        self.finish(key, node, entry)
    }

    // Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:415 Evaluate.run
    fn run_function(&self, key: &SkyKey) -> DoneEntry {
        let function_name = key.function_name();
        // Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:457
        // PORT: Java also prints the NodeEntry's toString; the node here is simply in progress.
        let Some(function) = self.ev.functions.get(&function_name).cloned() else {
            panic!(
                "NullPointerException: Unable to find SkyFunction '{function_name}' for node with \
                 key {key:?}, NodeEntry{{state=EVALUATING}}"
            );
        };
        self.ev.computed.fetch_add(1, Ordering::Relaxed);
        let mut env = Environment::new(self, key, &*function);
        let r = function.compute(key, &mut env);
        self.resolve(key, env, r)
    }

    /// Decides how a node finishes from what its function returned and what it saw of its deps.
    /// Ports the commit paths of `AbstractParallelEvaluator.Evaluate.run`, the result of
    /// `ParallelEvaluator.bubbleErrorUp` (no keep_going), and `SkyFunctionEnvironment`'s
    /// `commitAndGetParents` error construction.
    fn resolve(&self, key: &SkyKey, env: Environment<'_>, r: ComputeResult) -> DoneEntry {
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
                    return DoneEntry::undone(DoneKind::Interrupted, st.deps);
                }
                Ret::Abandoned
            }
        };
        if st.interrupted_dep {
            self.saw_interrupt.store(true, Ordering::SeqCst);
            return DoneEntry::undone(DoneKind::Interrupted, st.deps);
        }
        let any_child_transitively_transient = st
            .child_error_infos
            .iter()
            .any(|e| e.is_transitively_transient());

        if !keep_going {
            if let Some(child_error) = &st.dep_error {
                // Java: com/google/devtools/build/skyframe/ParallelEvaluator.java:290
                // bubbleErrorUp. The parent saw the child's error during its normal compute (the
                // blocking engine's equivalent of being re-run in error bubbling) and may have
                // thrown its own exception; its return value is ignored.
                let error = match &ret {
                    Ret::Threw(e) => {
                        ErrorInfo::from_exception(e, /* is_transitively_transient= */ false)
                    }
                    _ => (**child_error).clone(),
                };
                let error = ErrorInfo::from_child_errors(key, [&error]);
                self.aborting.store(true, Ordering::SeqCst);
                listener.flush();
                return DoneEntry {
                    kind: DoneKind::Bubbled,
                    value: None,
                    error: Some(Arc::new(error)),
                    deps: st.deps,
                };
            }
            match ret {
                Ret::Threw(e) => {
                    // Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:511
                    // shouldFailFast.
                    if self
                        .aborting
                        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                        .is_err()
                    {
                        // This is not the first error encountered, so we ignore it so that we can
                        // terminate with the first error.
                        return DoneEntry::undone(DoneKind::Aborted, st.deps);
                    }
                    let error = ErrorInfo::from_exception(
                        &e,
                        e.is_transient() || any_child_transitively_transient,
                    );
                    listener.flush();
                    return self.committed(None, Some(error), st);
                }
                Ret::Abandoned => return DoneEntry::undone(DoneKind::Aborted, st.deps),
                Ret::Value(_) | Ret::Null => {}
            }
            if st.aborted_dep {
                return DoneEntry::undone(DoneKind::Aborted, st.deps);
            }
            if st.in_cycle_dep {
                return DoneEntry::undone(DoneKind::InCycle, st.deps);
            }
            return match ret {
                Ret::Value(v) => {
                    check_state!(
                        !values_missing,
                        "Evaluation of {:?} returned non-null value but requested dependencies \
                         that weren't computed yet (one of {:?}), NodeEntry: {:?}",
                        key,
                        st.deps,
                        "EVALUATING"
                    );
                    listener.flush();
                    self.committed(Some(v), None, st)
                }
                _ => panic!(
                    "IllegalStateException: Evaluation of SkyKey failed and no dependencies were \
                     requested: {key:?} {:?}",
                    st.deps
                ),
            };
        }

        // keep_going.
        if let Ret::Threw(e) = &ret
            && !values_missing
        {
            let error =
                ErrorInfo::from_exception(e, e.is_transient() || any_child_transitively_transient);
            if e.is_catastrophic() {
                // Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:511
                // shouldFailFast also holds for catastrophes in keep_going.
                if self
                    .aborting
                    .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                    .is_err()
                {
                    return DoneEntry::undone(DoneKind::Aborted, st.deps);
                }
                self.catastrophe.store(true, Ordering::SeqCst);
            }
            listener.flush();
            if self.catastrophe.load(Ordering::SeqCst)
                && st.child_error_infos.iter().any(|c| c.is_catastrophic())
            {
                // On the error-bubbling path of a catastrophe: not committed in Java.
                return DoneEntry {
                    kind: DoneKind::Bubbled,
                    value: None,
                    error: Some(Arc::new(error)),
                    deps: st.deps,
                };
            }
            return self.committed(None, Some(error), st);
        }
        // In keep-going mode, we do not let SkyFunctions complete with a thrown error if they
        // have missing deps. Instead, we wait until their deps are done and restart the
        // SkyFunction, so we can have a definitive error and definitive graph structure, thus
        // avoiding non-determinism. It's completely reasonable for SkyFunctions to throw eagerly
        // because they do not know if they are in keep-going mode.
        let catastrophic_child = st.child_error_infos.iter().any(|c| c.is_catastrophic());
        if !catastrophic_child {
            if st.aborted_dep {
                return DoneEntry::undone(DoneKind::Aborted, st.deps);
            }
            if st.in_cycle_dep {
                return DoneEntry::undone(DoneKind::InCycle, st.deps);
            }
        }
        match ret {
            Ret::Value(v) => {
                check_state!(
                    !values_missing,
                    "Evaluation of {:?} returned non-null value but requested dependencies that \
                     weren't computed yet (one of {:?}), NodeEntry: {:?}",
                    key,
                    st.deps,
                    "EVALUATING"
                );
                // Java: ErrorInfoManager.UseChildErrorInfoIfNecessary.getErrorInfoToUse
                let error = if st.child_error_infos.is_empty() {
                    None
                } else {
                    Some(ErrorInfo::with_value(&ErrorInfo::from_child_errors(
                        key,
                        st.child_error_infos.iter().map(|e| &**e),
                    )))
                };
                listener.flush();
                self.committed(Some(v), error, st)
            }
            Ret::Null | Ret::Threw(_) | Ret::Abandoned => {
                if st.child_error_infos.is_empty() {
                    if matches!(ret, Ret::Abandoned) {
                        return DoneEntry::undone(DoneKind::Aborted, st.deps);
                    }
                    panic!(
                        "IllegalStateException: Evaluation of SkyKey failed and no dependencies \
                         were requested: {key:?} {:?}",
                        st.deps
                    );
                }
                let error =
                    ErrorInfo::from_child_errors(key, st.child_error_infos.iter().map(|e| &**e));
                listener.flush();
                if error.is_catastrophic() {
                    return DoneEntry {
                        kind: DoneKind::Bubbled,
                        value: None,
                        error: Some(Arc::new(error)),
                        deps: st.deps,
                    };
                }
                self.committed(None, Some(error), st)
            }
        }
    }

    /// A node committed to the graph. Deps that were not done when requested are not recorded
    /// (Java: `SkyFunctionEnvironment.removeUndoneNewlyRequestedDeps`), and a directly transient
    /// error adds a dependency on `ErrorTransienceValue.KEY`, always the last dep
    /// (`SkyFunctionEnvironment.setError`).
    fn committed(
        &self,
        value: Option<SkyValue>,
        error: Option<ErrorInfo>,
        st: EnvState,
    ) -> DoneEntry {
        let undone: HashSet<SkyKey> = st
            .requested
            .iter()
            .filter(|(_, e)| e.is_undone_view())
            .map(|(k, _)| k.clone())
            .collect();
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
        }
    }

    fn finish(&self, key: &SkyKey, node: &Node, entry: DoneEntry) -> Arc<DoneEntry> {
        let entry = Arc::new(entry);
        if entry.is_done() {
            self.ev.register_rdeps(key, &entry.deps);
        } else {
            self.uncommitted.lock().push(key.clone());
        }
        *node.state.lock() = NodeState::Done(entry.clone());
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
        });
        self.ev.register_rdeps(key, &entry.deps);
        *node.state.lock() = NodeState::Done(entry);
    }

    /// Blocks until `node` is done, detecting cycles first. Releases the permit while blocked.
    /// Returns an "in cycle" entry when waiting would close a cycle.
    fn wait_for(&self, key: &SkyKey, node: &Arc<Node>) -> Arc<DoneEntry> {
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

    /// Computes several keys, spreading unstarted ones over helper threads.
    pub(crate) fn eval_keys(&self, keys: &[SkyKey]) -> Vec<Arc<DoneEntry>> {
        if keys.len() <= 1 || self.parallelism() <= 1 {
            return keys.iter().map(|k| self.eval_key(k)).collect();
        }
        // Already-done keys need no thread.
        let mut results: Vec<Option<Arc<DoneEntry>>> = keys
            .iter()
            .map(|k| self.ev.graph.get(k).and_then(|n| n.done_entry()))
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
                results[i] = Some(self.eval_key(&keys[i]));
            }
            return results.into_iter().map(|r| r.expect("filled")).collect();
        }
        // The current thread computes the first pending key itself; helpers and the current
        // thread then take the rest from a queue.
        let me = std::thread::current().id();
        let rest: Vec<SkyKey> = pending[1..].iter().map(|&i| keys[i].clone()).collect();
        let queue = Mutex::new(rest.iter().cloned().enumerate().collect::<VecDeque<_>>());
        let rest_results: Vec<Mutex<Option<Arc<DoneEntry>>>> =
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
                results[i] = Some(self.eval_key(&keys[i]));
            }
            return results.into_iter().map(|r| r.expect("filled")).collect();
        }
        let drain = |queue: &Mutex<VecDeque<(usize, SkyKey)>>| {
            loop {
                let Some((i, k)) = queue.lock().pop_front() else {
                    break;
                };
                let r = self.eval_key(&k);
                *rest_results[i].lock() = Some(r);
            }
        };
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..helper_count)
                .map(|_| {
                    let queue = &queue;
                    let drain = &drain;
                    self.spawn_builder()
                        .spawn_scoped(scope, move || {
                            self.permits.acquire();
                            drain(queue);
                            self.ev.unregister_thread();
                            self.permits.release();
                        })
                        .expect("spawn evaluator helper")
                })
                .collect();
            let first = pending[0];
            results[first] = Some(self.eval_key(&keys[first]));
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

    /// Constructs the result from the graph. Looks for cycles if there are unfinished roots but
    /// no error was already found through bubbling up.
    // Java: com/google/devtools/build/skyframe/ParallelEvaluator.java:506 ParallelEvaluator.constructResult
    fn construct_result(&self, roots: &[SkyKey]) -> EvaluationResult {
        let keep_going = self.ctx.keep_going;
        // Java: bubbleErrorInfo != null.
        let bubbled = self.aborting.load(Ordering::SeqCst);
        let catastrophe = self.catastrophe.load(Ordering::SeqCst);
        let mut result = EvaluationResult::builder();
        let mut cycle_roots: Vec<SkyKey> = vec![];
        let mut bubble_reported = false;
        for sky_key in roots {
            check_state!(
                !catastrophe || keep_going,
                "Catastrophe not consistent with keepGoing mode: {:?} {} {}",
                sky_key,
                catastrophe,
                bubbled
            );
            let entry = self.ev.done_entry(sky_key).expect("root evaluated");
            match entry.kind {
                DoneKind::Committed => {
                    let error = entry.error.as_deref();
                    if let Some(error) = error
                        && !keep_going
                    {
                        // value will be null here unless the value was already built on a prior
                        // keepGoing build.
                        result.add_error(sky_key.clone(), error.clone());
                        continue;
                    }
                    match &entry.value {
                        // Note that we must be in the keepGoing case. Only make this value an
                        // error if it doesn't have a value. The error shouldn't matter to the
                        // caller since the value succeeded after a fashion.
                        None => result
                            .add_error(sky_key.clone(), error.expect("value or error").clone()),
                        Some(v) => result.add_result(sky_key.clone(), v.clone()),
                    }
                }
                DoneKind::Bubbled => {
                    // PORT: Java bubbles the error up one chain of in-progress parents to one
                    // root. Here every root that waited on the failed node saw the error, so only
                    // the first such root, in request order, reports it; the others have neither
                    // value nor error, as in Java.
                    if !bubble_reported {
                        bubble_reported = true;
                        let error = entry.error.as_deref().expect("bubbled error");
                        result.add_error(sky_key.clone(), error.clone());
                    }
                }
                DoneKind::InCycle => {
                    // Don't look for cycles if the build failed for a known reason.
                    if !bubbled {
                        cycle_roots.push(sky_key.clone());
                    }
                }
                DoneKind::Aborted | DoneKind::Interrupted => {}
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
            !result.is_empty() || roots.is_empty(),
            "No result for keys {:?} ({} {})",
            roots,
            bubbled,
            catastrophe
        );
        result.maybe_ensure_catastrophe(catastrophe);
        check_state!(
            !bubbled || result.has_error(),
            "If an error bubbled up, some top-level node must be in error: {:?}",
            roots
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
