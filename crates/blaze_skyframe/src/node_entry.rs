//! Port of `NodeEntry` and its nested types.

use crate::grouped_deps::GroupedDeps;
use crate::value_with_metadata::ValueWithMetadata;
use crate::version::Version;
use crate::{ErrorInfo, SkyKey, SkyValue};
use indexmap::IndexSet;
use std::collections::HashSet;
use std::fmt;

/// Return code for `addReverseDepAndCheckIfDone` and `checkIfDoneForDirtyReverseDep`.
// Java: com/google/devtools/build/skyframe/NodeEntry.java:41 NodeEntry.DependencyState
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum DependencyState {
    /// The node is done.
    Done,
    /// The node has not started evaluating, and needs to be scheduled for its first evaluation
    /// pass. The caller getting this return value is responsible for scheduling its evaluation
    /// and signaling the reverse dependency node when this node is done.
    NeedsScheduling,
    /// The node was already created, but isn't done yet. The evaluator is responsible for
    /// signaling the reverse dependency node.
    AlreadyEvaluating,
}

impl fmt::Debug for DependencyState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            DependencyState::Done => "DONE",
            DependencyState::NeedsScheduling => "NEEDS_SCHEDULING",
            DependencyState::AlreadyEvaluating => "ALREADY_EVALUATING",
        })
    }
}

/// Represents the various states in a node's lifecycle.
// Java: com/google/devtools/build/skyframe/NodeEntry.java:60 NodeEntry.LifecycleState
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum LifecycleState {
    /// The entry has never started evaluating. The next call to `addReverseDepAndCheckIfDone`
    /// will put the entry into the `NEEDS_REBUILDING` state and return
    /// `DependencyState.NEEDS_SCHEDULING`.
    NotYetEvaluating,
    /// The node's dependencies need to be checked to see if it needs to be rebuilt. The
    /// dependencies must be obtained through calls to `getNextDirtyDirectDeps` and checked.
    CheckDependencies,
    /// All of the node's dependencies are unchanged, and the value itself was not marked
    /// changed, so its current value is still valid -- it need not be rebuilt.
    VerifiedClean,
    /// A rebuilding is required for one of the following reasons:
    ///
    /// 1. One of the node's dependencies changed.
    /// 2. The node is built by a `FunctionHermeticity#NONHERMETIC` function and its value is
    ///    known to have changed due to state outside of Skyframe.
    /// 3. The node was rewound (`DirtyType#REWIND`).
    NeedsRebuilding,
    /// A rebuilding is in progress.
    Rebuilding,
    /// The node `isDone`.
    Done,
}

impl fmt::Debug for LifecycleState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LifecycleState::NotYetEvaluating => "NOT_YET_EVALUATING",
            LifecycleState::CheckDependencies => "CHECK_DEPENDENCIES",
            LifecycleState::VerifiedClean => "VERIFIED_CLEAN",
            LifecycleState::NeedsRebuilding => "NEEDS_REBUILDING",
            LifecycleState::Rebuilding => "REBUILDING",
            LifecycleState::Done => "DONE",
        })
    }
}

/// Ways that a node may be dirtied.
// Java: com/google/devtools/build/skyframe/NodeEntry.java:95 NodeEntry.DirtyType
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum DirtyType {
    /// Indicates that the node is being marked dirty because it has a dependency that was marked
    /// dirty.
    ///
    /// A node P dirtied with `DIRTY` is re-evaluated during the evaluation phase if it is
    /// requested and directly depends on some node C whose value changed since the last
    /// evaluation of P. If it is requested and there is no such node C, P is marked clean
    /// (`markClean`).
    Dirty,
    /// Indicates that the node is being marked dirty because its value from a previous
    /// evaluation is no longer valid, even if none of its dependencies change.
    ///
    /// This is typically used to indicate that a value produced by a
    /// `FunctionHermeticity#NONHERMETIC` function is no longer valid because some state outside
    /// of Skyframe has changed (e.g. a change to the filesystem).
    ///
    /// A node dirtied with `CHANGE` is re-evaluated during the evaluation phase if it is
    /// requested, regardless of the state of its dependencies. If it re-evaluates to the same
    /// value, dirty parents are not necessarily re-evaluated.
    Change,
    /// Similar to `CHANGE` except may be used intra-evaluation to indicate that the node's value
    /// (which may be from either a previous evaluation or the current evaluation) is no longer
    /// valid.
    ///
    /// A node dirtied with `REWIND` is re-evaluated during the evaluation phase if it is
    /// requested, regardless of the state of its dependencies. Even if it re-evaluates to the
    /// same value, dirty parents are re-evaluated.
    ///
    /// Rewinding is tolerated but no-op if the node is already dirty or is done with an error
    /// (regardless of the error's `Transience`).
    Rewind,
}

