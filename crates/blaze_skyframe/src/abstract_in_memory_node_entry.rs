//! Port of `AbstractInMemoryNodeEntry`.

use crate::dirty_building_state::DirtyBuildingState;
use crate::grouped_deps::GroupedDeps;
use crate::node_entry::LifecycleState;
use crate::value_with_metadata::ValueWithMetadata;
use crate::{ErrorInfo, SkyKey, SkyValue};
use blaze_util::check_state;
use indexmap::IndexSet;
use std::collections::HashSet;
use std::fmt;

/// The `DirtyBuildingState` part of an entry's building state (Java: `D extends
/// DirtyBuildingState`).
pub(crate) trait AsDirtyBuildingState: fmt::Debug + Send {
    fn dirty_building_state(&self) -> &DirtyBuildingState;
    fn dirty_building_state_mut(&mut self) -> &mut DirtyBuildingState;
}

impl AsDirtyBuildingState for DirtyBuildingState {
    fn dirty_building_state(&self) -> &DirtyBuildingState {
        self
    }
    fn dirty_building_state_mut(&mut self) -> &mut DirtyBuildingState {
        self
    }
}

/// The fields of `AbstractInMemoryNodeEntry`.
///
/// PORT: the abstract class is split into this struct (its fields, embedded by each subclass as
/// `base`) and the [`AbstractInMemoryNodeEntry`] trait (its methods, which call the subclass's
/// `getNumTemporaryDirectDeps` and `getTemporaryDirectDeps`).
// Java: com/google/devtools/build/skyframe/AbstractInMemoryNodeEntry.java:65 AbstractInMemoryNodeEntry
pub(crate) struct AbstractInMemoryNodeEntryFields<D> {
    pub(crate) key: SkyKey,
    /// Actual data stored in this entry when it is done.
    pub(crate) value: Option<ValueWithMetadata>,
    /// Tracks state of this entry while it is evaluating (either on its initial build or after
    /// being marked dirty).
    pub(crate) dirty_building_state: Option<D>,
}

impl<D> AbstractInMemoryNodeEntryFields<D> {
    // Java: com/google/devtools/build/skyframe/AbstractInMemoryNodeEntry.java:79 AbstractInMemoryNodeEntry.<init>
    pub(crate) fn new(key: SkyKey) -> Self {
        AbstractInMemoryNodeEntryFields {
            key,
            value: None,
            dirty_building_state: None,
        }
    }
}

/// Partial implementation of `InMemoryNodeEntry` containing behavior common to both
/// `IncrementalInMemoryNodeEntry` and `NonIncrementalInMemoryNodeEntry`.
///
/// Care was taken to provide certain compound operations to avoid certain check-then-act races.
/// That means this class is somewhat closely tied to the exact Evaluator implementation.
///
/// Consider the example with two threads working on two nodes, where one depends on the other,
/// say b depends on a. If a completes first, it's done. If it completes second, it needs to
/// signal b, and potentially re-schedule it. If b completes first, it must exit, because it will
/// be signaled (and re-scheduled) by a. If it completes second, it must signal (and re-schedule)
/// itself. However, if the Evaluator supported re-entrancy for a node, then this wouldn't have to
/// be so strict, because duplicate scheduling would be less problematic.
///
/// During its life, a node can go through states as follows:
///
/// 1. Non-existent
/// 2. Just created or marked as affected (`isDone` is false; `isDirty` is false)
/// 3. Evaluating (`isDone` is false; `isDirty` is true)
/// 4. Done (`isDone` is true; `isDirty` is false)
///
/// The "just created" state is there to allow the `ProcessableGraph.createIfAbsentBatch` and
/// `NodeEntry.addReverseDepAndCheckIfDone` methods to be separate. All callers have to call both
/// methods in that order if they want to create a node. The second method returns the
/// NEEDS_SCHEDULING state only on the first time it was called. A caller that gets
/// NEEDS_SCHEDULING back from that call must start the evaluation of this node, while any
/// subsequent callers must not.
///
/// An entry is set to ALREADY_EVALUATING as soon as it is scheduled for evaluation. Thus, even a
/// node that is never actually built (for instance, a dirty node that is verified as clean) is in
/// the ALREADY_EVALUATING state until it is DONE.
///
/// From the DONE state, the node can go back to the "marked as affected" state.
///
/// PORT: `synchronized` is the caller's lock (the graph's `Mutex`).
// Java: com/google/devtools/build/skyframe/AbstractInMemoryNodeEntry.java:65 AbstractInMemoryNodeEntry
pub(crate) trait AbstractInMemoryNodeEntry: fmt::Debug {
    type D: AsDirtyBuildingState;

