//! Hand-derived scenarios: the expected `ran`, `change_pruned` and `marked_clean` sets are
//! worked out from the Java (reasoning in the comments) and written as literals, then checked
//! against the model. Graph versions: the first evaluation is version 0, each `evaluate` adds 1
//! (AbstractInMemoryMemoizingEvaluator.java:495-503). "vN" below is a node's version
//! (`lastChanged`) or last-evaluated version.

use super::graph::{MKey, MState, ModelGraph};
use super::scratch::eval_from_scratch;
use super::spec::{
    Dep, Edit, ErrClass, EvalRequest, GraphSpec, NodeId, NodeKind, NodeSpec, Program, RootResult,
    Step, Throw, ThrowWhen,
};
use std::collections::BTreeSet;

fn leaf(v: i64) -> NodeSpec {
    NodeSpec {
        kind: NodeKind::Leaf(v),
        comparable: true,
    }
}

fn opaque(mut n: NodeSpec) -> NodeSpec {
    n.comparable = false;
    n
}

fn program(steps: Vec<Step>, modulus: i64) -> Program {
    Program {
        steps,
        modulus,
        bias: 0,
        throws: None,
        warning: false,
        stored_event: false,
    }
}

fn func(steps: Vec<Step>, modulus: i64) -> NodeSpec {
    NodeSpec {
        kind: NodeKind::Function(program(steps, modulus)),
        comparable: true,
    }
}

fn throwing(steps: Vec<Step>, transient: bool) -> NodeSpec {
    let mut p = program(steps, 100);
    p.throws = Some(Throw {
        transient,
        when: ThrowWhen::Always,
    });
    NodeSpec {
        kind: NodeKind::Function(p),
        comparable: true,
    }
}

fn req(nodes: &[NodeId]) -> Step {
    Step::Request(
        nodes
            .iter()
            .map(|n| Dep {
                node: *n,
                catch: None,
            })
            .collect(),
    )
}

fn catching(node: NodeId, value: i64) -> Step {
    Step::Request(vec![Dep {
        node,
        catch: Some(value),
    }])
}

fn kg(roots: &[NodeId]) -> EvalRequest {
    EvalRequest {
        roots: roots.to_vec(),
        keep_going: true,
        parallelism: 1,
        interrupt_after_runs: None,
    }
}

fn nkg(roots: &[NodeId]) -> EvalRequest {
    EvalRequest {
        keep_going: false,
        ..kg(roots)
    }
}

fn inject_all(spec: &GraphSpec) -> Vec<Edit> {
    spec.leaves()
        .map(|(node, value)| Edit::SetLeaf { node, value })
        .collect()
}

struct Expect {
    ran: &'static [NodeId],
    change_pruned: &'static [NodeId],
    marked_clean: &'static [NodeId],
    /// Results per distinct root, when the step checks them.
    results: Option<Vec<(NodeId, RootResult)>>,
}

fn sets(ran: &'static [NodeId], pruned: &'static [NodeId], clean: &'static [NodeId]) -> Expect {
    Expect {
        ran,
        change_pruned: pruned,
        marked_clean: clean,
        results: None,
    }
}

impl Expect {
    fn with(mut self, results: Vec<(NodeId, RootResult)>) -> Expect {
        self.results = Some(results);
        self
    }
}

/// Runs the steps (the first step's edits are prefixed with injections of every leaf) and
/// returns the model for extra checks.
fn run_case(
    name: &str,
    mut spec: GraphSpec,
    steps: Vec<(Vec<Edit>, EvalRequest, Expect)>,
) -> (ModelGraph, GraphSpec) {
    spec.validate().unwrap();
    let mut model = ModelGraph::new();
    for (i, (mut edits, request, expect)) in steps.into_iter().enumerate() {
        if i == 0 {
            let mut all = inject_all(&spec);
            all.append(&mut edits);
            edits = all;
        }
        for e in &edits {
            model.apply_edit(&mut spec, e);
        }
        let run = model.evaluate(&spec, &request);
        let at = format!("{name} step {i}: {run:?}");
        assert!(run.exact, "{at}");
        let set = |s: &[NodeId]| s.iter().copied().collect::<BTreeSet<NodeId>>();
        assert_eq!(run.ran, set(expect.ran), "ran: {at}");
        assert_eq!(
            run.change_pruned,
            set(expect.change_pruned),
            "change_pruned: {at}"
        );
        assert_eq!(
            run.marked_clean,
            set(expect.marked_clean),
            "marked_clean: {at}"
        );
        let results = run.results.clone().expect("results");
        if let Some(r) = expect.results {
            assert_eq!(results, r, "results: {at}");
        }
        eval_from_scratch(&spec, &request.roots, request.keep_going)
            .check(&results, run.has_error.unwrap())
            .unwrap_or_else(|e| panic!("{at}: {e}"));
    }
    (model, spec)
}

