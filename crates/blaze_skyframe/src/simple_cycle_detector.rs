//! Port of `SimpleCycleDetector`.

use crate::evaluation_result::EvaluationResultBuilder;
use crate::evaluator::{DoneEntry, Run};
use crate::{CycleInfo, ErrorInfo, GroupedDeps, SkyKey};
use blaze_util::check_state;
use std::collections::HashSet;
use std::sync::Arc;

/// The max number of cycles we will report to the user for a given root, to avoid OOMing.
// Java: com/google/devtools/build/skyframe/SimpleCycleDetector.java:45 MAX_CYCLES_TO_STORE
const MAX_CYCLES_TO_STORE: usize = 20;

/// Marker value that we push onto a stack before we push a node's children on. When the marker
/// value is popped, we know that all the children are finished.
// Java: com/google/devtools/build/skyframe/SimpleCycleDetector.java:358 CHILDREN_FINISHED
enum Visit {
    Key(SkyKey),
    ChildrenFinished,
}

/// Depth-first implementation of cycle detection after an evaluation has completed with at least
/// one root unfinished.
///
/// PORT: "not done" nodes are the ones that finished "in cycle" in the blocking engine; their
/// recorded deps stand in for Java's temporary direct deps. Dirty-node handling is skipped
/// (incremental).
// Java: com/google/devtools/build/skyframe/SimpleCycleDetector.java:41 SimpleCycleDetector
pub(crate) struct SimpleCycleDetector {
    store_exact_cycles: bool,
}

impl SimpleCycleDetector {
    pub(crate) fn new(store_exact_cycles: bool) -> Self {
        SimpleCycleDetector { store_exact_cycles }
    }

    // Java: com/google/devtools/build/skyframe/SimpleCycleDetector.java:55 SimpleCycleDetector.checkForCycles
    pub(crate) fn check_for_cycles(
        &self,
        bad_roots: &[SkyKey],
        result: &mut EvaluationResultBuilder,
        evaluator_context: &Run<'_>,
    ) {
        let keep_going = evaluator_context.ctx.keep_going;
        for root in bad_roots {
            let Some(error_info) = self.check_for_cycles_root(root, evaluator_context) else {
                // This node just wasn't finished when evaluation aborted -- there were no cycles
                // below it.
                check_state!(
                    !keep_going,
                    "Missing error info with keep going (root={:?}, badRoots={:?})",
                    root,
                    bad_roots
                );
                continue;
            };
            check_state!(
                !error_info.get_cycle_info().is_empty(),
                "{:?} was not evaluated, but was not part of a cycle",
                root
            );
            result.add_error(root.clone(), error_info);
            if !keep_going {
                return;
            }
        }
    }

