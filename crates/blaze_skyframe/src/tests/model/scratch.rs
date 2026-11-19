//! From-scratch semantics: what a fresh evaluator must answer for a spec, roots and keep_going.
//!
//! Written from Skyframe's rules (crate docs in `src/lib.rs`; ParallelEvaluator.java,
//! AbstractParallelEvaluator.java, SkyFunctionEnvironment.java, SimpleCycleDetector.java), not
//! from the incremental model and not from Blaze's evaluator.
//!
//! keep_going (unique outcome): every node a root reaches is evaluated as far as it can be. A
//! node is *stuck* (it never finishes; the cycle detector then gives it a cycle error) when it
//! waits, directly or through stuck deps, on a node that waits on it; a depth-first evaluation
//! that treats "requested while on the stack" as not done finds exactly these nodes, because in
//! Java a node requesting a node that (transitively) requested it waits for it forever, and a
//! node whose deps all finish is restarted until it finishes. The order of evaluation does not
//! change any node's outcome: reads depend only on the deps' outcomes.
//!
//! no keep_going: unique only when no node the roots reach throws and none is in or above a
//! cycle: then every root has its value. Otherwise the outcome is a set ([`Allowed::Loose`]):
//! with exceptions the first one thrown wins, and which one that is depends on scheduling
//! (ParallelEvaluator.java:205-238, `bubbleErrorUp` :290-480); with cycles only, a fresh
//! evaluator reports the cycle of the first unfinished root and nothing for the other unfinished
//! roots (SimpleCycleDetector.java:55-80 returns after it), but an incremental one that finds a
//! root done with a cycle error committed by an earlier keep_going evaluation stops before doing
//! anything and reports exactly the roots done with an error (ParallelEvaluator.java:670-690).

use super::error_model::MError;
use super::spec::{
    DepRead, ErrClass, GraphSpec, NodeId, NodeKind, ProgramEnv, RootResult, RunResult, run_program,
    stored_event_text, warning_text,
};
use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;

/// keep_going outcome of a node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kg {
    Value {
        v: i64,
        err: Option<MError>,
    },
    Error(MError),
    /// In or above a cycle: never finishes during the evaluation.
    Stuck,
}

/// What the result may say about one root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Allowed {
    Exactly(RootResult),
    /// No keep_going with an exception reachable: the root is absent, or has its keep_going
    /// value (only when that value carries no error), or an error whose exception comes from a
    /// node it reaches that throws (any transience; in keep_going, or in error bubbling, see
    /// [`eval_from_scratch`]), or a cycle error when it reaches a cycle.
    Loose {
        value: Option<i64>,
        exception_origins: BTreeSet<NodeId>,
        cycle: bool,
    },
}

impl Allowed {
    pub fn allows(&self, r: &RootResult) -> bool {
        match self {
            Allowed::Exactly(e) => e == r,
            Allowed::Loose {
                value,
                exception_origins,
                cycle,
            } => match r {
                RootResult::Absent => true,
                RootResult::Value(v) => *value == Some(*v),
                RootResult::Error(ErrClass::Cycle) => *cycle,
                RootResult::Error(ErrClass::Exception { origin, .. }) => {
                    exception_origins.contains(origin)
                }
            },
        }
    }
}

/// The from-scratch answer.
#[derive(Clone, Debug)]
pub struct Expected {
    /// One entry per distinct root, in root order.
    pub per_root: Vec<(NodeId, Allowed)>,
    /// True when every root has an `Allowed::Exactly`.
    pub unique: bool,
    /// `EvaluationResult.hasError()`, when determined.
    pub has_error: Option<bool>,
    /// keep_going outcome of every node the roots reach.
    pub kg: BTreeMap<NodeId, Kg>,
    /// Nodes the roots reach (keep_going requests).
    pub closure: BTreeSet<NodeId>,
    /// Nodes of the closure that throw (keep_going).
    #[allow(dead_code)] // part of the from-scratch answer; read by no check yet
    pub throwers: BTreeSet<NodeId>,
    /// Warnings a fresh evaluator reports (keep_going: every completed run; each node runs to
    /// completion at most once).
    pub warnings: BTreeSet<String>,
    /// Stored events a fresh evaluator reports with either event filter (keep_going): own events
    /// of every completed run.
    pub stored_events: BTreeSet<String>,
}

impl Expected {
    /// Checks an actual result (one entry per distinct root, any order; `Absent` may be left out).
    pub fn check(&self, actual: &[(NodeId, RootResult)], has_error: bool) -> Result<(), String> {
        let actual: BTreeMap<NodeId, RootResult> = actual.iter().copied().collect();
        for (root, allowed) in &self.per_root {
            let r = actual.get(root).copied().unwrap_or(RootResult::Absent);
            if !allowed.allows(&r) {
                return Err(format!("root n{root}: got {r:?}, allowed {allowed:?}"));
            }
        }
        if let Some(extra) = actual
            .keys()
            .find(|k| !self.per_root.iter().any(|(r, _)| r == *k))
        {
            return Err(format!("n{extra} is not a root"));
        }
        if let Some(h) = self.has_error
            && h != has_error
        {
            return Err(format!("has_error {has_error}, expected {h}"));
        }
        Ok(())
    }
}