impl fmt::Debug for DirtyType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            DirtyType::Dirty => "DIRTY",
            DirtyType::Change => "CHANGE",
            DirtyType::Rewind => "REWIND",
        })
    }
}

/// Returned by `markDirty` if that call changed the node from done to dirty.
///
/// For nodes marked dirty during invalidation (`DirtyType#DIRTY` and `DirtyType#CHANGE`),
/// contains a collection of the node's reverse deps for efficiency, so that the invalidator can
/// schedule the invalidation of a node's reverse deps immediately afterwards.
///
/// For nodes marked dirty intra-evaluation (`DirtyType#REWIND`), reverse deps are not needed by
/// the caller, so `getReverseDepsUnsafe` must not be called.
///
/// PORT: Java may hand out a live view of the reverse deps; here it is a copy (same content and
/// order), so the warning about iterating it while it may change does not apply.
// Java: com/google/devtools/build/skyframe/NodeEntry.java:187 NodeEntry.MarkedDirtyResult
#[derive(Clone, Debug)]
pub(crate) enum MarkedDirtyResult {
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:209 MarkedDirtyResult.ResultWithReverseDeps
    ResultWithReverseDeps(Vec<SkyKey>),
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:189 MarkedDirtyResult.RESULT_FOR_REWINDING
    ResultForRewinding,
}

impl MarkedDirtyResult {
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:197 MarkedDirtyResult.withReverseDeps
    pub(crate) fn with_reverse_deps(reverse_deps_unsafe: Vec<SkyKey>) -> MarkedDirtyResult {
        MarkedDirtyResult::ResultWithReverseDeps(reverse_deps_unsafe)
    }

    // Java: com/google/devtools/build/skyframe/NodeEntry.java:201 MarkedDirtyResult.forRewinding
    pub(crate) fn for_rewinding() -> MarkedDirtyResult {
        MarkedDirtyResult::ResultForRewinding
    }

    // Java: com/google/devtools/build/skyframe/NodeEntry.java:207 MarkedDirtyResult.getReverseDepsUnsafe
    pub(crate) fn get_reverse_deps_unsafe(&self) -> &[SkyKey] {
        match self {
            MarkedDirtyResult::ResultWithReverseDeps(reverse_deps_unsafe) => reverse_deps_unsafe,
            MarkedDirtyResult::ResultForRewinding => {
                panic!("IllegalStateException: Should not need reverse deps for rewinding")
            }
        }
    }
}

/// Returned by `markClean` after making a node as clean. This is an aggregate object that
/// contains the NodeEntry's SkyValue and its reverse dependencies that signal this node is done
/// (a subset of all of the node's reverse dependencies).
// Java: com/google/devtools/build/skyframe/NodeEntry.java:431 NodeEntry.NodeValueAndRdepsToSignal
#[derive(Clone, Debug)]
pub(crate) struct NodeValueAndRdepsToSignal {
    value: ValueWithMetadata,
    r_deps_to_signal: IndexSet<SkyKey>,
}

impl NodeValueAndRdepsToSignal {
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:435 NodeValueAndRdepsToSignal.<init>
    pub(crate) fn new(
        value: ValueWithMetadata,
        r_deps_to_signal: IndexSet<SkyKey>,
    ) -> NodeValueAndRdepsToSignal {
        NodeValueAndRdepsToSignal {
            value,
            r_deps_to_signal,
        }
    }

    // Java: com/google/devtools/build/skyframe/NodeEntry.java:440 NodeValueAndRdepsToSignal.getValue
    pub(crate) fn get_value(&self) -> &ValueWithMetadata {
        &self.value
    }

