//! Port of `InMemoryNodeEntry`: the node entry of the in-memory graph, incremental or not.

use crate::abstract_in_memory_node_entry::AbstractInMemoryNodeEntry;
use crate::grouped_deps::GroupedDeps;
use crate::incremental_in_memory_node_entry::IncrementalInMemoryNodeEntry;
use crate::node_entry::{
    DependencyState, DirtyType, LifecycleState, MarkedDirtyResult, NodeEntry,
    NodeValueAndRdepsToSignal,
};
use crate::non_incremental_in_memory_node_entry::NonIncrementalInMemoryNodeEntry;
use crate::value_with_metadata::ValueWithMetadata;
use crate::version::Version;
use crate::{ErrorInfo, SkyKey, SkyValue};
use indexmap::IndexSet;
use std::collections::HashSet;
use std::fmt;

/// A `NodeEntry` that is stored in memory.
///
/// PORT: Java's interface has two implementations in the engine,
/// `IncrementalInMemoryNodeEntry` (keeps edges) and `NonIncrementalInMemoryNodeEntry`; which one
/// a graph creates is fixed by `keepEdges`. A closed set of classes is a Rust enum
/// (PORTING.md "Types"); an enum also keeps each entry inline in the graph's `Mutex` without a
/// box, and its match is the same dispatch as Java's virtual call. Construct with
/// [`InMemoryNodeEntry::new`].
// Java: com/google/devtools/build/skyframe/InMemoryNodeEntry.java:24 InMemoryNodeEntry
pub(crate) enum InMemoryNodeEntry {
    Incremental(IncrementalInMemoryNodeEntry),
    NonIncremental(NonIncrementalInMemoryNodeEntry),
}

macro_rules! dispatch {
    ($self:expr, $e:ident => $body:expr) => {
        match $self {
            InMemoryNodeEntry::Incremental($e) => $body,
            InMemoryNodeEntry::NonIncremental($e) => $body,
        }
    };
}

impl InMemoryNodeEntry {
    /// The entry the in-memory graph creates for `key`: an `IncrementalInMemoryNodeEntry` when
    /// the graph keeps edges, a `NonIncrementalInMemoryNodeEntry` otherwise.
    // Java: com/google/devtools/build/skyframe/InMemoryGraphImpl.java:195 InMemoryGraphImpl.newNodeEntry
    // Java: com/google/devtools/build/skyframe/InMemoryGraphImpl.java:316 EdgelessInMemoryGraphImpl.newNodeEntry
    pub(crate) fn new(key: SkyKey, keep_edges: bool) -> InMemoryNodeEntry {
        if keep_edges {
            InMemoryNodeEntry::Incremental(IncrementalInMemoryNodeEntry::new(key))
        } else {
            InMemoryNodeEntry::NonIncremental(NonIncrementalInMemoryNodeEntry::new(key))
        }
    }

    /// Returns the `SkyKey` associated with this node.
    // Java: com/google/devtools/build/skyframe/InMemoryNodeEntry.java:27 InMemoryNodeEntry.getKey
    #[allow(dead_code)] // ported API, used by tests
    pub(crate) fn get_key(&self) -> &SkyKey {
        dispatch!(self, e => e.get_key())
    }

    /// Whether this node stores edges (deps and rdeps).
    // Java: com/google/devtools/build/skyframe/InMemoryNodeEntry.java:30 InMemoryNodeEntry.keepsEdges
    #[allow(dead_code)] // ported API, used by tests
    pub(crate) fn keeps_edges(&self) -> bool {
        dispatch!(self, e => e.keeps_edges())
    }

    /// Returns the direct deps of a done entry. Can only be called if this node `isDone` and
    /// `keepsEdges`.
    ///
    /// PORT: Java returns them compressed; here they are the `GroupedDeps` (memory format).
    // Java: com/google/devtools/build/skyframe/InMemoryNodeEntry.java:37 InMemoryNodeEntry.getCompressedDirectDepsForDoneEntry
    #[allow(dead_code)] // ported API, used by tests
    pub(crate) fn get_compressed_direct_deps_for_done_entry(&self) -> &GroupedDeps {
        dispatch!(self, e => e.get_compressed_direct_deps_for_done_entry())
    }
}

impl NodeEntry for InMemoryNodeEntry {
    fn is_done(&self) -> bool {
        dispatch!(self, e => e.is_done())
    }

    fn is_dirty(&self) -> bool {
        dispatch!(self, e => e.is_dirty())
    }

    fn is_changed(&self) -> bool {
        dispatch!(self, e => e.is_changed())
    }

    fn mark_dirty(&mut self, dirty_type: DirtyType) -> Option<MarkedDirtyResult> {
        dispatch!(self, e => e.mark_dirty(dirty_type))
    }

    fn get_value(&self) -> Option<SkyValue> {
        dispatch!(self, e => e.get_value())
    }

    fn get_direct_deps(&self) -> Vec<SkyKey> {
        dispatch!(self, e => e.get_direct_deps())
    }

    fn has_at_least_one_dep(&self) -> bool {
        dispatch!(self, e => e.has_at_least_one_dep())
    }

    fn remove_reverse_dep(&mut self, reverse_dep: SkyKey) {
        dispatch!(self, e => e.remove_reverse_dep(reverse_dep))
    }