struct Scratch<'a> {
    spec: &'a GraphSpec,
    memo: BTreeMap<NodeId, Kg>,
    stack: BTreeSet<NodeId>,
    requested: BTreeMap<NodeId, Vec<NodeId>>,
    completed: BTreeMap<NodeId, i64>,
    throwers: BTreeSet<NodeId>,
}

struct ScratchEnv<'s, 'a> {
    s: &'s mut Scratch<'a>,
    requested: Vec<NodeId>,
    child_errors: Vec<MError>,
    any_stuck: bool,
    values_missing: bool,
}

impl ProgramEnv for ScratchEnv<'_, '_> {
    type Stop = Infallible;

    fn request(&mut self, group: &[super::spec::Dep]) -> Result<Vec<DepRead>, Infallible> {
        let mut reads = vec![];
        for d in group {
            self.requested.push(d.node);
            let read = match self.s.eval(d.node) {
                Kg::Value { v, err } => {
                    // keep_going: the value of a node with a value and an error is returned
                    // (SkyFunctionEnvironment.java:767-775 handleError).
                    if let Some(e) = err {
                        self.child_errors.push(e);
                    }
                    DepRead::Value(v)
                }
                Kg::Error(e) => {
                    self.child_errors.push(e);
                    // SkyFunctionEnvironment.java:649-653: a cycle always marks values missing.
                    if e.cycle {
                        self.values_missing = true;
                    }
                    if e.exception.is_some() {
                        DepRead::Exception
                    } else {
                        self.values_missing = true;
                        DepRead::Missing
                    }
                }
                Kg::Stuck => {
                    self.any_stuck = true;
                    self.values_missing = true;
                    DepRead::Missing
                }
            };
            reads.push(read);
        }
        Ok(reads)
    }

    fn values_missing(&self) -> bool {
        self.values_missing
    }
}

impl Scratch<'_> {
    fn eval(&mut self, n: NodeId) -> Kg {
        if let Some(r) = self.memo.get(&n) {
            return *r;
        }
        if self.stack.contains(&n) {
            // n waits (transitively) for the requester, which now waits for n.
            return Kg::Stuck;
        }
        let r = match &self.spec.node(n).kind {
            NodeKind::Leaf(v) => {
                self.requested.insert(n, vec![]);
                Kg::Value { v: *v, err: None }
            }
            NodeKind::Function(p) => {
                self.stack.insert(n);
                let mut env = ScratchEnv {
                    s: self,
                    requested: vec![],
                    child_errors: vec![],
                    any_stuck: false,
                    values_missing: false,
                };
                let Ok(out) = run_program(p, &mut env);
                let ScratchEnv {
                    requested,
                    child_errors,
                    any_stuck,
                    ..
                } = env;
                self.stack.remove(&n);
                self.requested.insert(n, requested);
                if let Some(pre) = out.completed {
                    self.completed.insert(n, pre);
                }
                match out.result {
                    // keep_going: a function that returned null with every dep done gets its
                    // children's error (AbstractParallelEvaluator.java:703-716); with a dep not
                    // done it waits for ever.
                    RunResult::Missing if any_stuck => Kg::Stuck,
                    RunResult::Missing => Kg::Error(MError::from_child_errors(&child_errors)),
                    // AbstractParallelEvaluator.java:527-534: transitively transient if this
                    // exception is transient or a dep's error is.
                    RunResult::Throw { transient } => {
                        self.throwers.insert(n);
                        Kg::Error(MError::from_exception(
                            n,
                            transient || child_errors.iter().any(|e| e.transitively_transient),
                        ))
                    }
                    // ErrorInfoManager.java:55-66: a value with child errors carries them.
                    RunResult::Value(v) => Kg::Value {
                        v,
                        err: (!child_errors.is_empty())
                            .then(|| MError::from_child_errors(&child_errors).with_value()),
                    },
                }
            }
        };
        self.memo.insert(n, r);
        r
    }

    fn closure_of(&self, root: NodeId) -> BTreeSet<NodeId> {
        let mut seen = BTreeSet::new();
        let mut todo = vec![root];
        while let Some(n) = todo.pop() {
            if seen.insert(n) {
                todo.extend(self.requested.get(&n).into_iter().flatten().copied());
            }
        }
        seen
    }
}

/// Dedups roots keeping the first occurrence (Java `ImmutableSet.copyOf(roots)`).
pub fn distinct_roots(roots: &[NodeId]) -> Vec<NodeId> {
    let mut seen = BTreeSet::new();
    roots.iter().copied().filter(|r| seen.insert(*r)).collect()
}