/// A leaf changes but the middle node's value stays equal: it is rebuilt and change pruned, and
/// its parents are marked clean without running.
///
/// n0 = leaf 1, n1 = n0 mod 2, n2 = n1, n3 = n2. Step 1: n0 := 3. n0 is CHANGE-dirtied and its
/// reverse closure DIRTY (InvalidatingNodeVisitor.java:529-566); n0 is injected at v1. n3 checks
/// its only group [n2], n2 checks [n1], n1 checks [n0]: v1 > n1's last-evaluated v0, so n1 is
/// NEEDS_REBUILDING (DirtyBuildingState.java:192-195) and runs: 3 mod 2 = 1, equal to its last
/// value, so it keeps last-changed v0 (IncrementalInMemoryNodeEntry.java:149-155). n2 is signalled
/// with v0 <= v0 after its last group: VERIFIED_CLEAN (:196-199), markClean; same for n3.
#[test]
fn leaf_change_pruned_at_middle_node_marks_parents_clean() {
    let spec = GraphSpec {
        nodes: vec![
            leaf(1),
            func(vec![req(&[0])], 2),
            func(vec![req(&[1])], 100),
            func(vec![req(&[2])], 100),
        ],
    };
    run_case(
        "pruned middle",
        spec,
        vec![
            (vec![], kg(&[3]), sets(&[1, 2, 3], &[], &[])),
            (
                vec![Edit::SetLeaf { node: 0, value: 3 }],
                kg(&[3]),
                sets(&[1], &[1], &[2, 3]).with(vec![(3, RootResult::Value(1))]),
            ),
        ],
    );
}

/// A dirty node whose first dep group changed never has its second group evaluated.
///
/// n3 = [n0], if-even(n0) [n2] else []; n2 = n1. Step 1: n0 := 1, n1 := 7. n3 is DIRTY (via n0
/// and via n2). It checks group [n0] first (AbstractParallelEvaluator.java:238-252): changed, so
/// it rebuilds; n0 = 1 is odd, so it does not request n2, which is never evaluated and stays
/// dirty; at commit n3 drops its reverse dep on n2 (SkyFunctionEnvironment.java:960-968).
/// Step 2: n1 := 8 dirties nothing new (n2 is already dirty and no longer n3's dep), every root
/// is done: nothing runs (ParallelEvaluator.java:659-666).
#[test]
fn changed_first_group_skips_second_group() {
    let spec = GraphSpec {
        nodes: vec![
            leaf(0),
            leaf(0),
            func(vec![req(&[1])], 100),
            func(
                vec![
                    req(&[0]),
                    Step::IfEven {
                        on: 0,
                        then: vec![Dep {
                            node: 2,
                            catch: None,
                        }],
                        otherwise: vec![],
                    },
                ],
                100,
            ),
        ],
    };
    let (model, _) = run_case(
        "first group",
        spec,
        vec![
            (vec![], kg(&[3]), sets(&[2, 3], &[], &[])),
            (
                vec![
                    Edit::SetLeaf { node: 0, value: 1 },
                    Edit::SetLeaf { node: 1, value: 7 },
                ],
                kg(&[3]),
                sets(&[3], &[], &[]).with(vec![(3, RootResult::Value(1))]),
            ),
            (
                vec![Edit::SetLeaf { node: 1, value: 8 }],
                kg(&[3]),
                sets(&[], &[], &[]),
            ),
        ],
    );
    assert!(matches!(
        model.node(MKey::Node(2)).unwrap().state,
        MState::Dirty { .. }
    ));
}

