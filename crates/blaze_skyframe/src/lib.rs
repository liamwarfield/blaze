//! Blaze's evaluation engine: a port of Bazel's Skyframe (`com.google.devtools.build.skyframe`)
//! shaped so that `SkyFunction`s port line by line.
//!
//! # Model
//!
//! Bazel's evaluator restarts a function whenever `env.getValue` returns null for a dependency
//! that is not computed yet. Blaze instead *blocks*: `Environment::get_value` computes a missing
//! dependency (inline on this thread, or by waiting for the thread already computing it) and then
//! returns it. The Java protocol is kept so ported code reads the same:
//!
//! * `get_value` returns `Option`. It returns `None` only when the dependency finished with an
//!   error (or is part of a cycle). As in Java, the function should then return `Ok(None)`,
//!   and the engine fails the parent with the dependency's error ("error bubbling").
//! * `get_value_or_throw::<E>` returns `Err(E)` when the dependency failed with an `E`, letting
//!   the function catch it, exactly like `getValueOrThrow(key, E.class)`.
//! * `values_missing()` reports whether any requested dependency was unavailable.
//!
//! A single evaluation per process is assumed (no incrementality, no invalidation, no change
//! pruning). `// PORT: skipped, incremental` marks the Java features this leaves out.
//!
//! # Threads
//!
//! At most `parallelism` threads compute at once (a permit pool). A thread that blocks on another
//! thread's computation, or on a batch it handed out, gives its permit back while it waits.
//! `get_values` with several unstarted keys spreads them over helper threads.
//!
//! # Cycles
//!
//! Each thread records the stack of keys it is computing; a waiting thread records the keys it
//! waits for. Before waiting, a thread walks this wait-for graph under one lock. If the walk
//! reaches itself, the request fails with a [`CycleInfo`] instead of deadlocking.

use parking_lot::{Condvar, Mutex};
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::ThreadId;

/// Java: com/google/devtools/build/skyframe/SkyFunctionName.java
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SkyFunctionName(pub &'static str);

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
    pub fn function_name(&self) -> SkyFunctionName {
        self.0.function_name()
    }
    pub fn downcast<K: Key>(&self) -> Option<&K> {
        self.0.as_any().downcast_ref::<K>()
    }
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

/// Java: SkyFunctionException.Transience
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transience {
    Transient,
    Persistent,
}

/// Java: com/google/devtools/build/skyframe/SkyFunctionException.java
#[derive(Clone, Debug)]
pub struct SkyFunctionException {
    pub cause: SkyError,
    pub transience: Transience,
}

impl SkyFunctionException {
    pub fn new<E: std::error::Error + Send + Sync + 'static>(
        cause: E,
        transience: Transience,
    ) -> Self {
        SkyFunctionException {
            cause: Arc::new(cause),
            transience,
        }
    }
    pub fn persistent<E: std::error::Error + Send + Sync + 'static>(cause: E) -> Self {
        Self::new(cause, Transience::Persistent)
    }
}

/// Java: com/google/devtools/build/skyframe/CycleInfo.java
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CycleInfo {
    /// Keys from the requesting top-level key down to (not including) the cycle.
    pub path_to_cycle: Vec<SkyKey>,
    /// The keys on the cycle, starting with the key that closes it.
    pub cycle: Vec<SkyKey>,
}

/// Java: com/google/devtools/build/skyframe/ErrorInfo.java (simplified: one root cause).
#[derive(Clone, Debug)]
pub struct ErrorInfo {
    /// The exception, when the failure is not purely a cycle.
    pub exception: Option<SkyError>,
    /// The key whose function threw `exception` (Java: root causes).
    pub root_cause: Option<SkyKey>,
    pub transience: Transience,
    pub cycles: Vec<CycleInfo>,
}