/// The keep_going outcome of every node reached from `roots`, with the requests each made.
pub fn keep_going_outcomes(spec: &GraphSpec, roots: &[NodeId]) -> BTreeMap<NodeId, Kg> {
    let mut s = Scratch {
        spec,
        memo: BTreeMap::new(),
        stack: BTreeSet::new(),
        requested: BTreeMap::new(),
        completed: BTreeMap::new(),
        throwers: BTreeSet::new(),
    };
    for r in roots {
        s.eval(*r);
    }
    s.memo
}

/// The from-scratch answer for `roots` (see the module docs for which cases are unique).
pub fn eval_from_scratch(spec: &GraphSpec, roots: &[NodeId], keep_going: bool) -> Expected {
    let roots = distinct_roots(roots);
    let mut s = Scratch {
        spec,
        memo: BTreeMap::new(),
        stack: BTreeSet::new(),
        requested: BTreeMap::new(),
        completed: BTreeMap::new(),
        throwers: BTreeSet::new(),
    };
    for r in &roots {
        s.eval(*r);
    }
    let closure: BTreeSet<NodeId> = s.memo.keys().copied().collect();
    let mut warnings = BTreeSet::new();
    let mut stored_events = BTreeSet::new();
    for (n, pre) in &s.completed {
        let p = spec.program(*n).expect("leaves do not run here");
        if p.warning {
            warnings.insert(warning_text(*n, *pre));
        }
        if p.stored_event {
            stored_events.insert(stored_event_text(*n, *pre));
        }
    }
    let kg_result = |r: NodeId| match s.memo[&r] {
        Kg::Value { v, .. } => RootResult::Value(v),
        Kg::Error(e) => RootResult::Error(e.class()),
        Kg::Stuck => RootResult::Error(ErrClass::Cycle),
    };
    let mut per_root = vec![];
    let unique;
    let has_error;
    if keep_going {
        for r in &roots {
            per_root.push((*r, Allowed::Exactly(kg_result(*r))));
        }
        unique = true;
        has_error = Some(
            per_root
                .iter()
                .any(|(_, a)| matches!(a, Allowed::Exactly(RootResult::Error(_)))),
        );
    } else if s.throwers.is_empty() && !s.memo.values().any(|k| *k == Kg::Stuck) {
        // Neither an exception nor a cycle: nothing stops the evaluation and every root has its
        // value.
        for r in &roots {
            let a = match s.memo[r] {
                Kg::Value { v, err: None } => RootResult::Value(v),
                other => unreachable!("no exception, no cycle, but {other:?}"),
            };
            per_root.push((*r, Allowed::Exactly(a)));
        }
        unique = true;
        has_error = Some(false);
    } else {
        // A fresh evaluator without exceptions reports the cycle of the first unfinished root
        // only (SimpleCycleDetector.java:55-80) and the values of the others; an incremental one
        // may instead find cycle errors committed by an earlier keep_going evaluation, and then
        // reports exactly the roots done with an error (ParallelEvaluator.java:670-690, "Errors,
        // even cached ones, should halt evaluations not in keepGoing mode"). With exceptions the
        // first one thrown wins (scheduling). Hence a set.
        // Error bubbling (ParallelEvaluator.java:290-470 `bubbleErrorUp`) re-runs each parent on
        // the path from the failed node up to a root with the errors built so far
        // (`SkyFunctionEnvironment.createForError`, :414-420); `getValueOrThrow` hands it a
        // bubbled exception (SkyFunctionEnvironment.java:464-467, :767-791 `handleError`:
        // "The exception may only propagate in keepGoing mode or during error bubbling"), and a
        // parent that catches it and throws its own exception replaces the error on the way up
        // (ParallelEvaluator.java:433-447). Such a parent need not throw in keep_going: there it
        // may wait on a cycle through another dep, which bubbling never evaluates. So an
        // origin is a keep_going thrower or any node that can throw and reaches one.
        let may_throw_in_bubbling = |n: NodeId| {
            spec.program(n).is_some_and(|p| p.throws.is_some())
                && !s.closure_of(n).is_disjoint(&s.throwers)
        };
        for r in &roots {
            let c = s.closure_of(*r);
            let value = match s.memo[r] {
                Kg::Value { v, err: None } => Some(v),
                _ => None,
            };
            per_root.push((
                *r,
                Allowed::Loose {
                    value,
                    exception_origins: c
                        .iter()
                        .copied()
                        .filter(|n| s.throwers.contains(n) || may_throw_in_bubbling(*n))
                        .collect(),
                    cycle: c.iter().any(|n| s.memo[n] == Kg::Stuck),
                },
            ));
        }
        unique = false;
        has_error = Some(true);
    }
    Expected {
        per_root,
        unique,
        has_error,
        kg: s.memo.clone(),
        closure,
        throwers: s.throwers.clone(),
        warnings,
        stored_events,
    }
}