    /// The algorithm for this cycle detector is as follows. We visit the graph depth-first,
    /// keeping track of the path we are currently on. We skip any DONE nodes (they are
    /// transitively error-free). If we come to a node already on the path, we immediately
    /// construct a cycle. If we are in the noKeepGoing case, we return ErrorInfo with that cycle
    /// to the caller. Otherwise, we continue. Once all of a node's children are done, we
    /// construct an error value for it, based on those children. Finally, when the original
    /// root's node is constructed, we return its ErrorInfo.
    // Java: com/google/devtools/build/skyframe/SimpleCycleDetector.java:92 SimpleCycleDetector.checkForCycles
    fn check_for_cycles_root(&self, root: &SkyKey, ctx: &Run<'_>) -> Option<ErrorInfo> {
        let keep_going = ctx.ctx.keep_going;
        // The number of cycles found. Do not keep on searching for more cycles after this many
        // were found.
        let mut cycles_found = 0usize;
        // The path through the graph currently being visited.
        let mut graph_path: Vec<SkyKey> = vec![];
        // Set of nodes on the path, to avoid expensive searches through the path for cycles.
        let mut path_set: HashSet<SkyKey> = HashSet::new();

        // Maintain a stack explicitly instead of recursion to avoid stack overflows on extreme
        // graphs (with long dependency chains).
        let mut to_visit: Vec<Visit> = vec![Visit::Key(root.clone())];

        // The procedure for this check is as follows: we visit a node, push it onto the graph
        // path, push a marker value onto the toVisit stack, and then push all of its children
        // onto the toVisit stack. Thus, when the marker node comes to the top of the toVisit
        // stack, we have visited the downward transitive closure of the value. At that point, all
        // of its children must be finished, and so we can build the definitive error info for
        // the node, popping it off the graph path.
        while let Some(visit) = to_visit.pop() {
            let (key, entry) = match visit {
                Visit::ChildrenFinished => {
                    // We have reached the marker node - that means all children of a node have
                    // been visited. Since all nodes have errors, we must have found errors in the
                    // children at this point.
                    let key = graph_path.pop().expect("path");
                    let entry = get_entry(ctx, &key);
                    path_set.remove(&key);
                    // Skip this node if it was first/last node of a cycle, and so has already
                    // been processed.
                    if entry.is_done() {
                        continue;
                    }
                    if !keep_going {
                        // in the --nokeep_going mode, we would have already returned if we'd
                        // found a cycle below this node. We haven't, so there are no cycles below
                        // this node; skip further evaluation
                        continue;
                    }
                    let mut direct_deps = entry.deps.clone();
                    let mut removed_deps: HashSet<SkyKey> = HashSet::new();
                    if cycles_found < MAX_CYCLES_TO_STORE || !self.store_exact_cycles {
                        // Value must be ready, because all of its children have finished, so we
                        // can build its error.
                        check_state!(
                            !has_unsignaled_deps(ctx, &direct_deps),
                            "{:?} has unsignaled deps. ValueEntry: {:?}",
                            key,
                            entry
                        );
                    } else if has_unsignaled_deps(ctx, &direct_deps) {
                        let all: Vec<SkyKey> = direct_deps
                            .get_all_elements_as_iterable()
                            .cloned()
                            .collect();
                        removed_deps = remove_incomplete_children_for_cycle(ctx, &all);
                        direct_deps.remove(&removed_deps);
                    }
                    // PORT: maybeHandleVerifiedCleanNode and maybeMarkRebuilding skipped,
                    // incremental.
                    // Find out which children have errors. Similar logic to that in
                    // Evaluate#run().
                    let error_deps = get_children_errors_for_cycle(
                        ctx,
                        &key,
                        &direct_deps,
                        &entry,
                        &removed_deps,
                    );
                    check_state!(
                        !error_deps.is_empty(),
                        "Node {:?} was not successfully evaluated, but had no child errors. \
                         NodeEntry: {:?}",
                        key,
                        entry
                    );
                    ctx.commit_cycle_error(
                        &key,
                        ErrorInfo::from_child_errors(&key, error_deps.iter().map(|e| &**e)),
                        direct_deps,
                    );
                    continue;
                }
                Visit::Key(key) => {
                    let entry = get_entry(ctx, &key);
                    (key, entry)
                }
            };

            // Nothing to be done for this node if it already has an entry.
            if entry.is_done() {
                continue;
            }
            if cycles_found >= MAX_CYCLES_TO_STORE && self.store_exact_cycles {
                // Do not keep on searching for cycles indefinitely, to avoid excessive
                // runtime/OOMs.
                continue;
            }

            if path_set.contains(&key) {
                let cycle_start = graph_path.iter().position(|k| *k == key).expect("on path");
                // Found a cycle!
                cycles_found += 1;
                let cycle: Vec<SkyKey> = graph_path[cycle_start..].to_vec();
                // PORT: entry.isDirty() handling skipped, incremental.
                if keep_going {
                    // Any children of this node that we haven't already visited are not worth
                    // visiting, since this node is about to be done. Thus, the only child worth
                    // visiting is the one in this cycle, the cycleChild (which may == key if this
                    // cycle is a self-edge).
                    let cycle_child = select_cycle_child(&key, &graph_path, cycle_start);
                    let removed_deps = remove_descendants_of_cycle_value(
                        ctx,
                        &key,
                        &entry,
                        &cycle_child,
                        &mut to_visit,
                        graph_path.len() - cycle_start,
                    );
                    let mut direct_deps = entry.deps.clone();
                    direct_deps.remove(&removed_deps);

                    // Construct full error info for this node. Get errors from children, which
                    // are all done except possibly for the cycleChild.
                    let mut all_errors = self.get_children_errors_for_cycle_checking(
                        ctx,
                        &direct_deps,
                        &cycle_child,
                    );
                    let cycle_info = if self.store_exact_cycles {
                        CycleInfo::create_cycle_info_without_path(cycle)
                    } else {
                        CycleInfo::cycle_info_no_details()
                    };
                    // Add in this cycle.
                    all_errors.push(Arc::new(ErrorInfo::from_cycle(cycle_info)));
                    ctx.commit_cycle_error(
                        &key,
                        ErrorInfo::from_child_errors(&key, all_errors.iter().map(|e| &**e)),
                        direct_deps,
                    );
                    continue;
                } else {
                    // We need to return right away in the noKeepGoing case, so construct the
                    // cycle (with the path) and return.
                    check_state!(
                        graph_path[0] == *root,
                        "{:?} not reached from {:?}. ValueEntry: {:?}",
                        key,
                        root,
                        entry
                    );
                    return Some(ErrorInfo::from_cycle(CycleInfo::create_cycle_info(
                        graph_path[..cycle_start].to_vec(),
                        cycle,
                    )));
                }
            }

            // This node is not yet known to be in a cycle. So process its children.
            let temporary_direct_deps = &entry.deps;
            if temporary_direct_deps.is_empty() {
                continue;
            }
            // This marker flag will tell us when all this node's children have been processed.
            to_visit.push(Visit::ChildrenFinished);
            // This node is now part of the path through the graph.
            graph_path.push(key.clone());
            path_set.insert(key.clone());
            for child_key in temporary_direct_deps.get_all_elements_as_iterable() {
                let child_entry = ctx.ev.done_entry(child_key).unwrap_or_else(|| {
                    panic!(
                        "NullPointerException: Missing already declared dep {:?} (parent={:?})",
                        child_key, key
                    )
                });
                if !child_entry.is_done() {
                    to_visit.push(Visit::Key(child_key.clone()));
                }
            }
        }
        if keep_going {
            let entry = check_done(ctx, root);
            Some((**entry.error.as_ref().expect("root in error")).clone())
        } else {
            None
        }
    }

