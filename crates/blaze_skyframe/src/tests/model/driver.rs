//! The driver: the real incremental [`Evaluator`] against the reference model, for generated
//! scenarios (the contract is the doc comment of `tests/model.rs`).
//!
//! For each seed and each of four modes (parallelism 1 or more, `EventFilter::NO_STORAGE` or
//! `FULL_STORAGE`) one evaluator runs every step of the scenario; after every evaluation the
//! driver checks, in this order (see `docs/skyframe-incremental.d/model-test.md`):
//!
//! 1. the roots' results against the from-scratch semantics ([`eval_from_scratch`]);
//! 2. the roots' results against a fresh evaluator built from the current spec (equal where the
//!    outcome is unique, else both in the allowed set);
//! 3. the results, `ran`, the progress receiver's view and the events against the model where it
//!    is exact, `ran` against its bounds elsewhere;
//! 4. with `FULL_STORAGE`, the stored events against a fresh `FULL_STORAGE` evaluator;
//! 5. the graph invariants, and, while the model is certain, the whole graph against the model's.
//!
//! Seeds: `BLAZE_SKYFRAME_MODEL_SEEDS` (a count, default [`DEFAULT_SEEDS`]) from
//! `BLAZE_SKYFRAME_MODEL_SEED_START` (default 0). A failure prints the seed, the mode, the step
//! and the scenario.

use super::generate::{GenConfig, generate_with};
use super::graph::{MKey, MState, ModelGraph, ModelRun};
use super::scratch::{Expected, Kg, distinct_roots, eval_from_scratch};
use super::spec::{
    Dep, DepRead, Edit, ErrClass, GraphSpec, NodeId, NodeKind, ProgramEnv, RootResult, RunResult,
    Scenario, exception_message, run_program, stored_event_text, warning_text,
};
use crate::abstract_in_memory_node_entry::AbstractInMemoryNodeEntry;
use crate::evaluator::ErrorTransienceKey;
use crate::in_memory_node_entry::InMemoryNodeEntry;
use crate::node_entry::NodeEntry;
use crate::reverse_deps_utility::{Op, ReverseDepsUtility};
use crate::tests::SomeErrorException;
use crate::version::Version;
use crate::{
    ComputeResult, DirtyType, Environment, ErrorInfo, EvaluationContext,
    EvaluationProgressReceiver, EvaluationResult, EvaluationState, Evaluator, EvaluatorBuilder,
    EventFilter, FunctionHermeticity, GroupedDeps, Interrupted, Key, SkyFunction,
    SkyFunctionException, SkyFunctionName, SkyKey, SkyValue, Transience,
};
use blaze_events::{Event, EventKind, StoredEventHandler};
use indexmap::IndexSet;
use parking_lot::Mutex;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, OnceLock, Weak};

/// Seeds of the normal test run (each runs in four modes).
const DEFAULT_SEEDS: u64 = 500;

/// Key of a node whose values compare equal by value (`NodeSpec::comparable`).
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct ModelKey(pub(crate) NodeId);

impl fmt::Debug for ModelKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "n{}", self.0)
    }
}

impl Key for ModelKey {
    type Value = i64;
    const FUNCTION: SkyFunctionName = SkyFunctionName("MODEL");
    // Java: GraphTester.NonHermeticKey (CHANGE-invalidating a hermetic key throws,
    // InvalidatingNodeVisitor.java:467).
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Nonhermetic;
    fn values_equal(old: &i64, new: &i64) -> bool {
        old == new
    }
}

/// Key of a node whose values are never equal (Java identity equality).
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct OpaqueModelKey(pub(crate) NodeId);

impl fmt::Debug for OpaqueModelKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "n{}", self.0)
    }
}

impl Key for OpaqueModelKey {
    type Value = i64;
    const FUNCTION: SkyFunctionName = SkyFunctionName("MODEL_OPAQUE");
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Nonhermetic;
}

fn node_of(key: &SkyKey) -> Option<NodeId> {
    if let Some(k) = key.downcast::<ModelKey>() {
        return Some(k.0);
    }
    key.downcast::<OpaqueModelKey>().map(|k| k.0)
}

fn mkey_of(key: &SkyKey) -> MKey {
    if let Some(n) = node_of(key) {
        return MKey::Node(n);
    }
    assert!(
        key.downcast::<ErrorTransienceKey>().is_some(),
        "unexpected key {key:?}"
    );
    MKey::Et
}

fn value_i64(v: &SkyValue) -> i64 {
    *v.downcast_ref::<i64>().expect("model values are i64")
}

/// What the function reads and records: the current spec (shared with the driver) and this
/// evaluation's runs.
struct Shared {
    comparable: Vec<bool>,
    spec: Mutex<GraphSpec>,
    runs: Mutex<BTreeMap<NodeId, u32>>,
    counter: AtomicU32,
    /// `interrupt_after_runs`, or `u32::MAX`.
    interrupt_at: AtomicU32,
    interrupt_fired: AtomicBool,
    ev: OnceLock<Weak<Evaluator>>,
}

impl Shared {
    fn new(spec: &GraphSpec) -> Arc<Shared> {
        Arc::new(Shared {
            comparable: spec.nodes.iter().map(|n| n.comparable).collect(),
            spec: Mutex::new(spec.clone()),
            runs: Mutex::new(BTreeMap::new()),
            counter: AtomicU32::new(0),
            interrupt_at: AtomicU32::new(u32::MAX),
            interrupt_fired: AtomicBool::new(false),
            ev: OnceLock::new(),
        })
    }

    fn key(&self, n: NodeId) -> SkyKey {
        if self.comparable[n as usize] {
            SkyKey::new(ModelKey(n))
        } else {
            SkyKey::new(OpaqueModelKey(n))
        }
    }

    fn reset(&self, interrupt_at: Option<u32>) {
        self.runs.lock().clear();
        self.counter.store(0, Ordering::SeqCst);
        self.interrupt_at
            .store(interrupt_at.unwrap_or(u32::MAX), Ordering::SeqCst);
        self.interrupt_fired.store(false, Ordering::SeqCst);
    }

    fn ran(&self) -> BTreeSet<NodeId> {
        self.runs.lock().keys().copied().collect()
    }
}

