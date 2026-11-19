//! Java's incremental algorithm, simulated on the spec: `ModelGraph` holds per node what
//! `IncrementalInMemoryNodeEntry` holds and runs one canonical schedule of an evaluation
//! (depth first, requests in order: the schedule Blaze's blocking engine uses at parallelism 1).
//!
//! The rules, each with the Java it follows:
//!
//! * Start of `evaluate` (AbstractInMemoryMemoizingEvaluator.java:118-152): next graph version
//!   (0, then +1, :495-503); keys in flight at the end of the previous evaluation are queued for
//!   deletion (:134); the differencer's lists (`invalidate`, `inject`) are taken; if anything is
//!   pending: `pruneInjectedValues` (:432-465), injected keys are invalidated, then
//!   `performInvalidation` (:481-493: `EagerInvalidator.delete`, then `EagerInvalidator.invalidate`)
//!   and `injectValues` (:468-479, ParallelEvaluator.java:599-632).
//! * Deleting (InvalidatingNodeVisitor.java:229-400, `traverseGraph = keepEdges = true`): the keys
//!   and their reverse transitive closure leave the graph; reverse-dep edges of surviving deps
//!   go with them. Reverse deps are derived here from the deps every node records (done: its
//!   deps; dirty: the deps of its last build; in flight: its temporary deps plus the last-build
//!   groups not checked yet, `getAllDirectDepsForIncompleteNode`), which is what Java's
//!   reverse-dep bookkeeping keeps consistent with.
//! * Dirtying (InvalidatingNodeVisitor.java:402-570): given keys are marked CHANGE, their reverse
//!   deps DIRTY, transitively; a node already changed, or already dirty when only marked dirty,
//!   is skipped; a DIRTY node marked CHANGE is `markChanged` without propagating
//!   (IncrementalInMemoryNodeEntry.java:287-333). Every model key is non-hermetic (the driver's
//!   key declares `FunctionHermeticity::Nonhermetic`, like GraphTester.NonHermeticKey), so the
//!   "hermetic functions cannot be marked changed" check (:467) never fires.
//! * `ParallelEvaluator.eval` (ParallelEvaluator.java:641-700): all roots done: nothing runs; no
//!   keep_going and a root done with an error: the result is those errors and nothing runs;
//!   else the error-transience node is injected if it is not done (ParallelEvaluator.java:187-202, its value equals
//!   nothing, ErrorTransienceValue.java:59) and the roots are evaluated.
//! * A dirty node (AbstractParallelEvaluator.java:233-380): CHANGE starts in NEEDS_REBUILDING,
//!   DIRTY in CHECK_DEPENDENCIES (DirtyBuildingState.java:126-135). While checking: the next
//!   group of the last build; the error-transience group forces a rebuild when that node is newer
//!   than this node's last-changed version (:224-231, :254-262); without keep_going a done dep in
//!   error in the group stops the evaluation (:264-299); the group is re-added as temporary deps
//!   and evaluated; `signalDep` (DirtyBuildingState.java:178-200) moves to NEEDS_REBUILDING when a
//!   dep's last-changed version is newer than this node's last-evaluated version, to
//!   VERIFIED_CLEAN after the last group. A dep that does not finish leaves the node waiting.
//!   VERIFIED_CLEAN: `markClean` keeps value and versions (IncrementalInMemoryNodeEntry.java:
//!   336-348); without keep_going a clean node with an error stops the evaluation (:360-369).
//! * Running a function (AbstractParallelEvaluator.java:418-760): the checked groups are the
//!   previously requested deps; reads follow SkyFunctionEnvironment.java:612-660 and :720-791;
//!   a value commits with its children's errors attached (ErrorInfoManager.java:55-66); an
//!   exception commits (transitively transient if any dep error is, :527-534; a directly
//!   transient one adds the error-transience dep, SkyFunctionEnvironment.java:422-447) and stops
//!   a no-keep-going evaluation; null with a dep not done waits; null with every dep done
//!   commits the children's errors in keep_going (:703-716) and stops a no-keep-going
//!   evaluation (`getDepErrorKey`, :625-686).
//! * Commit (SkyFunctionEnvironment.java:916-1016, IncrementalInMemoryNodeEntry.java:139-163):
//!   deps = temporary deps; if the new value is comparable and equal to the last build's value
//!   (`unchangedFromLastBuild`, DirtyBuildingState.java:214-219; values with an error are
//!   `ErrorInfoValue implements NotComparableSkyValue`, ValueWithMetadata.java:152) the old value
//!   and last-changed version are kept and last-evaluated becomes the graph version (change
//!   pruned); else both versions become the graph version.
//! * After the roots (ParallelEvaluator.java:506-580 `constructResult`): unfinished roots go to
//!   the cycle detector, ported below from SimpleCycleDetector.java (keep_going: it commits cycle
//!   errors, dropping unfinished deps of nodes on a cycle; no keep_going: the first unfinished
//!   root gets a cycle error and nothing is committed).

use super::error_model::MError;
use super::scratch::distinct_roots;
use super::spec::{
    Dep, DepRead, ErrClass, EvalRequest, GraphSpec, NodeId, NodeKind, ProgramEnv, RootResult,
    RunResult, run_program, stored_event_text, warning_text,
};
use crate::DirtyType;
use std::collections::{BTreeMap, BTreeSet};

/// A key of the model graph: a spec node or `ErrorTransienceValue.KEY`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MKey {
    Node(NodeId),
    Et,
}

/// A graph version; -1 is `Version.minimal()` (a node never built).
pub type Ver = i64;

/// `ValueWithMetadata` reduced to value and error (events are not stored: the driver uses
/// `EventFilter::NO_STORAGE`, see the driver contract).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MValue {
    pub value: Option<i64>,
    pub error: Option<MError>,
}

/// `NodeEntry.LifecycleState` while a node is in flight (NodeEntry.java:60-91).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Life {
    CheckDependencies,
    VerifiedClean,
    NeedsRebuilding,
    Rebuilding,
}

/// `DirtyBuildingState` plus the temporary direct deps.
#[derive(Clone, Debug)]
pub struct InProgress {
    /// Value and deps of the last build (`IncrementalBuildingState`); `None` for a node never
    /// built (`InitialBuildingState`).
    pub last: Option<(MValue, Vec<Vec<MKey>>)>,
    /// `dirtyDirectDepIndex`.
    pub idx: usize,
    pub life: Life,
    pub temp: Vec<Vec<MKey>>,
}

