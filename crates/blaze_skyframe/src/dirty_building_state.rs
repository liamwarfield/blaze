//! Port of `DirtyBuildingState`, `InitialBuildingState` and
//! `IncrementalInMemoryNodeEntry.IncrementalBuildingState`.

use crate::SkyKey;
use crate::grouped_deps::GroupedDeps;
use crate::node_entry::{DirtyType, LifecycleState};
use crate::value_with_metadata::ValueWithMetadata;
use crate::version::{NodeVersion, Version};
use blaze_util::check_state;
use indexmap::IndexSet;
use std::fmt;

// Java: com/google/devtools/build/skyframe/DirtyBuildingState.java:39 DirtyBuildingState.NOT_EVALUATING_SENTINEL
const NOT_EVALUATING_SENTINEL: i32 = -1;

/// State for a node that either has not been built yet or has been dirtied.
///
/// If the node has previously been built and the state tracks the previous value and
/// dependencies for purposes of pruning, `isIncremental` returns true. Deps are checked to see if
/// re-evaluation is needed, and the node will either marked clean or re-evaluated.
///
/// This class does not attempt to synchronize operations. It is assumed that the calling
/// `InMemoryNodeEntry` performs the appropriate synchronization when necessary.
///
/// PORT: Java's abstract class and its subclasses `InitialBuildingState` and
/// `IncrementalInMemoryNodeEntry.IncrementalBuildingState` are one struct whose
/// [`DirtyBuildingStateKind`] holds what the subclass adds. `NonIncrementalBuildingState`
/// (which extends `InitialBuildingState`) wraps an `Initial` state in
/// `non_incremental_in_memory_node_entry.rs`. The `Reset*BuildingState` and
/// `RewoundNonIncrementalBuildingState` subclasses are not ported (rewinding).
// Java: com/google/devtools/build/skyframe/DirtyBuildingState.java:38 DirtyBuildingState
pub(crate) struct DirtyBuildingState {
    /// The state of a dirty node.
    ///
    /// Initialized to either `CHECK_DEPENDENCIES` or `NEEDS_REBUILDING` depending on the
    /// `DirtyType` (see `initialState`). May take on any `LifecycleState` value except
    /// `NOT_YET_EVALUATING` and `DONE`.
    state: LifecycleState,

    /// The number of dependencies that are known to be done in a `NodeEntry`.
    ///
    /// There is a potential check-then-act race here during evaluation, so we need to make sure
    /// that when this is increased, we always check if the new value is equal to the number of
    /// required dependencies, and if so, we must re-schedule the node for evaluation.
    ///
    /// There are two potential pitfalls here: 1) If multiple dependencies signal this node in
    /// close succession, this node should be scheduled exactly once. 2) If a thread is still
    /// working on this node, it should not be scheduled.
    ///
    /// To solve the first problem, the `NodeEntry.signalDep` method also returns if the node
    /// needs to be re-scheduled, and ensures that only one thread gets a true return value.
    ///
    /// The second problem is solved by first adding the newly discovered deps to a node's
    /// `IncrementalInMemoryNodeEntry.directDeps`, and then looping through the direct deps and
    /// registering this node as a reverse dependency. This ensures that the signaledDeps counter
    /// can only reach `GroupedDeps.numElements` on the very last iteration of the loop, i.e., the
    /// thread is not working on the node anymore. Note that this requires that there is no code
    /// after the loop in `ParallelEvaluator.Evaluate.run`.
    signaled_deps: i32,

    /// The number of external dependencies (in contrast to the number of internal dependencies
    /// which are tracked in NodeEntry). We never keep information about external dependencies
    /// across Skyframe calls.
    external_deps: i32,

    /// Group of children to be checked next in the process of determining if this entry needs to
    /// be re-evaluated. Used by `getNextDirtyDirectDeps` and `signalDep`.
    dirty_direct_dep_index: i32,

    kind: DirtyBuildingStateKind,
}