    // Java: com/google/devtools/build/skyframe/NodeEntry.java:444 NodeValueAndRdepsToSignal.getRdepsToSignal
    pub(crate) fn get_rdeps_to_signal(&self) -> &IndexSet<SkyKey> {
        &self.r_deps_to_signal
    }
}

/// A node in the graph.
///
/// PORT: Java's "all operations on this class are thread-safe" (`synchronized` methods) is the
/// caller's lock here: the graph keeps each entry in a `Mutex`, and every method takes `&self` or
/// `&mut self`. Java's `InterruptedException` on accessors is for other graph implementations;
/// the in-memory entry never throws it, so no method returns `Result<_, Interrupted>`.
///
/// Collections: Java `ImmutableSet` and the insertion-ordered `CompactHashSet` are `IndexSet`;
/// `Collection`/`Iterable` returns are `Vec` in Java's iteration order.
// Java: com/google/devtools/build/skyframe/NodeEntry.java:35 NodeEntry
pub(crate) trait NodeEntry: fmt::Debug + Send {
    /// Returns whether the entry has been built and is finished evaluating.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:140 NodeEntry.isDone
    fn is_done(&self) -> bool;

    /// Inverse of `isDone`.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:144 NodeEntry.isDirty
    fn is_dirty(&self) -> bool;

    /// Returns true if the entry is marked changed, meaning that it must be re-evaluated even if
    /// its dependencies' values have not changed.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:151 NodeEntry.isChanged
    fn is_changed(&self) -> bool;

    /// Marks this node dirty as specified by the provided `DirtyType`.
    ///
    /// `markDirty(DirtyType.DIRTY)` may only be called on a node P for which
    /// `P.isDone() || P.isChanged()` (the latter is permitted but has no effect). Similarly,
    /// `markDirty(DirtyType.CHANGE)` may only be called on a node P for which
    /// `P.isDone() || !P.isChanged()`. Otherwise, this will throw `IllegalStateException`.
    ///
    /// `markDirty(DirtyType.REWIND)` may be called at any time (even multiple times
    /// concurrently), although it only has an effect if the node `isDone` with no error.
    ///
    /// Returns, if the node transitioned from done to dirty as a result of this call, a
    /// `MarkedDirtyResult` which may include the node's reverse deps; otherwise `None`.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:169 NodeEntry.markDirty
    fn mark_dirty(&mut self, dirty_type: DirtyType) -> Option<MarkedDirtyResult>;

    /// Returns the value stored in this entry, or `None` if it has only an error.
    ///
    /// This method may only be called when the node `isDone`.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:230 NodeEntry.getValue
    fn get_value(&self) -> Option<SkyValue>;

    /// Returns the direct deps of this node. This method may only be called after the evaluation
    /// of this node is complete.
    ///
    /// This method is not very efficient, but is only be called in limited circumstances --
    /// when the node is about to be deleted, or when the node is expected to have no direct deps
    /// (in which case the overhead is not so bad). It should not be called repeatedly for the
    /// same node, since each call takes time proportional to the number of direct deps of the
    /// node.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:242 NodeEntry.getDirectDeps
    fn get_direct_deps(&self) -> Vec<SkyKey>;

    /// Returns `true` if this node has at least one direct dep.
    ///
    /// Prefer calling this over `getDirectDeps` if possible.
    ///
    /// This method may only be called after the evaluation of this node is complete.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:252 NodeEntry.hasAtLeastOneDep
    fn has_at_least_one_dep(&self) -> bool;

    /// Removes a reverse dependency, which must be present.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:256 NodeEntry.removeReverseDep
    fn remove_reverse_dep(&mut self, reverse_dep: SkyKey);

    /// Removes any reverse dependencies that are in `deleted_keys`. Must only be called from an
    /// invalidation that is deleting nodes from the graph. Sacrifices correctness checks (that
    /// the deleted rdeps were actually rdeps of this entry) for better performance.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:264 NodeEntry.removeReverseDepsFromDoneEntryDueToDeletion
    fn remove_reverse_deps_from_done_entry_due_to_deletion(
        &mut self,
        deleted_keys: &HashSet<SkyKey>,
    );