impl ErrorInfo {
    fn from_exception(key: &SkyKey, e: SkyFunctionException) -> Self {
        ErrorInfo {
            exception: Some(e.cause),
            root_cause: Some(key.clone()),
            transience: e.transience,
            cycles: vec![],
        }
    }
    fn from_cycle(cycle: CycleInfo) -> Self {
        ErrorInfo {
            exception: None,
            root_cause: None,
            transience: Transience::Persistent,
            cycles: vec![cycle],
        }
    }
    /// Downcasts the exception to a concrete error type.
    pub fn exception_as<E: std::error::Error + 'static>(&self) -> Option<&E> {
        self.exception.as_ref().and_then(|e| e.downcast_ref::<E>())
    }
}

type NodeResult = Result<SkyValue, ErrorInfo>;

/// The result type of a [`SkyFunction`]: `Ok(Some(v))` computed, `Ok(None)` a dependency was
/// unavailable (Java returns null), `Err(e)` the function threw.
pub type ComputeResult = Result<Option<SkyValue>, SkyFunctionException>;

/// Java: com/google/devtools/build/skyframe/SkyFunction.java
pub trait SkyFunction: Send + Sync + 'static {
    fn compute(&self, key: &SkyKey, env: &mut Environment<'_>) -> ComputeResult;
}

impl<F> SkyFunction for F
where
    F: Fn(&SkyKey, &mut Environment<'_>) -> ComputeResult + Send + Sync + 'static,
{
    fn compute(&self, key: &SkyKey, env: &mut Environment<'_>) -> ComputeResult {
        self(key, env)
    }
}

/// Wraps a typed function `Fn(&K, &mut Environment) -> Result<Option<K::Value>, _>`.
pub fn typed<K, F>(f: F) -> impl SkyFunction
where
    K: Key,
    F: Fn(&K, &mut Environment<'_>) -> Result<Option<K::Value>, SkyFunctionException>
        + Send
        + Sync
        + 'static,
{
    move |key: &SkyKey, env: &mut Environment<'_>| {
        let k = key.downcast::<K>().expect("key type matches function name");
        Ok(f(k, env)?.map(|v| Arc::new(v) as SkyValue))
    }
}

enum NodeState {
    InProgress { owner: ThreadId },
    Done(NodeResult),
}

struct Node {
    state: Mutex<NodeState>,
    done: Condvar,
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
    /// Keys each thread waits for (a stack: a thread can wait on a batch while computing).
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
        let node = Arc::new(Node {
            state: Mutex::new(NodeState::InProgress {
                owner: std::thread::current().id(),
            }),
            done: Condvar::new(),
        });
        shard.insert(key.clone(), node.clone());
        (node, true)
    }
    fn get(&self, key: &SkyKey) -> Option<Arc<Node>> {
        self.shard(key).lock().get(key).cloned()
    }
}

/// Settings for one evaluation. Java: `EvaluationContext`.
#[derive(Clone, Debug)]
pub struct EvaluationContext {
    pub keep_going: bool,
    pub parallelism: usize,
    /// Stack size of helper threads. Starlark evaluation is stack hungry.
    pub thread_stack_size: usize,
}

impl Default for EvaluationContext {
    fn default() -> Self {
        EvaluationContext {
            keep_going: false,
            parallelism: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4),
            thread_stack_size: 256 << 20,
        }
    }
}

/// The shared engine: functions, graph and the client context.
pub struct Evaluator {
    functions: HashMap<SkyFunctionName, Arc<dyn SkyFunction>>,
    graph: Graph,
    waits: Mutex<WaitGraph>,
    permits: Permits,
    ctx: EvaluationContext,
    /// Set when an error occurs without keep_going: no new computations start.
    aborting: AtomicBool,
    client: Arc<dyn Any + Send + Sync>,
    computed: AtomicU64,
}

/// Java: com/google/devtools/build/skyframe/EvaluationResult.java
pub struct EvaluationResult {
    pub values: Vec<(SkyKey, SkyValue)>,
    pub errors: Vec<(SkyKey, ErrorInfo)>,
}