    fn remove_reverse_deps_from_done_entry_due_to_deletion(
        &mut self,
        deleted_keys: &HashSet<SkyKey>,
    ) {
        dispatch!(self, e => e.remove_reverse_deps_from_done_entry_due_to_deletion(deleted_keys))
    }

    fn get_reverse_deps_for_done_entry(&mut self) -> Vec<SkyKey> {
        dispatch!(self, e => e.get_reverse_deps_for_done_entry())
    }

    fn get_value_maybe_with_metadata(&self) -> Option<&ValueWithMetadata> {
        dispatch!(self, e => e.get_value_maybe_with_metadata())
    }

    fn to_value(&self) -> Option<SkyValue> {
        dispatch!(self, e => e.to_value())
    }

    fn get_error_info(&self) -> Option<&ErrorInfo> {
        dispatch!(self, e => e.get_error_info())
    }

    fn get_in_progress_reverse_deps(&self) -> IndexSet<SkyKey> {
        dispatch!(self, e => e.get_in_progress_reverse_deps())
    }

    fn set_value(
        &mut self,
        value: ValueWithMetadata,
        graph_version: Version,
        max_transitive_source_version: Option<Version>,
    ) -> IndexSet<SkyKey> {
        dispatch!(self, e => e.set_value(value, graph_version, max_transitive_source_version))
    }

    fn add_reverse_dep_and_check_if_done(
        &mut self,
        reverse_dep: Option<SkyKey>,
    ) -> DependencyState {
        dispatch!(self, e => e.add_reverse_dep_and_check_if_done(reverse_dep))
    }

    fn check_if_done_for_dirty_reverse_dep(&mut self, reverse_dep: SkyKey) -> DependencyState {
        dispatch!(self, e => e.check_if_done_for_dirty_reverse_dep(reverse_dep))
    }

    fn get_all_reverse_deps_for_node_being_deleted(&mut self) -> Vec<SkyKey> {
        dispatch!(self, e => e.get_all_reverse_deps_for_node_being_deleted())
    }

    fn signal_dep(&mut self, child_version: Version, child_for_debugging: Option<&SkyKey>) -> bool {
        dispatch!(self, e => e.signal_dep(child_version, child_for_debugging))
    }

    fn mark_clean(&mut self) -> NodeValueAndRdepsToSignal {
        dispatch!(self, e => e.mark_clean())
    }

    fn force_rebuild(&mut self) {
        dispatch!(self, e => e.force_rebuild())
    }

    fn get_version(&self) -> Version {
        dispatch!(self, e => e.get_version())
    }

    fn get_lifecycle_state(&self) -> LifecycleState {
        dispatch!(self, e => e.get_lifecycle_state())
    }

    fn get_next_dirty_direct_deps(&mut self) -> Vec<SkyKey> {
        dispatch!(self, e => e.get_next_dirty_direct_deps())
    }

    fn get_all_direct_deps_for_incomplete_node(&mut self) -> IndexSet<SkyKey> {
        dispatch!(self, e => e.get_all_direct_deps_for_incomplete_node())
    }

    fn get_all_remaining_dirty_direct_deps(&mut self) -> IndexSet<SkyKey> {
        dispatch!(self, e => e.get_all_remaining_dirty_direct_deps())
    }

    fn mark_rebuilding(&mut self) {
        dispatch!(self, e => e.mark_rebuilding())
    }

    fn get_temporary_direct_deps(&mut self) -> &mut GroupedDeps {
        dispatch!(self, e => e.get_temporary_direct_deps())
    }

    fn no_deps_last_build(&self) -> bool {
        dispatch!(self, e => e.no_deps_last_build())
    }

    fn remove_unfinished_deps(&mut self, unfinished_deps: &HashSet<SkyKey>) {
        dispatch!(self, e => e.remove_unfinished_deps(unfinished_deps))
    }

    fn reset_evaluation_from_scratch(&mut self) {
        dispatch!(self, e => e.reset_evaluation_from_scratch())
    }

    fn get_reset_direct_deps(&self) -> IndexSet<SkyKey> {
        dispatch!(self, e => e.get_reset_direct_deps())
    }

    fn add_singleton_temporary_direct_dep(&mut self, dep: SkyKey) {
        dispatch!(self, e => e.add_singleton_temporary_direct_dep(dep))
    }

    fn add_temporary_direct_dep_group(&mut self, group: Vec<SkyKey>) {
        dispatch!(self, e => e.add_temporary_direct_dep_group(group))
    }

    fn add_temporary_direct_deps_in_groups(
        &mut self,
        deps: &IndexSet<SkyKey>,
        group_sizes: &[i32],
    ) {
        dispatch!(self, e => e.add_temporary_direct_deps_in_groups(deps, group_sizes))
    }

    fn add_external_dep(&mut self) {
        dispatch!(self, e => e.add_external_dep())
    }

    fn is_ready_to_evaluate(&self) -> bool {
        dispatch!(self, e => e.is_ready_to_evaluate())
    }

    fn has_unsignaled_deps(&self) -> bool {
        dispatch!(self, e => e.has_unsignaled_deps())
    }
}

impl fmt::Debug for InMemoryNodeEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        dispatch!(self, e => fmt::Debug::fmt(e, f))
    }
}