    /// Get all the errors of child nodes.
    // Java: com/google/devtools/build/skyframe/SimpleCycleDetector.java:419 SimpleCycleDetector.getChildrenErrorsForCycleChecking
    fn get_children_errors_for_cycle_checking(
        &self,
        ctx: &Run<'_>,
        children: &GroupedDeps,
        unfinished_child: &SkyKey,
    ) -> Vec<Arc<ErrorInfo>> {
        let mut all_errors = vec![];
        for child_key in children.get_all_elements_as_iterable() {
            let error_info = get_error_maybe(ctx, child_key, child_key == unfinished_child);
            if let Some(error_info) = error_info {
                // Drop child cycle error if not storing cycles, as these will be redundant with
                // the cycle error of the parent node.
                let drop_error_info =
                    !self.store_exact_cycles && !error_info.get_cycle_info().is_empty();
                if !drop_error_info {
                    all_errors.push(error_info);
                }
            }
        }
        all_errors
    }
}

fn get_entry(ctx: &Run<'_>, key: &SkyKey) -> Arc<DoneEntry> {
    ctx.ev
        .done_entry(key)
        .unwrap_or_else(|| panic!("NullPointerException: {key:?}"))
}

/// Whether some dep is not done (Java: `NodeEntry.hasUnsignaledDeps`).
fn has_unsignaled_deps(ctx: &Run<'_>, deps: &GroupedDeps) -> bool {
    deps.get_all_elements_as_iterable()
        .any(|d| !get_entry(ctx, d).is_done())
}

/// Returns the child of this node that is in the cycle that was just found. If the cycle is a
/// self-edge, returns the node itself.
// Java: com/google/devtools/build/skyframe/SimpleCycleDetector.java:364 SimpleCycleDetector.selectCycleChild
fn select_cycle_child(key: &SkyKey, graph_path: &[SkyKey], cycle_start: usize) -> SkyKey {
    if cycle_start + 1 == graph_path.len() {
        key.clone()
    } else {
        graph_path[cycle_start + 1].clone()
    }
}

/// Get all the errors of child nodes. There must be at least one cycle amongst them.
// Java: com/google/devtools/build/skyframe/SimpleCycleDetector.java:374 SimpleCycleDetector.getChildrenErrorsForCycle
fn get_children_errors_for_cycle(
    ctx: &Run<'_>,
    parent: &SkyKey,
    children: &GroupedDeps,
    entry_for_debugging: &DoneEntry,
    removed_deps_for_debugging: &HashSet<SkyKey>,
) -> Vec<Arc<ErrorInfo>> {
    let mut all_errors = vec![];
    let mut found_cycle = false;
    for child_key in children.get_all_elements_as_iterable() {
        let child_entry = check_done(ctx, child_key);
        if let Some(error_info) = &child_entry.error {
            found_cycle |= !error_info.get_cycle_info().is_empty();
            all_errors.push(error_info.clone());
        }
    }
    check_state!(
        found_cycle,
        "Key {:?} with entry {:?} had no cycle beneath it: {:?}; Removed deps: {:?}",
        parent,
        entry_for_debugging,
        all_errors,
        removed_deps_for_debugging
    );
    all_errors
}