/// What the Java subclass of `DirtyBuildingState` adds.
pub(crate) enum DirtyBuildingStateKind {
    /// `DirtyBuildingState` for a node on its initial build.
    // Java: com/google/devtools/build/skyframe/InitialBuildingState.java:23 InitialBuildingState
    Initial,
    /// `DirtyBuildingState` for a node on an incremental build.
    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:446 IncrementalInMemoryNodeEntry.IncrementalBuildingState
    Incremental {
        last_build_direct_deps: GroupedDeps,
        last_build_value: ValueWithMetadata,
    },
}

impl DirtyBuildingState {
    // Java: com/google/devtools/build/skyframe/DirtyBuildingState.java:123 DirtyBuildingState.<init>
    fn new(dirty_type: DirtyType, kind: DirtyBuildingStateKind) -> DirtyBuildingState {
        DirtyBuildingState {
            state: DirtyBuildingState::initial_state(dirty_type),
            signaled_deps: NOT_EVALUATING_SENTINEL,
            external_deps: 0,
            dirty_direct_dep_index: 0,
            kind,
        }
    }

    // Java: com/google/devtools/build/skyframe/InitialBuildingState.java:25 InitialBuildingState.<init>
    pub(crate) fn new_initial_building_state() -> DirtyBuildingState {
        DirtyBuildingState::new(DirtyType::Change, DirtyBuildingStateKind::Initial)
    }

    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:450 IncrementalBuildingState.<init>
    pub(crate) fn new_incremental_building_state(
        dirty_type: DirtyType,
        last_build_direct_deps: GroupedDeps,
        last_build_value: ValueWithMetadata,
    ) -> DirtyBuildingState {
        DirtyBuildingState::new(
            dirty_type,
            DirtyBuildingStateKind::Incremental {
                last_build_direct_deps,
                last_build_value,
            },
        )
    }

    /// Returns the `GroupedDeps` requested last time the node was built, or `None` if on its
    /// initial build.
    ///
    /// Dependencies from the last build are be compared to dependencies requested on this build
    /// to check whether this node has changed in `NodeEntry.setValue`. See
    /// `IncrementalInMemoryNodeEntry.directDeps` for more on dependency group storage.
    // Java: com/google/devtools/build/skyframe/DirtyBuildingState.java:99 DirtyBuildingState.getLastBuildDirectDeps
    // Java: com/google/devtools/build/skyframe/InitialBuildingState.java:31 InitialBuildingState.getLastBuildDirectDeps
    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:468 IncrementalBuildingState.getLastBuildDirectDeps
    pub(crate) fn get_last_build_direct_deps(&self) -> Option<&GroupedDeps> {
        match &self.kind {
            DirtyBuildingStateKind::Initial => None,
            DirtyBuildingStateKind::Incremental {
                last_build_direct_deps,
                ..
            } => Some(last_build_direct_deps),
        }
    }

    /// The number of groups of the dependencies requested last time when the node was built, or
    /// `0` if on its initial build.
    // Java: com/google/devtools/build/skyframe/DirtyBuildingState.java:107 DirtyBuildingState.getNumOfGroupsInLastBuildDirectDeps
    // Java: com/google/devtools/build/skyframe/InitialBuildingState.java:36 InitialBuildingState.getNumOfGroupsInLastBuildDirectDeps
    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:473 IncrementalBuildingState.getNumOfGroupsInLastBuildDirectDeps
    fn get_num_of_groups_in_last_build_direct_deps(&self) -> i32 {
        match &self.kind {
            DirtyBuildingStateKind::Initial => 0,
            DirtyBuildingStateKind::Incremental {
                last_build_direct_deps,
                ..
            } => last_build_direct_deps.num_groups() as i32,
        }
    }

    /// The value of the node the last time it was built, or `None` if on its initial build.
    // Java: com/google/devtools/build/skyframe/DirtyBuildingState.java:115 DirtyBuildingState.getLastBuildValue
    // Java: com/google/devtools/build/skyframe/InitialBuildingState.java:42 InitialBuildingState.getLastBuildValue
    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:463 IncrementalBuildingState.getLastBuildValue
    pub(crate) fn get_last_build_value(&self) -> Option<&ValueWithMetadata> {
        match &self.kind {
            DirtyBuildingStateKind::Initial => None,
            DirtyBuildingStateKind::Incremental {
                last_build_value, ..
            } => Some(last_build_value),
        }
    }