#[derive(Clone, Debug)]
pub enum MState {
    Done {
        value: MValue,
        deps: Vec<Vec<MKey>>,
    },
    /// Marked dirty, not yet requested.
    Dirty {
        kind: DirtyType,
        last_value: MValue,
        last_deps: Vec<Vec<MKey>>,
    },
    /// Requested in the current (or, between evaluations, the previous) evaluation and not done.
    InFlight(InProgress),
}

/// `IncrementalInMemoryNodeEntry`.
#[derive(Clone, Debug)]
pub struct MNode {
    pub state: MState,
    /// `NodeVersion.lastChanged()`, what `getVersion()` returns.
    pub last_changed: Ver,
    /// `NodeVersion.lastEvaluated()`.
    pub last_evaluated: Ver,
}

/// The result of one model evaluation.
#[derive(Clone, Debug, Default)]
pub struct ModelRun {
    pub version: Ver,
    /// `ran`, `marked_clean`, `change_pruned`, the results and the events are determined by
    /// Java's algorithm (see the notes: keep_going, or no keep_going without an early stop, no
    /// interrupt, and every earlier evaluation of the model exact too).
    pub exact: bool,
    /// A no-keep-going evaluation stopped at an error (the canonical schedule's first error).
    pub aborted: bool,
    pub interrupted: bool,
    /// Per distinct root in root order, `Absent` included. `None` when aborted or interrupted
    /// (check the real result against `eval_from_scratch` only).
    pub results: Option<Vec<(NodeId, RootResult)>>,
    pub has_error: Option<bool>,
    /// Nodes whose function was invoked at least once (canonical schedule).
    pub ran: BTreeSet<NodeId>,
    /// Every node whose function may run in this evaluation under any schedule and any state
    /// the earlier inexact evaluations may have left: nodes reachable from the roots through
    /// every branch of the current programs, the deps the model graph records, and every such
    /// edge recorded at an inexact evaluation.
    pub ran_upper: BTreeSet<NodeId>,
    /// Nodes `markClean`ed (VERIFIED_CLEAN): Java calls `EvaluationProgressReceiver.changePruned`
    /// for them (AbstractParallelEvaluator.java:426-429) and `evaluated(state =
    /// *_VERSION_UNCHANGED, directDeps = null)`.
    pub marked_clean: BTreeSet<NodeId>,
    /// Nodes that ran and whose new value equals the last build's, so they kept their version:
    /// `evaluated(..., *_VERSION_UNCHANGED, directDeps != null)` (SkyFunctionEnvironment.java:
    /// 997-1014). Not Java's `changePruned` callback.
    pub change_pruned: BTreeSet<NodeId>,
    /// Warnings reported (each emitted once per completed run; the driver clears
    /// `EmittedEventState` before each evaluation).
    pub warnings: BTreeSet<String>,
    /// Stored-kind events reported with `EventFilter::NO_STORAGE`: those of runs that commit.
    pub stored_events: BTreeSet<String>,
    /// Nodes `markDirty`ed at the start of this evaluation (the set of `dirtied` callbacks).
    pub dirtied: BTreeSet<MKey>,
    /// Nodes deleted at the start of this evaluation (the set of `deleted` callbacks).
    pub deleted: BTreeSet<MKey>,
    /// Spec nodes in the graph (done or dirty) after the start-of-evaluation invalidation.
    pub present_at_start: BTreeSet<NodeId>,
    /// No keep_going, not every root done, and a root done with an error: the evaluation
    /// returns the cached errors and evaluates nothing (ParallelEvaluator.java:668-690). An
    /// annotation for the driver (it counts these), not a rule.
    pub cached_error_stop: bool,
}

impl ModelRun {
    /// `(lower, upper)` bounds of the set of nodes whose function the real evaluator runs.
    pub fn ran_bounds(&self) -> (BTreeSet<NodeId>, BTreeSet<NodeId>) {
        if self.exact {
            (self.ran.clone(), self.ran.clone())
        } else {
            (BTreeSet::new(), self.ran_upper.clone())
        }
    }
}

/// The in-memory graph and the differencer of Java's `InMemoryMemoizingEvaluator`.
#[derive(Clone, Debug)]
pub struct ModelGraph {
    nodes: BTreeMap<MKey, MNode>,
    last_graph_version: Option<Ver>,
    /// `SequencedRecordingDifferencer.valuesToInvalidate`.
    pending_invalidate: Vec<MKey>,
    /// `SequencedRecordingDifferencer.valuesToInject`.
    pending_inject: BTreeMap<MKey, i64>,
    /// `AbstractInMemoryMemoizingEvaluator.valuesToDelete`.
    values_to_delete: BTreeSet<MKey>,
    /// `DirtyAndInflightTrackingProgressReceiver.inflightKeys`.
    inflight: BTreeSet<MKey>,
    /// `DirtyAndInflightTrackingProgressReceiver.dirtyKeys`.
    dirty_unenqueued: BTreeSet<MKey>,
    /// False once an evaluation was not exact: the real graph may then differ from this one.
    certain: bool,
    possible_edges: BTreeSet<(NodeId, NodeId)>,
}

impl Default for ModelGraph {
    fn default() -> Self {
        Self::new()
    }
}

/// Java: `SimpleCycleDetector.MAX_CYCLES_TO_STORE`.
const MAX_CYCLES_TO_STORE: usize = 20;

struct Ctx<'s> {
    spec: &'s GraphSpec,
    keep_going: bool,
    version: Ver,
    stack: Vec<MKey>,
    aborted: bool,
    ran: BTreeSet<NodeId>,
    marked_clean: BTreeSet<NodeId>,
    change_pruned: BTreeSet<NodeId>,
    warnings: BTreeSet<String>,
    stored_events: BTreeSet<String>,
}

enum Visit {
    Key(MKey),
    ChildrenFinished,
}

/// The evaluation stopped (no keep_going error): unwinds the program.
struct Aborted;

fn flat(groups: &[Vec<MKey>]) -> impl Iterator<Item = MKey> + '_ {
    groups.iter().flatten().copied()
}

/// `DirtyBuildingState.unchangedFromLastBuild` (DirtyBuildingState.java:214-219) for the model's
/// values: the error-transience value equals nothing; values with an error are
/// `NotComparableSkyValue`s (and `ErrorInfoValue` equals no plain value); plain values are equal
/// when the key is comparable and the numbers are.
fn values_equal(spec: &GraphSpec, key: MKey, old: &MValue, new: &MValue) -> bool {
    let MKey::Node(n) = key else {
        return false;
    };
    new.error.is_none()
        && old.error.is_none()
        && spec.node(n).comparable
        && old.value.is_some()
        && old.value == new.value
}