impl EvaluationResult {
    pub fn has_error(&self) -> bool {
        !self.errors.is_empty()
    }
    pub fn get<K: Key>(&self, key: &K) -> Option<Arc<K::Value>> {
        self.values
            .iter()
            .find(|(k, _)| k.downcast::<K>() == Some(key))
            .and_then(|(_, v)| v.clone().downcast::<K::Value>().ok())
    }
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
        F: Fn(&K, &mut Environment<'_>) -> Result<Option<K::Value>, SkyFunctionException>
            + Send
            + Sync
            + 'static,
    {
        self.function(K::FUNCTION, typed::<K, F>(f))
    }
    /// Injects a precomputed value. Java: `PrecomputedValue` / `injectGraphTransformerForTesting`.
    pub fn inject<K: Key>(mut self, key: K, value: K::Value) -> Self {
        self.injected.push((SkyKey::new(key), Arc::new(value)));
        self
    }
    /// Sets the client context functions can read with [`Environment::client`].
    pub fn client<T: Any + Send + Sync>(mut self, client: Arc<T>) -> Self {
        self.client = client;
        self
    }
    pub fn build(self, ctx: EvaluationContext) -> Evaluator {
        let parallelism = ctx.parallelism.max(1);
        let ev = Evaluator {
            functions: self.functions,
            graph: Graph::new(),
            waits: Mutex::new(WaitGraph::default()),
            permits: Permits {
                available: Mutex::new(parallelism),
                cv: Condvar::new(),
            },
            ctx,
            aborting: AtomicBool::new(false),
            client: self.client,
            computed: AtomicU64::new(0),
        };
        for (k, v) in self.injected {
            let (node, _) = ev.graph.get_or_claim(&k);
            *node.state.lock() = NodeState::Done(Ok(v));
        }
        ev
    }
}

#[derive(Debug)]
struct Aborted;
impl fmt::Display for Aborted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("evaluation aborted after an earlier error (no --keep_going)")
    }
}
impl std::error::Error for Aborted {}

impl Evaluator {
    /// Number of functions computed so far.
    pub fn computed_count(&self) -> u64 {
        self.computed.load(Ordering::Relaxed)
    }