    /// Returns a copy of the set of reverse dependencies. Note that this introduces a potential
    /// check-then-act race; `removeReverseDep` may fail for a key that is returned here.
    ///
    /// May only be called on a done node entry.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:273 NodeEntry.getReverseDepsForDoneEntry
    fn get_reverse_deps_for_done_entry(&mut self) -> Vec<SkyKey>;

    /// Returns raw `SkyValue` stored in this entry, which may include metadata associated with
    /// it (like events and errors).
    ///
    /// This method returns `None` if the evaluation of this node is not complete, i.e., after
    /// node creation or dirtying and before `setValue` has been called. Callers should assert
    /// that the returned value is not `None` whenever they expect the node should be done.
    ///
    /// Use the static methods of `ValueWithMetadata` to extract metadata if necessary.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:287 NodeEntry.getValueMaybeWithMetadata
    fn get_value_maybe_with_metadata(&self) -> Option<&ValueWithMetadata>;

    /// Returns the last known value of this node, even if it was marked dirty.
    ///
    /// If this node `isDone`, this is equivalent to `getValue`. Unlike `getValue`, however,
    /// this method may be called at any point in the node's lifecycle. Returns `None` if this
    /// node was never built or has no value because it is in error.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:298 NodeEntry.toValue
    fn to_value(&self) -> Option<SkyValue>;

    /// Returns the error, if any, associated to this node. This method may only be called after
    /// the evaluation of this node is complete, i.e., after `setValue` has been called.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:306 NodeEntry.getErrorInfo
    fn get_error_info(&self) -> Option<&ErrorInfo>;

    /// Returns the set of reverse deps that have been declared so far this build. Only for use
    /// in debugging and when bubbling errors up in the --nokeep_going case, where we need to
    /// know what parents this entry has.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:314 NodeEntry.getInProgressReverseDeps
    fn get_in_progress_reverse_deps(&self) -> IndexSet<SkyKey>;

    /// Transitions the node from the EVALUATING to the DONE state and simultaneously sets it to
    /// the given value and error state. It then returns the set of reverse dependencies that
    /// need to be signaled.
    ///
    /// This is an atomic operation to avoid a race where two threads work on two nodes, where
    /// one node depends on another (b depends on a). When a finishes, it signals *exactly* the
    /// set of reverse dependencies that are registered at the time of the `setValue` call. If b
    /// comes in before a, it is signaled (and re-scheduled) by a, otherwise it needs to do that
    /// itself.
    ///
    /// Nodes may elect to use either `graphVersion` or `maxTransitiveSourceVersion` (if not
    /// `None`) for their version. The choice can be distinguished by calling
    /// `getMaxTransitiveSourceVersion` - a return of `None` indicates that the node uses the
    /// graph version.
    ///
    /// If the entry determines that the new value is equal to the previous value, the entry may
    /// keep its current version. Callers can query that version to see if the node considers its
    /// value to have changed.
    ///
    /// PORT: `maxTransitiveSourceVersion` is accepted and ignored, as by both in-memory entries
    /// at the pin (max transitive source versions are not ported).
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:341 NodeEntry.setValue
    fn set_value(
        &mut self,
        value: ValueWithMetadata,
        graph_version: Version,
        max_transitive_source_version: Option<Version>,
    ) -> IndexSet<SkyKey>;

    /// Sets the max transitive source version of this node so far while it is being evaluated.
    /// May only be called when `isDirty()` is `true`.
    ///
    /// This function is a no-op if source versions are not being tracked.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:354 NodeEntry.setTemporaryMaxTransitiveSourceVersion
    #[allow(dead_code)] // ported API; not called by the blocking engine (restarts, rewinding, external deps)
    fn set_temporary_max_transitive_source_version(
        &mut self,
        _max_transitive_source_version: Option<Version>,
    ) {
    }