struct ModelFunction(Arc<Shared>);

struct Adapter<'a, 'e> {
    env: &'a mut Environment<'e>,
    shared: &'a Shared,
}

impl ProgramEnv for Adapter<'_, '_> {
    type Stop = Interrupted;

    fn request(&mut self, group: &[Dep]) -> Result<Vec<DepRead>, Interrupted> {
        let read = |v: Option<SkyValue>| match v {
            Some(v) => DepRead::Value(value_i64(&v)),
            None => DepRead::Missing,
        };
        if let [d] = group {
            let k = self.shared.key(d.node);
            let r = match d.catch {
                None => read(self.env.get_value_dyn(&k)?),
                Some(_) => match self.env.get_value_or_throw_dyn::<SomeErrorException>(&k)? {
                    Ok(v) => read(v),
                    Err(_) => DepRead::Exception,
                },
            };
            return Ok(vec![r]);
        }
        let keys: Vec<SkyKey> = group.iter().map(|d| self.shared.key(d.node)).collect();
        let result = self.env.get_values_and_exceptions(&keys)?;
        Ok(group
            .iter()
            .zip(&keys)
            .map(|(d, k)| match d.catch {
                None => read(result.get(k)),
                Some(_) => match result.get_or_throw::<SomeErrorException>(k) {
                    Ok(v) => read(v),
                    Err(_) => DepRead::Exception,
                },
            })
            .collect())
    }

    fn values_missing(&self) -> bool {
        self.env.values_missing()
    }
}

impl SkyFunction for ModelFunction {
    fn compute(&self, key: &SkyKey, env: &mut Environment<'_>) -> ComputeResult {
        let shared = &*self.0;
        let n = node_of(key).expect("model key");
        let i = shared.counter.fetch_add(1, Ordering::SeqCst);
        *shared.runs.lock().entry(n).or_default() += 1;
        if i == shared.interrupt_at.load(Ordering::SeqCst) {
            shared.interrupt_fired.store(true, Ordering::SeqCst);
            if let Some(ev) = shared.ev.get().and_then(Weak::upgrade) {
                ev.interrupt();
            }
        }
        let node = shared.spec.lock().node(n).clone();
        let program = match node.kind {
            NodeKind::Leaf(v) => return Ok(Some(Arc::new(v))),
            NodeKind::Function(p) => p,
        };
        let out = {
            let mut adapter = Adapter { env, shared };
            run_program(&program, &mut adapter)?
        };
        if let Some(pre) = out.completed {
            if program.warning {
                env.listener().handle(&Event::warn(warning_text(n, pre)));
            }
            if program.stored_event {
                env.listener()
                    .handle(&Event::error(stored_event_text(n, pre)));
            }
        }
        match out.result {
            RunResult::Value(v) => Ok(Some(Arc::new(v))),
            RunResult::Throw { transient } => Err(SkyFunctionException::new(
                SomeErrorException(exception_message(n)),
                if transient {
                    Transience::Transient
                } else {
                    Transience::Persistent
                },
            )
            .into()),
            RunResult::Missing => Ok(None),
        }
    }
}

/// The progress receiver's view of one evaluation.
#[derive(Default, Debug)]
struct Rec {
    dirtied: BTreeSet<MKey>,
    deleted: BTreeSet<MKey>,
    /// `changePruned` callbacks (the model's `marked_clean`).
    marked_clean: BTreeSet<NodeId>,
    /// `evaluated(SUCCESS_VERSION_UNCHANGED, directDeps != null)` (the model's `change_pruned`).
    change_pruned: BTreeSet<NodeId>,
}

#[derive(Default)]
struct Recorder(Mutex<Rec>);

impl EvaluationProgressReceiver for Recorder {
    fn dirtied(&self, sky_key: &SkyKey, _dirty_type: DirtyType) {
        self.0.lock().dirtied.insert(mkey_of(sky_key));
    }

    fn deleted(&self, sky_key: &SkyKey) {
        self.0.lock().deleted.insert(mkey_of(sky_key));
    }

    fn evaluated(
        &self,
        sky_key: &SkyKey,
        state: EvaluationState,
        _new_value: Option<&SkyValue>,
        _new_error: Option<&ErrorInfo>,
        direct_deps: Option<&GroupedDeps>,
    ) {
        if direct_deps.is_some()
            && state == EvaluationState::SuccessVersionUnchanged
            && let Some(n) = node_of(sky_key)
        {
            self.0.lock().change_pruned.insert(n);
        }
    }

    fn change_pruned(&self, sky_key: &SkyKey) {
        if let Some(n) = node_of(sky_key) {
            self.0.lock().marked_clean.insert(n);
        }
    }
}

fn build_evaluator(
    shared: &Arc<Shared>,
    recorder: Option<Arc<Recorder>>,
    full_storage: bool,
) -> Arc<Evaluator> {
    let mut b = EvaluatorBuilder::new()
        .function(ModelKey::FUNCTION, ModelFunction(shared.clone()))
        .function(OpaqueModelKey::FUNCTION, ModelFunction(shared.clone()))
        .keep_edges(true)
        .event_filter(Arc::new(if full_storage {
            <dyn EventFilter>::FULL_STORAGE
        } else {
            <dyn EventFilter>::NO_STORAGE
        }));
    if let Some(r) = recorder {
        b = b.progress_receiver(r);
    }
    let ev = Arc::new(b.build());
    shared
        .ev
        .set(Arc::downgrade(&ev))
        .expect("one evaluator per Shared");
    ev
}

fn ctx(
    keep_going: bool,
    parallelism: usize,
    handler: Arc<StoredEventHandler>,
) -> EvaluationContext {
    EvaluationContext {
        keep_going,
        parallelism,
        event_handler: handler,
        thread_stack_size: 16 << 20,
        ..EvaluationContext::default()
    }
}

/// An error as compared between two real evaluators. With cycle infos, the exception (and its
/// transience) is not compared: the model's notes ("Known gaps") say it depends on how far a
/// dirty node got in checking its deps before it was stuck.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ErrOutcome {
    exception: Option<String>,
    transient: Option<bool>,
    cycles: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Outcome {
    value: Option<i64>,
    error: Option<ErrOutcome>,
}