/// A node with a transient error reruns after `invalidate_transient_errors` and not otherwise.
///
/// n1 = [n0] throws transient. Its deps are [[n0], [ErrorTransienceValue.KEY]]
/// (SkyFunctionEnvironment.java:426-444). Step 1: nothing pending, the root is done: nothing
/// runs. Step 2: the error-transience node is CHANGEd (RecordingDifferencer.java:33-37), n1
/// DIRTY; the evaluation re-injects the error-transience node at v2 (ParallelEvaluator.java:
/// 187-202; it equals nothing, ErrorTransienceValue.java:59). n1 checks [n0] (unchanged), then
/// the error-transience group, newer than n1's v0: forceRebuild (AbstractParallelEvaluator.java:
/// 254-262); n1 runs and fails again (errors are never pruned). Step 3: injecting n0's own value
/// is pruned (AbstractInMemoryMemoizingEvaluator.java:441-447): nothing runs.
#[test]
fn transient_error_reruns_only_after_invalidate_transient_errors() {
    let spec = GraphSpec {
        nodes: vec![leaf(0), throwing(vec![req(&[0])], true)],
    };
    let err = RootResult::Error(ErrClass::Exception {
        origin: 1,
        transient: true,
    });
    run_case(
        "transient",
        spec,
        vec![
            (vec![], kg(&[1]), sets(&[1], &[], &[]).with(vec![(1, err)])),
            (vec![], kg(&[1]), sets(&[], &[], &[]).with(vec![(1, err)])),
            (
                vec![Edit::InvalidateTransientErrors],
                kg(&[1]),
                sets(&[1], &[], &[]).with(vec![(1, err)]),
            ),
            (
                vec![Edit::SetLeaf { node: 0, value: 0 }],
                kg(&[1]),
                sets(&[], &[], &[]),
            ),
        ],
    );
}

/// An injected value equal to the old one dirties nothing (`pruneInjectedValues`,
/// AbstractInMemoryMemoizingEvaluator.java:441-447).
#[test]
fn equal_injection_dirties_nothing() {
    let spec = GraphSpec {
        nodes: vec![leaf(2), func(vec![req(&[0])], 100)],
    };
    let mut model = ModelGraph::new();
    let mut s = spec.clone();
    for e in inject_all(&spec) {
        model.apply_edit(&mut s, &e);
    }
    model.evaluate(&s, &kg(&[1]));
    model.apply_edit(&mut s, &Edit::SetLeaf { node: 0, value: 2 });
    let run = model.evaluate(&s, &kg(&[1]));
    assert!(run.dirtied.is_empty(), "{run:?}");
    run_case(
        "equal injection",
        spec,
        vec![
            (vec![], kg(&[1]), sets(&[1], &[], &[])),
            (
                vec![Edit::SetLeaf { node: 0, value: 2 }],
                kg(&[1]),
                sets(&[], &[], &[]),
            ),
        ],
    );
}

/// The same injection on a leaf whose values are not comparable: pruneInjectedValues uses
/// `equals` (identity here), so the leaf is CHANGEd and re-injected at v1 (ParallelEvaluator.java:
/// 625-629, not equal); n1 checks [n0], v1 > v0, reruns, computes the same comparable value and
/// is change pruned.
#[test]
fn opaque_equal_injection_reruns_parent_which_is_pruned() {
    let spec = GraphSpec {
        nodes: vec![opaque(leaf(2)), func(vec![req(&[0])], 100)],
    };
    run_case(
        "opaque injection",
        spec,
        vec![
            (vec![], kg(&[1]), sets(&[1], &[], &[])),
            (
                vec![Edit::SetLeaf { node: 0, value: 2 }],
                kg(&[1]),
                sets(&[1], &[1], &[]),
            ),
        ],
    );
}

/// Invalidating a function node (CHANGE, DirtyBuildingState.java:126-135: NEEDS_REBUILDING) whose
/// value stays the same: it runs and is change pruned; its parent is marked clean.
#[test]
fn invalidated_node_with_same_value_is_pruned() {
    let spec = GraphSpec {
        nodes: vec![
            leaf(1),
            func(vec![req(&[0])], 100),
            func(vec![req(&[1])], 100),
        ],
    };
    run_case(
        "invalidate same",
        spec,
        vec![
            (vec![], kg(&[2]), sets(&[1, 2], &[], &[])),
            (
                vec![Edit::Invalidate { node: 1 }],
                kg(&[2]),
                sets(&[1], &[1], &[2]),
            ),
        ],
    );
}