    /// Evaluates `roots` (in parallel) and returns their values and errors.
    /// Java: MemoizingEvaluator.evaluate
    pub fn evaluate(&self, roots: &[SkyKey]) -> EvaluationResult {
        let results: Vec<(SkyKey, NodeResult)> = std::thread::scope(|scope| {
            let handles: Vec<_> = roots
                .iter()
                .cloned()
                .map(|k| {
                    std::thread::Builder::new()
                        .stack_size(self.ctx.thread_stack_size)
                        .spawn_scoped(scope, move || {
                            self.permits.acquire();
                            self.register_thread();
                            let r = self.eval_key(&k);
                            self.unregister_thread();
                            self.permits.release();
                            (k, r)
                        })
                        .expect("spawn evaluator thread")
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("evaluator thread panicked"))
                .collect()
        });
        let mut out = EvaluationResult {
            values: vec![],
            errors: vec![],
        };
        for (k, r) in results {
            match r {
                Ok(v) => out.values.push((k, v)),
                Err(e) => out.errors.push((k, e)),
            }
        }
        out
    }

    /// Returns the finished result of a key, if it was computed.
    pub fn get_done(&self, key: &SkyKey) -> Option<NodeResultRef> {
        let node = self.graph.get(key)?;
        match &*node.state.lock() {
            NodeState::Done(r) => Some(r.clone()),
            NodeState::InProgress { .. } => None,
        }
    }

    fn register_thread(&self) {
        self.waits
            .lock()
            .stacks
            .entry(std::thread::current().id())
            .or_default();
    }

    fn unregister_thread(&self) {
        let mut w = self.waits.lock();
        let id = std::thread::current().id();
        w.stacks.remove(&id);
        w.waiting.remove(&id);
    }

    /// Computes or waits for `key` on the current thread (which holds a permit).
    fn eval_key(&self, key: &SkyKey) -> NodeResult {
        let (node, claimed) = self.graph.get_or_claim(key);
        if claimed {
            return self.compute_claimed(key, &node);
        }
        {
            let state = node.state.lock();
            if let NodeState::Done(r) = &*state {
                return r.clone();
            }
        }
        self.wait_for(&[key.clone()], &[node])
            .into_iter()
            .next()
            .expect("one result")
    }

    fn compute_claimed(&self, key: &SkyKey, node: &Arc<Node>) -> NodeResult {
        let me = std::thread::current().id();
        self.waits
            .lock()
            .stacks
            .entry(me)
            .or_default()
            .push(key.clone());
        let result = if self.aborting.load(Ordering::Relaxed) {
            Err(ErrorInfo::from_exception(
                key,
                SkyFunctionException::new(Aborted, Transience::Transient),
            ))
        } else {
            self.run_function(key)
        };
        if result.is_err() && !self.ctx.keep_going {
            self.aborting.store(true, Ordering::Relaxed);
        }
        self.waits.lock().stacks.get_mut(&me).map(|s| s.pop());
        self.computed.fetch_add(1, Ordering::Relaxed);
        *node.state.lock() = NodeState::Done(result.clone());
        node.done.notify_all();
        result
    }

    fn run_function(&self, key: &SkyKey) -> NodeResult {
        let name = key.function_name();
        let Some(f) = self.functions.get(&name).cloned() else {
            panic!("no SkyFunction registered for {name} (key {key:?})");
        };
        let mut env = Environment {
            evaluator: self,
            key,
            first_dep_error: None,
            values_missing: false,
        };
        match f.compute(key, &mut env) {
            Ok(Some(v)) => Ok(v),
            Ok(None) => match env.first_dep_error {
                // Java: error bubbling — the parent fails with the child's error.
                Some(e) => Err(e),
                None => panic!(
                    "SkyFunction {name} returned None for {key:?} without a missing dependency"
                ),
            },
            Err(e) => Err(ErrorInfo::from_exception(key, e)),
        }
    }

    /// Blocks until all `nodes` are done, detecting cycles first. Releases the permit while
    /// blocked.
    fn wait_for(&self, keys: &[SkyKey], nodes: &[Arc<Node>]) -> Vec<NodeResult> {
        let me = std::thread::current().id();
        {
            let mut w = self.waits.lock();
            let frames = w.stacks.get(&me).map_or(0, |s| s.len());
            w.waiting
                .entry(me)
                .or_default()
                .push((frames, keys.to_vec()));
            if let Some(cycle) = self.find_cycle(&w, me) {
                w.waiting.get_mut(&me).map(|v| v.pop());
                drop(w);
                return keys
                    .iter()
                    .zip(nodes)
                    .map(|(_, n)| match &*n.state.lock() {
                        NodeState::Done(r) => r.clone(),
                        NodeState::InProgress { .. } => Err(ErrorInfo::from_cycle(cycle.clone())),
                    })
                    .collect();
            }
        }
        self.permits.release();
        let results = nodes
            .iter()
            .map(|n| {
                let mut state = n.state.lock();
                loop {
                    if let NodeState::Done(r) = &*state {
                        break r.clone();
                    }
                    n.done.wait(&mut state);
                }
            })
            .collect();
        self.permits.acquire();
        self.waits.lock().waiting.get_mut(&me).map(|v| v.pop());
        results
    }

    /// Walks the wait-for graph from `me`. Returns the cycle if the walk returns to `me`.
    fn find_cycle(&self, w: &WaitGraph, me: ThreadId) -> Option<CycleInfo> {
        // Walk (thread, stack index) states: "the key at this index of this thread's stack
        // cannot finish until the thread's waits that block that index finish".
        struct Hop {
            frames: usize,   // the wait entry used on the waiting thread
            owner: ThreadId, // owner of the awaited key
            index: usize,    // the awaited key's index in the owner's stack
        }
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
            path: &mut Vec<Hop>,
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
                    path.push(Hop {
                        frames: *frames,
                        owner,
                        index: j,
                    });
                    if owner == me {
                        return true;
                    }
                    if seen.insert(k.clone()) && dfs(ev, w, owner, j, me, seen, path) {
                        return true;
                    }
                    path.pop();
                }
            }
            false
        }
        let my_stack = w.stacks.get(&me).cloned().unwrap_or_default();
        if my_stack.is_empty() {
            return None; // nothing on this thread can be awaited by others
        }
        let mut path = vec![];
        if !dfs(
            self,
            w,
            me,
            my_stack.len() - 1,
            me,
            &mut HashSet::new(),
            &mut path,
        ) {
            return None;
        }
        // Cycle keys: me's frames from the key that closes the cycle up to the first wait, then
        // each owner's frames from the awaited key up to the wait it makes next.
        let last = path.last().expect("non-empty path");
        let mut cycle: Vec<SkyKey> = my_stack[last.index..path[0].frames].to_vec();
        for (i, hop) in path.iter().enumerate().take(path.len() - 1) {
            let stack = w.stacks.get(&hop.owner).cloned().unwrap_or_default();
            let until = path[i + 1].frames.min(stack.len());
            cycle.extend(stack[hop.index.min(until)..until].iter().cloned());
        }
        Some(CycleInfo {
            path_to_cycle: my_stack[..last.index].to_vec(),
            cycle,
        })
    }

    /// Computes several keys, spreading unstarted ones over helper threads.
    fn eval_keys(&self, keys: &[SkyKey]) -> Vec<NodeResult> {
        if keys.len() <= 1 || self.ctx.parallelism <= 1 {
            return keys.iter().map(|k| self.eval_key(k)).collect();
        }
        // Already-done keys need no thread.
        let mut results: Vec<Option<NodeResult>> = keys
            .iter()
            .map(|k| {
                self.graph.get(k).and_then(|n| match &*n.state.lock() {
                    NodeState::Done(r) => Some(r.clone()),
                    NodeState::InProgress { .. } => None,
                })
            })
            .collect();
        let pending: Vec<usize> = (0..keys.len()).filter(|&i| results[i].is_none()).collect();
        if pending.len() <= 1 {
            for i in pending {
                results[i] = Some(self.eval_key(&keys[i]));
            }
            return results.into_iter().map(|r| r.expect("filled")).collect();
        }
        // The current thread computes the first pending key itself; helpers take the rest.
        let me = std::thread::current().id();
        let rest: Vec<SkyKey> = pending[1..].iter().map(|&i| keys[i].clone()).collect();
        let helper_count = rest.len().min(self.ctx.parallelism);
        let queue = Mutex::new(rest.iter().cloned().enumerate().collect::<Vec<_>>());
        let rest_results: Vec<Mutex<Option<NodeResult>>> =
            rest.iter().map(|_| Mutex::new(None)).collect();
        // The parent depends on every key of the batch from now on: record that before any
        // helper starts, so a helper that asks for a key on this thread's stack sees the cycle.
        {
            let mut w = self.waits.lock();
            let frames = w.stacks.get(&me).map_or(0, |s| s.len());
            w.waiting
                .entry(me)
                .or_default()
                .push((frames, rest.clone()));
        }
        std::thread::scope(|scope| {
            for _ in 0..helper_count {
                let queue = &queue;
                let rest_results = &rest_results;
                std::thread::Builder::new()
                    .stack_size(self.ctx.thread_stack_size)
                    .spawn_scoped(scope, move || {
                        self.permits.acquire();
                        self.register_thread();
                        loop {
                            let Some((i, k)) = queue.lock().pop() else {
                                break;
                            };
                            let r = self.eval_key(&k);
                            *rest_results[i].lock() = Some(r);
                        }
                        self.unregister_thread();
                        self.permits.release();
                    })
                    .expect("spawn evaluator helper");
            }
            let first = pending[0];
            results[first] = Some(self.eval_key(&keys[first]));
            // Give back the permit while the helpers finish.
            self.permits.release();
        });
        self.permits.acquire();
        self.waits.lock().waiting.get_mut(&me).map(|v| v.pop());
        for (j, &i) in pending[1..].iter().enumerate() {
            results[i] = rest_results[j].lock().take();
        }
        results.into_iter().map(|r| r.expect("filled")).collect()
    }
}