    /// Queries if the node is done and adds the given key as a reverse dependency. The return
    /// code indicates whether a) the node is done, b) the reverse dependency is the first one,
    /// so the node needs to be scheduled, or c) the reverse dependency was added, and the node
    /// does not need to be scheduled.
    ///
    /// This method *must* be called before any processing of the entry. This encourages callers
    /// to check that the entry is ready to be processed.
    ///
    /// Adding the dependency and checking if the node needs to be scheduled is an atomic
    /// operation to avoid a race where two threads work on two nodes, where one depends on the
    /// other (b depends on a). In that case, we need to ensure that b is re-scheduled exactly
    /// once when a is done. However, a may complete first, in which case b has to re-schedule
    /// itself. Also see `setValue`.
    ///
    /// If the parameter is `None`, then no reverse dependency is added, but we still check if
    /// the node needs to be scheduled.
    ///
    /// If `reverse_dep` is a rebuilding dirty entry that was already a reverse dep of this
    /// entry, then `checkIfDoneForDirtyReverseDep` must be called instead.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:379 NodeEntry.addReverseDepAndCheckIfDone
    fn add_reverse_dep_and_check_if_done(&mut self, reverse_dep: Option<SkyKey>)
    -> DependencyState;

    /// Similar to `addReverseDepAndCheckIfDone`, except that `reverse_dep` must already be a
    /// reverse dep of this entry. Should be used when reverseDep has been marked dirty and is
    /// checking its dependencies for changes or is rebuilding. The caller must treat the return
    /// value just as they would the return value of `addReverseDepAndCheckIfDone` by scheduling
    /// this node for evaluation if needed.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:390 NodeEntry.checkIfDoneForDirtyReverseDep
    fn check_if_done_for_dirty_reverse_dep(&mut self, reverse_dep: SkyKey) -> DependencyState;

    // Java: com/google/devtools/build/skyframe/NodeEntry.java:392 NodeEntry.getAllReverseDepsForNodeBeingDeleted
    fn get_all_reverse_deps_for_node_being_deleted(&mut self) -> Vec<SkyKey>;

    /// Tell this entry that one of its dependencies is now done. Callers must check the return
    /// value, and if true, they must re-schedule this node for evaluation.
    ///
    /// Even if `child_version` is not at most `getVersion`, this entry may not rebuild, in the
    /// case that the entry already rebuilt at `child_version` and discovered that it had the
    /// same value as at an earlier version. For instance, after evaluating at version v1, at
    /// version v2, child has a new value, but parent re-evaluates and finds it has the same
    /// value, child.getVersion() will return v2 and parent.getVersion() will return v1. At v3
    /// parent is dirtied and checks its dep on child. child signals parent with version v2.
    /// That should not in and of itself trigger a rebuild, since parent has already rebuilt with
    /// child at v2.
    ///
    /// `child_version`: if this entry `isDirty` and the last version at which this entry was
    /// evaluated did not include the changes at version `child_version` (for instance, if
    /// `child_version` is after the last version at which this entry was evaluated), then this
    /// entry records that one of its children has changed since it was last evaluated. Thus,
    /// the next call to `getLifecycleState` will return `LifecycleState#NEEDS_REBUILDING`.
    ///
    /// `child_for_debugging`: for use in debugging (can be used to identify specific children
    /// that invalidate this node).
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:416 NodeEntry.signalDep
    fn signal_dep(&mut self, child_version: Version, child_for_debugging: Option<&SkyKey>) -> bool;

    /// Marks this entry as up-to-date at this version.
    ///
    /// Returns `NodeValueAndRdepsToSignal` containing the SkyValue and reverse deps to signal.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:424 NodeEntry.markClean
    fn mark_clean(&mut self) -> NodeValueAndRdepsToSignal;

    /// Called on a dirty node during dependency checking (`LifecycleState#CHECK_DEPENDENCIES`)
    /// to force the node to be re-evaluated, even if none of its dependencies are known to have
    /// changed.
    ///
    /// Used when a caller has reason to believe that re-evaluating may yield a new result, such
    /// as when the prior evaluation encountered a transient error.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:458 NodeEntry.forceRebuild
    fn force_rebuild(&mut self);

    /// Returns the current version of this node.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:462 NodeEntry.getVersion
    fn get_version(&self) -> Version;

    /// Returns the maximal version of this node's dependencies from source.
    ///
    /// This version should only be tracked when non-hermetic functions inject source versions.
    /// Otherwise, returns `None` to signal that source versions are not being tracked.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:473 NodeEntry.getMaxTransitiveSourceVersion
    fn get_max_transitive_source_version(&self) -> Option<Version> {
        None
    }

