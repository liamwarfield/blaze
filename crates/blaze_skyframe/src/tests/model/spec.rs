//! The graph spec (pure data), edits, evaluation requests and the program interpreter shared by
//! the from-scratch semantics, the incremental model and (later) the driver's `SkyFunction`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// A node of the spec: `0..spec.nodes.len()`.
pub type NodeId = u32;

/// A random graph. Node `i` is `nodes[i]`. The kind of a node (leaf or function) never changes
/// within a scenario.
#[derive(Clone, PartialEq, Eq)]
pub struct GraphSpec {
    pub nodes: Vec<NodeSpec>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeSpec {
    pub kind: NodeKind,
    /// Whether the node's values compare equal by value (Java: a value class with `equals`, the
    /// Rust `Key::values_equal` returning `a == b`). `false` is Java identity equality / a
    /// `NotComparableSkyValue`: two values computed by two runs are never equal. Used by change
    /// pruning (`DirtyBuildingState.unchangedFromLastBuild`, DirtyBuildingState.java:214-219) and
    /// by `pruneInjectedValues` (AbstractInMemoryMemoizingEvaluator.java:432-465).
    pub comparable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeKind {
    /// An injected value. If the node is ever evaluated by its function (it was deleted, or never
    /// injected) the function returns this value, like GraphTester's constant `TestFunction`.
    Leaf(i64),
    Function(Program),
}

/// What a function node's `SkyFunction` does, interpreted by [`run_program`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub steps: Vec<Step>,
    /// Result: `(bias + sum of the dep values read) mod modulus` (`rem_euclid`), so that equal
    /// values recur and change pruning has something to prune.
    pub modulus: i64,
    pub bias: i64,
    pub throws: Option<Throw>,
    /// Emits a warning (`EventKind::Warning`, not stored for replay, Event.java:75) with text
    /// [`warning_text`] when the run read every dep it asked for.
    pub warning: bool,
    /// Emits an error event (`EventKind::Error`, stored for replay) with text
    /// [`stored_event_text`] under the same condition.
    pub stored_event: bool,
}

/// One dependency request of a run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// One group: a single dep is `env.getValue` (or `getValueOrThrow` when it catches), several
    /// deps are one `env.getValuesAndExceptions` batch read with `get` / `getOrThrow` in order.
    Request(Vec<Dep>),
    /// Requests `then` if the value read for `on` (requested by an earlier `Request` step) is
    /// even, else `otherwise`. If `on` is missing the run stops here and returns null, as a Java
    /// function must (it cannot branch on a value it does not have).
    IfEven {
        on: NodeId,
        then: Vec<Dep>,
        otherwise: Vec<Dep>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dep {
    pub node: NodeId,
    /// `Some(v)`: request with `getValueOrThrow(dep, SomeErrorException.class)` and use `v` when
    /// the dep's exception is thrown (keep_going or error bubbling only; a cycle is never
    /// thrown, SkyFunctionEnvironment.java:767-791 `handleError`).
    pub catch: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Throw {
    /// `Transience.TRANSIENT` (the node then depends on `ErrorTransienceValue.KEY`,
    /// SkyFunctionEnvironment.java:422-447 `setError`).
    pub transient: bool,
    pub when: ThrowWhen,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThrowWhen {
    Always,
    /// Throws when the result would be this value.
    ResultIs(i64),
}

impl GraphSpec {
    pub fn len(&self) -> u32 {
        self.nodes.len() as u32
    }

    pub fn node(&self, n: NodeId) -> &NodeSpec {
        &self.nodes[n as usize]
    }

    pub fn program(&self, n: NodeId) -> Option<&Program> {
        match &self.node(n).kind {
            NodeKind::Function(p) => Some(p),
            NodeKind::Leaf(_) => None,
        }
    }

    pub fn leaves(&self) -> impl Iterator<Item = (NodeId, i64)> + '_ {
        self.nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| match n.kind {
                NodeKind::Leaf(v) => Some((i as NodeId, v)),
                NodeKind::Function(_) => None,
            })
    }

    /// Every dep any branch of the node's program may request (the edges an upper bound must
    /// follow).
    pub fn all_possible_deps(&self, n: NodeId) -> BTreeSet<NodeId> {
        let mut out = BTreeSet::new();
        if let Some(p) = self.program(n) {
            for s in &p.steps {
                match s {
                    Step::Request(g) => out.extend(g.iter().map(|d| d.node)),
                    Step::IfEven {
                        then, otherwise, ..
                    } => {
                        out.extend(then.iter().map(|d| d.node));
                        out.extend(otherwise.iter().map(|d| d.node));
                    }
                }
            }
        }
        out
    }

    /// Checks the invariants the generator keeps: a dep appears at most once in a program (all
    /// branches together), `IfEven.on` names a dep of an earlier `Request` step, modulus >= 1.
    pub fn validate(&self) -> Result<(), String> {
        for (i, n) in self.nodes.iter().enumerate() {
            let NodeKind::Function(p) = &n.kind else {
                continue;
            };
            if p.modulus < 1 {
                return Err(format!("n{i}: modulus {}", p.modulus));
            }
            let mut seen = BTreeSet::new();
            let mut requested_unconditionally = BTreeSet::new();
            for s in &p.steps {
                let deps: Vec<&Dep> = match s {
                    Step::Request(g) => g.iter().collect(),
                    Step::IfEven {
                        on,
                        then,
                        otherwise,
                    } => {
                        if !requested_unconditionally.contains(on) {
                            return Err(format!("n{i}: IfEven on n{on} not requested before"));
                        }
                        then.iter().chain(otherwise.iter()).collect()
                    }
                };
                for d in &deps {
                    if d.node >= self.len() {
                        return Err(format!("n{i}: dep n{} out of range", d.node));
                    }
                    if !seen.insert(d.node) {
                        return Err(format!("n{i}: dep n{} requested twice", d.node));
                    }
                }
                if let Step::Request(g) = s {
                    requested_unconditionally.extend(g.iter().map(|d| d.node));
                }
            }
        }
        Ok(())
    }
}

/// Deterministic, stable output (node per line), so a failing seed prints a readable spec.
impl fmt::Debug for GraphSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "GraphSpec {{")?;
        for (i, n) in self.nodes.iter().enumerate() {
            let c = if n.comparable { "" } else { " !cmp" };
            match &n.kind {
                NodeKind::Leaf(v) => writeln!(f, "  n{i}{c}: leaf {v}")?,
                NodeKind::Function(p) => writeln!(f, "  n{i}{c}: {p}")?,
            }
        }
        write!(f, "}}")
    }
}

