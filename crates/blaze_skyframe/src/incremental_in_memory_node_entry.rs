//! Port of `IncrementalInMemoryNodeEntry`.

use crate::SkyKey;
use crate::abstract_in_memory_node_entry::{
    AbstractInMemoryNodeEntry, AbstractInMemoryNodeEntryFields, Nullable,
};
use crate::dirty_building_state::DirtyBuildingState;
use crate::grouped_deps::GroupedDeps;
use crate::node_entry::{DependencyState, DirtyType, MarkedDirtyResult, NodeValueAndRdepsToSignal};
use crate::reverse_deps_utility::{KeyToConsolidate, Op, ReverseDepsUtility};
use crate::value_with_metadata::ValueWithMetadata;
use crate::version::{NodeVersion, Version};
use blaze_util::check_state;
use indexmap::IndexSet;
use std::collections::HashSet;
use std::fmt;

/// An `InMemoryNodeEntry` that `keepsEdges` for use in incremental evaluations.
// Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:32 IncrementalInMemoryNodeEntry
pub(crate) struct IncrementalInMemoryNodeEntry {
    pub(crate) base: AbstractInMemoryNodeEntryFields<DirtyBuildingState>,

    pub(crate) version: NodeVersion,

    /// This object represents the direct deps of the node, in groups if the `SkyFunction`
    /// requested them that way. It contains either the in-progress direct deps before the node
    /// is finished building, or the full direct deps after the node is done.
    ///
    /// It is initialized lazily in getTemporaryDirectDeps() to save a little memory.
    ///
    /// PORT: Java compresses the deps of a done node (`GroupedDeps.compress`); the Rust
    /// `GroupedDeps` is kept as is (memory format).
    direct_deps: Option<GroupedDeps>,

    /// This list stores the reverse dependencies of this node that have been declared so far.
    ///
    /// When an entry is being re-evaluated, this object stores the reverse deps from the previous
    /// evaluation. At the end of evaluation, the changed reverse dep operations from
    /// `reverseDepsDataToConsolidate` are merged in here.
    ///
    /// PORT: Java stores a single object unwrapped (memory format); see `ReverseDepsUtility`.
    pub(crate) reverse_deps: Vec<SkyKey>,

    /// This list stores `KeyToConsolidate` operations. Created lazily to save memory.
    ///
    /// This list serves double duty. For a done node, when a reverse dep is removed, checked for
    /// presence, or possibly added, we store the mutation in this object instead of immediately
    /// doing the operation. That is because removals/checks in reverseDeps are O(N). Originally
    /// reverseDeps was a HashSet, but because of memory consumption we switched to a list.
    ///
    /// Internally, `ReverseDepsUtility` consolidates this data periodically, and when the set of
    /// reverse deps is requested. While this operation is not free, it can be done more
    /// effectively than trying to remove/check each dirty reverse dependency individually (O(N)
    /// each time).
    ///
    /// When the node entry is evaluating, this list serves to declare the reverse dep operations
    /// that have taken place on it during this evaluation. When evaluation finishes, this list
    /// will be merged into the existing reverse deps if any, but furthermore, this list will also
    /// be used to calculate the set of reverse deps to signal when this entry finishes
    /// evaluating. That is done by `ReverseDepsUtility.consolidateDataAndReturnNewElements`.
    pub(crate) reverse_deps_data_to_consolidate: Option<Vec<KeyToConsolidate>>,
}

impl AbstractInMemoryNodeEntry for IncrementalInMemoryNodeEntry {
    type D = DirtyBuildingState;

    fn base(&self) -> &AbstractInMemoryNodeEntryFields<DirtyBuildingState> {
        &self.base
    }