/// The same with a middle node whose values are not comparable: it gets a new version, so its
/// parent rebuilds (and, comparable, is pruned).
#[test]
fn invalidated_opaque_node_rebuilds_parent() {
    let spec = GraphSpec {
        nodes: vec![
            leaf(1),
            opaque(func(vec![req(&[0])], 100)),
            func(vec![req(&[1])], 100),
        ],
    };
    run_case(
        "invalidate opaque",
        spec,
        vec![
            (vec![], kg(&[2]), sets(&[1, 2], &[], &[])),
            (
                vec![Edit::Invalidate { node: 1 }],
                kg(&[2]),
                sets(&[1, 2], &[2], &[]),
            ),
        ],
    );
}

/// A program change makes a node fail and then stop failing (keep_going). An error is never
/// equal to anything (ValueWithMetadata.java:152), so both times the parent reruns; with the
/// child in error it returns null with every dep done and gets the child's error
/// (AbstractParallelEvaluator.java:703-716).
#[test]
fn node_starts_and_stops_failing() {
    let spec = GraphSpec {
        nodes: vec![
            leaf(1),
            func(vec![req(&[0])], 100),
            func(vec![req(&[1])], 100),
        ],
    };
    let failing = match throwing(vec![req(&[0])], false).kind {
        NodeKind::Function(p) => p,
        NodeKind::Leaf(_) => unreachable!(),
    };
    run_case(
        "start and stop failing",
        spec,
        vec![
            (vec![], kg(&[2]), sets(&[1, 2], &[], &[])),
            (
                vec![Edit::SetProgram {
                    node: 1,
                    program: failing,
                }],
                kg(&[2]),
                sets(&[1, 2], &[], &[]).with(vec![(
                    2,
                    RootResult::Error(ErrClass::Exception {
                        origin: 1,
                        transient: false,
                    }),
                )]),
            ),
            (
                vec![Edit::SetProgram {
                    node: 1,
                    program: program(vec![req(&[0])], 100),
                }],
                kg(&[2]),
                sets(&[1, 2], &[], &[]).with(vec![(2, RootResult::Value(1))]),
            ),
        ],
    );
}

/// keep_going: a node catches its dep's exception and returns a value; the node carries the
/// child's error (ErrorInfoManager.java:55-66), so it is never pruned. Without keep_going the
/// done root with an error is reported as an error and nothing runs (ParallelEvaluator.java:
/// 659-666 then `constructResult` :539-542).
#[test]
fn caught_error_value_carries_the_error() {
    let spec = GraphSpec {
        nodes: vec![
            leaf(1),
            throwing(vec![req(&[0])], false),
            func(vec![catching(1, 5)], 100),
        ],
    };
    let err = RootResult::Error(ErrClass::Exception {
        origin: 1,
        transient: false,
    });
    run_case(
        "caught",
        spec,
        vec![
            (
                vec![],
                kg(&[2]),
                sets(&[1, 2], &[], &[]).with(vec![(2, RootResult::Value(5))]),
            ),
            (vec![], kg(&[2]), sets(&[], &[], &[])),
            (
                vec![Edit::Invalidate { node: 1 }],
                kg(&[2]),
                sets(&[1, 2], &[], &[]).with(vec![(2, RootResult::Value(5))]),
            ),
            (vec![], nkg(&[2]), sets(&[], &[], &[]).with(vec![(2, err)])),
        ],
    );
}

/// A cycle appears and disappears (keep_going).
///
/// Step 1: n1 := [n0], [n2] (n2 = [n1]). n1 CHANGE, n2 DIRTY. n2 checks [n1]; n1 runs, asks for
/// n2 (waiting on n1): neither finishes. n2's function never ran (it is stuck checking its
/// deps). The cycle detector (SimpleCycleDetector.java:92-300) finds n2 -> n1 -> n2, commits n2
/// (cycle error) then n1. Step 2: n1 := [n0]. n1 CHANGE; n2 DIRTY (n1 is its dep); n1 is n2's
/// reverse dep but already changed. n2 checks [n1], n1 runs (0, not equal to its last value, an
/// error), n2 reruns.
#[test]
fn cycle_appears_and_disappears() {
    let spec = GraphSpec {
        nodes: vec![
            leaf(0),
            func(vec![req(&[0])], 100),
            func(vec![req(&[1])], 100),
        ],
    };
    run_case(
        "cycle",
        spec,
        vec![
            (vec![], kg(&[2]), sets(&[1, 2], &[], &[])),
            (
                vec![Edit::SetProgram {
                    node: 1,
                    program: program(vec![req(&[0]), req(&[2])], 100),
                }],
                kg(&[2]),
                sets(&[1], &[], &[]).with(vec![(2, RootResult::Error(ErrClass::Cycle))]),
            ),
            (
                vec![Edit::SetProgram {
                    node: 1,
                    program: program(vec![req(&[0])], 100),
                }],
                kg(&[2]),
                sets(&[1, 2], &[], &[]).with(vec![(2, RootResult::Value(0))]),
            ),
        ],
    );
}