impl ModelGraph {
    pub fn new() -> Self {
        ModelGraph {
            nodes: BTreeMap::new(),
            last_graph_version: None,
            pending_invalidate: vec![],
            pending_inject: BTreeMap::new(),
            values_to_delete: BTreeSet::new(),
            inflight: BTreeSet::new(),
            dirty_unenqueued: BTreeSet::new(),
            certain: true,
            possible_edges: BTreeSet::new(),
        }
    }

    pub fn is_certain(&self) -> bool {
        self.certain
    }

    pub fn node(&self, k: MKey) -> Option<&MNode> {
        self.nodes.get(&k)
    }

    /// Keys in flight at the end of the last evaluation (deleted at the start of the next).
    pub fn in_flight(&self) -> &BTreeSet<MKey> {
        &self.inflight
    }

    fn is_done(&self, k: MKey) -> bool {
        matches!(
            self.nodes.get(&k),
            Some(MNode {
                state: MState::Done { .. },
                ..
            })
        )
    }

    fn done_value(&self, k: MKey) -> MValue {
        match &self.nodes[&k].state {
            MState::Done { value, .. } => *value,
            s => panic!("{k:?} not done: {s:?}"),
        }
    }

    fn in_progress(&mut self, k: MKey) -> &mut InProgress {
        match &mut self.nodes.get_mut(&k).expect("in graph").state {
            MState::InFlight(ip) => ip,
            s => panic!("{k:?} not in flight: {s:?}"),
        }
    }

    fn deps_of(node: &MNode) -> Vec<MKey> {
        match &node.state {
            MState::Done { deps, .. } => flat(deps).collect(),
            MState::Dirty { last_deps, .. } => flat(last_deps).collect(),
            MState::InFlight(ip) => {
                let mut v: Vec<MKey> = flat(&ip.temp).collect();
                if let Some((_, last)) = &ip.last {
                    v.extend(flat(&last[ip.idx.min(last.len())..]));
                }
                v
            }
        }
    }

    fn rdeps_map(&self) -> BTreeMap<MKey, BTreeSet<MKey>> {
        let mut m: BTreeMap<MKey, BTreeSet<MKey>> = BTreeMap::new();
        for (k, n) in &self.nodes {
            for d in Self::deps_of(n) {
                m.entry(d).or_default().insert(*k);
            }
        }
        m
    }

    /// Applies an edit to the spec and records what the driver's evaluator call records.
    pub fn apply_edit(&mut self, spec: &mut GraphSpec, e: &super::spec::Edit) {
        use super::spec::Edit;
        e.apply(spec);
        match e {
            // RecordingDifferencer.inject: put, so the last injection of a key wins.
            Edit::SetLeaf { node, value } => {
                self.pending_inject.insert(MKey::Node(*node), *value);
            }
            Edit::SetProgram { node, .. } | Edit::Invalidate { node } => {
                self.pending_invalidate.push(MKey::Node(*node));
            }
            // AbstractInMemoryMemoizingEvaluator.java:212-225: every dirty node (isDirty() is
            // !isDone(), AbstractInMemoryNodeEntry.java:113) and every node the predicate takes.
            Edit::Delete { nodes } => {
                for (k, n) in &self.nodes {
                    let dirty = !matches!(n.state, MState::Done { .. });
                    let matched = matches!(k, MKey::Node(id) if nodes.contains(id));
                    if dirty || matched {
                        self.values_to_delete.insert(*k);
                    }
                }
            }
            // AbstractInMemoryMemoizingEvaluator.java:243-255.
            Edit::DeleteDirty { age } => {
                let last = self
                    .last_graph_version
                    .expect("deleteDirty before the first evaluation");
                let threshold = last - *age as Ver;
                for k in &self.dirty_unenqueued {
                    let n = self.nodes.get(k).expect("dirty key in the graph");
                    assert!(!matches!(n.state, MState::Done { .. }), "{k:?} not dirty");
                    if n.last_changed <= threshold {
                        self.values_to_delete.insert(*k);
                    }
                }
            }
            // RecordingDifferencer.java:33-37: invalidate the error-transience node.
            Edit::InvalidateTransientErrors => self.pending_invalidate.push(MKey::Et),
        }
    }

    /// Start of `AbstractInMemoryMemoizingEvaluator.evaluate` (:118-152).
    fn start(&mut self, spec: &GraphSpec, run: &mut ModelRun) -> Ver {
        let version = self.last_graph_version.map_or(0, |v| v + 1);
        // :134-135. (No key is ever rewound: getAndClearUnsuccessfullyRewoundKeys is empty.)
        let inflight = std::mem::take(&mut self.inflight);
        self.values_to_delete.extend(inflight);
        let invalidate = std::mem::take(&mut self.pending_invalidate);
        let mut values_to_inject = std::mem::take(&mut self.pending_inject);
        if !(invalidate.is_empty()
            && values_to_inject.is_empty()
            && self.values_to_delete.is_empty())
        {
            let mut values_to_dirty: BTreeSet<MKey> = invalidate.into_iter().collect();
            self.prune_injected_values(spec, &mut values_to_inject, &mut values_to_dirty);
            values_to_dirty.extend(values_to_inject.keys().copied());
            let to_delete = std::mem::take(&mut self.values_to_delete);
            run.deleted = self.delete(to_delete);
            run.dirtied = self.dirty(values_to_dirty);
            self.inject_values(spec, &values_to_inject, version);
        }
        // Java sets lastGraphVersion in a finally block.
        self.last_graph_version = Some(version);
        version
    }

    // Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:432 AbstractInMemoryMemoizingEvaluator.pruneInjectedValues
    fn prune_injected_values(
        &self,
        spec: &GraphSpec,
        values_to_inject: &mut BTreeMap<MKey, i64>,
        values_to_dirty: &mut BTreeSet<MKey>,
    ) {
        values_to_inject.retain(|k, v| {
            let Some(MNode {
                state: MState::Done { value, deps },
                ..
            }) = self.nodes.get(k)
            else {
                return true;
            };
            if deps.is_empty() {
                // `newValue.equals(prevEntry.getValue())`: value equality of comparable values.
                let equal = matches!(k, MKey::Node(n) if spec.node(*n).comparable)
                    && value.error.is_none()
                    && value.value == Some(*v);
                !(equal && !values_to_dirty.contains(k) && !self.values_to_delete.contains(k))
            } else {
                // An injected key that has deps is invalidated instead.
                values_to_dirty.insert(*k);
                false
            }
        });
    }