    // Java: com/google/devtools/build/skyframe/DirtyBuildingState.java:127 DirtyBuildingState.initialState
    fn initial_state(dirty_type: DirtyType) -> LifecycleState {
        match dirty_type {
            DirtyType::Dirty => LifecycleState::CheckDependencies,
            DirtyType::Change | DirtyType::Rewind => LifecycleState::NeedsRebuilding,
        }
    }

    /// Returns true if this state has information about a previously built version.
    // Java: com/google/devtools/build/skyframe/DirtyBuildingState.java:139 DirtyBuildingState.isIncremental
    pub(crate) fn is_incremental(&self) -> bool {
        match &self.kind {
            DirtyBuildingStateKind::Initial => false,
            DirtyBuildingStateKind::Incremental { .. } => true,
        }
    }

    // Java: com/google/devtools/build/skyframe/DirtyBuildingState.java:141 DirtyBuildingState.markChanged
    pub(crate) fn mark_changed(&mut self) {
        check_state!(
            self.state == LifecycleState::CheckDependencies,
            "{:?}",
            self
        );
        check_state!(
            self.dirty_direct_dep_index == 0,
            "Unexpected evaluation: {:?}",
            self
        );
        self.state = LifecycleState::NeedsRebuilding;
    }

    // Java: com/google/devtools/build/skyframe/DirtyBuildingState.java:147 DirtyBuildingState.forceRebuild
    pub(crate) fn force_rebuild(&mut self, num_temporary_direct_deps: i32) {
        check_state!(
            self.state == LifecycleState::CheckDependencies,
            "{:?}",
            self
        );
        check_state!(
            num_temporary_direct_deps + self.external_deps == self.signaled_deps,
            "{:?}",
            self
        );
        check_state!(
            self.get_num_of_groups_in_last_build_direct_deps() == self.dirty_direct_dep_index,
            "{:?}",
            self
        );
        self.state = LifecycleState::Rebuilding;
    }

    // Java: com/google/devtools/build/skyframe/DirtyBuildingState.java:154 DirtyBuildingState.isEvaluating
    pub(crate) fn is_evaluating(&self) -> bool {
        self.signaled_deps > NOT_EVALUATING_SENTINEL
    }

    // Java: com/google/devtools/build/skyframe/DirtyBuildingState.java:158 DirtyBuildingState.isChanged
    pub(crate) fn is_changed(&self) -> bool {
        self.state == LifecycleState::NeedsRebuilding || self.state == LifecycleState::Rebuilding
    }

    // Java: com/google/devtools/build/skyframe/DirtyBuildingState.java:162 DirtyBuildingState.checkFinishedBuildingWhenAboutToSetValue
    fn check_finished_building_when_about_to_set_value(&self) {
        check_state!(
            self.state == LifecycleState::VerifiedClean || self.state == LifecycleState::Rebuilding,
            "not done building {:?}",
            self
        );
    }

    /// Signals that a child is done.
    ///
    /// If this node is not yet known to need rebuilding, sets `state` to `NEEDS_REBUILDING` if
    /// the child has changed, and `VERIFIED_CLEAN` if the child has not changed and this was the
    /// last child to be checked (as determined by `isReady` and comparing `dirtyDirectDepIndex`
    /// and `getNumOfGroupsInLastBuildDirectDeps()`.
    ///
    /// PORT: Java passes the entry, for `entry.getNumTemporaryDirectDeps()` and for the message
    /// of the first check. The entry owns this state, so Rust cannot lend both: the entry makes
    /// the first check (`checkState(isEvaluating(), "%s %s", entry, childForDebugging)`) itself
    /// before calling, and passes its number of temporary direct deps (a pure read).
    // Java: com/google/devtools/build/skyframe/DirtyBuildingState.java:178 DirtyBuildingState.signalDep
    pub(crate) fn signal_dep(
        &mut self,
        num_temporary_direct_deps: i32,
        version: &NodeVersion,
        child_version: &Version,
        _child_for_debugging: Option<&SkyKey>,
    ) {
        self.signaled_deps += 1;
        if self.is_changed() {
            return;
        }

        // childVersion > version.lastEvaluated() means the child has changed since the last
        // evaluation.
        let child_changed = !child_version.at_most(version.last_evaluated());
        if child_changed {
            self.state = LifecycleState::NeedsRebuilding;
        } else if self.state == LifecycleState::CheckDependencies
            && self.is_ready(num_temporary_direct_deps)
            && self.get_num_of_groups_in_last_build_direct_deps() == self.dirty_direct_dep_index
        {
            // No other dep already marked this as NEEDS_REBUILDING, no deps outstanding, and this
            // was the last block of deps to be checked.
            self.state = LifecycleState::VerifiedClean;
        }
    }