/// Deleting a node deletes its reverse transitive closure (InvalidatingNodeVisitor.java:
/// 300-305), not its deps; the deleted nodes are new at the next evaluation (no last value, so
/// nothing is pruned).
#[test]
fn delete_removes_reverse_closure_only() {
    let spec = GraphSpec {
        nodes: vec![
            leaf(1),
            func(vec![req(&[0])], 100),
            func(vec![req(&[1])], 100),
            func(vec![req(&[0])], 100),
        ],
    };
    run_case(
        "delete",
        spec,
        vec![
            (vec![], kg(&[2, 3]), sets(&[1, 2, 3], &[], &[])),
            (
                vec![Edit::Delete {
                    nodes: BTreeSet::from([1]),
                }],
                kg(&[2, 3]),
                sets(&[1, 2], &[], &[]),
            ),
        ],
    );
}

fn pruning_chain() -> GraphSpec {
    GraphSpec {
        nodes: vec![
            leaf(1),
            func(vec![req(&[0])], 2),
            func(vec![req(&[1])], 100),
        ],
    }
}

/// `delete` also deletes every dirty node (AbstractInMemoryMemoizingEvaluator.java:216-222):
/// n1, n2 dirty after step 1 (only the leaf was requested) are gone, so both run fresh instead of
/// n1 being pruned and n2 marked clean (the contrast below).
#[test]
fn delete_with_empty_predicate_deletes_dirty_nodes() {
    run_case(
        "delete dirty",
        pruning_chain(),
        vec![
            (vec![], kg(&[2]), sets(&[1, 2], &[], &[])),
            (
                vec![Edit::SetLeaf { node: 0, value: 3 }],
                kg(&[0]),
                sets(&[], &[], &[]),
            ),
            (
                vec![Edit::Delete {
                    nodes: BTreeSet::new(),
                }],
                kg(&[2]),
                sets(&[1, 2], &[], &[]),
            ),
        ],
    );
    run_case(
        "keep dirty",
        pruning_chain(),
        vec![
            (vec![], kg(&[2]), sets(&[1, 2], &[], &[])),
            (
                vec![Edit::SetLeaf { node: 0, value: 3 }],
                kg(&[0]),
                sets(&[], &[], &[]),
            ),
            (vec![], kg(&[2]), sets(&[1], &[1], &[2])),
        ],
    );
}

/// `deleteDirty(age)` (AbstractInMemoryMemoizingEvaluator.java:243-255) deletes dirty nodes not
/// enqueued whose version is at most `lastGraphVersion - age`. After step 1 (v1) n1 and n2 are
/// dirty with version v0: age 1 (threshold v0) deletes them, age 2 (threshold v-1) keeps them.
#[test]
fn delete_dirty_respects_age() {
    run_case(
        "delete dirty age 1",
        pruning_chain(),
        vec![
            (vec![], kg(&[2]), sets(&[1, 2], &[], &[])),
            (
                vec![Edit::SetLeaf { node: 0, value: 3 }],
                kg(&[0]),
                sets(&[], &[], &[]),
            ),
            (
                vec![Edit::DeleteDirty { age: 1 }],
                kg(&[2]),
                sets(&[1, 2], &[], &[]),
            ),
        ],
    );
    run_case(
        "delete dirty age 2",
        pruning_chain(),
        vec![
            (vec![], kg(&[2]), sets(&[1, 2], &[], &[])),
            (
                vec![Edit::SetLeaf { node: 0, value: 3 }],
                kg(&[0]),
                sets(&[], &[], &[]),
            ),
            (
                vec![Edit::DeleteDirty { age: 2 }],
                kg(&[2]),
                sets(&[1], &[1], &[2]),
            ),
        ],
    );
}