    // Java: com/google/devtools/build/skyframe/InvalidatingNodeVisitor.java:229 DeletingNodeVisitor
    fn delete(&mut self, keys: BTreeSet<MKey>) -> BTreeSet<MKey> {
        let rdeps = self.rdeps_map();
        let mut visited = BTreeSet::new();
        let mut todo: Vec<MKey> = keys.into_iter().collect();
        while let Some(k) = todo.pop() {
            if !self.nodes.contains_key(&k) || !visited.insert(k) {
                continue;
            }
            todo.extend(rdeps.get(&k).into_iter().flatten().copied());
        }
        for k in &visited {
            self.nodes.remove(k);
            // DirtyAndInflightTrackingProgressReceiver.deleted.
            self.dirty_unenqueued.remove(k);
        }
        visited
    }

    // Java: com/google/devtools/build/skyframe/InvalidatingNodeVisitor.java:401 DirtyingNodeVisitor
    fn dirty(&mut self, changed: BTreeSet<MKey>) -> BTreeSet<MKey> {
        let rdeps = self.rdeps_map();
        let mut dirtied = BTreeSet::new();
        let mut seen_changed = BTreeSet::new();
        let mut seen_dirtied = BTreeSet::new();
        let mut todo: Vec<(MKey, DirtyType)> = changed
            .into_iter()
            .map(|k| (k, DirtyType::Change))
            .collect();
        while let Some((k, t)) = todo.pop() {
            let seen = if t == DirtyType::Change {
                &mut seen_changed
            } else {
                &mut seen_dirtied
            };
            if !seen.insert(k) {
                continue;
            }
            let Some(node) = self.nodes.get_mut(&k) else {
                continue;
            };
            let is_changed = matches!(
                node.state,
                MState::Dirty {
                    kind: DirtyType::Change,
                    ..
                }
            );
            let is_dirty = !matches!(node.state, MState::Done { .. });
            if is_changed || (t == DirtyType::Dirty && is_dirty) {
                continue;
            }
            match &mut node.state {
                MState::Done { value, deps } => {
                    assert!(
                        t != DirtyType::Dirty || !deps.is_empty(),
                        "{k:?} is being marked dirty but has no children that could have dirtied it"
                    );
                    node.state = MState::Dirty {
                        kind: t,
                        last_value: *value,
                        last_deps: std::mem::take(deps),
                    };
                    dirtied.insert(k);
                    self.dirty_unenqueued.insert(k);
                    for r in rdeps.get(&k).into_iter().flatten() {
                        todo.push((*r, DirtyType::Dirty));
                    }
                }
                // markDirty(CHANGE) on a dirty node: markChanged, returns null.
                MState::Dirty { kind, .. } => *kind = DirtyType::Change,
                MState::InFlight(_) => unreachable!("in-flight nodes are deleted first: {k:?}"),
            }
        }
        dirtied
    }

    // Java: com/google/devtools/build/skyframe/ParallelEvaluator.java:599 ParallelEvaluator.injectValues
    fn inject_values(&mut self, spec: &GraphSpec, values: &BTreeMap<MKey, i64>, version: Ver) {
        for (k, v) in values {
            let new = MValue {
                value: Some(*v),
                error: None,
            };
            self.inject_one(spec, *k, new, version);
        }
    }

    fn inject_one(&mut self, spec: &GraphSpec, k: MKey, new: MValue, version: Ver) {
        match self.nodes.get_mut(&k) {
            None => {
                self.nodes.insert(
                    k,
                    MNode {
                        state: MState::Done {
                            value: new,
                            deps: vec![],
                        },
                        last_changed: version,
                        last_evaluated: version,
                    },
                );
            }
            Some(node) => {
                let MState::Dirty {
                    last_value,
                    last_deps,
                    ..
                } = &node.state
                else {
                    panic!("injecting into {k:?} which is {:?}", node.state)
                };
                assert!(last_deps.is_empty(), "existing entry for {k:?} has deps");
                let last_value = *last_value;
                if values_equal(spec, k, &last_value, &new) {
                    node.last_evaluated = version;
                    node.state = MState::Done {
                        value: last_value,
                        deps: vec![],
                    };
                } else {
                    node.last_changed = version;
                    node.last_evaluated = version;
                    node.state = MState::Done {
                        value: new,
                        deps: vec![],
                    };
                }
            }
        }
        // InflightTrackingProgressReceiver.injected.
        self.inflight.remove(&k);
        self.dirty_unenqueued.remove(&k);
    }

    /// Every spec edge an evaluation may follow (see `ModelRun::ran_upper`).
    fn upper_bound(&self, spec: &GraphSpec, roots: &[NodeId]) -> BTreeSet<NodeId> {
        let mut edges: BTreeMap<NodeId, BTreeSet<NodeId>> = BTreeMap::new();
        for n in 0..spec.len() {
            edges.insert(n, spec.all_possible_deps(n));
        }
        for (k, node) in &self.nodes {
            if let MKey::Node(n) = k {
                for d in Self::deps_of(node) {
                    if let MKey::Node(d) = d {
                        edges.entry(*n).or_default().insert(d);
                    }
                }
            }
        }
        for (a, b) in &self.possible_edges {
            edges.entry(*a).or_default().insert(*b);
        }
        let mut seen = BTreeSet::new();
        let mut todo: Vec<NodeId> = roots.to_vec();
        while let Some(n) = todo.pop() {
            if seen.insert(n) {
                todo.extend(edges.get(&n).into_iter().flatten().copied());
            }
        }
        seen
    }

    fn remember_possible_edges(&mut self, spec: &GraphSpec) {
        for n in 0..spec.len() {
            for d in spec.all_possible_deps(n) {
                self.possible_edges.insert((n, d));
            }
        }
        let recorded: Vec<(NodeId, NodeId)> = self
            .nodes
            .iter()
            .flat_map(|(k, node)| {
                let from = match k {
                    MKey::Node(n) => Some(*n),
                    MKey::Et => None,
                };
                Self::deps_of(node)
                    .into_iter()
                    .filter_map(move |d| match (from, d) {
                        (Some(a), MKey::Node(b)) => Some((a, b)),
                        _ => None,
                    })
            })
            .collect();
        self.possible_edges.extend(recorded);
    }

    fn present_nodes(&self) -> BTreeSet<NodeId> {
        self.nodes
            .keys()
            .filter_map(|k| match k {
                MKey::Node(n) => Some(*n),
                MKey::Et => None,
            })
            .collect()
    }