fn err_outcome(e: &ErrorInfo) -> ErrOutcome {
    let cycles: BTreeSet<String> = e
        .get_cycle_info()
        .iter()
        .map(|c| format!("{:?} -> {:?}", c.get_path_to_cycle(), c.get_cycle()))
        .collect();
    if cycles.is_empty() {
        ErrOutcome {
            exception: e.get_exception().map(|x| x.to_string()),
            transient: Some(e.is_transitively_transient()),
            cycles,
        }
    } else {
        ErrOutcome {
            exception: None,
            transient: None,
            cycles,
        }
    }
}

fn outcomes(r: &EvaluationResult, shared: &Shared, roots: &[NodeId]) -> Vec<(NodeId, Outcome)> {
    distinct_roots(roots)
        .into_iter()
        .map(|n| {
            let k = shared.key(n);
            (
                n,
                Outcome {
                    value: r.get_dyn(&k).map(value_i64),
                    error: r.get_error(&k).map(err_outcome),
                },
            )
        })
        .collect()
}

/// Checks that `cycle` (path to the cycle, then the cycle) is a cycle of the current spec
/// reached from `root`: every edge is one a program may request.
fn check_cycle_is_real(spec: &GraphSpec, root: NodeId, cycle: &str) -> Result<(), String> {
    let parse = |s: &str| -> Vec<NodeId> {
        let s = s.trim().trim_start_matches('[').trim_end_matches(']');
        s.split(", ")
            .filter(|x| !x.is_empty())
            .map(|x| x.trim_start_matches('n').parse().expect("model key"))
            .collect()
    };
    let (path, cyc) = cycle.split_once(" -> ").expect("cycle text");
    let (path, cyc) = (parse(path), parse(cyc));
    let mut walk: Vec<NodeId> = path.clone();
    walk.extend(&cyc);
    walk.push(cyc[0]);
    if walk[0] != root {
        return Err(format!("cycle {cycle} does not start at root n{root}"));
    }
    for w in walk.windows(2) {
        if !spec.all_possible_deps(w[0]).contains(&w[1]) {
            return Err(format!("cycle {cycle}: n{} cannot request n{}", w[0], w[1]));
        }
    }
    Ok(())
}

/// The incremental result against a fresh evaluator's, where the outcome is unique.
///
/// Narrowed for cycle infos (counted in `Coverage::cycle_infos_differ`): which cycles Java's
/// `SimpleCycleDetector` reports depends on the graph the evaluation leaves, not only on the
/// spec. It skips done nodes and takes their stored errors (SimpleCycleDetector.java:196-199,
/// :253-257 `getChildrenErrorsForCycleChecking`), walks each in-flight node's temporary direct
/// deps (:286-305), which for a dirty node are the last-build groups it re-requested so far
/// (AbstractParallelEvaluator.java:233-380), and in keep_going drops the unvisited children of a
/// node found on a cycle (:238-248 `removeDescendantsOfCycleValue`). An incremental evaluation
/// that finds a dep done with a cycle error (kept from an earlier evaluation) or a dirty node
/// checking old groups therefore reports other (more or fewer) cycles than a fresh one. There
/// both must have cycles, and every reported cycle must be a real cycle of the current spec
/// reached from the root.
fn compare_with_fresh(
    inc: &[(NodeId, Outcome)],
    fresh: &[(NodeId, Outcome)],
    spec: &GraphSpec,
    cov: &mut Coverage,
) -> Result<(), String> {
    for ((root, a), (root_b, b)) in inc.iter().zip(fresh) {
        assert_eq!(root, root_b);
        let what = || format!("incremental vs fresh evaluator, root n{root}: {a:?} vs {b:?}");
        if a.value != b.value {
            return Err(what());
        }
        match (&a.error, &b.error) {
            (None, None) => {}
            (Some(x), Some(y)) if !x.cycles.is_empty() && !y.cycles.is_empty() => {
                if x.cycles != y.cycles {
                    cov.cycle_infos_differ += 1;
                    for c in x.cycles.iter().chain(&y.cycles) {
                        check_cycle_is_real(spec, *root, c)
                            .map_err(|e| format!("{}: {e}", what()))?;
                    }
                }
            }
            (Some(x), Some(y)) if x == y => {}
            _ => return Err(what()),
        }
    }
    Ok(())
}

fn parse_origin(message: &str) -> NodeId {
    message
        .strip_prefix('n')
        .and_then(|s| s.strip_suffix(" failed"))
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| panic!("not a model exception: {message:?}"))
}

/// The contract's mapping of a result to the model's [`RootResult`]s.
fn root_results(
    r: &EvaluationResult,
    shared: &Shared,
    roots: &[NodeId],
) -> Vec<(NodeId, RootResult)> {
    distinct_roots(roots)
        .into_iter()
        .map(|n| {
            let k = shared.key(n);
            let rr = if let Some(v) = r.get_dyn(&k) {
                RootResult::Value(value_i64(v))
            } else if let Some(e) = r.get_error(&k) {
                if !e.get_cycle_info().is_empty() {
                    RootResult::Error(ErrClass::Cycle)
                } else {
                    let msg = e.get_exception().expect("an exception").to_string();
                    RootResult::Error(ErrClass::Exception {
                        origin: parse_origin(&msg),
                        transient: e.is_transitively_transient(),
                    })
                }
            } else {
                RootResult::Absent
            };
            (n, rr)
        })
        .collect()
}

fn events_of(h: &StoredEventHandler, kind: EventKind) -> BTreeSet<String> {
    h.get_events()
        .iter()
        .filter(|e| e.get_kind() == kind)
        .map(|e| e.get_message().to_string())
        .collect()
}

fn event_node(message: &str) -> NodeId {
    message
        .strip_prefix('n')
        .and_then(|s| s.split(' ').next())
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| panic!("not a model event: {message:?}"))
}

/// A fresh evaluator from `spec` (every leaf injected) evaluating `roots`.
struct Fresh {
    shared: Arc<Shared>,
    result: EvaluationResult,
    events: Arc<StoredEventHandler>,
}