    fn base_mut(&mut self) -> &mut AbstractInMemoryNodeEntryFields<DirtyBuildingState> {
        &mut self.base
    }

    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:385 IncrementalInMemoryNodeEntry.getNumTemporaryDirectDeps
    fn get_num_temporary_direct_deps(&self) -> i32 {
        match &self.direct_deps {
            None => 0,
            Some(direct_deps) => {
                // Java: getTemporaryDirectDeps().numElements().
                check_state!(!self.is_done(), "temporary shouldn't be done: {:?}", self);
                direct_deps.num_elements() as i32
            }
        }
    }

    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:370 IncrementalInMemoryNodeEntry.getTemporaryDirectDeps
    fn get_temporary_direct_deps(&mut self) -> &mut GroupedDeps {
        check_state!(!self.is_done(), "temporary shouldn't be done: {:?}", self);
        if self.direct_deps.is_none() {
            // Initialize lazily, to save a little memory.
            self.direct_deps = Some(self.new_grouped_deps());
        }
        self.direct_deps.as_mut().expect("set above")
    }
}

impl IncrementalInMemoryNodeEntry {
    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:82 IncrementalInMemoryNodeEntry.<init>
    pub(crate) fn new(key: SkyKey) -> IncrementalInMemoryNodeEntry {
        IncrementalInMemoryNodeEntry {
            base: AbstractInMemoryNodeEntryFields::new(key),
            version: NodeVersion::Version(Version::minimal()),
            direct_deps: None,
            reverse_deps: Vec::new(),
            reverse_deps_data_to_consolidate: None,
        }
    }

    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:87 IncrementalInMemoryNodeEntry.keepsEdges
    pub(crate) fn keeps_edges(&self) -> bool {
        true
    }

    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:92 IncrementalInMemoryNodeEntry.getDirectDeps
    pub(crate) fn get_direct_deps(&self) -> Vec<SkyKey> {
        self.get_compressed_direct_deps_for_done_entry()
            .get_all_elements_as_iterable()
            .cloned()
            .collect()
    }

    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:97 IncrementalInMemoryNodeEntry.hasAtLeastOneDep
    pub(crate) fn has_at_least_one_dep(&self) -> bool {
        !self.get_compressed_direct_deps_for_done_entry().is_empty()
    }

    /// PORT: returns the deps uncompressed (see `direct_deps`).
    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:102 IncrementalInMemoryNodeEntry.getCompressedDirectDepsForDoneEntry
    pub(crate) fn get_compressed_direct_deps_for_done_entry(&self) -> &GroupedDeps {
        check_state!(self.is_done(), "no deps until done. NodeEntry: {:?}", self);
        check_not_null!(self.direct_deps.as_ref(), "deps can't be null: {:?}", self)
    }

    /// Puts entry in "done" state, as checked by `isDone`. Subclasses that override one may need
    /// to override the other.
    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:113 IncrementalInMemoryNodeEntry.markDone
    fn mark_done(&mut self) {
        self.base.dirty_building_state = None;
    }

    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:117 IncrementalInMemoryNodeEntry.setStateFinishedAndReturnReverseDepsToSignal
    fn set_state_finished_and_return_reverse_deps_to_signal(&mut self) -> IndexSet<SkyKey> {
        let reverse_deps_to_signal =
            ReverseDepsUtility::consolidate_data_and_return_new_elements(self);
        // Java: directDeps = getTemporaryDirectDeps().compress().
        self.get_temporary_direct_deps();
        self.mark_done();
        reverse_deps_to_signal
    }

    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:125 IncrementalInMemoryNodeEntry.getInProgressReverseDeps
    pub(crate) fn get_in_progress_reverse_deps(&self) -> IndexSet<SkyKey> {
        check_state!(!self.is_done(), "{:?}", self);
        ReverseDepsUtility::return_new_elements(self)
    }