    /// An evaluation the driver interrupted: the start of `evaluate` happens, then any node the
    /// evaluation can reach may have run, committed or stayed in flight. The model keeps the
    /// graph as it is after the start and is not exact from now on.
    pub fn evaluate_interrupted(&mut self, spec: &GraphSpec, req: &EvalRequest) -> ModelRun {
        let mut run = ModelRun::default();
        self.remember_possible_edges(spec);
        run.version = self.start(spec, &mut run);
        run.present_at_start = self.present_nodes();
        run.ran_upper = self.upper_bound(spec, &distinct_roots(&req.roots));
        run.interrupted = true;
        self.certain = false;
        self.remember_possible_edges(spec);
        run
    }

    /// One `evaluate(roots, keep_going)` (the request's `interrupt_after_runs` and `parallelism`
    /// do not change the model; an interrupted evaluation is [`ModelGraph::evaluate_interrupted`]).
    pub fn evaluate(&mut self, spec: &GraphSpec, req: &EvalRequest) -> ModelRun {
        let certain_before = self.certain;
        let mut run = ModelRun::default();
        let version = self.start(spec, &mut run);
        run.version = version;
        run.present_at_start = self.present_nodes();
        let roots: Vec<NodeId> = distinct_roots(&req.roots);
        run.ran_upper = self.upper_bound(spec, &roots);
        let root_keys: Vec<MKey> = roots.iter().map(|r| MKey::Node(*r)).collect();
        let mut cx = Ctx {
            spec,
            keep_going: req.keep_going,
            version,
            stack: vec![],
            aborted: false,
            ran: BTreeSet::new(),
            marked_clean: BTreeSet::new(),
            change_pruned: BTreeSet::new(),
            warnings: BTreeSet::new(),
            stored_events: BTreeSet::new(),
        };

        // ParallelEvaluator.java:659-666: every root done, nothing to do.
        let all_done = root_keys.iter().all(|k| self.is_done(*k));
        // ParallelEvaluator.java:670-690: no keep_going and a root done with an error.
        let cached_errors: Vec<MKey> = if req.keep_going || all_done {
            vec![]
        } else {
            root_keys
                .iter()
                .copied()
                .filter(|k| self.is_done(*k) && self.done_value(*k).error.is_some())
                .collect()
        };
        let results: Vec<(NodeId, RootResult)>;
        if all_done {
            results = roots
                .iter()
                .map(|r| (*r, self.done_result(MKey::Node(*r), req.keep_going)))
                .collect();
        } else if !cached_errors.is_empty() {
            run.cached_error_stop = true;
            results = roots
                .iter()
                .map(|r| {
                    let k = MKey::Node(*r);
                    if cached_errors.contains(&k) {
                        (*r, self.done_result(k, false))
                    } else {
                        (*r, RootResult::Absent)
                    }
                })
                .collect();
        } else {
            // ParallelEvaluator.java:187-202 injectErrorTransienceValue.
            if !self.is_done(MKey::Et) {
                self.inject_one(
                    spec,
                    MKey::Et,
                    MValue {
                        value: None,
                        error: None,
                    },
                    version,
                );
            }
            for k in &root_keys {
                self.request(&mut cx, *k);
            }
            if cx.aborted {
                results = vec![];
            } else {
                results = self.construct_result(&mut cx, &root_keys);
            }
        }

        run.aborted = cx.aborted;
        run.ran = cx.ran;
        run.marked_clean = cx.marked_clean;
        run.change_pruned = cx.change_pruned;
        run.warnings = cx.warnings;
        run.stored_events = cx.stored_events;
        if run.aborted {
            run.results = None;
            run.has_error = Some(true);
            if self.certain {
                self.remember_possible_edges(spec);
            }
            self.certain = false;
            self.remember_possible_edges(spec);
        } else {
            run.has_error = Some(
                results
                    .iter()
                    .any(|(_, r)| matches!(r, RootResult::Error(_))),
            );
            run.results = Some(results);
        }
        run.exact = certain_before && !run.aborted;
        if !self.certain {
            self.remember_possible_edges(spec);
        }
        run
    }

    /// What `constructResult` says about a done root.
    fn done_result(&self, k: MKey, keep_going: bool) -> RootResult {
        let v = self.done_value(k);
        match (v.value, v.error) {
            (_, Some(e)) if !keep_going => RootResult::Error(e.class()),
            (Some(x), _) => RootResult::Value(x),
            (None, Some(e)) => RootResult::Error(e.class()),
            (None, None) => unreachable!("{k:?} has neither value nor error"),
        }
    }

    // Java: com/google/devtools/build/skyframe/ParallelEvaluator.java:483 ParallelEvaluator.constructResult
    fn construct_result(&mut self, cx: &mut Ctx<'_>, roots: &[MKey]) -> Vec<(NodeId, RootResult)> {
        let mut results: BTreeMap<MKey, RootResult> = BTreeMap::new();
        let mut cycle_roots = vec![];
        for k in roots {
            if self.is_done(*k) {
                results.insert(*k, self.done_result(*k, cx.keep_going));
            } else {
                cycle_roots.push(*k);
            }
        }
        // SimpleCycleDetector.java:55-80.
        let mut first = true;
        for k in cycle_roots {
            if cx.keep_going {
                let e = self.check_for_cycles(cx, k);
                assert!(
                    e.cycle,
                    "{k:?} was not evaluated, but was not part of a cycle"
                );
                results.insert(k, RootResult::Error(e.class()));
            } else {
                // Without an abort an unfinished root is in or above a cycle; the detector
                // returns after the first root and commits nothing.
                let r = if first {
                    RootResult::Error(ErrClass::Cycle)
                } else {
                    RootResult::Absent
                };
                first = false;
                results.insert(k, r);
            }
        }
        roots
            .iter()
            .map(|k| {
                let MKey::Node(n) = k else { unreachable!() };
                (*n, results[k])
            })
            .collect()
    }

    // Java: com/google/devtools/build/skyframe/DirtyAndInflightTrackingProgressReceiver.java:63 DirtyAndInflightTrackingProgressReceiver.enqueueing
    fn enqueueing(&mut self, k: MKey) {
        self.inflight.insert(k);
        self.dirty_unenqueued.remove(&k);
    }

    /// `evaluated`: built or marked clean.
    fn evaluated(&mut self, k: MKey) {
        self.inflight.remove(&k);
        self.dirty_unenqueued.remove(&k);
    }