fn fresh_eval(spec: &GraphSpec, roots: &[NodeId], keep_going: bool, p: usize, full: bool) -> Fresh {
    let shared = Shared::new(spec);
    let ev = build_evaluator(&shared, None, full);
    for (n, v) in spec.leaves() {
        ev.inject_dyn(shared.key(n), Arc::new(v));
    }
    let events = Arc::new(StoredEventHandler::new());
    let keys: Vec<SkyKey> = roots.iter().map(|n| shared.key(*n)).collect();
    let result = ev
        .evaluate(&keys, &ctx(keep_going, p, events.clone()))
        .expect("a fresh evaluation is not interrupted");
    Fresh {
        shared,
        result,
        events,
    }
}

// ---------------------------------------------------------------------------------------------
// The graph as the driver reads it.

fn ver(v: &Version) -> i64 {
    match v {
        Version::Minimal => -1,
        Version::Int(i) => i.get_val(),
        Version::Constant => panic!("constant version in an incremental graph"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GState {
    Done,
    /// Marked dirty, not evaluating.
    Dirty,
    InFlight,
}

#[derive(Debug)]
struct GNode {
    state: GState,
    value: Option<i64>,
    error: Option<ErrorInfo>,
    last_changed: i64,
    last_evaluated: i64,
    /// Done: its deps; dirty: the deps of its last build; in flight: none.
    deps: Vec<Vec<MKey>>,
    rdeps: Vec<MKey>,
}

fn groups(g: &GroupedDeps) -> Vec<Vec<MKey>> {
    g.groups()
        .iter()
        .map(|grp| grp.iter().map(mkey_of).collect())
        .collect()
}

/// The reverse deps of an entry without consolidating it: the stored ones with the delayed
/// operations applied (ReverseDepsUtility.java:288 consolidateData, without the checks).
fn rdeps_of(
    e: &crate::incremental_in_memory_node_entry::IncrementalInMemoryNodeEntry,
) -> Vec<MKey> {
    let mut set: IndexSet<SkyKey> = e.reverse_deps.iter().cloned().collect();
    if let Some(data) = &e.reverse_deps_data_to_consolidate {
        let bare = ReverseDepsUtility::get_op_to_store_bare(e);
        for k in data {
            match k.op(bare) {
                Op::Add => {
                    set.insert(k.key().clone());
                }
                Op::Remove => {
                    set.shift_remove(k.key());
                }
                Op::Check => {}
            }
        }
    }
    set.iter().map(mkey_of).collect()
}

fn read_graph(ev: &Evaluator) -> BTreeMap<MKey, GNode> {
    let mut out = BTreeMap::new();
    for (k, node) in ev.graph.get_all_node_entries() {
        let st = node.lock();
        let InMemoryNodeEntry::Incremental(e) = &st.entry else {
            panic!("keep_edges(true) makes incremental entries");
        };
        let (state, deps) = if e.is_done() {
            (
                GState::Done,
                groups(e.get_compressed_direct_deps_for_done_entry()),
            )
        } else {
            let dbs = e.dbs().expect("not done: building state");
            if dbs.is_evaluating() {
                (GState::InFlight, vec![])
            } else {
                (
                    GState::Dirty,
                    dbs.get_last_build_direct_deps()
                        .map(groups)
                        .unwrap_or_default(),
                )
            }
        };
        let (value, error) = if e.is_done() {
            let mk = mkey_of(&k);
            let v = match mk {
                MKey::Et => None,
                MKey::Node(_) => st.entry.get_value().map(|v| value_i64(&v)),
            };
            (v, st.entry.get_error_info().cloned())
        } else {
            (None, None)
        };
        out.insert(
            mkey_of(&k),
            GNode {
                state,
                value,
                error,
                last_changed: ver(e.version.last_changed()),
                last_evaluated: ver(e.version.last_evaluated()),
                deps,
                rdeps: rdeps_of(e),
            },
        );
    }
    out
}

/// Invariants of the graph between evaluations (`graph_version`: the last evaluation's).
fn check_invariants(
    g: &BTreeMap<MKey, GNode>,
    graph_version: i64,
    receiver_inflight: &BTreeSet<MKey>,
) -> Result<(), String> {
    let has_rdep = |d: &MKey, p: &MKey| g.get(d).is_some_and(|n| n.rdeps.contains(p));
    for (k, n) in g {
        for p in &n.rdeps {
            let Some(pn) = g.get(p) else {
                return Err(format!(
                    "{k:?} has reverse dep {p:?}, which is not in the graph"
                ));
            };
            match pn.state {
                GState::Done | GState::Dirty => {
                    if !pn.deps.iter().flatten().any(|d| d == k) {
                        return Err(format!(
                            "{k:?} has reverse dep {p:?} ({:?}) whose deps {:?} do not have it",
                            pn.state, pn.deps
                        ));
                    }
                }
                GState::InFlight => {}
            }
        }
        match n.state {
            GState::Done => {
                if !(n.last_changed <= n.last_evaluated && n.last_evaluated <= graph_version) {
                    return Err(format!(
                        "{k:?}: versions {} / {} at graph version {graph_version}",
                        n.last_changed, n.last_evaluated
                    ));
                }
                for d in n.deps.iter().flatten() {
                    let Some(dn) = g.get(d) else {
                        return Err(format!("done {k:?} has dep {d:?}, not in the graph"));
                    };
                    if dn.state != GState::Done {
                        return Err(format!("done {k:?} has dep {d:?} that is {:?}", dn.state));
                    }
                    if !has_rdep(d, k) {
                        return Err(format!("done {k:?} has dep {d:?} without the reverse dep"));
                    }
                    if dn.last_changed > n.last_evaluated {
                        return Err(format!(
                            "done {k:?} last evaluated at {} has dep {d:?} changed at {}",
                            n.last_evaluated, dn.last_changed
                        ));
                    }
                }
            }
            GState::Dirty => {
                if !(n.last_changed <= n.last_evaluated && n.last_evaluated <= graph_version) {
                    return Err(format!(
                        "dirty {k:?}: versions {} / {} at graph version {graph_version}",
                        n.last_changed, n.last_evaluated
                    ));
                }
                for d in n.deps.iter().flatten() {
                    if !has_rdep(d, k) {
                        return Err(format!(
                            "dirty {k:?} has last-build dep {d:?} without the reverse dep"
                        ));
                    }
                }
            }
            GState::InFlight => {}
        }
    }
    let in_flight: BTreeSet<MKey> = g
        .iter()
        .filter(|(_, n)| n.state == GState::InFlight)
        .map(|(k, _)| *k)
        .collect();
    if &in_flight != receiver_inflight {
        return Err(format!(
            "nodes in flight {in_flight:?}, the receiver's in-flight keys {receiver_inflight:?}"
        ));
    }
    Ok(())
}

/// The real graph against the model graph (while the model is certain).
///
/// With FULL_STORAGE the model's change pruning is not Java's (it does not simulate
/// `NestedSet.shallowEquals`, ValueWithMetadata.java:105-124): a node the model marks clean or
/// prunes may be re-run, and a dirty node the model leaves checking old groups in a cycle may be
/// rebuilt with the current program's deps. Then only the values and errors of nodes done in
/// both graphs are compared; `Ok(true)` reports that the structure (presence, state, versions,
/// deps) differed.
fn check_graph_against_model(
    g: &BTreeMap<MKey, GNode>,
    model: &ModelGraph,
    spec: &GraphSpec,
    full_storage: bool,
) -> Result<bool, String> {
    let keys = (0..spec.len()).map(MKey::Node).chain([MKey::Et]);
    let mut structure_differs = false;
    let mut structural = |m: String| -> Result<(), String> {
        if full_storage {
            structure_differs = true;
            Ok(())
        } else {
            Err(m)
        }
    };
    for k in keys {
        match (g.get(&k), model.node(k)) {
            (None, None) => {}
            (Some(_), None) => structural(format!("{k:?} in the graph, not in the model"))?,
            (None, Some(_)) => structural(format!("{k:?} in the model, not in the graph"))?,
            (Some(n), Some(m)) => {
                let what = |s: &str| format!("{k:?}: {s}\n  graph {n:?}\n  model {m:?}");
                let (mstate, mdeps) = match &m.state {
                    MState::Done { deps, .. } => (GState::Done, deps.clone()),
                    MState::Dirty { last_deps, .. } => (GState::Dirty, last_deps.clone()),
                    MState::InFlight(_) => (GState::InFlight, vec![]),
                };
                if n.state != mstate {
                    structural(what("state differs"))?;
                    continue;
                }
                if mstate == GState::InFlight {
                    continue;
                }
                if (n.last_changed, n.last_evaluated) != (m.last_changed, m.last_evaluated) {
                    structural(what("versions differ"))?;
                }
                if n.deps != mdeps {
                    structural(what("deps differ"))?;
                }
                if let MState::Done { value, .. } = &m.state
                    && k != MKey::Et
                {
                    if n.value != value.value {
                        return Err(what("value differs"));
                    }
                    let real = n.error.as_ref().map(|e| {
                        if e.get_cycle_info().is_empty() {
                            let msg = e.get_exception().expect("exception").to_string();
                            ErrClass::Exception {
                                origin: parse_origin(&msg),
                                transient: e.is_transitively_transient(),
                            }
                        } else {
                            ErrClass::Cycle
                        }
                    });
                    if real != value.error.map(|e| e.class()) {
                        return Err(what("error differs"));
                    }
                }
            }
        }
    }
    Ok(structure_differs)
}

// ---------------------------------------------------------------------------------------------
// The run.

#[derive(Clone, Copy, Debug)]
struct Mode {
    /// `None`: the request's parallelism raised to at least 2.
    sequential: bool,
    full_storage: bool,
}

const MODES: [Mode; 4] = [
    Mode {
        sequential: true,
        full_storage: false,
    },
    Mode {
        sequential: false,
        full_storage: false,
    },
    Mode {
        sequential: true,
        full_storage: true,
    },
    Mode {
        sequential: false,
        full_storage: true,
    },
];

#[derive(Default, Debug, Clone)]
struct Coverage {
    /// The step being run (for a panic report).
    current_step: usize,
    scenarios: u64,
    evaluations: u64,
    exact: u64,
    edits: BTreeMap<&'static str, u64>,
    change_pruned_rebuild: u64,
    marked_clean: u64,
    deleted_in_flight: u64,
    dirty_in_cycle: u64,
    transient_error_rerun: u64,
    injected_equal_noop: u64,
    no_keep_going_stop: u64,
    interrupted: u64,
    interrupt_fired_but_completed: u64,
    cached_error_stop: u64,
    unique_compared_with_fresh: u64,
    cycle_infos_differ: u64,
    full_storage_graph_differs: u64,
    full_storage_invalidation_not_compared: u64,
    loose_compared_with_fresh: u64,
    full_storage_events_compared: u64,
    graph_compared_with_model: u64,
}

impl Coverage {
    fn add(&mut self, o: &Coverage) {
        self.scenarios += o.scenarios;
        self.evaluations += o.evaluations;
        self.exact += o.exact;
        for (k, v) in &o.edits {
            *self.edits.entry(k).or_default() += v;
        }
        self.change_pruned_rebuild += o.change_pruned_rebuild;
        self.marked_clean += o.marked_clean;
        self.deleted_in_flight += o.deleted_in_flight;
        self.dirty_in_cycle += o.dirty_in_cycle;
        self.transient_error_rerun += o.transient_error_rerun;
        self.injected_equal_noop += o.injected_equal_noop;
        self.no_keep_going_stop += o.no_keep_going_stop;
        self.interrupted += o.interrupted;
        self.interrupt_fired_but_completed += o.interrupt_fired_but_completed;
        self.cached_error_stop += o.cached_error_stop;
        self.unique_compared_with_fresh += o.unique_compared_with_fresh;
        self.cycle_infos_differ += o.cycle_infos_differ;
        self.full_storage_graph_differs += o.full_storage_graph_differs;
        self.full_storage_invalidation_not_compared += o.full_storage_invalidation_not_compared;
        self.loose_compared_with_fresh += o.loose_compared_with_fresh;
        self.full_storage_events_compared += o.full_storage_events_compared;
        self.graph_compared_with_model += o.graph_compared_with_model;
    }
}

fn edit_name(e: &Edit) -> &'static str {
    match e {
        Edit::SetLeaf { .. } => "set_leaf",
        Edit::SetProgram { .. } => "set_program",
        Edit::Invalidate { .. } => "invalidate",
        Edit::Delete { .. } => "delete",
        Edit::DeleteDirty { .. } => "delete_dirty",
        Edit::InvalidateTransientErrors => "invalidate_transient_errors",
    }
}

fn subset_of<T: Ord + fmt::Debug>(
    a: &BTreeSet<T>,
    b: &BTreeSet<T>,
    what: &str,
) -> Result<(), String> {
    if a.is_subset(b) {
        Ok(())
    } else {
        Err(format!("{what}: {a:?} not within {b:?}"))
    }
}

fn equal<T: PartialEq + fmt::Debug>(a: &T, b: &T, what: &str) -> Result<(), String> {
    if a == b {
        Ok(())
    } else {
        Err(format!("{what}: engine {a:?}, expected {b:?}"))
    }
}

/// Runs one scenario in one mode. `Err((step, message))` on the first failed check.
fn run_scenario(sc: &Scenario, mode: Mode, cov: &mut Coverage) -> Result<(), (usize, String)> {
    let shared = Shared::new(&sc.spec);
    let recorder = Arc::new(Recorder::default());
    let ev = build_evaluator(&shared, Some(recorder.clone()), mode.full_storage);
    let mut model = ModelGraph::new();
    let mut spec = sc.spec.clone();
    let mut prev_in_flight: BTreeSet<MKey> = BTreeSet::new();
    let mut full_storage_deleted_dirty = false;
    cov.scenarios += 1;
    for (i, (edits, req)) in sc.steps.iter().enumerate() {
        cov.current_step = i;
        let fail = |m: String| (i, m);
        // State before the edits, for the coverage counters.
        let before = read_graph(&ev);
        for e in edits {
            *cov.edits.entry(edit_name(e)).or_default() += 1;
            model.apply_edit(&mut spec, e);
            *shared.spec.lock() = spec.clone();
            match e {
                Edit::SetLeaf { node, value } => {
                    if shared.comparable[*node as usize]
                        && before.get(&MKey::Node(*node)).is_some_and(|n| {
                            n.state == GState::Done && n.value == Some(*value) && n.error.is_none()
                        })
                    {
                        cov.injected_equal_noop += 1;
                    }
                    ev.inject_dyn(shared.key(*node), Arc::new(*value));
                }
                Edit::SetProgram { node, .. } | Edit::Invalidate { node } => {
                    ev.invalidate([shared.key(*node)]);
                }
                Edit::Delete { nodes } => {
                    ev.delete(|k, _| node_of(k).is_some_and(|n| nodes.contains(&n)));
                }
                Edit::DeleteDirty { age } => ev.delete_dirty(*age as i64),
                Edit::InvalidateTransientErrors => ev.invalidate_transient_errors(),
            }
        }
        let transient_before: BTreeSet<NodeId> = before
            .iter()
            .filter_map(|(k, n)| match k {
                MKey::Node(id)
                    if n.error
                        .as_ref()
                        .is_some_and(|e| e.is_transitively_transient()) =>
                {
                    Some(*id)
                }
                _ => None,
            })
            .collect();
        let dirty_before: BTreeSet<MKey> = before
            .iter()
            .filter(|(_, n)| n.state == GState::Dirty)
            .map(|(k, _)| *k)
            .collect();

        let trace = std::env::var_os("BLAZE_SKYFRAME_MODEL_TRACE").is_some();
        if trace {
            eprintln!("--- {mode:?} step {i} before evaluate (after edits recorded):");
            for (k, n) in &before {
                eprintln!("  {k:?}: {n:?}");
            }
        }

        // Evaluate.
        ev.emitted_event_state().clear();
        shared.reset(req.interrupt_after_runs);
        *recorder.0.lock() = Rec::default();
        let events = Arc::new(StoredEventHandler::new());
        let parallelism = if mode.sequential {
            1
        } else {
            req.parallelism.max(2)
        };
        let keys: Vec<SkyKey> = req.roots.iter().map(|n| shared.key(*n)).collect();
        let certain_before = model.is_certain();
        let result = ev.evaluate(&keys, &ctx(req.keep_going, parallelism, events.clone()));
        let fired = shared.interrupt_fired.load(Ordering::SeqCst);
        ev.clear_interrupt();
        let ran = shared.ran();
        let rec = std::mem::take(&mut *recorder.0.lock());
        if trace {
            eprintln!(
                "--- step {i}: result {:?}\n  ran {ran:?} receiver {rec:?}",
                result.as_ref().map(|r| (r.values.len(), &r.errors))
            );
        }
        let interrupted = result.is_err() || fired;
        let run: ModelRun = if interrupted {
            model.evaluate_interrupted(&spec, req)
        } else {
            model.evaluate(&spec, req)
        };
        cov.evaluations += 1;
        cov.exact += run.exact as u64;
        if fired && result.is_ok() {
            cov.interrupt_fired_but_completed += 1;
        }
        if result.is_err() {
            cov.interrupted += 1;
        }
        if run.cached_error_stop {
            cov.cached_error_stop += 1;
        }

        // In-flight nodes of the last evaluation are deleted at the start of this one
        // (AbstractInMemoryMemoizingEvaluator.java:134).
        subset_of(
            &prev_in_flight,
            &rec.deleted,
            "in flight before, deleted now",
        )
        .map_err(fail)?;
        if !prev_in_flight.is_empty() {
            cov.deleted_in_flight += 1;
        }

        // The start of the evaluation: dirtied and deleted keys. With FULL_STORAGE the
        // versions differ from the model's (stricter change pruning, see `ran`), and
        // `deleteDirty` picks nodes by version (AbstractInMemoryMemoizingEvaluator.java:243-255
        // `getVersion().atMost(threshold)`): from the first `DeleteDirty` on, the graphs may
        // hold different nodes.
        if mode.full_storage && edits.iter().any(|e| matches!(e, Edit::DeleteDirty { .. })) {
            full_storage_deleted_dirty = true;
        }
        if certain_before && full_storage_deleted_dirty {
            cov.full_storage_invalidation_not_compared += 1;
        }
        if certain_before && !full_storage_deleted_dirty {
            equal(&rec.dirtied, &run.dirtied, "dirtied").map_err(fail)?;
            equal(&rec.deleted, &run.deleted, "deleted").map_err(fail)?;
        }

        // Functions that ran.
        if mode.full_storage {
            // FULL_STORAGE makes change pruning stricter (ValueWithMetadata.java:105-124:
            // `ValueWithEvents.equals` needs `NestedSet.shallowEquals` of the events), which the
            // model does not simulate: `ran` is compared by bounds only. Lower bound: a node the
            // model re-runs is changed or has a dep whose value changed, which stays so when
            // fewer nodes are pruned.
            subset_of(&ran, &run.ran_upper, "ran (upper bound)").map_err(fail)?;
            if !full_storage_deleted_dirty {
                let (lower, _) = run.ran_bounds();
                subset_of(&lower, &ran, "ran (lower bound)").map_err(fail)?;
            }
        } else if run.exact {
            equal(&ran, &run.ran, "ran").map_err(fail)?;
        } else {
            let (lower, upper) = run.ran_bounds();
            subset_of(&lower, &ran, "ran (lower bound)").map_err(fail)?;
            subset_of(&ran, &upper, "ran (upper bound)").map_err(fail)?;
        }

        let warnings = events_of(&events, EventKind::Warning);
        let errors = events_of(&events, EventKind::Error);
        if !mode.full_storage {
            // NO_STORAGE: only functions that ran report events.
            for m in warnings.iter().chain(&errors) {
                if !ran.contains(&event_node(m)) {
                    return Err(fail(format!("event {m:?} of a function that did not run")));
                }
            }
        }

        // An evaluation that returned normally although a function interrupted the evaluator
        // (the interrupt came after the last request) is checked like any other, except
        // against the model, which was told the evaluation was interrupted.
        if let Ok(result) = &result {
            let actual = root_results(result, &shared, &req.roots);
            let expected: Expected = eval_from_scratch(&spec, &req.roots, req.keep_going);
            expected
                .check(&actual, result.has_error())
                .map_err(|e| fail(format!("engine vs from-scratch semantics: {e}")))?;
            if !req.keep_going && result.has_error() {
                cov.no_keep_going_stop += 1;
            }

            // A fresh evaluator.
            let fresh = fresh_eval(
                &spec,
                &req.roots,
                req.keep_going,
                parallelism,
                mode.full_storage,
            );
            let fresh_actual = root_results(&fresh.result, &fresh.shared, &req.roots);
            expected
                .check(&fresh_actual, fresh.result.has_error())
                .map_err(|e| fail(format!("fresh engine vs from-scratch semantics: {e}")))?;
            if expected.unique && !run.cached_error_stop {
                cov.unique_compared_with_fresh += 1;
                compare_with_fresh(
                    &outcomes(result, &shared, &req.roots),
                    &outcomes(&fresh.result, &fresh.shared, &req.roots),
                    &spec,
                    cov,
                )
                .map_err(fail)?;
                equal(
                    &result.has_error(),
                    &fresh.result.has_error(),
                    "has_error vs fresh",
                )
                .map_err(fail)?;
            } else {
                cov.loose_compared_with_fresh += 1;
            }

            // Against the model.
            if run.exact {
                let results = run.results.as_ref().expect("exact runs have results");
                equal(&actual, results, "results vs model").map_err(fail)?;
                equal(
                    &Some(result.has_error()),
                    &run.has_error,
                    "has_error vs model",
                )
                .map_err(fail)?;
                if !mode.full_storage {
                    equal(
                        &rec.marked_clean,
                        &run.marked_clean,
                        "marked clean (changePruned)",
                    )
                    .map_err(fail)?;
                    equal(
                        &rec.change_pruned,
                        &run.change_pruned,
                        "change pruned (evaluated, version unchanged)",
                    )
                    .map_err(fail)?;
                    equal(&warnings, &run.warnings, "warnings").map_err(fail)?;
                    equal(&errors, &run.stored_events, "stored events").map_err(fail)?;
                }
            }

            // FULL_STORAGE: stored events replayed for nodes that are not re-run.
            let stuck = expected.kg.values().any(|k| *k == Kg::Stuck);
            if mode.full_storage && (req.keep_going || expected.unique) && !stuck {
                cov.full_storage_events_compared += 1;
                equal(
                    &errors,
                    &events_of(&fresh.events, EventKind::Error),
                    "stored events vs a fresh FULL_STORAGE evaluator",
                )
                .map_err(fail)?;
                subset_of(
                    &warnings,
                    &events_of(&fresh.events, EventKind::Warning),
                    "warnings vs a fresh FULL_STORAGE evaluator",
                )
                .map_err(fail)?;
            }
        }

        // Coverage of what happened.
        cov.change_pruned_rebuild += !rec.change_pruned.is_empty() as u64;
        cov.marked_clean += !rec.marked_clean.is_empty() as u64;
        if !transient_before.is_disjoint(&ran) && edits.contains(&Edit::InvalidateTransientErrors) {
            cov.transient_error_rerun += 1;
        }

        // The graph.
        let g = read_graph(&ev);
        let receiver_inflight: BTreeSet<MKey> = ev
            .progress_receiver
            .inflight_keys_for_testing()
            .iter()
            .map(mkey_of)
            .collect();
        check_invariants(&g, i as i64, &receiver_inflight)
            .map_err(|e| fail(format!("graph invariant: {e}")))?;
        if model.is_certain() {
            cov.graph_compared_with_model += 1;
            let differs = check_graph_against_model(&g, &model, &spec, mode.full_storage)
                .map_err(|e| fail(format!("graph vs model: {e}")))?;
            cov.full_storage_graph_differs += differs as u64;
        }
        let dirty_now: BTreeSet<MKey> = rec.dirtied.union(&dirty_before).copied().collect();
        if dirty_now.iter().any(|k| {
            g.get(k).is_some_and(|n| {
                n.error
                    .as_ref()
                    .is_some_and(|e| !e.get_cycle_info().is_empty())
            })
        }) {
            cov.dirty_in_cycle += 1;
        }
        prev_in_flight = receiver_inflight;
    }
    Ok(())
}

/// A deterministic, compact dump of a scenario: the spec (one node per line), then per step the
/// edits and the request.
fn dump(sc: &Scenario) -> String {
    let mut out = format!("scenario seed {}\n{:?}\n", sc.seed, sc.spec);
    for (i, (edits, req)) in sc.steps.iter().enumerate() {
        let edits: Vec<String> = edits
            .iter()
            .map(|e| match e {
                Edit::SetProgram { node, program } => format!("SetProgram n{node}: {program}"),
                other => format!("{other:?}"),
            })
            .collect();
        out += &format!(
            "step {i}: edits [{}]\n        roots {:?} keep_going {} parallelism {} interrupt_after_runs {:?}\n",
            edits.join("; "),
            req.roots,
            req.keep_going,
            req.parallelism,
            req.interrupt_after_runs
        );
    }
    out
}

fn gen_config(seed: u64) -> (&'static str, GenConfig) {
    match seed % 3 {
        0 => ("default", GenConfig::default()),
        1 => ("wide", GenConfig::wide()),
        _ => (
            "guarded-larger",
            GenConfig {
                guarded_requests: true,
                max_nodes: 14,
                steps: 10,
                ..GenConfig::default()
            },
        ),
    }
}

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name).ok().map(|v| {
        v.parse()
            .unwrap_or_else(|_| panic!("{name}={v:?} is not a number"))
    })
}