    /// In this method it is crucial that `version` is set prior to `value` because although this
    /// method itself is synchronized, there are unsynchronized consumers of the version and the
    /// value.
    ///
    /// PORT: `max_transitive_source_version` is ignored, as in Java.
    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:139 IncrementalInMemoryNodeEntry.setValue
    pub(crate) fn set_value(
        &mut self,
        value: ValueWithMetadata,
        graph_version: Version,
        _max_transitive_source_version: Option<Version>,
    ) -> IndexSet<SkyKey> {
        check_state!(
            !self.has_unsignaled_deps(),
            "Has unsignaled deps (this={:?}, value={:?})",
            self,
            value
        );
        check_state!(
            self.version.last_changed().at_most(&graph_version)
                && self.version.last_evaluated().at_most(&graph_version),
            "Bad version (this={:?}, version={:?}, value={:?})",
            self,
            graph_version,
            value
        );

        let dirty_building_state = check_not_null!(self.base.dirty_building_state.as_ref());
        if dirty_building_state.unchanged_from_last_build(&self.base.key, &value) {
            // If the value is the same as before, just use the old value. Note that we don't use
            // the new value, because preserving == equality is even better than .equals()
            // equality.
            let last_changed = *self.version.last_changed();
            self.version = NodeVersion::of(last_changed, graph_version);
            self.base.value = dirty_building_state.get_last_build_value().cloned();
        } else {
            // If this is a new value, or it has changed since the last build, set the version to
            // the current graph version.
            self.version = NodeVersion::Version(graph_version);
            self.base.value = Some(value);
        }
        self.set_state_finished_and_return_reverse_deps_to_signal()
    }

    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:167 IncrementalInMemoryNodeEntry.addReverseDepAndCheckIfDone
    pub(crate) fn add_reverse_dep_and_check_if_done(
        &mut self,
        reverse_dep: Option<SkyKey>,
    ) -> DependencyState {
        if reverse_dep.is_none() && self.is_done() {
            return DependencyState::Done;
        }

        let done = self.is_done();
        if !done && self.base.dirty_building_state.is_none() {
            self.base.dirty_building_state = Some(DirtyBuildingState::new_initial_building_state());
        }
        if let Some(reverse_dep) = reverse_dep {
            if done {
                ReverseDepsUtility::add_reverse_dep(self, reverse_dep);
            } else {
                self.append_to_reverse_dep_operations(reverse_dep, Op::Add);
            }
        }
        if done {
            return DependencyState::Done;
        }
        let dirty_building_state = self.base.dirty_building_state.as_mut().expect("set above");
        if dirty_building_state.is_evaluating() {
            return DependencyState::AlreadyEvaluating;
        }
        dirty_building_state.start_evaluating();
        DependencyState::NeedsScheduling
    }

    /// Sets `reverseDeps`. Does not alter `reverseDepsDataToConsolidate`.
    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:196 IncrementalInMemoryNodeEntry.setReverseDepsForReverseDepsUtil
    pub(crate) fn set_reverse_deps_for_reverse_deps_util(&mut self, reverse_deps: Vec<SkyKey>) {
        self.reverse_deps = reverse_deps;
    }