    /// Makes `k` in flight: a new entry (`InitialBuildingState`, NEEDS_REBUILDING) or a dirty one
    /// (`IncrementalBuildingState`, DirtyBuildingState.java:126-135).
    fn start_evaluating(&mut self, k: MKey) {
        match self.nodes.get_mut(&k) {
            None => {
                self.nodes.insert(
                    k,
                    MNode {
                        state: MState::InFlight(InProgress {
                            last: None,
                            idx: 0,
                            life: Life::NeedsRebuilding,
                            temp: vec![],
                        }),
                        last_changed: -1,
                        last_evaluated: -1,
                    },
                );
            }
            Some(node) => {
                let MState::Dirty {
                    kind,
                    last_value,
                    last_deps,
                } = &mut node.state
                else {
                    panic!("{k:?} is {:?}", node.state)
                };
                let life = match kind {
                    DirtyType::Dirty => Life::CheckDependencies,
                    DirtyType::Change | DirtyType::Rewind => Life::NeedsRebuilding,
                };
                node.state = MState::InFlight(InProgress {
                    last: Some((*last_value, std::mem::take(last_deps))),
                    idx: 0,
                    life,
                    temp: vec![],
                });
            }
        }
        self.enqueueing(k);
    }

    /// A request of `k` (by a root or a parent). Returns whether `k` is done.
    fn request(&mut self, cx: &mut Ctx<'_>, k: MKey) -> bool {
        if self.is_done(k) {
            return true;
        }
        if cx.aborted {
            // Not evaluated after the error, but in flight (enqueueAfterError), so deleted at
            // the next evaluation.
            if !matches!(
                self.nodes.get(&k),
                Some(MNode {
                    state: MState::InFlight(_),
                    ..
                })
            ) {
                self.start_evaluating(k);
            }
            return false;
        }
        if cx.stack.contains(&k) {
            return false;
        }
        if matches!(
            self.nodes.get(&k),
            Some(MNode {
                state: MState::InFlight(_),
                ..
            })
        ) {
            // Already evaluated in this evaluation and waiting.
            return false;
        }
        self.start_evaluating(k);
        cx.stack.push(k);
        let done = self.process(cx, k);
        cx.stack.pop();
        done
    }

    // Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:413 Evaluate.run
    fn process(&mut self, cx: &mut Ctx<'_>, k: MKey) -> bool {
        // maybeHandleDirtyNode (:233-380).
        loop {
            let last_changed = self.nodes[&k].last_changed;
            let et_version = self.nodes.get(&MKey::Et).map(|n| n.last_changed);
            let ip = self.in_progress(k);
            if ip.life != Life::CheckDependencies {
                break;
            }
            let group = ip.last.as_ref().expect("dirty").1[ip.idx].clone();
            ip.idx += 1;
            // invalidatedByErrorTransience (:224-231).
            if group == [MKey::Et] && !(et_version.expect("injected") <= last_changed) {
                // forceRebuild; the error-transience node's reverse dep is removed.
                ip.life = Life::Rebuilding;
                break;
            }
            if !cx.keep_going {
                let err_dep = group
                    .iter()
                    .copied()
                    .find(|d| self.is_done(*d) && self.done_value(*d).error.is_some());
                if let Some(d) = err_dep {
                    // The node depends on the child in error; the others lose their rdep.
                    self.in_progress(k).temp.push(vec![d]);
                    cx.aborted = true;
                    return false;
                }
            }
            self.in_progress(k).temp.push(group.clone());
            let mut all_done = true;
            for d in &group {
                let done = self.request(cx, *d);
                if cx.aborted {
                    return false;
                }
                if !done {
                    all_done = false;
                    continue;
                }
                // signalDep (DirtyBuildingState.java:178-200).
                let child_version = self.nodes[d].last_changed;
                let last_evaluated = self.nodes[&k].last_evaluated;
                let ip = self.in_progress(k);
                let is_changed = matches!(ip.life, Life::NeedsRebuilding | Life::Rebuilding);
                if !is_changed && child_version > last_evaluated {
                    ip.life = Life::NeedsRebuilding;
                }
            }
            if !all_done {
                // Waits for a dep that never finishes in this evaluation.
                return false;
            }
            let ip = self.in_progress(k);
            if ip.life == Life::CheckDependencies && ip.idx == ip.last.as_ref().unwrap().1.len() {
                ip.life = Life::VerifiedClean;
            }
        }
        let life = self.in_progress(k).life;
        match life {
            Life::VerifiedClean => {
                self.mark_clean(k);
                if let MKey::Node(n) = k {
                    cx.marked_clean.insert(n);
                }
                // AbstractParallelEvaluator.java:360-369.
                if !cx.keep_going && self.done_value(k).error.is_some() {
                    cx.aborted = true;
                }
                return true;
            }
            Life::NeedsRebuilding => self.in_progress(k).life = Life::Rebuilding,
            Life::Rebuilding => {}
            Life::CheckDependencies => unreachable!(),
        }
        self.run_function(cx, k)
    }

    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:336 IncrementalInMemoryNodeEntry.markClean
    fn mark_clean(&mut self, k: MKey) {
        let node = self.nodes.get_mut(&k).unwrap();
        let MState::InFlight(ip) = &mut node.state else {
            unreachable!()
        };
        let (last_value, last_deps) = ip.last.take().expect("dirty");
        let temp = std::mem::take(&mut ip.temp);
        assert_eq!(
            temp, last_deps,
            "deps must be the same as last build to be marked clean"
        );
        node.state = MState::Done {
            value: last_value,
            deps: temp,
        };
        self.evaluated(k);
    }

    fn run_function(&mut self, cx: &mut Ctx<'_>, k: MKey) -> bool {
        let MKey::Node(n) = k else {
            unreachable!("the error-transience node is never built")
        };
        cx.ran.insert(n);
        let spec = cx.spec;
        let (out, child_errors, any_not_done, dep_error) = match &spec.node(n).kind {
            NodeKind::Leaf(v) => (
                super::spec::RunOutcome {
                    result: RunResult::Value(*v),
                    completed: None,
                },
                vec![],
                false,
                false,
            ),
            NodeKind::Function(p) => {
                let mut env = ModelEnv {
                    g: self,
                    cx,
                    key: k,
                    child_errors: vec![],
                    any_not_done: false,
                    values_missing: false,
                    dep_error: false,
                };
                let Ok(out) = run_program(p, &mut env) else {
                    return false;
                };
                (out, env.child_errors, env.any_not_done, env.dep_error)
            }
        };
        if let Some(pre) = out.completed {
            let p = spec.program(n).unwrap();
            if p.warning {
                cx.warnings.insert(warning_text(n, pre));
            }
        }
        let stored = out
            .completed
            .filter(|_| spec.program(n).is_some_and(|p| p.stored_event))
            .map(|pre| stored_event_text(n, pre));
        match out.result {
            RunResult::Value(v) => {
                let error = (!child_errors.is_empty())
                    .then(|| MError::from_child_errors(&child_errors).with_value());
                self.commit(
                    cx,
                    k,
                    MValue {
                        value: Some(v),
                        error,
                    },
                    None,
                    stored,
                );
                true
            }
            RunResult::Throw { transient } => {
                let e = MError::from_exception(
                    n,
                    transient || child_errors.iter().any(|e| e.transitively_transient),
                );
                let extra = transient.then_some(MKey::Et);
                self.commit(
                    cx,
                    k,
                    MValue {
                        value: None,
                        error: Some(e),
                    },
                    extra,
                    stored,
                );
                if !cx.keep_going {
                    cx.aborted = true;
                }
                true
            }
            RunResult::Missing => {
                if !cx.keep_going && dep_error {
                    cx.aborted = true;
                    return false;
                }
                if any_not_done {
                    return false;
                }
                assert!(cx.keep_going, "{k:?}: missing without a reason");
                let e = MError::from_child_errors(&child_errors);
                self.commit(
                    cx,
                    k,
                    MValue {
                        value: None,
                        error: Some(e),
                    },
                    None,
                    stored,
                );
                true
            }
        }
    }