fn panic_message(p: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = p.downcast_ref::<&str>() {
        s.to_string()
    } else {
        "<panic>".into()
    }
}

/// Runs seeds `start..start + count` in every mode; returns the coverage and the failures.
fn run_seeds(start: u64, count: u64) -> (Coverage, Vec<String>) {
    let mut total = Coverage::default();
    let mut failures = vec![];
    for seed in start..start + count {
        let (cfg_name, cfg) = gen_config(seed);
        let sc = generate_with(seed, &cfg);
        for mode in MODES {
            let mut cov = Coverage::default();
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_scenario(&sc, mode, &mut cov)
            }));
            let err = match r {
                Ok(Ok(())) => None,
                Ok(Err((step, m))) => Some(format!("step {step}: {m}")),
                Err(p) => Some(format!(
                    "step {}: panic: {}",
                    cov.current_step,
                    panic_message(&*p)
                )),
            };
            total.add(&cov);
            if let Some(err) = err {
                failures.push(format!(
                    "seed {seed} (generator config {cfg_name}, {mode:?}): {err}\n\
                     replay: BLAZE_SKYFRAME_MODEL_SEED_START={seed} BLAZE_SKYFRAME_MODEL_SEEDS=1 \
                     scripts/cargo-capped test -p blaze_skyframe model_driver -- --nocapture\n\
                     {}",
                    dump(&sc)
                ));
                // One failure per seed is enough to replay it.
                break;
            }
        }
    }
    (total, failures)
}