fn fmt_group(g: &[Dep]) -> String {
    let parts: Vec<String> = g
        .iter()
        .map(|d| match d.catch {
            None => format!("n{}", d.node),
            Some(v) => format!("n{}?{v}", d.node),
        })
        .collect();
    format!("[{}]", parts.join(","))
}

impl fmt::Display for Program {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for s in &self.steps {
            match s {
                Step::Request(g) => write!(f, "{} ", fmt_group(g))?,
                Step::IfEven {
                    on,
                    then,
                    otherwise,
                } => write!(
                    f,
                    "if-even(n{on})?{}:{} ",
                    fmt_group(then),
                    fmt_group(otherwise)
                )?,
            }
        }
        write!(f, "=> (sum+{}) mod {}", self.bias, self.modulus)?;
        if let Some(t) = &self.throws {
            let tr = if t.transient {
                "transient"
            } else {
                "persistent"
            };
            match t.when {
                ThrowWhen::Always => write!(f, " throws {tr}")?,
                ThrowWhen::ResultIs(r) => write!(f, " throws {tr} if {r}")?,
            }
        }
        if self.warning {
            write!(f, " +warn")?;
        }
        if self.stored_event {
            write!(f, " +event")?;
        }
        Ok(())
    }
}

/// The exception message of node `n` (unique per node). The driver's `SkyFunction` throws
/// `SomeErrorException(exception_message(n))`.
pub fn exception_message(n: NodeId) -> String {
    format!("n{n} failed")
}