    // Java: com/google/devtools/build/skyframe/DirtyBuildingState.java:202 DirtyBuildingState.addExternalDep
    #[allow(dead_code)] // its only caller (external deps) is not ported
    pub(crate) fn add_external_dep(&mut self) {
        check_state!(self.is_evaluating());
        self.external_deps += 1;
    }

    /// Returns true if `new_value`.equals the value from the last time this node was built.
    /// Should only be used by `NodeEntry.setValue`.
    ///
    /// Changes in direct deps do *not* force this to return false. Only the value is considered.
    ///
    /// PORT: Rust values carry no `equals`; equality is the key's ([`crate::Key::values_equal`],
    /// decision D10), so the node's key is passed. `newValue instanceof NotComparableSkyValue`
    /// is true here for an `ErrorInfoValue`; for any other value the key's `values_equal`
    /// answers (a not-comparable value's key never returns true).
    // Java: com/google/devtools/build/skyframe/DirtyBuildingState.java:214 DirtyBuildingState.unchangedFromLastBuild
    pub(crate) fn unchanged_from_last_build(
        &self,
        key: &SkyKey,
        new_value: &ValueWithMetadata,
    ) -> bool {
        self.check_finished_building_when_about_to_set_value();
        !new_value.is_error_info_value()
            && self.get_last_build_value().is_some()
            && self
                .get_last_build_value()
                .expect("checked above")
                .equals(key, new_value)
    }

    /// Returns true if the deps requested during this evaluation (`direct_deps`) are exactly
    /// those requested the last time this node was built, in the same order.
    // Java: com/google/devtools/build/skyframe/DirtyBuildingState.java:225 DirtyBuildingState.depsUnchangedFromLastBuild
    pub(crate) fn deps_unchanged_from_last_build(&self, direct_deps: &GroupedDeps) -> bool {
        self.check_finished_building_when_about_to_set_value();
        check_not_null!(self.get_last_build_direct_deps()) == direct_deps
    }

    // Java: com/google/devtools/build/skyframe/DirtyBuildingState.java:230 DirtyBuildingState.noDepsLastBuild
    pub(crate) fn no_deps_last_build(&self) -> bool {
        self.get_num_of_groups_in_last_build_direct_deps() == 0
    }

    /// Returns the `LifecycleState` as documented by `NodeEntry.getLifecycleState`.
    // Java: com/google/devtools/build/skyframe/DirtyBuildingState.java:235 DirtyBuildingState.getLifecycleState
    pub(crate) fn get_lifecycle_state(&self) -> LifecycleState {
        self.state
    }

    /// Gets the next children to be re-evaluated to see if this dirty node needs to be
    /// re-evaluated.
    ///
    /// See `NodeEntry.getNextDirtyDirectDeps`.
    // Java: com/google/devtools/build/skyframe/DirtyBuildingState.java:244 DirtyBuildingState.getNextDirtyDirectDeps
    pub(crate) fn get_next_dirty_direct_deps(&mut self) -> Vec<SkyKey> {
        check_state!(
            self.state == LifecycleState::CheckDependencies,
            "{:?}",
            self
        );
        check_state!(
            self.dirty_direct_dep_index < self.get_num_of_groups_in_last_build_direct_deps(),
            "{:?}",
            self
        );
        let index = self.dirty_direct_dep_index;
        self.dirty_direct_dep_index += 1;
        check_not_null!(self.get_last_build_direct_deps())
            .get_dep_group(index as usize)
            .to_vec()
    }