    /// Sets `reverseDepsDataToConsolidate`. Does not alter `reverseDeps`.
    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:201 IncrementalInMemoryNodeEntry.setReverseDepsDataToConsolidateForReverseDepsUtil
    pub(crate) fn set_reverse_deps_data_to_consolidate_for_reverse_deps_util(
        &mut self,
        data_to_consolidate: Option<Vec<KeyToConsolidate>>,
    ) {
        self.reverse_deps_data_to_consolidate = data_to_consolidate;
    }

    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:206 IncrementalInMemoryNodeEntry.getReverseDepsRawForReverseDepsUtil
    pub(crate) fn get_reverse_deps_raw_for_reverse_deps_util(&self) -> &Vec<SkyKey> {
        &self.reverse_deps
    }

    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:210 IncrementalInMemoryNodeEntry.getReverseDepsDataToConsolidateForReverseDepsUtil
    pub(crate) fn get_reverse_deps_data_to_consolidate_for_reverse_deps_util(
        &self,
    ) -> Option<&Vec<KeyToConsolidate>> {
        self.reverse_deps_data_to_consolidate.as_ref()
    }

    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:214 IncrementalInMemoryNodeEntry.appendToReverseDepOperations
    fn append_to_reverse_dep_operations(&mut self, reverse_dep: SkyKey, op: Op) {
        check_state!(
            !self.is_done(),
            "Don't append to done {:?} {:?} {:?}",
            self,
            reverse_dep,
            op
        );
        if self.reverse_deps_data_to_consolidate.is_none() {
            self.reverse_deps_data_to_consolidate = Some(Vec::new());
        }
        check_state!(
            self.is_dirty() || op != Op::Check,
            "Not dirty check {:?} {:?}",
            self,
            reverse_dep
        );
        let key_to_consolidate = KeyToConsolidate::create(reverse_dep, op, self);
        self.reverse_deps_data_to_consolidate
            .as_mut()
            .expect("set above")
            .push(key_to_consolidate);
    }

    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:224 IncrementalInMemoryNodeEntry.checkIfDoneForDirtyReverseDep
    pub(crate) fn check_if_done_for_dirty_reverse_dep(
        &mut self,
        reverse_dep: SkyKey,
    ) -> DependencyState {
        if self.is_done() {
            return DependencyState::Done;
        }
        self.append_to_reverse_dep_operations(reverse_dep, Op::Check);
        self.add_reverse_dep_and_check_if_done(None)
    }

    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:234 IncrementalInMemoryNodeEntry.removeReverseDep
    pub(crate) fn remove_reverse_dep(&mut self, reverse_dep: SkyKey) {
        if self.is_done() {
            ReverseDepsUtility::remove_reverse_dep(self, reverse_dep);
        } else {
            // Removing a reverse dep from an in-flight node is rare -- it should only happen when
            // there is a cycle or this node is about to be cleaned from the graph.
            self.append_to_reverse_dep_operations(reverse_dep, Op::Remove);
        }
    }

    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:245 IncrementalInMemoryNodeEntry.removeReverseDepsFromDoneEntryDueToDeletion
    pub(crate) fn remove_reverse_deps_from_done_entry_due_to_deletion(
        &mut self,
        deleted_keys: &HashSet<SkyKey>,
    ) {
        check_state!(self.is_done(), "{:?}", self);
        ReverseDepsUtility::remove_reverse_deps_matching(self, deleted_keys);
    }

    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:251 IncrementalInMemoryNodeEntry.getReverseDepsForDoneEntry
    pub(crate) fn get_reverse_deps_for_done_entry(&mut self) -> Vec<SkyKey> {
        check_state!(self.is_done(), "Called on not done {:?}", self);
        ReverseDepsUtility::consolidate_and_get_reverse_deps(
            self, /* check_consistency= */ true,
        )
    }

    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:257 IncrementalInMemoryNodeEntry.getAllReverseDepsForNodeBeingDeleted
    pub(crate) fn get_all_reverse_deps_for_node_being_deleted(&mut self) -> Vec<SkyKey> {
        if !self.is_done() {
            // This consolidation loses information about pending reverse deps to signal, but that
            // is unimportant since this node is being deleted.
            ReverseDepsUtility::consolidate_data_and_return_new_elements(self);
        }
        ReverseDepsUtility::consolidate_and_get_reverse_deps(
            self, /* check_consistency= */ false,
        )
    }

    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:267 IncrementalInMemoryNodeEntry.signalDep
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
            dirty_building_state.is_evaluating(),
            "{:?} {:?}",
            self,
            Nullable(child_for_debugging)
        );
        let num_temporary_direct_deps = self.get_num_temporary_direct_deps();
        self.base
            .dirty_building_state
            .as_mut()
            .expect("checked above")
            .signal_dep(
                num_temporary_direct_deps,
                &self.version,
                &child_version,
                child_for_debugging,
            );
        !self.has_unsignaled_deps()
    }

    /// Creates a `DirtyBuildingState` for the case where this node is done and is being marked
    /// dirty.
    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:280 IncrementalInMemoryNodeEntry.createDirtyBuildingStateForDoneNode
    fn create_dirty_building_state_for_done_node(
        &self,
        dirty_type: DirtyType,
        direct_deps: GroupedDeps,
        value: ValueWithMetadata,
    ) -> DirtyBuildingState {
        DirtyBuildingState::new_incremental_building_state(dirty_type, direct_deps, value)
    }

    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:287 IncrementalInMemoryNodeEntry.markDirty
    pub(crate) fn mark_dirty(&mut self, dirty_type: DirtyType) -> Option<MarkedDirtyResult> {
        if dirty_type == DirtyType::Rewind {
            // PORT: rewinding is not ported (docs/decisions.md 2026-11-19): Java's no-op on a node
            // that is not done or is in error, and the rewind of a done node
            // (`ReverseDepsUtility.consolidateData`, `MarkedDirtyResult.forRewinding`).
            blaze_util::unported!("IncrementalInMemoryNodeEntry.markDirty(DirtyType.REWIND)");
        }

        if self.is_done() {
            // Java: `if (dirtyType == DirtyType.REWIND && getErrorInfo() != null)`: rewinding,
            // handled above.
            let direct_deps = self.get_compressed_direct_deps_for_done_entry().clone();
            check_state!(
                dirty_type != DirtyType::Dirty || !direct_deps.is_empty(),
                "{:?} is being marked dirty but has no children that could have dirtied it",
                self.get_key()
            );
            let value = self.base.value.take().expect("done");
            self.base.dirty_building_state = Some(self.create_dirty_building_state_for_done_node(
                dirty_type,
                direct_deps,
                value,
            ));
            // Java: value = null (taken above).
            self.direct_deps = None;
            return Some(MarkedDirtyResult::with_reverse_deps(
                ReverseDepsUtility::consolidate_and_get_reverse_deps(
                    self, /* check_consistency= */ true,
                ),
            ));
        }

        // The caller may be simultaneously trying to mark this node dirty and changed, and the
        // dirty thread may have lost the race, but it is the caller's responsibility not to try to
        // mark this node changed twice. The end result of racing markers must be a changed node,
        // since one of the markers is trying to mark the node changed.
        check_state!(
            self.base.value.is_none(),
            "Value should have been reset already {:?}",
            self
        );
        match dirty_type {
            DirtyType::Change => {
                check_state!(
                    !self.is_changed(),
                    "Cannot mark node changed twice: {:?}",
                    self
                );
                check_not_null!(self.base.dirty_building_state.as_ref(), "{:?}", self);
                self.base
                    .dirty_building_state
                    .as_mut()
                    .expect("checked")
                    .mark_changed();
            }
            DirtyType::Dirty => {
                check_state!(
                    self.is_changed(),
                    "Cannot mark node dirty twice: {:?}",
                    self
                );
            }
            DirtyType::Rewind => {
                // Rewinding is legal at any time in the node's lifecycle, but is no-op when it is
                // not done. (Not reached: see the PORT note above.)
            }
        }
        None
    }

    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:336 IncrementalInMemoryNodeEntry.markClean
    pub(crate) fn mark_clean(&mut self) -> NodeValueAndRdepsToSignal {
        let dirty_building_state =
            check_not_null!(self.base.dirty_building_state.as_ref(), "{:?}", self);
        let last_build_value = check_not_null!(dirty_building_state.get_last_build_value()).clone();
        self.base.value = Some(last_build_value);
        check_state!(!self.has_unsignaled_deps(), "{:?}", self);
        // Java evaluates the argument getTemporaryDirectDeps() (which creates the deps lazily)
        // before depsUnchangedFromLastBuild.
        self.get_temporary_direct_deps();
        let deps_unchanged = self
            .base
            .dirty_building_state
            .as_ref()
            .expect("checked above")
            .deps_unchanged_from_last_build(self.direct_deps.as_ref().expect("created above"));
        check_state!(
            deps_unchanged,
            "Direct deps must be the same as those found last build for node to be marked clean: {:?}",
            self
        );
        check_state!(self.is_dirty(), "{:?}", self);
        check_state!(
            !self
                .base
                .dirty_building_state
                .as_ref()
                .expect("checked above")
                .is_changed(),
            "shouldn't be changed: {:?}",
            self
        );
        let r_deps_to_signal = self.set_state_finished_and_return_reverse_deps_to_signal();
        NodeValueAndRdepsToSignal::new(
            self.base.value.clone().expect("set above"),
            r_deps_to_signal,
        )
    }

    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:351 IncrementalInMemoryNodeEntry.getVersion
    pub(crate) fn get_version(&self) -> Version {
        *self.version.last_changed()
    }

    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:356 IncrementalInMemoryNodeEntry.getAllDirectDepsForIncompleteNode
    pub(crate) fn get_all_direct_deps_for_incomplete_node(&mut self) -> IndexSet<SkyKey> {
        check_state!(!self.is_done(), "{:?}", self);
        if self.base.dirty_building_state.is_none() {
            return IndexSet::new();
        }
        let mut result: IndexSet<SkyKey> = IndexSet::new();
        result.extend(
            self.get_temporary_direct_deps()
                .get_all_elements_as_iterable()
                .cloned(),
        );
        result.extend(
            self.base
                .dirty_building_state
                .as_mut()
                .expect("checked above")
                .get_all_remaining_dirty_direct_deps(/* preserve_position= */ false),
        );
        result.extend(self.get_reset_direct_deps());
        result
    }

    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:380 IncrementalInMemoryNodeEntry.forceRebuild
    pub(crate) fn force_rebuild(&mut self) {
        check_not_null!(self.base.dirty_building_state.as_ref(), "{:?}", self);
        let num_temporary_direct_deps = self.get_num_temporary_direct_deps();
        self.base
            .dirty_building_state
            .as_mut()
            .expect("checked above")
            .force_rebuild(num_temporary_direct_deps);
    }

    /// PORT: not ported (rewinding, docs/decisions.md 2026-11-19).
    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:390 IncrementalInMemoryNodeEntry.resetEvaluationFromScratch
    #[allow(dead_code)] // rewinding, Skyfocus and graph compaction are not ported
    pub(crate) fn reset_evaluation_from_scratch(&mut self) {
        blaze_util::unported!("IncrementalInMemoryNodeEntry.resetEvaluationFromScratch")
    }

    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:414 IncrementalInMemoryNodeEntry.getResetDirectDeps
    pub(crate) fn get_reset_direct_deps(&self) -> IndexSet<SkyKey> {
        check_not_null!(self.base.dirty_building_state.as_ref(), "{:?}", self)
            .get_reset_direct_deps()
    }

    /// For Skyfocus only: clears out all direct dep edges of this node. It is not safe to call
    /// this otherwise.
    ///
    /// PORT: not ported (Skyfocus, docs/decisions.md 2026-11-19).
    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:422 IncrementalInMemoryNodeEntry.clearDirectDepsForSkyfocus
    #[allow(dead_code)] // rewinding, Skyfocus and graph compaction are not ported
    pub(crate) fn clear_direct_deps_for_skyfocus(&mut self) {
        blaze_util::unported!("IncrementalInMemoryNodeEntry.clearDirectDepsForSkyfocus")
    }

    /// Flushes pending reverse dep operations, which potentially saves memory.
    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:429 IncrementalInMemoryNodeEntry.consolidateReverseDeps
    #[allow(dead_code)] // rewinding, Skyfocus and graph compaction are not ported
    pub(crate) fn consolidate_reverse_deps(&mut self) {
        check_state!(self.is_done(), "{:?}", self);
        ReverseDepsUtility::consolidate_data(self);
    }
}

impl fmt::Debug for IncrementalInMemoryNodeEntry {
    // Java: com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntry.java:436 IncrementalInMemoryNodeEntry.toStringHelper
    // Java: com/google/devtools/build/skyframe/AbstractInMemoryNodeEntry.java:258 AbstractInMemoryNodeEntry.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("IncrementalInMemoryNodeEntry{")?;
        self.fmt_to_string_helper_fields(f)?;
        write!(
            f,
            ", version={:?}, directDeps={:?}, reverseDeps={}}}",
            self.version,
            Nullable(self.direct_deps.as_ref()),
            ReverseDepsUtility::to_string(self)
        )
    }
}
