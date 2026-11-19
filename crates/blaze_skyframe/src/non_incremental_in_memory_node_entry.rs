//! Port of `NonIncrementalInMemoryNodeEntry`.

use crate::SkyKey;
use crate::abstract_in_memory_node_entry::{
    AbstractInMemoryNodeEntry, AbstractInMemoryNodeEntryFields, AsDirtyBuildingState, Nullable,
};
use crate::dirty_building_state::DirtyBuildingState;
use crate::grouped_deps::GroupedDeps;
use crate::node_entry::{DependencyState, DirtyType, MarkedDirtyResult, NodeValueAndRdepsToSignal};
use crate::reverse_deps_utility::ReverseDepsUtility;
use crate::value_with_metadata::ValueWithMetadata;
use crate::version::{NodeVersion, Version};
use blaze_util::{check_argument, check_state};
use indexmap::IndexSet;
use std::collections::HashSet;
use std::fmt;

/// An `InMemoryNodeEntry` that does not store edges (direct deps and reverse deps) once the node
/// is done. Used to save memory when the graph will not be reused for incremental builds.
///
/// Edges are stored as usual while the node is being built, but are discarded once the node is
/// done.
///
/// It is illegal to access edges once the node `isDone`.
// Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:40 NonIncrementalInMemoryNodeEntry
pub(crate) struct NonIncrementalInMemoryNodeEntry {
    pub(crate) base: AbstractInMemoryNodeEntryFields<NonIncrementalBuildingState>,
}

impl AbstractInMemoryNodeEntry for NonIncrementalInMemoryNodeEntry {
    type D = NonIncrementalBuildingState;

    fn base(&self) -> &AbstractInMemoryNodeEntryFields<NonIncrementalBuildingState> {
        &self.base
    }

    fn base_mut(&mut self) -> &mut AbstractInMemoryNodeEntryFields<NonIncrementalBuildingState> {
        &mut self.base
    }

    // Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:135 NonIncrementalInMemoryNodeEntry.getNumTemporaryDirectDeps
    fn get_num_temporary_direct_deps(&self) -> i32 {
        let Some(dirty_building_state) = &self.base.dirty_building_state else {
            return 0;
        };
        match &dirty_building_state.direct_deps {
            None => 0,
            Some(direct_deps) => direct_deps.num_elements() as i32,
        }
    }

    // Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:110 NonIncrementalInMemoryNodeEntry.getTemporaryDirectDeps
    fn get_temporary_direct_deps(&mut self) -> &mut GroupedDeps {
        check_not_null!(
            self.base.dirty_building_state.as_ref(),
            "Not evaluating: {:?}",
            self
        );
        let new_grouped_deps = self.new_grouped_deps();
        self.base
            .dirty_building_state
            .as_mut()
            .expect("checked above")
            .get_temporary_direct_deps(new_grouped_deps)
    }
}

impl NonIncrementalInMemoryNodeEntry {
    // Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:43 NonIncrementalInMemoryNodeEntry.<init>
    pub(crate) fn new(key: SkyKey) -> NonIncrementalInMemoryNodeEntry {
        NonIncrementalInMemoryNodeEntry {
            base: AbstractInMemoryNodeEntryFields::new(key),
        }
    }

    // Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:48 NonIncrementalInMemoryNodeEntry.keepsEdges
    #[allow(dead_code)] // ported API, used by tests or not reached without rewinding
    pub(crate) fn keeps_edges(&self) -> bool {
        false
    }

    /// PORT: `max_transitive_source_version` is ignored, as in Java.
    // Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:54 NonIncrementalInMemoryNodeEntry.setValue
    pub(crate) fn set_value(
        &mut self,
        value: ValueWithMetadata,
        graph_version: Version,
        _max_transitive_source_version: Option<Version>,
    ) -> IndexSet<SkyKey> {
        check_argument!(
            graph_version == Version::constant(),
            "Non-incremental evaluations must be at a constant version: {:?}",
            graph_version
        );
        check_state!(
            !self.has_unsignaled_deps(),
            "Has unsignaled deps (this={:?}, value={:?})",
            self,
            value
        );
        self.base.value = Some(value);
        let reverse_deps_to_signal = self
            .base
            .dirty_building_state
            .as_ref()
            .expect("evaluating")
            .get_reverse_deps(self);
        self.base.dirty_building_state = None;
        reverse_deps_to_signal
    }

    // Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:69 NonIncrementalInMemoryNodeEntry.addReverseDepAndCheckIfDone
    pub(crate) fn add_reverse_dep_and_check_if_done(
        &mut self,
        reverse_dep: Option<SkyKey>,
    ) -> DependencyState {
        // Fast path check before locking. If this node is already done, there is nothing to do
        // since we aren't storing reverse deps.
        if self.is_done() {
            return DependencyState::Done;
        }

        // Check again under a lock.
        if self.is_done() {
            return DependencyState::Done;
        }
        let dirty_building_state = self
            .base
            .dirty_building_state
            .get_or_insert_with(NonIncrementalBuildingState::new);
        if let Some(reverse_dep) = reverse_dep {
            dirty_building_state.add_reverse_dep(reverse_dep);
        }
        if dirty_building_state.base.is_evaluating() {
            return DependencyState::AlreadyEvaluating;
        }
        dirty_building_state.base.start_evaluating();
        DependencyState::NeedsScheduling
    }

    /// A `NonIncrementalInMemoryNodeEntry` can only ever be at one of two versions: either
    /// `Version#constant` when a value is available, or `Version#minimal` otherwise.
    ///
    /// All non-incremental evaluations must use `Version#constant` as the graph version. This is
    /// enforced in `setValue`.
    // Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:105 NonIncrementalInMemoryNodeEntry.getVersion
    pub(crate) fn get_version(&self) -> Version {
        if self.base.value.is_some() {
            Version::constant()
        } else {
            Version::minimal()
        }
    }

    /// PORT: not ported (rewinding, docs/decisions.md 2026-11-19).
    // Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:116 NonIncrementalInMemoryNodeEntry.resetEvaluationFromScratch
    #[allow(dead_code)] // ported API, used by tests or not reached without rewinding
    pub(crate) fn reset_evaluation_from_scratch(&mut self) {
        blaze_util::unported!("NonIncrementalInMemoryNodeEntry.resetEvaluationFromScratch")
    }

    // Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:130 NonIncrementalInMemoryNodeEntry.getResetDirectDeps
    #[allow(dead_code)] // ported API, used by tests or not reached without rewinding
    pub(crate) fn get_reset_direct_deps(&self) -> IndexSet<SkyKey> {
        IndexSet::new() // No accounting necessary since rdeps are not stored.
    }

    // Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:145 NonIncrementalInMemoryNodeEntry.markDirty
    pub(crate) fn mark_dirty(&mut self, dirty_type: DirtyType) -> Option<MarkedDirtyResult> {
        check_argument!(
            dirty_type == DirtyType::Rewind,
            "Unexpected dirty type: {:?}",
            dirty_type
        );
        // PORT: rewinding is not ported (docs/decisions.md 2026-11-19): Java tolerates
        // concurrent requests, ignores errors, and installs a RewoundNonIncrementalBuildingState.
        blaze_util::unported!("NonIncrementalInMemoryNodeEntry.markDirty(DirtyType.REWIND)")
    }

    // Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:159 NonIncrementalInMemoryNodeEntry.getInProgressReverseDeps
    pub(crate) fn get_in_progress_reverse_deps(&self) -> IndexSet<SkyKey> {
        check_state!(!self.is_done(), "{:?}", self);
        match &self.base.dirty_building_state {
            None => IndexSet::new(),
            Some(dirty_building_state) => dirty_building_state.get_reverse_deps(self),
        }
    }

    // Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:165 NonIncrementalInMemoryNodeEntry.signalDep
    pub(crate) fn signal_dep(
        &mut self,
        child_version: Version,
        child_for_debugging: Option<&SkyKey>,
    ) -> bool {
        check_state!(
            !self.is_done(),
            "Value must not be done in signalDep {:?} child={:?}",
            self,
            Nullable(child_for_debugging)
        );
        let dirty_building_state = check_not_null!(
            self.base.dirty_building_state.as_ref(),
            "{:?} {:?}",
            self,
            Nullable(child_for_debugging)
        );
        // Java: the first check of DirtyBuildingState.signalDep (see its PORT note).
        check_state!(
            dirty_building_state.base.is_evaluating(),
            "{:?} {:?}",
            self,
            Nullable(child_for_debugging)
        );
        let num_temporary_direct_deps = self.get_num_temporary_direct_deps();
        self.base
            .dirty_building_state
            .as_mut()
            .expect("checked above")
            .base
            .signal_dep(
                num_temporary_direct_deps,
                &NodeVersion::Version(Version::minimal()),
                &child_version,
                child_for_debugging,
            );
        !self.has_unsignaled_deps()
    }

    // Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:175 NonIncrementalInMemoryNodeEntry.removeReverseDep
    pub(crate) fn remove_reverse_dep(&mut self, reverse_dep: SkyKey) {
        check_not_null!(
            self.base.dirty_building_state.as_ref(),
            "Not evaluating: {:?}",
            self
        );
        self.base
            .dirty_building_state
            .as_mut()
            .expect("checked above")
            .remove_reverse_dep(&reverse_dep);
    }

    // Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:180 NonIncrementalInMemoryNodeEntry.getCompressedDirectDepsForDoneEntry
    #[allow(dead_code)] // ported API, used by tests or not reached without rewinding
    pub(crate) fn get_compressed_direct_deps_for_done_entry(&self) -> &GroupedDeps {
        self.unsupported()
    }

    // Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:185 NonIncrementalInMemoryNodeEntry.getDirectDeps
    pub(crate) fn get_direct_deps(&self) -> Vec<SkyKey> {
        self.unsupported()
    }

    // Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:190 NonIncrementalInMemoryNodeEntry.getAllDirectDepsForIncompleteNode
    pub(crate) fn get_all_direct_deps_for_incomplete_node(&mut self) -> IndexSet<SkyKey> {
        self.unsupported()
    }

    // Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:195 NonIncrementalInMemoryNodeEntry.hasAtLeastOneDep
    pub(crate) fn has_at_least_one_dep(&self) -> bool {
        self.unsupported()
    }

    // Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:200 NonIncrementalInMemoryNodeEntry.removeReverseDepsFromDoneEntryDueToDeletion
    pub(crate) fn remove_reverse_deps_from_done_entry_due_to_deletion(
        &mut self,
        _deleted_keys: &HashSet<SkyKey>,
    ) {
        self.unsupported()
    }

    // Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:205 NonIncrementalInMemoryNodeEntry.getReverseDepsForDoneEntry
    pub(crate) fn get_reverse_deps_for_done_entry(&mut self) -> Vec<SkyKey> {
        self.unsupported()
    }

    // Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:210 NonIncrementalInMemoryNodeEntry.getAllReverseDepsForNodeBeingDeleted
    pub(crate) fn get_all_reverse_deps_for_node_being_deleted(&mut self) -> Vec<SkyKey> {
        self.unsupported()
    }

    // Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:215 NonIncrementalInMemoryNodeEntry.checkIfDoneForDirtyReverseDep
    pub(crate) fn check_if_done_for_dirty_reverse_dep(
        &mut self,
        _reverse_dep: SkyKey,
    ) -> DependencyState {
        self.unsupported()
    }

    // Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:220 NonIncrementalInMemoryNodeEntry.markClean
    pub(crate) fn mark_clean(&mut self) -> NodeValueAndRdepsToSignal {
        self.unsupported()
    }

    // Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:225 NonIncrementalInMemoryNodeEntry.forceRebuild
    pub(crate) fn force_rebuild(&mut self) {
        self.unsupported()
    }

    // Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:229 NonIncrementalInMemoryNodeEntry.unsupported
    fn unsupported(&self) -> ! {
        panic!(
            "UnsupportedOperationException: Not keeping edges: {:?}",
            self
        )
    }
}