/// Without keep_going, a root done with an error stops the evaluation before anything runs
/// (ParallelEvaluator.java:670-690): the result has only that root's error; the other root is
/// absent although it was never built.
#[test]
fn no_keep_going_cached_error_root_runs_nothing() {
    let spec = GraphSpec {
        nodes: vec![
            leaf(0),
            throwing(vec![req(&[0])], false),
            func(vec![req(&[0])], 100),
        ],
    };
    run_case(
        "cached error",
        spec,
        vec![
            (vec![], kg(&[1]), sets(&[1], &[], &[])),
            (
                vec![],
                nkg(&[2, 1]),
                sets(&[], &[], &[]).with(vec![
                    (2, RootResult::Absent),
                    (
                        1,
                        RootResult::Error(ErrClass::Exception {
                            origin: 1,
                            transient: false,
                        }),
                    ),
                ]),
            ),
        ],
    );
}

/// A group of several deps (one `getValuesAndExceptions`) is checked as one group: both deps
/// changed, n2 reruns, gets the same sum and is pruned; n3 is marked clean.
#[test]
fn batch_group_changes_but_sum_is_pruned() {
    let spec = GraphSpec {
        nodes: vec![
            leaf(1),
            leaf(2),
            func(vec![req(&[0, 1])], 100),
            func(vec![req(&[2])], 100),
        ],
    };
    run_case(
        "batch",
        spec,
        vec![
            (vec![], kg(&[3]), sets(&[2, 3], &[], &[])),
            (
                vec![
                    Edit::SetLeaf { node: 0, value: 2 },
                    Edit::SetLeaf { node: 1, value: 1 },
                ],
                kg(&[3]),
                sets(&[2], &[2], &[3]).with(vec![(3, RootResult::Value(3))]),
            ),
        ],
    );
}

/// Invalidating a leaf (CHANGE) runs the leaf's function (it is not injected again); the same
/// value is pruned; a node with a cached error above it is marked clean and keeps its error, and
/// the node that caught that error is marked clean too.
#[test]
fn invalidated_leaf_reruns_and_cached_error_is_kept() {
    let spec = GraphSpec {
        nodes: vec![
            leaf(0),
            leaf(0),
            throwing(vec![req(&[0])], false),
            func(vec![catching(2, 7), req(&[1])], 100),
        ],
    };
    run_case(
        "leaf invalidate",
        spec,
        vec![
            (
                vec![],
                kg(&[3]),
                sets(&[2, 3], &[], &[]).with(vec![(3, RootResult::Value(7))]),
            ),
            (
                vec![Edit::Invalidate { node: 0 }],
                kg(&[3]),
                sets(&[0], &[0], &[2, 3]).with(vec![(3, RootResult::Value(7))]),
            ),
        ],
    );
}

/// Without keep_going a cycle leaves its nodes in flight (the detector reports the cycle and
/// commits nothing, SimpleCycleDetector.java:276-285); they are deleted at the start of the next
/// evaluation (AbstractInMemoryMemoizingEvaluator.java:134) and run again. With keep_going the
/// detector commits the cycle errors, so the evaluation after that runs nothing.
#[test]
fn no_keep_going_cycle_nodes_are_deleted_and_rerun() {
    let spec = GraphSpec {
        nodes: vec![func(vec![req(&[1])], 100), func(vec![req(&[0])], 100)],
    };
    let cycle = vec![(0, RootResult::Error(ErrClass::Cycle))];
    let (model, _) = run_case(
        "nkg cycle",
        spec,
        vec![
            (
                vec![],
                nkg(&[0]),
                sets(&[0, 1], &[], &[]).with(cycle.clone()),
            ),
            (
                vec![],
                nkg(&[0]),
                sets(&[0, 1], &[], &[]).with(cycle.clone()),
            ),
            (
                vec![],
                kg(&[0]),
                sets(&[0, 1], &[], &[]).with(cycle.clone()),
            ),
            (vec![], kg(&[0]), sets(&[], &[], &[]).with(cycle)),
        ],
    );
    assert!(model.in_flight().is_empty());
}