    /// Returns the remaining direct deps that have not been checked. If `preserve_position` is
    /// true, this method is non-mutating. If `preserve_position` is false, the caller must
    /// process the returned set, and so subsequent calls to this method will return the empty
    /// set.
    // Java: com/google/devtools/build/skyframe/DirtyBuildingState.java:255 DirtyBuildingState.getAllRemainingDirtyDirectDeps
    pub(crate) fn get_all_remaining_dirty_direct_deps(
        &mut self,
        preserve_position: bool,
    ) -> IndexSet<SkyKey> {
        let Some(last_build_direct_deps) = self.get_last_build_direct_deps() else {
            return IndexSet::new();
        };
        let mut result: IndexSet<SkyKey> = IndexSet::new();
        for ind in self.dirty_direct_dep_index..self.get_num_of_groups_in_last_build_direct_deps() {
            result.extend(
                last_build_direct_deps
                    .get_dep_group(ind as usize)
                    .iter()
                    .cloned(),
            );
        }
        if !preserve_position {
            self.dirty_direct_dep_index = self.get_num_of_groups_in_last_build_direct_deps();
        }
        result
    }

    /// PORT: only the base implementation (`ImmutableSet.of()`); the `Reset*BuildingState`
    /// overrides are not ported (rewinding).
    // Java: com/google/devtools/build/skyframe/DirtyBuildingState.java:270 DirtyBuildingState.getResetDirectDeps
    pub(crate) fn get_reset_direct_deps(&self) -> IndexSet<SkyKey> {
        IndexSet::new()
    }

    // Java: com/google/devtools/build/skyframe/DirtyBuildingState.java:274 DirtyBuildingState.markRebuilding
    pub(crate) fn mark_rebuilding(&mut self) {
        check_state!(self.state == LifecycleState::NeedsRebuilding, "{:?}", self);
        self.state = LifecycleState::Rebuilding;
    }

    // Java: com/google/devtools/build/skyframe/DirtyBuildingState.java:279 DirtyBuildingState.startEvaluating
    pub(crate) fn start_evaluating(&mut self) {
        check_state!(!self.is_evaluating(), "{:?}", self);
        self.signaled_deps = 0;
    }

    /// Returns whether all known children of this node have signaled that they are done.
    // Java: com/google/devtools/build/skyframe/DirtyBuildingState.java:285 DirtyBuildingState.isReady
    pub(crate) fn is_ready(&self, num_direct_deps: i32) -> bool {
        // Avoids calling Preconditions.checkState because it showed up in garbage profiles due to
        // boxing of the int format args.
        if self.signaled_deps > num_direct_deps + self.external_deps {
            panic!(
                "IllegalStateException: {} {} {:?}",
                num_direct_deps, self.external_deps, self
            );
        }
        self.signaled_deps == num_direct_deps + self.external_deps
    }

    /// The fields of `getStringHelper`, for subclasses that add their own.
    // Java: com/google/devtools/build/skyframe/DirtyBuildingState.java:294 DirtyBuildingState.getStringHelper
    pub(crate) fn fmt_string_helper_fields(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "state={:?}, signaledDeps={}, externalDeps={}, dirtyDirectDepIndex={}",
            self.state, self.signaled_deps, self.external_deps, self.dirty_direct_dep_index
        )?;
        match &self.kind {
            DirtyBuildingStateKind::Initial => Ok(()),
            // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:478 IncrementalBuildingState.getStringHelper
            DirtyBuildingStateKind::Incremental {
                last_build_direct_deps,
                last_build_value,
            } => write!(
                f,
                ", lastBuildDirectDeps={last_build_direct_deps:?}, lastBuildValue={last_build_value:?}"
            ),
        }
    }
}

impl fmt::Debug for DirtyBuildingState {
    // Java: com/google/devtools/build/skyframe/DirtyBuildingState.java:303 DirtyBuildingState.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match &self.kind {
            DirtyBuildingStateKind::Initial => "InitialBuildingState{",
            DirtyBuildingStateKind::Incremental { .. } => "IncrementalBuildingState{",
        })?;
        self.fmt_string_helper_fields(f)?;
        f.write_str("}")
    }
}