    fn base(&self) -> &AbstractInMemoryNodeEntryFields<Self::D>;
    fn base_mut(&mut self) -> &mut AbstractInMemoryNodeEntryFields<Self::D>;

    // Java: com/google/devtools/build/skyframe/AbstractInMemoryNodeEntry.java:221 AbstractInMemoryNodeEntry.getNumTemporaryDirectDeps
    fn get_num_temporary_direct_deps(&self) -> i32;

    // Java: com/google/devtools/build/skyframe/NodeEntry.java:561 NodeEntry.getTemporaryDirectDeps
    fn get_temporary_direct_deps(&mut self) -> &mut GroupedDeps;

    /// The `DirtyBuildingState` of the building state, if any.
    fn dbs(&self) -> Option<&DirtyBuildingState> {
        self.base()
            .dirty_building_state
            .as_ref()
            .map(AsDirtyBuildingState::dirty_building_state)
    }

    fn dbs_mut(&mut self) -> Option<&mut DirtyBuildingState> {
        self.base_mut()
            .dirty_building_state
            .as_mut()
            .map(AsDirtyBuildingState::dirty_building_state_mut)
    }

    // Java: com/google/devtools/build/skyframe/AbstractInMemoryNodeEntry.java:84 AbstractInMemoryNodeEntry.getKey
    fn get_key(&self) -> &SkyKey {
        &self.base().key
    }

    // Java: com/google/devtools/build/skyframe/AbstractInMemoryNodeEntry.java:88 AbstractInMemoryNodeEntry.isEvaluating
    fn is_evaluating(&self) -> bool {
        self.base().dirty_building_state.is_some()
    }

    // Java: com/google/devtools/build/skyframe/AbstractInMemoryNodeEntry.java:93 AbstractInMemoryNodeEntry.isDone
    fn is_done(&self) -> bool {
        self.base().value.is_some() && self.base().dirty_building_state.is_none()
    }

    /// PORT: Java's `|| key.supportsPartialReevaluation()` is always false here: Rust keys have
    /// no `supportsPartialReevaluation` (partial reevaluation is not ported, docs/decisions.md
    /// 2026-11-19), which is Java's default.
    // Java: com/google/devtools/build/skyframe/AbstractInMemoryNodeEntry.java:98 AbstractInMemoryNodeEntry.isReadyToEvaluate
    fn is_ready_to_evaluate(&self) -> bool {
        !self.is_done()
            && self.is_evaluating()
            && self
                .dbs()
                .expect("evaluating")
                .is_ready(self.get_num_temporary_direct_deps())
    }

    // Java: com/google/devtools/build/skyframe/AbstractInMemoryNodeEntry.java:106 AbstractInMemoryNodeEntry.hasUnsignaledDeps
    fn has_unsignaled_deps(&self) -> bool {
        check_state!(!self.is_done(), "{:?}", self);
        check_state!(self.is_evaluating(), "{:?}", self);
        !self
            .dbs()
            .expect("evaluating")
            .is_ready(self.get_num_temporary_direct_deps())
    }

    // Java: com/google/devtools/build/skyframe/AbstractInMemoryNodeEntry.java:113 AbstractInMemoryNodeEntry.isDirty
    fn is_dirty(&self) -> bool {
        !self.is_done()
    }

    // Java: com/google/devtools/build/skyframe/AbstractInMemoryNodeEntry.java:118 AbstractInMemoryNodeEntry.isChanged
    fn is_changed(&self) -> bool {
        !self.is_done() && self.dbs().is_some_and(DirtyBuildingState::is_changed)
    }