type NodeResultRef = NodeResult;

/// What a function sees while computing. Java: `SkyFunction.Environment`.
pub struct Environment<'a> {
    evaluator: &'a Evaluator,
    key: &'a SkyKey,
    first_dep_error: Option<ErrorInfo>,
    values_missing: bool,
}

impl Environment<'_> {
    /// The key being computed.
    pub fn key(&self) -> &SkyKey {
        self.key
    }

    /// The client context set with [`EvaluatorBuilder::client`].
    pub fn client<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.evaluator.client.downcast_ref::<T>()
    }

    fn note(&mut self, r: NodeResult) -> Option<SkyValue> {
        match r {
            Ok(v) => Some(v),
            Err(e) => {
                self.values_missing = true;
                if self.first_dep_error.is_none() {
                    self.first_dep_error = Some(e);
                }
                None
            }
        }
    }

    /// Java: Environment.getValue. `None` means the dependency failed; return `Ok(None)`.
    pub fn get_value<K: Key>(&mut self, key: K) -> Option<Arc<K::Value>> {
        self.get_value_dyn(&SkyKey::new(key))
            .map(downcast_value::<K>)
    }

    /// Untyped `getValue`.
    pub fn get_value_dyn(&mut self, key: &SkyKey) -> Option<SkyValue> {
        let r = self.evaluator.eval_key(key);
        self.note(r)
    }

    /// Java: Environment.getValueOrThrow(key, E.class). A dependency error of type `E` is
    /// returned as `Err` (and does not count as missing); other errors give `Ok(None)`.
    pub fn get_value_or_throw<K: Key, E>(&mut self, key: K) -> Result<Option<Arc<K::Value>>, E>
    where
        E: std::error::Error + Clone + 'static,
    {
        let r = self.evaluator.eval_key(&SkyKey::new(key));
        if let Err(info) = &r {
            if let Some(e) = info.exception_as::<E>() {
                return Err(e.clone());
            }
        }
        Ok(self.note(r).map(downcast_value::<K>))
    }

    /// Java: Environment.getValuesAndExceptions — computes all keys, in parallel where possible.
    /// Results are in the order of `keys`; failed dependencies are `Err(ErrorInfo)` and also
    /// mark values as missing.
    pub fn get_values(&mut self, keys: &[SkyKey]) -> Vec<Result<SkyValue, ErrorInfo>> {
        let rs = self.evaluator.eval_keys(keys);
        for r in &rs {
            if let Err(e) = r {
                self.values_missing = true;
                if self.first_dep_error.is_none() {
                    self.first_dep_error = Some(e.clone());
                }
            }
        }
        rs
    }

    /// Typed batch form of [`Environment::get_values`]; `None` for failed dependencies.
    pub fn get_values_typed<K: Key + Clone>(&mut self, keys: &[K]) -> Vec<Option<Arc<K::Value>>> {
        let dyn_keys: Vec<SkyKey> = keys.iter().cloned().map(SkyKey::new).collect();
        self.get_values(&dyn_keys)
            .into_iter()
            .map(|r| r.ok().map(downcast_value::<K>))
            .collect()
    }

    /// Java: Environment.valuesMissing
    pub fn values_missing(&self) -> bool {
        self.values_missing
    }
}

fn downcast_value<K: Key>(v: SkyValue) -> Arc<K::Value> {
    v.downcast::<K::Value>()
        .unwrap_or_else(|_| panic!("value of {:?} has the wrong type", K::FUNCTION))
}

#[cfg(test)]
mod tests;
