//! Port of `SimpleCycleDetector`.

use crate::environment::{DepValue, report_events_and_get_events_to_store};
use crate::evaluation_result::EvaluationResultBuilder;
use crate::evaluator::{DoneEntry, Node, Run, maybe_mark_rebuilding};
use crate::node_entry::{LifecycleState, NodeEntry};
use crate::{CycleInfo, ErrorInfo, SkyKey};
use blaze_util::check_state;
use indexmap::IndexSet;
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
/// PORT: "not done" nodes are the ones still in flight in the blocking engine (their function
/// returned null on a dep that was in a cycle, or the evaluation aborted).
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
            let (key, node) = match visit {
                Visit::ChildrenFinished => {
                    // We have reached the marker node - that means all children of a node have
                    // been visited. Since all nodes have errors, we must have found errors in the
                    // children at this point.
                    let key = graph_path.pop().expect("path");
                    let node = get_node(ctx, &key);
                    path_set.remove(&key);
                    // Skip this node if it was first/last node of a cycle, and so has already
                    // been processed.
                    if node.lock().entry.is_done() {
                        continue;
                    }
                    if !keep_going {
                        // in the --nokeep_going mode, we would have already returned if we'd
                        // found a cycle below this node. We haven't, so there are no cycles below
                        // this node; skip further evaluation
                        continue;
                    }
                    let mut removed_deps: HashSet<SkyKey> = HashSet::new();
                    if cycles_found < MAX_CYCLES_TO_STORE || !self.store_exact_cycles {
                        // Value must be ready, because all of its children have finished, so we
                        // can build its error.
                        let state = node.lock();
                        check_state!(
                            !state.entry.has_unsignaled_deps(),
                            "{:?} has unsignaled deps. ValueEntry: {:?}",
                            key,
                            state.entry
                        );
                    } else if node.lock().entry.has_unsignaled_deps() {
                        let children: Vec<SkyKey> = node
                            .lock()
                            .entry
                            .get_temporary_direct_deps()
                            .get_all_elements_as_iterable()
                            .cloned()
                            .collect();
                        removed_deps =
                            remove_incomplete_children_for_cycle(&key, &node, &children, ctx);
                    }
                    if maybe_handle_verified_clean_node(&key, &node, ctx, &graph_path) {
                        continue;
                    }
                    let (direct_deps, old_deps) = {
                        let mut state = node.lock();
                        maybe_mark_rebuilding(&mut state.entry);
                        let old_deps: IndexSet<SkyKey> = state
                            .entry
                            .get_all_remaining_dirty_direct_deps()
                            .into_iter()
                            .filter(|k| !removed_deps.contains(k))
                            .collect();
                        let direct_deps: Vec<SkyKey> = state
                            .entry
                            .get_temporary_direct_deps()
                            .get_all_elements_as_iterable()
                            .cloned()
                            .collect();
                        (direct_deps, old_deps)
                    };
                    // Find out which children have errors. Similar logic to that in
                    // Evaluate#run().
                    let error_deps = get_children_errors_for_cycle(
                        &key,
                        &direct_deps,
                        &node,
                        ctx,
                        &removed_deps,
                    );
                    check_state!(
                        !error_deps.is_empty(),
                        "Node {:?} was not successfully evaluated, but had no child errors. \
                         NodeEntry: {:?}",
                        key,
                        node.lock().entry
                    );
                    // Java: SkyFunctionEnvironment.create(key, directDeps, oldDeps, ...); the
                    // previously requested deps are all done (UndonePreviouslyRequestedDeps
                    // "Previously requested deps not done" cannot happen here).
                    commit_cycle_node(
                        ctx,
                        &key,
                        &node,
                        ErrorInfo::from_child_errors(&key, error_deps.iter().map(|e| &**e)),
                        &old_deps,
                        None,
                    );
                    continue;
                }
                Visit::Key(key) => {
                    let node = get_node(ctx, &key);
                    (key, node)
                }
            };

            // Nothing to be done for this node if it already has an entry.
            if node.lock().entry.is_done() {
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
                // Put this node into a consistent state for building if it is dirty.
                if node.lock().entry.is_dirty() {
                    // If this loop runs more than once, we are in the peculiar position of entry
                    // not needing rebuilding even though it was signaled with the graph version.
                    // This can happen when the entry was previously evaluated at this version,
                    // but then invalidated anyway, even though nothing changed.
                    let graph_version = ctx.graph_version;
                    let lifecycle_state = {
                        let mut state = node.lock();
                        while state.entry.get_lifecycle_state() == LifecycleState::CheckDependencies
                        {
                            state.entry.signal_dep(graph_version, None);
                        }
                        // PORT: Java reports a BugReport (not a crash) when this loop ran more
                        // than once without the graph version; bug reports are not ported.
                        state.entry.get_lifecycle_state()
                    };
                    if lifecycle_state == LifecycleState::NeedsRebuilding {
                        node.lock().entry.mark_rebuilding();
                    } else if maybe_handle_verified_clean_node(&key, &node, ctx, &graph_path) {
                        continue;
                    }
                }
                if keep_going {
                    // Any children of this node that we haven't already visited are not worth
                    // visiting, since this node is about to be done. Thus, the only child worth
                    // visiting is the one in this cycle, the cycleChild (which may == key if this
                    // cycle is a self-edge).
                    let cycle_child = select_cycle_child(&key, &graph_path, cycle_start);
                    let removed_deps = remove_descendants_of_cycle_value(
                        &key,
                        &node,
                        &cycle_child,
                        &mut to_visit,
                        graph_path.len() - cycle_start,
                        ctx,
                    );
                    let (direct_deps, old_deps) = {
                        let mut state = node.lock();
                        let old_deps: IndexSet<SkyKey> = state
                            .entry
                            .get_all_remaining_dirty_direct_deps()
                            .into_iter()
                            .filter(|k| !removed_deps.contains(k))
                            .collect();
                        let direct_deps: Vec<SkyKey> = state
                            .entry
                            .get_temporary_direct_deps()
                            .get_all_elements_as_iterable()
                            .cloned()
                            .collect();
                        (direct_deps, old_deps)
                    };

                    // Construct full error info for this node. Get errors from children, which
                    // are all done except possibly for the cycleChild.
                    let mut all_errors = self.get_children_errors_for_cycle_checking(
                        &direct_deps,
                        &cycle_child,
                        ctx,
                    );
                    let cycle_info = if self.store_exact_cycles {
                        CycleInfo::create_cycle_info_without_path(cycle)
                    } else {
                        CycleInfo::cycle_info_no_details()
                    };
                    // Add in this cycle.
                    all_errors.push(Arc::new(ErrorInfo::from_cycle(cycle_info)));
                    // Java: SkyFunctionEnvironment.createForError(key, temporary direct deps,
                    // {cycleChild: dummyValue}, oldDeps, ...).
                    commit_cycle_node(
                        ctx,
                        &key,
                        &node,
                        ErrorInfo::from_child_errors(&key, all_errors.iter().map(|e| &**e)),
                        &old_deps,
                        Some(&cycle_child),
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
                        node.lock().entry
                    );
                    return Some(ErrorInfo::from_cycle(CycleInfo::create_cycle_info(
                        graph_path[..cycle_start].to_vec(),
                        cycle,
                    )));
                }
            }

            // This node is not yet known to be in a cycle. So process its children.
            let children: Vec<SkyKey> = node
                .lock()
                .entry
                .get_temporary_direct_deps()
                .get_all_elements_as_iterable()
                .cloned()
                .collect();
            if children.is_empty() {
                continue;
            }
            // This marker flag will tell us when all this node's children have been processed.
            to_visit.push(Visit::ChildrenFinished);
            // This node is now part of the path through the graph.
            graph_path.push(key.clone());
            path_set.insert(key.clone());
            for child_key in children {
                let child_node = ctx.ev.node(&child_key).unwrap_or_else(|| {
                    panic!(
                        "NullPointerException: Missing already declared dep {:?} (parent={:?})",
                        child_key, key
                    )
                });
                if !child_node.lock().entry.is_done() {
                    to_visit.push(Visit::Key(child_key));
                }
            }
        }
        if keep_going {
            let node = check_done(ctx, root);
            let state = node.lock();
            Some(state.entry.get_error_info().expect("root in error").clone())
        } else {
            None
        }
    }

    /// Get all the errors of child nodes.
    // Java: com/google/devtools/build/skyframe/SimpleCycleDetector.java:419 SimpleCycleDetector.getChildrenErrorsForCycleChecking
    fn get_children_errors_for_cycle_checking(
        &self,
        children: &[SkyKey],
        unfinished_child: &SkyKey,
        ctx: &Run<'_>,
    ) -> Vec<Arc<ErrorInfo>> {
        let mut all_errors = vec![];
        for child_key in children {
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

/// Java: `env.setError(entry, error)`, `env.commitAndGetParents(entry, /*expectDoneDeps=*/
/// true)` and `signalParentsOnAbort` for a node the cycle detector finishes. `cycle_child` is the
/// key `createForError` maps to a dummy value (none for `SkyFunctionEnvironment.create`).
fn commit_cycle_node(
    ctx: &Run<'_>,
    key: &SkyKey,
    node: &Arc<Node>,
    error_info: ErrorInfo,
    old_deps: &IndexSet<SkyKey>,
    cycle_child: Option<&SkyKey>,
) {
    ctx.set_error(key, node, &error_info, old_deps);
    // The function never ran: no event to report, but the deps' events are collected.
    let events = report_events_and_get_events_to_store(
        ctx,
        key,
        || None,
        vec![],
        node,
        /* expect_done_deps= */ true,
        &|dep| {
            if Some(dep) == cycle_child {
                // Java: ValueWithMetadata.wrapWithMetadata(new SkyValue() {}): no events.
                return DepValue::Value(Arc::new(DoneEntry::undone(
                    crate::evaluator::DoneKind::Bubbled,
                )));
            }
            match ctx.ev.committed_entry(dep) {
                Some(v) => DepValue::Value(v),
                None => DepValue::NullMarker,
            }
        },
    );
    let (reverse_deps, snapshot) =
        ctx.commit_and_get_parents(key, node, None, Some(error_info), &[], events, old_deps);
    ctx.signal_parents_on_abort(key, &reverse_deps, snapshot.version);
    node.publish_outcome(snapshot);
}

/// Fully process `entry` if it is dirty but verified to be clean. This can only happen in rare
/// circumstances where a node with a cycle is invalidated at the same version. Returns true if
/// the entry was successfully processed, meaning that its value has been set and all reverse
/// deps signaled.
// Java: com/google/devtools/build/skyframe/SimpleCycleDetector.java:330 SimpleCycleDetector.maybeHandleVerifiedCleanNode
fn maybe_handle_verified_clean_node(
    key: &SkyKey,
    node: &Arc<Node>,
    ctx: &Run<'_>,
    _graph_path_for_debugging: &[SkyKey],
) -> bool {
    let (rdeps, version) = {
        let mut state = node.lock();
        if state.entry.get_lifecycle_state() != LifecycleState::VerifiedClean {
            return false;
        }
        let rdeps = state.entry.mark_clean().get_rdeps_to_signal().clone();
        state.refresh_snapshot();
        (rdeps, state.entry.get_version())
    };
    ctx.signal_parents_on_abort(key, &rdeps, version);
    // PORT: Java reports a BugReport (not a crash) when the error has no cycle ("Entry was
    // unchanged from last build, but cycle was found this time and not last time"); bug reports
    // are not ported.
    let snapshot = node.snapshot().expect("marked clean");
    node.publish_outcome(snapshot);
    true
}

fn get_node(ctx: &Run<'_>, key: &SkyKey) -> Arc<Node> {
    ctx.ev
        .node(key)
        .unwrap_or_else(|| panic!("NullPointerException: {key:?}"))
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
    parent: &SkyKey,
    children: &[SkyKey],
    entry_for_debugging: &Node,
    ctx: &Run<'_>,
    removed_deps_for_debugging: &HashSet<SkyKey>,
) -> Vec<Arc<ErrorInfo>> {
    let mut all_errors = vec![];
    let mut found_cycle = false;
    for child_key in children {
        let child_node = ctx.ev.node(child_key).unwrap_or_else(|| {
            panic!(
                "NullPointerException: Missing already declared dep {child_key:?} \
                 (parent={parent:?})"
            )
        });
        let error_info = {
            let state = child_node.lock();
            check_state!(state.entry.is_done(), "{:?} {:?}", child_key, state.entry);
            state.snapshot.as_ref().and_then(|s| s.error.clone())
        };
        if let Some(error_info) = error_info {
            found_cycle |= !error_info.get_cycle_info().is_empty();
            all_errors.push(error_info);
        }
    }
    check_state!(
        found_cycle,
        "Key {:?} with entry {:?} had no cycle beneath it: {:?}; Removed deps: {:?}",
        parent,
        entry_for_debugging.lock().entry,
        all_errors,
        removed_deps_for_debugging
    );
    all_errors
}

// Java: com/google/devtools/build/skyframe/SimpleCycleDetector.java:443 SimpleCycleDetector.getErrorMaybe
fn get_error_maybe(ctx: &Run<'_>, key: &SkyKey, allow_unfinished: bool) -> Option<Arc<ErrorInfo>> {
    let node = get_node(ctx, key);
    let state = node.lock();
    if !allow_unfinished {
        check_state!(state.entry.is_done(), "{:?} {:?}", key, state.entry);
    }
    state.snapshot.as_ref().and_then(|s| s.error.clone())
}

/// Removes direct children of key from toVisit and from the entry itself, and makes the entry
/// ready if necessary. We must do this because it would not make sense to try to build the
/// children after building the entry. It would violate the invariant that a parent can only be
/// built after its children are built; See bug "Precondition error while evaluating a Skyframe
/// graph with a cycle".
// Java: com/google/devtools/build/skyframe/SimpleCycleDetector.java:462 SimpleCycleDetector.removeDescendantsOfCycleValue
fn remove_descendants_of_cycle_value(
    key: &SkyKey,
    node: &Arc<Node>,
    cycle_child: &SkyKey,
    to_visit: &mut Vec<Visit>,
    mut cycle_length: usize,
    ctx: &Run<'_>,
) -> HashSet<SkyKey> {
    let unvisited_deps: Vec<SkyKey> = {
        let mut state = node.lock();
        let mut seen = HashSet::new();
        state
            .entry
            .get_temporary_direct_deps()
            .get_all_elements_as_iterable()
            .filter(|d| *d != cycle_child && seen.insert((*d).clone()))
            .cloned()
            .collect()
    };
    // Remove any children from this node that are not part of the cycle we just found. They are
    // irrelevant to the node as it stands, and if they are deleted from the graph because they
    // are not built by the end of cycle-checking, we would have dangling references.
    let removed_deps = remove_incomplete_children_for_cycle(key, node, &unvisited_deps, ctx);
    {
        let mut state = node.lock();
        if state.entry.has_unsignaled_deps() {
            // The entry has at most one undone dep now, its cycleChild. Signal to make entry
            // ready. Note that the entry can conceivably be ready if its cycleChild already found
            // a different cycle and was built.
            state.entry.signal_dep(ctx.graph_version, Some(cycle_child));
        }
        maybe_mark_rebuilding(&mut state.entry);
        check_state!(
            !state.entry.has_unsignaled_deps(),
            "{:?} {:?} {:?}",
            key,
            cycle_child,
            state.entry
        );
    }
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
                        node.lock().entry
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

// Java: com/google/devtools/build/skyframe/SimpleCycleDetector.java:516 SimpleCycleDetector.removeIncompleteChildrenForCycle
fn remove_incomplete_children_for_cycle(
    key: &SkyKey,
    node: &Arc<Node>,
    children: &[SkyKey],
    ctx: &Run<'_>,
) -> HashSet<SkyKey> {
    let mut unfinished_deps: HashSet<SkyKey> = HashSet::new();
    for child in children {
        if remove_incomplete_child_for_cycle(key, child, ctx) {
            unfinished_deps.insert(child.clone());
        }
    }
    node.lock().entry.remove_unfinished_deps(&unfinished_deps);
    unfinished_deps
}

// Java: com/google/devtools/build/skyframe/SimpleCycleDetector.java:531 SimpleCycleDetector.checkDone
fn check_done(ctx: &Run<'_>, key: &SkyKey) -> Arc<Node> {
    let node = get_node(ctx, key);
    {
        let state = node.lock();
        check_state!(state.entry.is_done(), "{:?} {:?}", key, state.entry);
    }
    node
}

/// If child is not done, removes `in_progress_parent` from `child`'s reverse deps. Returns
/// whether child should be removed from inProgressParent's entry's direct deps.
// Java: com/google/devtools/build/skyframe/SimpleCycleDetector.java:543 SimpleCycleDetector.removeIncompleteChildForCycle
fn remove_incomplete_child_for_cycle(
    in_progress_parent: &SkyKey,
    child: &SkyKey,
    ctx: &Run<'_>,
) -> bool {
    let child_node = get_node(ctx, child);
    let mut state = child_node.lock();
    if !state.entry.is_done() {
        state.entry.remove_reverse_dep(in_progress_parent.clone());
        return true;
    }
    false
}