    // Java: com/google/devtools/build/skyframe/SkyFunctionEnvironment.java:916 SkyFunctionEnvironment.commitAndGetParents
    fn commit(
        &mut self,
        cx: &mut Ctx<'_>,
        k: MKey,
        new: MValue,
        extra_dep: Option<MKey>,
        stored_event: Option<String>,
    ) {
        let spec = cx.spec;
        let version = cx.version;
        let node = self.nodes.get_mut(&k).unwrap();
        let MState::InFlight(ip) = &mut node.state else {
            unreachable!()
        };
        let mut deps = std::mem::take(&mut ip.temp);
        if let Some(d) = extra_dep {
            deps.push(vec![d]);
        }
        let last = ip.last.take().map(|(v, _)| v);
        match last {
            Some(lv) if values_equal(spec, k, &lv, &new) => {
                node.last_evaluated = version;
                node.state = MState::Done { value: lv, deps };
                if let MKey::Node(n) = k {
                    cx.change_pruned.insert(n);
                }
            }
            _ => {
                node.last_changed = version;
                node.last_evaluated = version;
                node.state = MState::Done { value: new, deps };
            }
        }
        if let Some(e) = stored_event {
            cx.stored_events.insert(e);
        }
        self.evaluated(k);
    }

    // ---------------------------------------------------------------------------------------
    // SimpleCycleDetector (keep_going), on the model graph.

    // Java: com/google/devtools/build/skyframe/SimpleCycleDetector.java:92 SimpleCycleDetector.checkForCycles
    fn check_for_cycles(&mut self, cx: &mut Ctx<'_>, root: MKey) -> MError {
        let mut cycles_found = 0usize;
        let mut graph_path: Vec<MKey> = vec![];
        let mut path_set: BTreeSet<MKey> = BTreeSet::new();
        let mut to_visit: Vec<Visit> = vec![Visit::Key(root)];
        while let Some(visit) = to_visit.pop() {
            let key = match visit {
                Visit::ChildrenFinished => {
                    let key = graph_path.pop().expect("path");
                    path_set.remove(&key);
                    if self.is_done(key) {
                        continue;
                    }
                    let temp: Vec<MKey> = flat(&self.in_progress(key).temp).collect();
                    if cycles_found < MAX_CYCLES_TO_STORE {
                        assert!(
                            temp.iter().all(|d| self.is_done(*d)),
                            "{key:?} has unsignaled deps"
                        );
                    } else if !temp.iter().all(|d| self.is_done(*d)) {
                        self.remove_incomplete_children_for_cycle(key, temp.into_iter().collect());
                    }
                    if self.maybe_handle_verified_clean_node(key) {
                        continue;
                    }
                    self.maybe_mark_rebuilding(key);
                    let mut errors = vec![];
                    let mut found_cycle = false;
                    for d in flat(&self.in_progress(key).temp.clone()) {
                        let v = self.done_value(d);
                        if let Some(e) = v.error {
                            found_cycle |= e.cycle;
                            errors.push(e);
                        }
                    }
                    assert!(found_cycle, "{key:?} had no cycle beneath it");
                    self.commit_cycle_error(cx, key, MError::from_child_errors(&errors));
                    continue;
                }
                Visit::Key(key) => key,
            };
            if self.is_done(key) {
                continue;
            }
            if cycles_found >= MAX_CYCLES_TO_STORE {
                continue;
            }
            if path_set.contains(&key) {
                let cycle_start = graph_path.iter().position(|k| *k == key).unwrap();
                cycles_found += 1;
                // Put this node into a consistent state for building if it is dirty (isDirty() is
                // !isDone()): signalDep with the graph version while checking dependencies.
                {
                    let version = cx.version;
                    let last_evaluated = self.nodes[&key].last_evaluated;
                    let ip = self.in_progress(key);
                    while ip.life == Life::CheckDependencies {
                        ip.life = if version > last_evaluated {
                            Life::NeedsRebuilding
                        } else {
                            Life::VerifiedClean
                        };
                    }
                    if ip.life == Life::NeedsRebuilding {
                        ip.life = Life::Rebuilding;
                    } else if self.maybe_handle_verified_clean_node(key) {
                        continue;
                    }
                }
                let cycle_child = if cycle_start + 1 == graph_path.len() {
                    key
                } else {
                    graph_path[cycle_start + 1]
                };
                self.remove_descendants_of_cycle_value(
                    cx,
                    key,
                    cycle_child,
                    &mut to_visit,
                    graph_path.len() - cycle_start,
                );
                let mut all_errors = vec![];
                for d in flat(&self.in_progress(key).temp.clone()) {
                    if d == cycle_child && !self.is_done(d) {
                        continue;
                    }
                    if let Some(e) = self.done_value(d).error {
                        all_errors.push(e);
                    }
                }
                all_errors.push(MError::from_cycle());
                self.commit_cycle_error(cx, key, MError::from_child_errors(&all_errors));
                continue;
            }
            let temp: Vec<MKey> = flat(&self.in_progress(key).temp).collect();
            if temp.is_empty() {
                continue;
            }
            to_visit.push(Visit::ChildrenFinished);
            graph_path.push(key);
            path_set.insert(key);
            for c in temp {
                if !self.is_done(c) {
                    to_visit.push(Visit::Key(c));
                }
            }
        }
        let v = self.done_value(root);
        v.error.expect("root in error")
    }