    // Java: com/google/devtools/build/skyframe/AbstractInMemoryNodeEntry.java:124 AbstractInMemoryNodeEntry.getValue
    fn get_value(&self) -> Option<SkyValue> {
        check_state!(
            self.is_done(),
            "no value until done. ValueEntry: {:?}",
            self
        );
        ValueWithMetadata::just_value(self.base().value.as_ref().expect("done"))
    }

    // Java: com/google/devtools/build/skyframe/AbstractInMemoryNodeEntry.java:131 AbstractInMemoryNodeEntry.getValueMaybeWithMetadata
    fn get_value_maybe_with_metadata(&self) -> Option<&ValueWithMetadata> {
        self.base().value.as_ref()
    }

    // Java: com/google/devtools/build/skyframe/AbstractInMemoryNodeEntry.java:137 AbstractInMemoryNodeEntry.toValue
    fn to_value(&self) -> Option<SkyValue> {
        let last_build_value: Option<&ValueWithMetadata> = if let Some(value) = &self.base().value {
            Some(value)
        } else {
            // `?`: an evaluation was never started.
            self.dbs()?.get_last_build_value()
        };
        last_build_value.and_then(ValueWithMetadata::just_value)
    }

    // Java: com/google/devtools/build/skyframe/AbstractInMemoryNodeEntry.java:160 AbstractInMemoryNodeEntry.getErrorInfo
    fn get_error_info(&self) -> Option<&ErrorInfo> {
        check_state!(
            self.is_done(),
            "no errors until done. NodeEntry: {:?}",
            self
        );
        ValueWithMetadata::get_maybe_error_info(self.base().value.as_ref().expect("done"))
    }

    /// PORT: not ported (external deps, `Environment.dependOnFuture`).
    // Java: com/google/devtools/build/skyframe/AbstractInMemoryNodeEntry.java:166 AbstractInMemoryNodeEntry.addExternalDep
    #[allow(dead_code)] // ported API; external deps and the restart path are not used by the blocking engine
    fn add_external_dep(&mut self) {
        blaze_util::unported!("AbstractInMemoryNodeEntry.addExternalDep")
    }

    // Java: com/google/devtools/build/skyframe/AbstractInMemoryNodeEntry.java:172 AbstractInMemoryNodeEntry.getLifecycleState
    fn get_lifecycle_state(&self) -> LifecycleState {
        if self.is_done() {
            LifecycleState::Done
        } else if let Some(dirty_building_state) = self.dbs() {
            dirty_building_state.get_lifecycle_state()
        } else {
            LifecycleState::NotYetEvaluating
        }
    }

    // Java: com/google/devtools/build/skyframe/AbstractInMemoryNodeEntry.java:183 AbstractInMemoryNodeEntry.getNextDirtyDirectDeps
    fn get_next_dirty_direct_deps(&mut self) -> Vec<SkyKey> {
        check_state!(!self.has_unsignaled_deps(), "{:?}", self);
        let dirty_building_state = check_not_null!(self.dbs(), "{:?}", self);
        check_state!(
            dirty_building_state.is_evaluating(),
            "Not evaluating during getNextDirty? {:?}",
            self
        );
        self.dbs_mut()
            .expect("checked")
            .get_next_dirty_direct_deps()
    }

    // Java: com/google/devtools/build/skyframe/AbstractInMemoryNodeEntry.java:191 AbstractInMemoryNodeEntry.getAllRemainingDirtyDirectDeps
    fn get_all_remaining_dirty_direct_deps(&mut self) -> IndexSet<SkyKey> {
        let dirty_building_state = check_not_null!(self.dbs(), "{:?}", self);
        check_state!(
            dirty_building_state.is_evaluating(),
            "Not evaluating for remaining dirty? {:?}",
            self
        );
        if self.is_dirty() {
            check_state!(
                dirty_building_state.get_lifecycle_state() == LifecycleState::Rebuilding,
                "{:?}",
                self
            );
            self.dbs_mut()
                .expect("checked")
                .get_all_remaining_dirty_direct_deps(/* preserve_position= */ true)
        } else {
            IndexSet::new()
        }
    }