    /// Returns the state of this entry as enumerated by `LifecycleState`.
    ///
    /// This method may be called at any time. Returns `LifecycleState#DONE` iff the node
    /// `isDone`.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:484 NodeEntry.getLifecycleState
    fn get_lifecycle_state(&self) -> LifecycleState;

    /// Should only be called if the entry is in the `LifecycleState#CHECK_DEPENDENCIES` state.
    /// During the examination to see if the entry must be re-evaluated, this method returns the
    /// next group of children to be checked. Callers should have already called
    /// `getLifecycleState` and received a return value of `CHECK_DEPENDENCIES` before calling
    /// this method -- any other return value from `getLifecycleState` means that this method
    /// must not be called, since whether or not the node needs to be rebuilt is already known.
    ///
    /// Deps are returned in groups. The deps in each group were requested in parallel by the
    /// `SkyFunction` last build, meaning independently of the values of any other deps in this
    /// group (although possibly depending on deps in earlier groups). Thus the caller may check
    /// all the deps in this group in parallel, since the deps in all previous groups are
    /// verified unchanged. See `SkyFunction.Environment#getValuesAndExceptions` for more on
    /// dependency groups.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:503 NodeEntry.getNextDirtyDirectDeps
    fn get_next_dirty_direct_deps(&mut self) -> Vec<SkyKey>;

    /// Returns all deps of a node that has not yet finished evaluating. In other words, if a
    /// node has a reverse dep on this node, its key will be in the returned set here.
    ///
    /// The returned set is the union of:
    ///
    /// * This node's temporary direct deps (`getTemporaryDirectDeps`).
    /// * Deps from a previous evaluation, if this this node was marked dirty (all the elements
    ///   that would have been returned by successive calls to `getNextDirtyDirectDeps` or,
    ///   equivalently, one call to `getAllRemainingDirtyDirectDeps`).
    /// * This node's reset direct deps (`getResetDirectDeps`).
    ///
    /// This method should only be called when this node is about to be deleted after an
    /// aborted evaluation. After such an evaluation, any nodes that did not finish evaluating
    /// are deleted, as are any nodes that depend on them, which are necessarily also not done.
    /// If this node is to be deleted because of this, we must delete it as a reverse dep from
    /// other nodes. This method returns that list of other nodes. This method may not be called
    /// on done nodes, since they do not need to be deleted after aborted evaluations.
    ///
    /// This method must not be called twice: the next thing done to this node after this
    /// method is called should be the removal of the node from the graph.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:530 NodeEntry.getAllDirectDepsForIncompleteNode
    fn get_all_direct_deps_for_incomplete_node(&mut self) -> IndexSet<SkyKey>;

    /// If an entry `isDirty`, returns all direct deps that were present last build, but have
    /// not yet been verified to be present during the current build. Implementations may lazily
    /// remove these deps, since in many cases they will be added back during this build, even
    /// though the node may have a changed value. However, any elements of this returned set that
    /// have not been added back by the end of evaluation *must* be removed from any done nodes,
    /// in order to preserve graph consistency.
    ///
    /// Returns the empty set if an entry is not dirty. In either case, the entry must already
    /// have started evaluation.
    ///
    /// This method does not mutate the entry. In particular, multiple calls to this method will
    /// always produce the same result until the entry finishes evaluation. Contrast with
    /// `getAllDirectDepsForIncompleteNode`.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:547 NodeEntry.getAllRemainingDirtyDirectDeps
    fn get_all_remaining_dirty_direct_deps(&mut self) -> IndexSet<SkyKey>;

    /// Notifies a node that it is about to be rebuilt. This method can only be called if the
    /// node `LifecycleState#NEEDS_REBUILDING`. After this call, this node is ready to be rebuilt
    /// (it will be in `LifecycleState#REBUILDING`).
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:554 NodeEntry.markRebuilding
    fn mark_rebuilding(&mut self);

    /// Returns the `GroupedDeps` of direct dependencies. This may only be called while the node
    /// is being evaluated (i.e. before `setValue` and after `markDirty`.
    ///
    /// PORT: `&mut` because Java creates the deps lazily on first access.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:561 NodeEntry.getTemporaryDirectDeps
    fn get_temporary_direct_deps(&mut self) -> &mut GroupedDeps;