    // Java: com/google/devtools/build/skyframe/SimpleCycleDetector.java:516 SimpleCycleDetector.removeIncompleteChildrenForCycle
    fn remove_incomplete_children_for_cycle(
        &mut self,
        key: MKey,
        children: BTreeSet<MKey>,
    ) -> BTreeSet<MKey> {
        let unfinished: BTreeSet<MKey> =
            children.into_iter().filter(|c| !self.is_done(*c)).collect();
        let ip = self.in_progress(key);
        for g in &mut ip.temp {
            g.retain(|d| !unfinished.contains(d));
        }
        ip.temp.retain(|g| !g.is_empty());
        unfinished
    }

    // Java: com/google/devtools/build/skyframe/SimpleCycleDetector.java:465 SimpleCycleDetector.removeDescendantsOfCycleValue
    fn remove_descendants_of_cycle_value(
        &mut self,
        cx: &mut Ctx<'_>,
        key: MKey,
        cycle_child: MKey,
        to_visit: &mut Vec<Visit>,
        cycle_length: usize,
    ) {
        let mut unvisited: BTreeSet<MKey> = flat(&self.in_progress(key).temp).collect();
        unvisited.remove(&cycle_child);
        self.remove_incomplete_children_for_cycle(key, unvisited.clone());
        // The entry has at most one undone dep now, its cycleChild: signal to make it ready.
        let version = cx.version;
        let last_evaluated = self.nodes[&key].last_evaluated;
        let ip = self.in_progress(key);
        if ip.life == Life::CheckDependencies && version > last_evaluated {
            ip.life = Life::NeedsRebuilding;
        }
        self.maybe_mark_rebuilding(key);
        let mut cycle_length = cycle_length;
        let mut i = to_visit.len();
        while i > 0 {
            i -= 1;
            match to_visit[i] {
                Visit::ChildrenFinished => {
                    cycle_length -= 1;
                    if cycle_length == 0 {
                        return;
                    }
                }
                Visit::Key(d) => {
                    if cycle_length == 1 {
                        assert!(unvisited.contains(&d), "{key:?} {d:?} {cycle_child:?}");
                        to_visit.remove(i);
                    }
                }
            }
        }
        panic!(
            "There were not {cycle_length} groups of children when trying to remove children of \
             {key:?} other than {cycle_child:?}"
        );
    }

    // Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:147 AbstractParallelEvaluator.maybeMarkRebuilding
    fn maybe_mark_rebuilding(&mut self, key: MKey) {
        let ip = self.in_progress(key);
        if ip.life != Life::Rebuilding {
            assert_eq!(ip.life, Life::NeedsRebuilding, "markRebuilding of {key:?}");
            ip.life = Life::Rebuilding;
        }
    }

    // Java: com/google/devtools/build/skyframe/SimpleCycleDetector.java:330 SimpleCycleDetector.maybeHandleVerifiedCleanNode
    fn maybe_handle_verified_clean_node(&mut self, key: MKey) -> bool {
        if self.in_progress(key).life != Life::VerifiedClean {
            return false;
        }
        // Only when a node with a cycle is invalidated at the same version; never in the model
        // (invalidation happens between evaluations).
        unreachable!("verified clean in the cycle detector: {key:?}")
    }

    fn commit_cycle_error(&mut self, cx: &mut Ctx<'_>, key: MKey, e: MError) {
        let version = cx.version;
        let node = self.nodes.get_mut(&key).unwrap();
        let MState::InFlight(ip) = &mut node.state else {
            unreachable!()
        };
        let deps = std::mem::take(&mut ip.temp);
        // An error is never equal to the last build's value.
        node.last_changed = version;
        node.last_evaluated = version;
        node.state = MState::Done {
            value: MValue {
                value: None,
                error: Some(e),
            },
            deps,
        };
        self.evaluated(key);
        // ParallelEvaluatorContext.signalParentsOnAbort: in-flight parents that registered.
        let parents: Vec<MKey> = self
            .nodes
            .iter()
            .filter_map(|(p, n)| match &n.state {
                MState::InFlight(ip) if flat(&ip.temp).any(|d| d == key) => Some(*p),
                _ => None,
            })
            .collect();
        for p in parents {
            let last_evaluated = self.nodes[&p].last_evaluated;
            let ip = self.in_progress(p);
            if ip.life == Life::CheckDependencies && version > last_evaluated {
                ip.life = Life::NeedsRebuilding;
            }
        }
    }
}

/// The environment of one run of a function in the model.
struct ModelEnv<'g, 'c, 's> {
    g: &'g mut ModelGraph,
    cx: &'c mut Ctx<'s>,
    key: MKey,
    child_errors: Vec<MError>,
    any_not_done: bool,
    values_missing: bool,
    /// `getDepErrorKey() != null`: no keep_going and a dep in error was read.
    dep_error: bool,
}

impl ProgramEnv for ModelEnv<'_, '_, '_> {
    type Stop = Aborted;

    fn request(&mut self, group: &[Dep]) -> Result<Vec<DepRead>, Aborted> {
        let keep_going = self.cx.keep_going;
        let already: BTreeSet<MKey> = flat(&self.g.in_progress(self.key).temp).collect();
        let mut new_group = vec![];
        let mut reads = vec![];
        for d in group {
            let dk = MKey::Node(d.node);
            if !already.contains(&dk) && !new_group.contains(&dk) {
                new_group.push(dk);
            }
            let done = self.g.request(self.cx, dk);
            if self.cx.aborted {
                return Err(Aborted);
            }
            if !done {
                self.any_not_done = true;
                self.values_missing = true;
                reads.push(DepRead::Missing);
                continue;
            }
            let v = self.g.done_value(dk);
            let read = match (v.value, v.error) {
                (Some(x), None) => DepRead::Value(x),
                // SkyFunctionEnvironment.java:612-660 processDepValue, :767-791 handleError.
                (Some(x), Some(e)) => {
                    self.child_errors.push(e);
                    if keep_going {
                        DepRead::Value(x)
                    } else {
                        self.values_missing = true;
                        self.dep_error = true;
                        DepRead::Missing
                    }
                }
                (None, Some(e)) => {
                    self.child_errors.push(e);
                    if e.cycle || !keep_going {
                        self.values_missing = true;
                        if !keep_going {
                            self.dep_error = true;
                        }
                    }
                    if keep_going && e.exception.is_some() {
                        DepRead::Exception
                    } else {
                        self.values_missing = true;
                        DepRead::Missing
                    }
                }
                (None, None) => unreachable!("{dk:?} has neither value nor error"),
            };
            reads.push(read);
        }
        if !new_group.is_empty() {
            self.g.in_progress(self.key).temp.push(new_group);
        }
        Ok(reads)
    }

    fn values_missing(&self) -> bool {
        self.values_missing
    }
}