    // Java: com/google/devtools/build/skyframe/AbstractInMemoryNodeEntry.java:204 AbstractInMemoryNodeEntry.markRebuilding
    fn mark_rebuilding(&mut self) {
        check_not_null!(self.dbs(), "{:?}", self);
        self.dbs_mut().expect("checked").mark_rebuilding();
    }

    /// PORT: always a plain `GroupedDeps`. Java uses `GroupedDeps.WithHashSet` when
    /// `key.skipsBatchPrefetch()`; Rust keys have no `skipsBatchPrefetch` (Java's default is
    /// false) and `WithHashSet` belongs to partial reevaluation, which is not ported.
    // Java: com/google/devtools/build/skyframe/AbstractInMemoryNodeEntry.java:208 AbstractInMemoryNodeEntry.newGroupedDeps
    fn new_grouped_deps(&self) -> GroupedDeps {
        GroupedDeps::new()
    }

    // Java: com/google/devtools/build/skyframe/AbstractInMemoryNodeEntry.java:224 AbstractInMemoryNodeEntry.noDepsLastBuild
    fn no_deps_last_build(&self) -> bool {
        check_state!(self.is_evaluating(), "{:?}", self);
        self.dbs().expect("evaluating").no_deps_last_build()
    }

    // Java: com/google/devtools/build/skyframe/AbstractInMemoryNodeEntry.java:230 AbstractInMemoryNodeEntry.removeUnfinishedDeps
    fn remove_unfinished_deps(&mut self, unfinished_deps: &HashSet<SkyKey>) {
        self.get_temporary_direct_deps().remove(unfinished_deps);
    }

    // Java: com/google/devtools/build/skyframe/AbstractInMemoryNodeEntry.java:235 AbstractInMemoryNodeEntry.addSingletonTemporaryDirectDep
    fn add_singleton_temporary_direct_dep(&mut self, dep: SkyKey) {
        self.get_temporary_direct_deps().append_singleton(dep);
    }

    // Java: com/google/devtools/build/skyframe/AbstractInMemoryNodeEntry.java:240 AbstractInMemoryNodeEntry.addTemporaryDirectDepGroup
    fn add_temporary_direct_dep_group(&mut self, group: Vec<SkyKey>) {
        self.get_temporary_direct_deps().append_group(group);
    }

    // Java: com/google/devtools/build/skyframe/AbstractInMemoryNodeEntry.java:245 AbstractInMemoryNodeEntry.addTemporaryDirectDepsInGroups
    #[allow(dead_code)] // ported API; external deps and the restart path are not used by the blocking engine
    fn add_temporary_direct_deps_in_groups(
        &mut self,
        deps: &IndexSet<SkyKey>,
        group_sizes: &[i32],
    ) {
        self.get_temporary_direct_deps()
            .append_groups(deps, group_sizes);
    }

    /// The fields of `toStringHelper` ("key=..., value=..., dirtyBuildingState=...").
    // Java: com/google/devtools/build/skyframe/AbstractInMemoryNodeEntry.java:250 AbstractInMemoryNodeEntry.toStringHelper
    fn fmt_to_string_helper_fields(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let base = self.base();
        write!(f, "key={:?}, value=", base.key)?;
        match &base.value {
            Some(value) => write!(f, "{value:?}")?,
            None => f.write_str("null")?,
        }
        f.write_str(", dirtyBuildingState=")?;
        match &base.dirty_building_state {
            Some(dirty_building_state) => write!(f, "{dirty_building_state:?}"),
            None => f.write_str("null"),
        }
    }
}

/// Java's `String.valueOf` of a `@Nullable` reference in a message: `null` or the value.
pub(crate) struct Nullable<'a, T>(pub(crate) Option<&'a T>);

impl<T: fmt::Debug> fmt::Debug for Nullable<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(v) => fmt::Debug::fmt(v, f),
            None => f.write_str("null"),
        }
    }
}