    // Java: com/google/devtools/build/skyframe/NodeEntry.java:564 NodeEntry.noDepsLastBuild
    fn no_deps_last_build(&self) -> bool;

    /// Remove dep from direct deps. This should only be called if this entry is about to be
    /// committed as a cycle node, but some of its children were not checked for cycles, either
    /// because the cycle was discovered before some children were checked; some children
    /// didn't have a chance to finish before the evaluator aborted; or too many cycles were
    /// found when it came time to check the children.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:574 NodeEntry.removeUnfinishedDeps
    fn remove_unfinished_deps(&mut self, unfinished_deps: &HashSet<SkyKey>);

    /// Prepares this node to reset its evaluation from scratch in order to recover from an
    /// inconsistency.
    ///
    /// Called in the following scenario:
    ///
    /// 1. This node is evaluating (`SkyFunction.compute`).
    /// 2. The function returns a `SkyFunction.Reset` value: this node and the nodes of its rewind
    ///    graph must be evaluated again.
    ///
    /// Temporary direct deps are cleared, and the deps registered so far become the reset deps
    /// (`getResetDirectDeps`). The node is ready to be evaluated again.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:606 NodeEntry.resetEvaluationFromScratch
    fn reset_evaluation_from_scratch(&mut self);

    /// If the graph keeps dependency edges and `resetEvaluationFromScratch` has been called on
    /// this node since it was last done, returns the set of temporary direct deps that were
    /// registered prior to the restart. Otherwise, returns an empty set.
    ///
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:627 NodeEntry.getResetDirectDeps
    fn get_reset_direct_deps(&self) -> IndexSet<SkyKey>;

    /// Adds a temporary direct dep in its own group.
    ///
    /// The given dep must not be present in this node's existing temporary direct deps.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:635 NodeEntry.addSingletonTemporaryDirectDep
    fn add_singleton_temporary_direct_dep(&mut self, dep: SkyKey);

    /// Adds a temporary direct group.
    ///
    /// The group must be duplicate-free and not contain any deps in common with this node's
    /// existing temporary direct deps.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:644 NodeEntry.addTemporaryDirectDepGroup
    fn add_temporary_direct_dep_group(&mut self, group: Vec<SkyKey>);

    /// Adds temporary direct deps in groups.
    ///
    /// The iteration order of the given deps along with the `group_sizes` parameter dictate how
    /// deps are grouped. For example, if `deps = {a,b,c}` and `group_sizes = [2, 1]`, then there
    /// will be two groups: `[a,b]` and `[c]`. The sum of `group_sizes` must equal the size of
    /// `deps`. Note that it only makes sense to call this method with a set implementation that
    /// has a stable iteration order.
    ///
    /// The given set of deps must not contain any deps in common with this node's existing
    /// temporary direct deps.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:659 NodeEntry.addTemporaryDirectDepsInGroups
    #[allow(dead_code)] // ported API; not called by the blocking engine (restarts, rewinding, external deps)
    fn add_temporary_direct_deps_in_groups(&mut self, deps: &IndexSet<SkyKey>, group_sizes: &[i32]);

    /// PORT: not ported (external deps, `Environment.dependOnFuture`); fails with `unported!`.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:661 NodeEntry.addExternalDep
    #[allow(dead_code)] // ported API; not called by the blocking engine (restarts, rewinding, external deps)
    fn add_external_dep(&mut self);

    /// Returns true if the node has been signaled exactly as many times as it has temporary
    /// dependencies, or if `getKey().supportsPartialReevaluation()`. This may only be called
    /// while the node is being evaluated (i.e. before `setValue` and after `markDirty`).
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:669 NodeEntry.isReadyToEvaluate
    fn is_ready_to_evaluate(&self) -> bool;

    /// Returns true if the node has not been signaled exactly as many times as it has temporary
    /// dependencies. This may only be called while the node is being evaluated (i.e. before
    /// `setValue` and after `markDirty`).
    ///
    /// The node must not complete or be reset while in this state because it may yet be
    /// signaled.
    // Java: com/google/devtools/build/skyframe/NodeEntry.java:679 NodeEntry.hasUnsignaledDeps
    fn has_unsignaled_deps(&self) -> bool;
}