/// The text of the warning of node `n` whose run computed `pre` (unique per node and value).
pub fn warning_text(n: NodeId, pre: i64) -> String {
    format!("n{n} warns {pre}")
}

/// The text of the stored (error-kind) event of node `n` whose run computed `pre`.
pub fn stored_event_text(n: NodeId, pre: i64) -> String {
    format!("n{n} reports {pre}")
}

/// Edits between evaluations, applied to the spec in order. Each one says which evaluator call
/// the driver makes (see the driver contract in `model.rs`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Leaf gets a new value (possibly the same one). Driver: `inject(key, value)`.
    SetLeaf { node: NodeId, value: i64 },
    /// Function node gets a new program (possibly equal). Driver: change the program, then
    /// `invalidate([key])` (Java CHANGE on a non-hermetic key, NodeEntry.java:95-135).
    SetProgram { node: NodeId, program: Program },
    /// No spec change. Driver: `invalidate([key])`.
    Invalidate { node: NodeId },
    /// Driver: `delete(|key, _value| nodes.contains(key))` (`MemoizingEvaluator.delete`, which
    /// also deletes every dirty node, AbstractInMemoryMemoizingEvaluator.java:212-225).
    Delete { nodes: BTreeSet<NodeId> },
    /// Driver: `delete_dirty(age)` (AbstractInMemoryMemoizingEvaluator.java:243-255).
    DeleteDirty { age: u64 },
    /// Driver: `invalidate_transient_errors()` (RecordingDifferencer.java:33).
    InvalidateTransientErrors,
}

impl Edit {
    pub fn apply(&self, spec: &mut GraphSpec) {
        match self {
            Edit::SetLeaf { node, value } => {
                let n = &mut spec.nodes[*node as usize];
                assert!(matches!(n.kind, NodeKind::Leaf(_)), "n{node} is not a leaf");
                n.kind = NodeKind::Leaf(*value);
            }
            Edit::SetProgram { node, program } => {
                let n = &mut spec.nodes[*node as usize];
                assert!(
                    matches!(n.kind, NodeKind::Function(_)),
                    "n{node} is not a function"
                );
                n.kind = NodeKind::Function(program.clone());
            }
            Edit::Invalidate { .. }
            | Edit::Delete { .. }
            | Edit::DeleteDirty { .. }
            | Edit::InvalidateTransientErrors => {}
        }
    }
}

/// One `evaluate` call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalRequest {
    /// In request order; duplicates are allowed (Java dedups them, `ImmutableSet.copyOf`).
    pub roots: Vec<NodeId>,
    pub keep_going: bool,
    pub parallelism: usize,
    /// `Some(k)`: the driver interrupts the evaluator when the `k`-th function invocation of
    /// this evaluation starts (0-based); if fewer run, the evaluation is not interrupted.
    pub interrupt_after_runs: Option<u32>,
}

/// A generated test case: the initial spec and the steps. The first step's edits inject every
/// leaf.
#[derive(Clone, Debug)]
pub struct Scenario {
    pub seed: u64,
    pub spec: GraphSpec,
    pub steps: Vec<(Vec<Edit>, EvalRequest)>,
}

/// What a dependency read gives the function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DepRead {
    /// The dep's value (in keep_going also the value of a dep that has a value and an error).
    Value(i64),
    /// The dep's exception (keep_going or error bubbling): thrown to a `getValueOrThrow` that
    /// catches it, else the dep is missing.
    Exception,
    /// Not done, in a cycle, or in error without being catchable: `null`, values missing.
    Missing,
}