impl fmt::Debug for NonIncrementalInMemoryNodeEntry {
    // Java: com/google/devtools/build/skyframe/AbstractInMemoryNodeEntry.java:258 AbstractInMemoryNodeEntry.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NonIncrementalInMemoryNodeEntry{")?;
        self.fmt_to_string_helper_fields(f)?;
        f.write_str("}")
    }
}

/// Specialized `DirtyBuildingState` for a non-incremental node.
///
/// The `directDeps` and `reverseDeps` fields are stored in this class instead of in
/// `NonIncrementalInMemoryNodeEntry` since they are not needed after the node is done. This way
/// we don't pay the memory cost of the fields for a done node.
///
/// PORT: Java extends `InitialBuildingState`; here it wraps one (`base`).
// Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:240 NonIncrementalInMemoryNodeEntry.NonIncrementalBuildingState
pub(crate) struct NonIncrementalBuildingState {
    pub(crate) base: DirtyBuildingState,
    direct_deps: Option<GroupedDeps>,
    reverse_deps: Option<Vec<SkyKey>>,
}

impl AsDirtyBuildingState for NonIncrementalBuildingState {
    fn dirty_building_state(&self) -> &DirtyBuildingState {
        &self.base
    }
    fn dirty_building_state_mut(&mut self) -> &mut DirtyBuildingState {
        &mut self.base
    }
}

impl NonIncrementalBuildingState {
    // Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:244 NonIncrementalBuildingState.<init>
    fn new() -> NonIncrementalBuildingState {
        NonIncrementalBuildingState {
            base: DirtyBuildingState::new_initial_building_state(),
            direct_deps: None,
            reverse_deps: None,
        }
    }

    /// PORT: Java passes the entry for `entry.newGroupedDeps()`; the entry owns this state, so it
    /// passes the new (empty) `GroupedDeps` instead, used only if none exists yet.
    // Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:246 NonIncrementalBuildingState.getTemporaryDirectDeps
    fn get_temporary_direct_deps(&mut self, new_grouped_deps: GroupedDeps) -> &mut GroupedDeps {
        self.direct_deps.get_or_insert(new_grouped_deps)
    }

    // Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:253 NonIncrementalBuildingState.addReverseDep
    fn add_reverse_dep(&mut self, reverse_dep: SkyKey) {
        self.reverse_deps
            .get_or_insert_with(Vec::new)
            .push(reverse_dep);
    }

    // Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:260 NonIncrementalBuildingState.removeReverseDep
    fn remove_reverse_dep(&mut self, reverse_dep: &SkyKey) {
        // Reverse dep removal on a non-incremental node is rare (only for cycles), so we can live
        // with inefficiently calling remove on an ArrayList.
        let reverse_deps = check_not_null!(self.reverse_deps.as_mut());
        let removed = match reverse_deps.iter().position(|k| k == reverse_dep) {
            Some(i) => {
                reverse_deps.remove(i);
                true
            }
            None => false,
        };
        check_state!(removed, "Reverse dep not present: {:?}", reverse_dep);
    }

    // Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:266 NonIncrementalBuildingState.getReverseDeps
    fn get_reverse_deps(&self, entry: &NonIncrementalInMemoryNodeEntry) -> IndexSet<SkyKey> {
        let Some(reverse_deps) = &self.reverse_deps else {
            return IndexSet::new();
        };
        let result: IndexSet<SkyKey> = reverse_deps.iter().cloned().collect();
        ReverseDepsUtility::check_for_duplicates(&result, reverse_deps, entry);
        result
    }
}

impl fmt::Debug for NonIncrementalBuildingState {
    // Java: com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntry.java:276 NonIncrementalBuildingState.getStringHelper
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NonIncrementalBuildingState{")?;
        self.base.fmt_string_helper_fields(f)?;
        write!(
            f,
            ", directDeps={:?}, reverseDeps={:?}}}",
            Nullable(self.direct_deps.as_ref()),
            Nullable(self.reverse_deps.as_ref())
        )
    }
}