// Java: com/google/devtools/build/skyframe/SimpleCycleDetector.java:443 SimpleCycleDetector.getErrorMaybe
fn get_error_maybe(ctx: &Run<'_>, key: &SkyKey, allow_unfinished: bool) -> Option<Arc<ErrorInfo>> {
    if !allow_unfinished {
        return check_done(ctx, key).error.clone();
    }
    let entry = get_entry(ctx, key);
    if entry.is_done() {
        entry.error.clone()
    } else {
        None
    }
}

/// Removes direct children of key from toVisit and from the entry itself, and makes the entry
/// ready if necessary. We must do this because it would not make sense to try to build the
/// children after building the entry. It would violate the invariant that a parent can only be
/// built after its children are built; See bug "Precondition error while evaluating a Skyframe
/// graph with a cycle".
// Java: com/google/devtools/build/skyframe/SimpleCycleDetector.java:462 SimpleCycleDetector.removeDescendantsOfCycleValue
fn remove_descendants_of_cycle_value(
    ctx: &Run<'_>,
    key: &SkyKey,
    entry: &DoneEntry,
    cycle_child: &SkyKey,
    to_visit: &mut Vec<Visit>,
    mut cycle_length: usize,
) -> HashSet<SkyKey> {
    let mut unvisited_deps: Vec<SkyKey> = vec![];
    for d in entry.deps.get_all_elements_as_iterable() {
        if d != cycle_child {
            unvisited_deps.push(d.clone());
        }
    }
    // Remove any children from this node that are not part of the cycle we just found. They are
    // irrelevant to the node as it stands, and if they are deleted from the graph because they
    // are not built by the end of cycle-checking, we would have dangling references.
    let removed_deps = remove_incomplete_children_for_cycle(ctx, &unvisited_deps);
    // PORT: signalDep / maybeMarkRebuilding have no equivalent (no restarts).
    let unvisited: HashSet<&SkyKey> = unvisited_deps.iter().collect();
    // Iterate from the top of the stack.
    let mut i = to_visit.len();
    while i > 0 {
        i -= 1;
        match &to_visit[i] {
            Visit::ChildrenFinished => {
                // Marker value, delineating the end of a group of children that were enqueued.
                cycle_length -= 1;
                if cycle_length == 0 {
                    // We have seen #cycleLength-1 marker values, and have arrived at the one for
                    // this value, so we are done.
                    return removed_deps;
                }
                continue; // Don't remove marker values.
            }
            Visit::Key(descendant) => {
                if cycle_length == 1 {
                    // Remove the direct children remaining to visit of the cycle node.
                    check_state!(
                        unvisited.contains(descendant),
                        "{:?} {:?} {:?} {:?}",
                        key,
                        descendant,
                        cycle_child,
                        entry
                    );
                    to_visit.remove(i);
                }
            }
        }
    }
    panic!(
        "IllegalStateException: There were not {} groups of children in <toVisit> when trying to \
         remove children of {:?} other than {:?}",
        cycle_length, key, cycle_child
    );
}

/// Returns the children that are not done (Java also removes the parent from their reverse deps;
/// reverse deps here are only recorded on commit).
// Java: com/google/devtools/build/skyframe/SimpleCycleDetector.java:516 SimpleCycleDetector.removeIncompleteChildrenForCycle
fn remove_incomplete_children_for_cycle(ctx: &Run<'_>, children: &[SkyKey]) -> HashSet<SkyKey> {
    children
        .iter()
        .filter(|child| !get_entry(ctx, child).is_done())
        .cloned()
        .collect()
}

// Java: com/google/devtools/build/skyframe/SimpleCycleDetector.java:531 SimpleCycleDetector.checkDone
fn check_done(ctx: &Run<'_>, key: &SkyKey) -> Arc<DoneEntry> {
    let entry = get_entry(ctx, key);
    check_state!(entry.is_done(), "{:?} {:?}", key, entry);
    entry
}