/// The function's view of `SkyFunction.Environment`, as far as programs use it.
pub trait ProgramEnv {
    type Stop;
    /// Requests `group` (one `getValue` / `getValueOrThrow` for one dep, one
    /// `getValuesAndExceptions` for several) and returns one read per dep in order.
    fn request(&mut self, group: &[Dep]) -> Result<Vec<DepRead>, Self::Stop>;
    /// `env.valuesMissing()`. Besides the reads that are [`DepRead::Missing`] (and exceptions
    /// not caught), Java sets it for a dep whose error has a cycle even when the error also has
    /// an exception that was thrown to the function (SkyFunctionEnvironment.java:649-653).
    fn values_missing(&self) -> bool;
}

/// The result of one run of a program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunResult {
    Value(i64),
    Throw {
        transient: bool,
    },
    /// Returned null with values missing.
    Missing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunOutcome {
    pub result: RunResult,
    /// `Some(pre)` when the run read every dep it asked for: it computed `pre` and emitted its
    /// warning and stored event (when the program has them) before returning or throwing.
    pub completed: Option<i64>,
}

/// Runs a program: the steps in order, then `if (env.valuesMissing()) return null`, then the
/// result (events are emitted, then the function throws or returns). Deterministic in the
/// reads.
pub fn run_program<E: ProgramEnv>(p: &Program, env: &mut E) -> Result<RunOutcome, E::Stop> {
    let mut values: BTreeMap<NodeId, Option<i64>> = BTreeMap::new();
    let mut missing = false;
    for step in &p.steps {
        let group: &[Dep] = match step {
            Step::Request(g) => g,
            Step::IfEven {
                on,
                then,
                otherwise,
            } => match values.get(on) {
                Some(Some(v)) if v.rem_euclid(2) == 0 => then,
                Some(Some(_)) => otherwise,
                _ => {
                    return Ok(RunOutcome {
                        result: RunResult::Missing,
                        completed: None,
                    });
                }
            },
        };
        if group.is_empty() {
            continue;
        }
        let reads = env.request(group)?;
        assert_eq!(reads.len(), group.len());
        for (d, r) in group.iter().zip(reads) {
            let v = match (r, d.catch) {
                (DepRead::Value(v), _) => Some(v),
                (DepRead::Exception, Some(c)) => Some(c),
                (DepRead::Exception, None) | (DepRead::Missing, _) => None,
            };
            if v.is_none() {
                missing = true;
            }
            values.insert(d.node, v);
        }
    }
    // A Java function checks `if (env.valuesMissing()) return null;` here.
    if missing || env.values_missing() {
        return Ok(RunOutcome {
            result: RunResult::Missing,
            completed: None,
        });
    }
    let sum: i64 = values.values().map(|v| v.expect("not missing")).sum();
    let pre = (p.bias + sum).rem_euclid(p.modulus);
    let result = match &p.throws {
        Some(t)
            if match t.when {
                ThrowWhen::Always => true,
                ThrowWhen::ResultIs(r) => r == pre,
            } =>
        {
            RunResult::Throw {
                transient: t.transient,
            }
        }
        _ => RunResult::Value(pre),
    };
    Ok(RunOutcome {
        result,
        completed: Some(pre),
    })
}

/// Error class compared between engines.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ErrClass {
    /// The representative exception is node `origin`'s; `transient` is
    /// `ErrorInfo.isTransitivelyTransient`.
    Exception { origin: NodeId, transient: bool },
    /// The error has cycle infos. Its exception, if any, is not compared (it depends on how far
    /// a dirty node got in checking its deps before it was stuck, see the notes).
    Cycle,
}

/// What `EvaluationResult` says about one root.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RootResult {
    /// `result.get(root)` is this value (in keep_going also when the node carries an error).
    Value(i64),
    /// `result.getError(root)`.
    Error(ErrClass),
    /// Neither (no keep_going: the root was not finished when the evaluation stopped, or a
    /// cycle root after the first one).
    Absent,
}