/// The real evaluator against the model for fixed seeds (more with `BLAZE_SKYFRAME_MODEL_SEEDS`).
#[test]
fn model_driver() {
    let start = env_u64("BLAZE_SKYFRAME_MODEL_SEED_START").unwrap_or(0);
    let explicit = env_u64("BLAZE_SKYFRAME_MODEL_SEEDS");
    let count = explicit.unwrap_or(DEFAULT_SEEDS);
    let (cov, failures) = run_seeds(start, count);
    eprintln!("model_driver: seeds {start}..{}: {cov:#?}", start + count);
    let dumps = env_u64("BLAZE_SKYFRAME_MODEL_DUMPS").unwrap_or(1) as usize;
    for f in failures.iter().take(dumps) {
        eprintln!("FAILED {f}\n");
    }
    for f in &failures {
        eprintln!("FAILED {}", f.lines().next().unwrap_or_default());
    }
    assert!(
        failures.is_empty(),
        "{} of {count} seeds failed; first: {}",
        failures.len(),
        failures[0].lines().next().unwrap_or_default()
    );
    if explicit.is_none() {
        // The fixed seeds exercise every edit and every outcome the test is for.
        for (name, n) in [
            ("change-pruned rebuild", cov.change_pruned_rebuild),
            ("marked-clean node", cov.marked_clean),
            ("deleted in-flight node", cov.deleted_in_flight),
            ("dirty node in a cycle", cov.dirty_in_cycle),
            ("transient error rerun", cov.transient_error_rerun),
            ("injected-equal no-op", cov.injected_equal_noop),
            ("no-keep-going stop", cov.no_keep_going_stop),
            ("interrupt", cov.interrupted),
            (
                "unique result compared with a fresh evaluator",
                cov.unique_compared_with_fresh,
            ),
            (
                "FULL_STORAGE events compared",
                cov.full_storage_events_compared,
            ),
            (
                "graph compared with the model",
                cov.graph_compared_with_model,
            ),
        ] {
            assert!(n > 0, "no {name}: {cov:#?}");
        }
        for e in [
            "set_leaf",
            "set_program",
            "invalidate",
            "delete",
            "delete_dirty",
            "invalidate_transient_errors",
        ] {
            assert!(
                cov.edits.get(e).copied().unwrap_or(0) > 0,
                "no {e}: {cov:#?}"
            );
        }
    }
}

fn run_one_seed(seed: u64) {
    let (_, failures) = run_seeds(seed, 1);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Regression (engine): a node committed with a cycle error by a keep_going evaluation kept the
/// requesters that closed the cycle through it (`NodeState::cycle_rdeps`); when it was rebuilt
/// and threw first in a later no-keep-going evaluation, its error did not bubble up to its dirty
/// parent, and `bubbleErrorUp` failed "Current key n3 has to be a top-level key".
#[test]
fn model_driver_seed_22969_rebuilt_cycle_member_bubbles_to_its_parent() {
    run_one_seed(22969);
}

/// Regression (model): without keep_going, error bubbling may replace the error with the
/// exception of a parent that catches it, although that parent waits on a cycle in keep_going
/// (`eval_from_scratch`'s allowed exception origins).
#[test]
fn model_driver_seed_3984_bubbling_parent_throws_its_own_exception() {
    run_one_seed(3984);
}
