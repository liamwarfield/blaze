//! Port of `IncrementalInMemoryNodeEntryTest`
//! (src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java): the
//! shared `InMemoryNodeEntryTest` cases run on an `IncrementalInMemoryNodeEntry` at
//! `IntVersion.of(0)`, then the class's own cases.
//!
//! Java runs every case with `isPartialReevaluation` false and true; the `true` runs are ignored
//! stubs (`<name>_partial_reevaluation`).

use super::in_memory_node_entry_test::{
    self as shared, Kind, as_integer, assert_contains_exactly, assert_throws_illegal_state,
    error_info, integer_value, key, new_sky_value, set_value, temporary_direct_deps,
};
use super::*;
use crate::in_memory_node_entry::InMemoryNodeEntry;
use crate::node_entry::{DependencyState, DirtyType, LifecycleState, NodeEntry};
use crate::value_with_metadata::ValueWithMetadata;
use crate::version::Version;

// Java: IncrementalInMemoryNodeEntryTest.initialVersion
fn initial_version() -> Version {
    Version::Int(IntVersion::of(0))
}

// Java: IncrementalInMemoryNodeEntryTest.incrementalVersion (initialVersion.next())
fn incremental_version() -> Version {
    Version::Int(IntVersion::of(0).next())
}

fn create_entry() -> InMemoryNodeEntry {
    Kind::incremental().create_entry()
}

// ---------------------------------------------------------------------------------------------
// InMemoryNodeEntryTest, run on IncrementalInMemoryNodeEntry.

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:83 InMemoryNodeEntryTest.entryAtStartOfEvaluation
#[test]
fn entry_at_start_of_evaluation_incremental() {
    shared::entry_at_start_of_evaluation(&Kind::incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:83 InMemoryNodeEntryTest.entryAtStartOfEvaluation
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn entry_at_start_of_evaluation_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.entryAtStartOfEvaluation")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:101 InMemoryNodeEntryTest.signalEntry
#[test]
fn signal_entry_incremental() {
    shared::signal_entry(&Kind::incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:101 InMemoryNodeEntryTest.signalEntry
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn signal_entry_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.signalEntry")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:136 InMemoryNodeEntryTest.signalExternalDep
#[test]
#[ignore = "external deps (NodeEntry.addExternalDep, Environment.dependOnFuture) are not ported"]
fn signal_external_dep_incremental() {
    unimplemented!("not ported: InMemoryNodeEntryTest.signalExternalDep")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:136 InMemoryNodeEntryTest.signalExternalDep
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn signal_external_dep_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.signalExternalDep")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:156 InMemoryNodeEntryTest.reverseDeps
#[test]
fn reverse_deps_incremental() {
    shared::reverse_deps(&Kind::incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:156 InMemoryNodeEntryTest.reverseDeps
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn reverse_deps_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.reverseDeps")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:179 InMemoryNodeEntryTest.errorValue
#[test]
fn error_value_incremental() {
    shared::error_value(&Kind::incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:179 InMemoryNodeEntryTest.errorValue
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn error_value_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.errorValue")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:196 InMemoryNodeEntryTest.errorAndValue
#[test]
fn error_and_value_incremental() {
    shared::error_and_value(&Kind::incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:196 InMemoryNodeEntryTest.errorAndValue
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn error_and_value_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.errorAndValue")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:211 InMemoryNodeEntryTest.crashOnNullErrorAndValue
#[test]
fn crash_on_null_error_and_value_incremental() {
    shared::crash_on_null_error_and_value(&Kind::incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:211 InMemoryNodeEntryTest.crashOnNullErrorAndValue
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn crash_on_null_error_and_value_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.crashOnNullErrorAndValue")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:221 InMemoryNodeEntryTest.crashOnTooManySignals
#[test]
fn crash_on_too_many_signals_incremental() {
    shared::crash_on_too_many_signals(&Kind::incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:221 InMemoryNodeEntryTest.crashOnTooManySignals
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn crash_on_too_many_signals_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.crashOnTooManySignals")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:229 InMemoryNodeEntryTest.crashOnSetValueWhenDone
#[test]
fn crash_on_set_value_when_done_incremental() {
    shared::crash_on_set_value_when_done(&Kind::incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:229 InMemoryNodeEntryTest.crashOnSetValueWhenDone
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn crash_on_set_value_when_done_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.crashOnSetValueWhenDone")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:241 InMemoryNodeEntryTest.crashOnAddReverseDepTwice
#[test]
fn crash_on_add_reverse_dep_twice_incremental() {
    shared::crash_on_add_reverse_dep_twice(&Kind::incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:241 InMemoryNodeEntryTest.crashOnAddReverseDepTwice
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn crash_on_add_reverse_dep_twice_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.crashOnAddReverseDepTwice")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:280 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups
#[test]
fn add_temporary_direct_deps_in_groups_incremental() {
    shared::add_temporary_direct_deps_in_groups(&Kind::incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:280 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn add_temporary_direct_deps_in_groups_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:298 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_toleratesEmpty
#[test]
fn add_temporary_direct_deps_in_groups_tolerates_empty_incremental() {
    shared::add_temporary_direct_deps_in_groups_tolerates_empty(&Kind::incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:298 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_toleratesEmpty
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn add_temporary_direct_deps_in_groups_tolerates_empty_incremental_partial_reevaluation() {
    unimplemented!(
        "not ported: InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_toleratesEmpty"
    )
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:306 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_toleratesGroupSizeOfZero
#[test]
fn add_temporary_direct_deps_in_groups_tolerates_group_size_of_zero_incremental() {
    shared::add_temporary_direct_deps_in_groups_tolerates_group_size_of_zero(&Kind::incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:306 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_toleratesGroupSizeOfZero
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn add_temporary_direct_deps_in_groups_tolerates_group_size_of_zero_incremental_partial_reevaluation()
 {
    unimplemented!(
        "not ported: InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_toleratesGroupSizeOfZero"
    )
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:314 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_notEnoughGroups_throws
#[test]
fn add_temporary_direct_deps_in_groups_not_enough_groups_throws_incremental() {
    shared::add_temporary_direct_deps_in_groups_not_enough_groups_throws(&Kind::incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:314 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_notEnoughGroups_throws
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn add_temporary_direct_deps_in_groups_not_enough_groups_throws_incremental_partial_reevaluation() {
    unimplemented!(
        "not ported: InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_notEnoughGroups_throws"
    )
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:324 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_tooManyGroups_throws
#[test]
fn add_temporary_direct_deps_in_groups_too_many_groups_throws_incremental() {
    shared::add_temporary_direct_deps_in_groups_too_many_groups_throws(&Kind::incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:324 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_tooManyGroups_throws
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn add_temporary_direct_deps_in_groups_too_many_groups_throws_incremental_partial_reevaluation() {
    unimplemented!(
        "not ported: InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_tooManyGroups_throws"
    )
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:332 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_depsLeftOver_throws
#[test]
fn add_temporary_direct_deps_in_groups_deps_left_over_throws_incremental() {
    shared::add_temporary_direct_deps_in_groups_deps_left_over_throws(&Kind::incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:332 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_depsLeftOver_throws
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn add_temporary_direct_deps_in_groups_deps_left_over_throws_incremental_partial_reevaluation() {
    unimplemented!(
        "not ported: InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_depsLeftOver_throws"
    )
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:343 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_depsExhausted_throws
#[test]
fn add_temporary_direct_deps_in_groups_deps_exhausted_throws_incremental() {
    shared::add_temporary_direct_deps_in_groups_deps_exhausted_throws(&Kind::incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:343 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_depsExhausted_throws
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn add_temporary_direct_deps_in_groups_deps_exhausted_throws_incremental_partial_reevaluation() {
    unimplemented!(
        "not ported: InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_depsExhausted_throws"
    )
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:354 InMemoryNodeEntryTest.resetLifecycle
#[test]
fn reset_lifecycle_incremental() {
    shared::reset_lifecycle(&Kind::incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:354 InMemoryNodeEntryTest.resetLifecycle
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn reset_lifecycle_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.resetLifecycle")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:399 InMemoryNodeEntryTest.resetTwice_moreDepsRequestedBeforeFirstReset
#[test]
fn reset_twice_more_deps_requested_before_first_reset_incremental() {
    shared::reset_twice_more_deps_requested_before_first_reset(&Kind::incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:399 InMemoryNodeEntryTest.resetTwice_moreDepsRequestedBeforeFirstReset
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn reset_twice_more_deps_requested_before_first_reset_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.resetTwice_moreDepsRequestedBeforeFirstReset")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:443 InMemoryNodeEntryTest.rewindingLifecycle
#[test]
fn rewinding_lifecycle_incremental() {
    shared::rewinding_lifecycle(&Kind::incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:443 InMemoryNodeEntryTest.rewindingLifecycle
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn rewinding_lifecycle_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.rewindingLifecycle")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:491 InMemoryNodeEntryTest.resetAfterRewind
#[test]
fn reset_after_rewind_incremental() {
    shared::reset_after_rewind(&Kind::incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:491 InMemoryNodeEntryTest.resetAfterRewind
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn reset_after_rewind_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.resetAfterRewind")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:555 InMemoryNodeEntryTest.concurrentRewindingAllowed
#[test]
fn concurrent_rewinding_allowed_incremental() {
    shared::concurrent_rewinding_allowed(&Kind::incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:555 InMemoryNodeEntryTest.concurrentRewindingAllowed
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn concurrent_rewinding_allowed_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.concurrentRewindingAllowed")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:571 InMemoryNodeEntryTest.rewindErrorfulNode_toleratedButNoOp
#[test]
fn rewind_errorful_node_tolerated_but_no_op_incremental() {
    shared::rewind_errorful_node_tolerated_but_no_op(&Kind::incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:571 InMemoryNodeEntryTest.rewindErrorfulNode_toleratedButNoOp
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn rewind_errorful_node_tolerated_but_no_op_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.rewindErrorfulNode_toleratedButNoOp")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:592 InMemoryNodeEntryTest.skipsBatchPrefetch_testTemporaryDepsContainsHashSet
#[test]
#[ignore = "SkyKey.skipsBatchPrefetch and GroupedDeps.WithHashSet are not ported (partial reevaluation)"]
fn skips_batch_prefetch_test_temporary_deps_contains_hash_set_incremental() {
    unimplemented!(
        "not ported: InMemoryNodeEntryTest.skipsBatchPrefetch_testTemporaryDepsContainsHashSet"
    )
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:592 InMemoryNodeEntryTest.skipsBatchPrefetch_testTemporaryDepsContainsHashSet
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn skips_batch_prefetch_test_temporary_deps_contains_hash_set_incremental_partial_reevaluation() {
    unimplemented!(
        "not ported: InMemoryNodeEntryTest.skipsBatchPrefetch_testTemporaryDepsContainsHashSet"
    )
}

// ---------------------------------------------------------------------------------------------
// IncrementalInMemoryNodeEntryTest's own cases.

/// The common start of many cases: a node built at the initial version with one dep `dep` and
/// the value `value`.
fn build_with_dep(entry: &mut InMemoryNodeEntry, dep: &SkyKey, value: SkyValue) {
    entry.add_reverse_dep_and_check_if_done(None); // Start evaluation.
    entry.mark_rebuilding();
    entry.add_singleton_temporary_direct_dep(dep.clone());
    entry.signal_dep(initial_version(), Some(dep));
    set_value(
        entry,
        Some(value),
        /* error_info= */ None,
        initial_version(),
    );
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:52 IncrementalInMemoryNodeEntryTest.dirtyLifecycle
#[test]
fn dirty_lifecycle() {
    let mut entry = create_entry();
    let dep = key("dep");
    build_with_dep(&mut entry, &dep, integer_value(1));
    assert!(!entry.is_dirty());
    assert!(entry.is_done());

    entry.mark_dirty(DirtyType::Dirty);
    assert!(entry.is_dirty());
    assert!(!entry.is_changed());
    assert!(!entry.is_done());
    assert_eq!(as_integer(entry.to_value()), Some(1));

    assert_eq!(
        entry.add_reverse_dep_and_check_if_done(None),
        DependencyState::NeedsScheduling
    );
    assert!(entry.is_ready_to_evaluate());
    assert!(!entry.has_unsignaled_deps());
    assert!(entry.get_temporary_direct_deps().is_empty());

    let parent = key("parent");
    entry.add_reverse_dep_and_check_if_done(Some(parent.clone()));
    assert_eq!(
        entry.get_lifecycle_state(),
        LifecycleState::CheckDependencies
    );

    assert_contains_exactly(
        &entry.get_next_dirty_direct_deps(),
        std::slice::from_ref(&dep),
    );
    entry.add_singleton_temporary_direct_dep(dep.clone());
    entry.signal_dep(incremental_version(), Some(&dep));
    assert_contains_exactly(&temporary_direct_deps(&mut entry), &[dep]);
    assert!(entry.is_ready_to_evaluate());
    assert!(!entry.has_unsignaled_deps());

    entry.mark_rebuilding();
    assert_contains_exactly(
        &set_value(
            &mut entry,
            Some(integer_value(2)),
            /* error_info= */ None,
            incremental_version(),
        ),
        &[parent],
    );
    assert_eq!(entry.get_version(), incremental_version());
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:97 IncrementalInMemoryNodeEntryTest.changedLifecycle
#[test]
fn changed_lifecycle() {
    let mut entry = create_entry();
    let dep = key("dep");
    build_with_dep(&mut entry, &dep, integer_value(1));
    assert!(!entry.is_dirty());
    assert!(entry.is_done());

    entry.mark_dirty(DirtyType::Change);
    assert!(entry.is_dirty());
    assert!(entry.is_changed());
    assert!(!entry.is_done());
    assert_eq!(as_integer(entry.to_value()), Some(1));

    assert_eq!(
        entry.add_reverse_dep_and_check_if_done(None),
        DependencyState::NeedsScheduling
    );
    assert!(entry.is_ready_to_evaluate());
    assert!(!entry.has_unsignaled_deps());

    let parent = key("parent");
    entry.add_reverse_dep_and_check_if_done(Some(parent.clone()));
    assert_eq!(entry.get_lifecycle_state(), LifecycleState::NeedsRebuilding);
    assert!(entry.is_ready_to_evaluate());
    assert!(!entry.has_unsignaled_deps());
    assert!(entry.get_temporary_direct_deps().is_empty());

    entry.mark_rebuilding();
    assert_contains_exactly(
        &set_value(
            &mut entry,
            Some(new_sky_value()),
            /* error_info= */ None,
            incremental_version(),
        ),
        &[parent],
    );
    assert_eq!(entry.get_version(), incremental_version());
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:135 IncrementalInMemoryNodeEntryTest.markDirtyThenChanged
#[test]
fn mark_dirty_then_changed() {
    let mut entry = create_entry();
    let dep = key("dep");
    build_with_dep(&mut entry, &dep, new_sky_value());
    assert!(!entry.is_dirty());
    assert!(entry.is_done());
    entry.mark_dirty(DirtyType::Dirty);
    assert!(entry.is_dirty());
    assert!(!entry.is_changed());
    assert!(!entry.is_done());
    entry.mark_dirty(DirtyType::Change);
    assert!(entry.is_dirty());
    assert!(entry.is_changed());
    assert!(!entry.is_done());
    assert_eq!(
        entry.add_reverse_dep_and_check_if_done(None),
        DependencyState::NeedsScheduling
    );
    assert!(entry.is_ready_to_evaluate());
    assert!(!entry.has_unsignaled_deps());
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:161 IncrementalInMemoryNodeEntryTest.markChangedThenDirty
#[test]
fn mark_changed_then_dirty() {
    let mut entry = create_entry();
    let dep = key("dep");
    build_with_dep(&mut entry, &dep, new_sky_value());
    assert!(!entry.is_dirty());
    assert!(entry.is_done());
    entry.mark_dirty(DirtyType::Change);
    assert!(entry.is_dirty());
    assert!(entry.is_changed());
    assert!(!entry.is_done());
    entry.mark_dirty(DirtyType::Dirty);
    assert!(entry.is_dirty());
    assert!(entry.is_changed());
    assert!(!entry.is_done());
    assert_eq!(
        entry.add_reverse_dep_and_check_if_done(None),
        DependencyState::NeedsScheduling
    );
    assert!(entry.is_ready_to_evaluate());
    assert!(!entry.has_unsignaled_deps());
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:187 IncrementalInMemoryNodeEntryTest.crashOnTwiceMarkedChanged
#[test]
fn crash_on_twice_marked_changed() {
    let mut entry = create_entry();
    entry.add_reverse_dep_and_check_if_done(None); // Start evaluation.
    entry.mark_rebuilding();
    set_value(
        &mut entry,
        Some(new_sky_value()),
        /* error_info= */ None,
        initial_version(),
    );
    assert!(!entry.is_dirty());
    assert!(entry.is_done());
    entry.mark_dirty(DirtyType::Change);
    // "Cannot mark entry changed twice"
    assert_throws_illegal_state(|| {
        entry.mark_dirty(DirtyType::Change);
    });
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:202 IncrementalInMemoryNodeEntryTest.crashOnTwiceMarkedDirty
#[test]
fn crash_on_twice_marked_dirty() {
    let mut entry = create_entry();
    let dep = key("dep");
    build_with_dep(&mut entry, &dep, new_sky_value());
    entry.mark_dirty(DirtyType::Dirty);
    // "Cannot mark entry dirty twice"
    assert_throws_illegal_state(|| {
        entry.mark_dirty(DirtyType::Dirty);
    });
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:218 IncrementalInMemoryNodeEntryTest.forceRebuildAfterTransientError
#[test]
fn force_rebuild_after_transient_error() {
    let mut entry = create_entry();
    entry.add_reverse_dep_and_check_if_done(None); // Start evaluation.
    entry.mark_rebuilding();
    let dep = key("dep");
    entry.add_singleton_temporary_direct_dep(dep.clone());
    entry.signal_dep(initial_version(), Some(&dep));
    entry.add_singleton_temporary_direct_dep(ERROR_TRANSIENCE_KEY.clone());
    entry.signal_dep(initial_version(), Some(&ERROR_TRANSIENCE_KEY));

    set_value(
        &mut entry,
        /* value= */ None,
        Some(error_info(
            "transient error",
            Transience::Transient,
            /* is_transitively_transient= */ true,
        )),
        initial_version(),
    );
    assert!(!entry.is_dirty());
    assert!(entry.is_done());

    entry.mark_dirty(DirtyType::Dirty);
    assert!(entry.is_dirty());
    assert!(!entry.is_changed());
    assert!(!entry.is_done());

    let parent = key("parent");
    assert_eq!(
        entry.add_reverse_dep_and_check_if_done(Some(parent.clone())),
        DependencyState::NeedsScheduling
    );
    assert!(entry.is_ready_to_evaluate());
    assert!(!entry.has_unsignaled_deps());
    assert_eq!(
        entry.get_lifecycle_state(),
        LifecycleState::CheckDependencies
    );
    assert!(entry.is_ready_to_evaluate());
    assert!(!entry.has_unsignaled_deps());
    assert!(entry.get_temporary_direct_deps().is_empty());

    assert_contains_exactly(
        &entry.get_next_dirty_direct_deps(),
        std::slice::from_ref(&dep),
    );
    entry.add_singleton_temporary_direct_dep(dep.clone());
    assert!(entry.signal_dep(initial_version(), Some(&dep)));
    assert_eq!(
        entry.get_lifecycle_state(),
        LifecycleState::CheckDependencies
    );

    assert_contains_exactly(
        &entry.get_next_dirty_direct_deps(),
        std::slice::from_ref(&*ERROR_TRANSIENCE_KEY),
    );
    entry.force_rebuild();
    assert_eq!(entry.get_lifecycle_state(), LifecycleState::Rebuilding);

    assert_contains_exactly(
        &set_value(
            &mut entry,
            Some(new_sky_value()),
            /* error_info= */ None,
            incremental_version(),
        ),
        &[parent],
    );
    assert_eq!(entry.get_version(), incremental_version());
    assert_contains_exactly(&entry.get_direct_deps(), &[dep]); // No more dep on error transience.
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:274 IncrementalInMemoryNodeEntryTest.crashOnAddReverseDepTwiceAfterDone
#[test]
fn crash_on_add_reverse_dep_twice_after_done() {
    let mut entry = create_entry();
    entry.add_reverse_dep_and_check_if_done(None); // Start evaluation.
    entry.mark_rebuilding();
    set_value(
        &mut entry,
        Some(new_sky_value()),
        /* error_info= */ None,
        initial_version(),
    );
    let parent = key("parent");
    assert_eq!(
        entry.add_reverse_dep_and_check_if_done(Some(parent.clone())),
        DependencyState::Done
    );
    entry.add_reverse_dep_and_check_if_done(Some(parent));
    // "Cannot add same dep twice". We only check for duplicates when we request all the reverse
    // deps.
    assert_throws_illegal_state(|| {
        entry.get_reverse_deps_for_done_entry();
    });
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:290 IncrementalInMemoryNodeEntryTest.crashOnAddReverseDepBeforeAfterDone
#[test]
fn crash_on_add_reverse_dep_before_after_done() {
    let mut entry = create_entry();
    let parent = key("parent");
    assert_eq!(
        entry.add_reverse_dep_and_check_if_done(Some(parent.clone())),
        DependencyState::NeedsScheduling
    );
    entry.mark_rebuilding();
    set_value(
        &mut entry,
        Some(new_sky_value()),
        /* error_info= */ None,
        initial_version(),
    );
    entry.add_reverse_dep_and_check_if_done(Some(parent));
    // "Cannot add same dep twice". We only check for duplicates when we request all the reverse
    // deps.
    assert_throws_illegal_state(|| {
        entry.get_reverse_deps_for_done_entry();
    });
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:306 IncrementalInMemoryNodeEntryTest.pruneBeforeBuild
#[test]
fn prune_before_build() {
    let mut entry = create_entry();
    let dep = key("dep");
    build_with_dep(&mut entry, &dep, new_sky_value());
    assert!(!entry.is_dirty());
    assert!(entry.is_done());
    entry.mark_dirty(DirtyType::Dirty);
    assert!(entry.is_dirty());
    assert!(!entry.is_changed());
    assert!(!entry.is_done());
    assert_eq!(
        entry.add_reverse_dep_and_check_if_done(None),
        DependencyState::NeedsScheduling
    );
    assert!(entry.is_ready_to_evaluate());
    assert!(!entry.has_unsignaled_deps());
    let parent = key("parent");
    entry.add_reverse_dep_and_check_if_done(Some(parent.clone()));
    assert_eq!(
        entry.get_lifecycle_state(),
        LifecycleState::CheckDependencies
    );
    assert_contains_exactly(
        &entry.get_next_dirty_direct_deps(),
        std::slice::from_ref(&dep),
    );
    entry.add_singleton_temporary_direct_dep(dep);
    entry.signal_dep(initial_version(), /* child_for_debugging= */ None);
    assert_eq!(entry.get_lifecycle_state(), LifecycleState::VerifiedClean);
    assert_contains_exactly(entry.mark_clean().get_rdeps_to_signal(), &[parent]);
    assert!(entry.is_done());
    assert_eq!(entry.get_version(), initial_version());
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:338 IncrementalInMemoryNodeEntryTest.pruneAfterBuild
#[test]
fn prune_after_build() {
    let mut entry = create_entry();
    let dep = key("dep");
    build_with_dep(&mut entry, &dep, integer_value(5));
    entry.mark_dirty(DirtyType::Dirty);
    entry.add_reverse_dep_and_check_if_done(None); // Start evaluation.
    assert_eq!(
        entry.get_lifecycle_state(),
        LifecycleState::CheckDependencies
    );
    entry.add_reverse_dep_and_check_if_done(None); // Start evaluation.
    assert_contains_exactly(
        &entry.get_next_dirty_direct_deps(),
        std::slice::from_ref(&dep),
    );
    entry.add_singleton_temporary_direct_dep(dep.clone());
    entry.signal_dep(incremental_version(), /* child_for_debugging= */ None);
    assert_eq!(entry.get_lifecycle_state(), LifecycleState::NeedsRebuilding);
    assert_contains_exactly(&temporary_direct_deps(&mut entry), &[dep]);
    entry.mark_rebuilding();
    set_value(
        &mut entry,
        Some(integer_value(5)),
        /* error_info= */ None,
        incremental_version(),
    );
    assert!(entry.is_done());
    assert_eq!(entry.get_version(), initial_version());
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:362 IncrementalInMemoryNodeEntryTest.noPruneWhenDetailsChange
#[test]
fn no_prune_when_details_change() {
    let mut entry = create_entry();
    let dep = key("dep");
    build_with_dep(&mut entry, &dep, integer_value(5));
    assert!(!entry.is_dirty());
    assert!(entry.is_done());
    entry.mark_dirty(DirtyType::Dirty);
    assert!(entry.is_dirty());
    assert!(!entry.is_changed());
    assert!(!entry.is_done());
    assert_eq!(
        entry.add_reverse_dep_and_check_if_done(None),
        DependencyState::NeedsScheduling
    );
    assert!(entry.is_ready_to_evaluate());
    assert!(!entry.has_unsignaled_deps());
    let parent = key("parent");
    entry.add_reverse_dep_and_check_if_done(Some(parent));
    assert_eq!(
        entry.get_lifecycle_state(),
        LifecycleState::CheckDependencies
    );
    assert_contains_exactly(
        &entry.get_next_dirty_direct_deps(),
        std::slice::from_ref(&dep),
    );
    entry.add_singleton_temporary_direct_dep(dep.clone());
    entry.signal_dep(incremental_version(), /* child_for_debugging= */ None);
    assert_eq!(entry.get_lifecycle_state(), LifecycleState::NeedsRebuilding);
    assert_contains_exactly(&temporary_direct_deps(&mut entry), &[dep]);
    entry.mark_rebuilding();
    set_value(
        &mut entry,
        Some(integer_value(5)),
        Some(error_info("oops", Transience::Persistent, false)),
        incremental_version(),
    );
    assert!(entry.is_done());
    // "Version increments when setValue changes"
    assert_eq!(entry.get_version(), Version::Int(IntVersion::of(1)));
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:402 IncrementalInMemoryNodeEntryTest.pruneWhenDepGroupReordered
#[test]
fn prune_when_dep_group_reordered() {
    let mut entry = create_entry();
    entry.add_reverse_dep_and_check_if_done(None); // Start evaluation.
    entry.mark_rebuilding();
    let dep = key("dep");
    let dep1_in_group = key("dep1InGroup");
    let dep2_in_group = key("dep2InGroup");
    entry.add_singleton_temporary_direct_dep(dep.clone());
    entry.add_temporary_direct_dep_group(vec![dep1_in_group.clone(), dep2_in_group.clone()]);
    entry.signal_dep(initial_version(), Some(&dep));
    entry.signal_dep(initial_version(), Some(&dep1_in_group));
    entry.signal_dep(initial_version(), Some(&dep2_in_group));
    set_value(
        &mut entry,
        Some(integer_value(5)),
        /* error_info= */ None,
        initial_version(),
    );
    assert!(!entry.is_dirty());
    assert!(entry.is_done());
    entry.mark_dirty(DirtyType::Dirty);
    assert!(entry.is_dirty());
    assert!(!entry.is_changed());
    assert!(!entry.is_done());
    assert_eq!(
        entry.add_reverse_dep_and_check_if_done(None),
        DependencyState::NeedsScheduling
    );
    assert!(entry.is_ready_to_evaluate());
    assert!(!entry.has_unsignaled_deps());
    entry.add_reverse_dep_and_check_if_done(None);
    assert_eq!(
        entry.get_lifecycle_state(),
        LifecycleState::CheckDependencies
    );
    assert_contains_exactly(
        &entry.get_next_dirty_direct_deps(),
        std::slice::from_ref(&dep),
    );
    entry.add_singleton_temporary_direct_dep(dep.clone());
    entry.signal_dep(incremental_version(), /* child_for_debugging= */ None);
    assert_eq!(entry.get_lifecycle_state(), LifecycleState::NeedsRebuilding);
    assert_contains_exactly(&temporary_direct_deps(&mut entry), &[dep]);
    entry.mark_rebuilding();
    entry.add_temporary_direct_dep_group(vec![dep2_in_group.clone(), dep1_in_group.clone()]);
    assert!(!entry.signal_dep(incremental_version(), Some(&dep2_in_group)));
    assert!(entry.signal_dep(incremental_version(), Some(&dep1_in_group)));
    set_value(
        &mut entry,
        Some(integer_value(5)),
        /* error_info= */ None,
        incremental_version(),
    );
    assert!(entry.is_done());
    // "Version does not change when dep group reordered"
    assert_eq!(entry.get_version(), Version::Int(IntVersion::of(0)));
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:445 IncrementalInMemoryNodeEntryTest.errorInfoCannotBePruned
#[test]
fn error_info_cannot_be_pruned() {
    let mut entry = create_entry();
    entry.add_reverse_dep_and_check_if_done(None); // Start evaluation.
    entry.mark_rebuilding();
    let dep = key("dep");
    entry.add_singleton_temporary_direct_dep(dep.clone());
    entry.signal_dep(initial_version(), Some(&dep));
    let error_info = error_info("oops", Transience::Persistent, false);
    set_value(
        &mut entry,
        /* value= */ None,
        Some(error_info.clone()),
        initial_version(),
    );
    entry.mark_dirty(DirtyType::Dirty);
    entry.add_reverse_dep_and_check_if_done(None); // Restart evaluation.
    assert_eq!(
        entry.get_lifecycle_state(),
        LifecycleState::CheckDependencies
    );
    assert_contains_exactly(
        &entry.get_next_dirty_direct_deps(),
        std::slice::from_ref(&dep),
    );
    entry.add_singleton_temporary_direct_dep(dep.clone());
    entry.signal_dep(incremental_version(), /* child_for_debugging= */ None);
    assert_eq!(entry.get_lifecycle_state(), LifecycleState::NeedsRebuilding);
    assert_contains_exactly(&temporary_direct_deps(&mut entry), &[dep]);
    entry.mark_rebuilding();
    set_value(
        &mut entry,
        /* value= */ None,
        Some(error_info),
        incremental_version(),
    );
    assert!(entry.is_done());
    // ErrorInfo is treated as a NotComparableSkyValue, so it is not pruned.
    assert_eq!(entry.get_version(), incremental_version());
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:473 IncrementalInMemoryNodeEntryTest.getDependencyGroup
#[test]
fn get_dependency_group() {
    let mut entry = create_entry();
    entry.add_reverse_dep_and_check_if_done(None); // Start evaluation.
    entry.mark_rebuilding();
    let dep = key("dep");
    let dep2 = key("dep2");
    let dep3 = key("dep3");
    entry.add_temporary_direct_dep_group(vec![dep.clone(), dep2.clone()]);
    entry.add_singleton_temporary_direct_dep(dep3.clone());
    entry.signal_dep(initial_version(), Some(&dep));
    entry.signal_dep(initial_version(), Some(&dep2));
    entry.signal_dep(initial_version(), Some(&dep3));
    set_value(&mut entry, Some(integer_value(5)), None, initial_version());
    entry.mark_dirty(DirtyType::Dirty);
    entry.add_reverse_dep_and_check_if_done(None); // Restart evaluation.
    assert_eq!(
        entry.get_lifecycle_state(),
        LifecycleState::CheckDependencies
    );
    assert_contains_exactly(
        &entry.get_next_dirty_direct_deps(),
        &[dep.clone(), dep2.clone()],
    );
    entry.add_temporary_direct_dep_group(vec![dep, dep2]);
    entry.signal_dep(initial_version(), /* child_for_debugging= */ None);
    entry.signal_dep(initial_version(), /* child_for_debugging= */ None);
    assert_eq!(
        entry.get_lifecycle_state(),
        LifecycleState::CheckDependencies
    );
    assert_contains_exactly(&entry.get_next_dirty_direct_deps(), &[dep3]);
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:498 IncrementalInMemoryNodeEntryTest.maintainDependencyGroupAfterRemoval
#[test]
fn maintain_dependency_group_after_removal() {
    let mut entry = create_entry();
    entry.add_reverse_dep_and_check_if_done(None); // Start evaluation.
    entry.mark_rebuilding();
    let dep = key("dep");
    let dep2 = key("dep2");
    let dep3 = key("dep3");
    let dep4 = key("dep4");
    let dep5 = key("dep5");
    entry.add_temporary_direct_dep_group(vec![dep.clone(), dep2.clone(), dep3.clone()]);
    entry.add_singleton_temporary_direct_dep(dep4.clone());
    entry.add_singleton_temporary_direct_dep(dep5.clone());
    entry.signal_dep(initial_version(), Some(&dep4));
    entry.signal_dep(initial_version(), Some(&dep));
    // Oops! Evaluation terminated with an error, but we're going to set this entry's value
    // anyway.
    entry.remove_unfinished_deps(&HashSet::from([dep2, dep3, dep5]));
    set_value(
        &mut entry,
        None,
        Some(error_info("oops", Transience::Persistent, false)),
        initial_version(),
    );
    entry.mark_dirty(DirtyType::Dirty);
    entry.add_reverse_dep_and_check_if_done(None); // Restart evaluation.
    assert_eq!(
        entry.get_lifecycle_state(),
        LifecycleState::CheckDependencies
    );
    assert_contains_exactly(
        &entry.get_next_dirty_direct_deps(),
        std::slice::from_ref(&dep),
    );
    entry.add_singleton_temporary_direct_dep(dep.clone());
    entry.signal_dep(initial_version(), Some(&dep));
    assert_eq!(
        entry.get_lifecycle_state(),
        LifecycleState::CheckDependencies
    );
    assert_contains_exactly(&entry.get_next_dirty_direct_deps(), &[dep4]);
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:529 IncrementalInMemoryNodeEntryTest.pruneWhenDepsChange
#[test]
fn prune_when_deps_change() {
    let mut entry = create_entry();
    let dep = key("dep");
    build_with_dep(&mut entry, &dep, integer_value(5));
    entry.mark_dirty(DirtyType::Dirty);
    entry.add_reverse_dep_and_check_if_done(None); // Start evaluation.
    assert_eq!(
        entry.get_lifecycle_state(),
        LifecycleState::CheckDependencies
    );
    assert_contains_exactly(
        &entry.get_next_dirty_direct_deps(),
        std::slice::from_ref(&dep),
    );
    entry.add_singleton_temporary_direct_dep(dep.clone());
    assert!(entry.signal_dep(incremental_version(), /* child_for_debugging= */ None));
    assert_eq!(entry.get_lifecycle_state(), LifecycleState::NeedsRebuilding);
    assert_contains_exactly(&temporary_direct_deps(&mut entry), &[dep]);
    entry.mark_rebuilding();
    entry.add_singleton_temporary_direct_dep(key("dep2"));
    assert!(entry.signal_dep(incremental_version(), /* child_for_debugging= */ None));
    set_value(
        &mut entry,
        Some(integer_value(5)),
        /* error_info= */ None,
        incremental_version(),
    );
    assert!(entry.is_done());
    assert_eq!(entry.get_version(), initial_version());
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:554 IncrementalInMemoryNodeEntryTest.checkDepsOneByOne
#[test]
fn check_deps_one_by_one() {
    let mut entry = create_entry();
    entry.add_reverse_dep_and_check_if_done(None); // Start evaluation.
    entry.mark_rebuilding();
    let mut deps: Vec<SkyKey> = vec![];
    for ii in 0..10 {
        let dep = key(&ii.to_string());
        deps.push(dep.clone());
        entry.add_singleton_temporary_direct_dep(dep.clone());
        entry.signal_dep(initial_version(), Some(&dep));
    }
    set_value(
        &mut entry,
        Some(integer_value(5)),
        /* error_info= */ None,
        initial_version(),
    );
    entry.mark_dirty(DirtyType::Dirty);
    entry.add_reverse_dep_and_check_if_done(None); // Start new evaluation.
    assert_eq!(
        entry.get_lifecycle_state(),
        LifecycleState::CheckDependencies
    );
    for (ii, dep) in deps.iter().enumerate() {
        assert_contains_exactly(
            &entry.get_next_dirty_direct_deps(),
            std::slice::from_ref(dep),
        );
        entry.add_singleton_temporary_direct_dep(dep.clone());
        assert!(entry.signal_dep(initial_version(), /* child_for_debugging= */ None));
        if ii < 9 {
            assert_eq!(
                entry.get_lifecycle_state(),
                LifecycleState::CheckDependencies
            );
        } else {
            assert_eq!(entry.get_lifecycle_state(), LifecycleState::VerifiedClean);
        }
    }
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:582 IncrementalInMemoryNodeEntryTest.signalOnlyNewParents
#[test]
fn signal_only_new_parents() {
    let mut entry = create_entry();
    entry.add_reverse_dep_and_check_if_done(Some(key("parent")));
    entry.mark_rebuilding();
    set_value(
        &mut entry,
        Some(new_sky_value()),
        /* error_info= */ None,
        initial_version(),
    );
    entry.mark_dirty(DirtyType::Change);
    let new_parent = key("new parent");
    entry.add_reverse_dep_and_check_if_done(Some(new_parent.clone()));
    assert_eq!(entry.get_lifecycle_state(), LifecycleState::NeedsRebuilding);
    entry.mark_rebuilding();
    assert_eq!(entry.get_lifecycle_state(), LifecycleState::Rebuilding);
    assert_contains_exactly(
        &set_value(
            &mut entry,
            Some(new_sky_value()),
            /* error_info= */ None,
            incremental_version(),
        ),
        &[new_parent],
    );
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:598 IncrementalInMemoryNodeEntryTest.getCompressedDirectDepsForDoneEntry
#[test]
fn get_compressed_direct_deps_for_done_entry() {
    let mut entry = create_entry();
    let grouped_direct_deps: Vec<Vec<SkyKey>> = vec![
        vec![key("1A")],
        vec![key("2A"), key("2B")],
        vec![key("3A"), key("3B"), key("3C")],
        vec![key("4A"), key("4B"), key("4C"), key("4D")],
    ];
    assert_eq!(
        entry.add_reverse_dep_and_check_if_done(None),
        DependencyState::NeedsScheduling
    );
    entry.mark_rebuilding();
    for dep_group in &grouped_direct_deps {
        entry.add_temporary_direct_dep_group(dep_group.clone());
        for dep in dep_group {
            entry.signal_dep(initial_version(), Some(dep));
        }
    }
    entry.set_value(
        ValueWithMetadata::Bare(integer_value(42)),
        Version::Int(IntVersion::of(42)),
        None,
    );
    // PORT: GroupedDeps.decompress is the identity here (no compression).
    assert_eq!(
        entry.get_compressed_direct_deps_for_done_entry().groups(),
        grouped_direct_deps.as_slice()
    );
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:623 IncrementalInMemoryNodeEntryTest.hasAtLeastOneDep_true
#[test]
fn has_at_least_one_dep_true() {
    let dep = key("dep");
    let mut entry = create_entry();
    assert_eq!(
        entry.add_reverse_dep_and_check_if_done(None),
        DependencyState::NeedsScheduling
    );
    entry.mark_rebuilding();
    entry.add_singleton_temporary_direct_dep(dep.clone());
    entry.signal_dep(initial_version(), Some(&dep));
    entry.set_value(
        ValueWithMetadata::Bare(integer_value(1)),
        initial_version(),
        None,
    );
    assert!(entry.has_at_least_one_dep());
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:637 IncrementalInMemoryNodeEntryTest.hasAtLeastOneDep_false
#[test]
fn has_at_least_one_dep_false() {
    let mut entry = create_entry();
    assert_eq!(
        entry.add_reverse_dep_and_check_if_done(None),
        DependencyState::NeedsScheduling
    );
    entry.mark_rebuilding();
    entry.add_temporary_direct_dep_group(vec![]);
    entry.set_value(
        ValueWithMetadata::Bare(integer_value(1)),
        initial_version(),
        None,
    );
    assert!(!entry.has_at_least_one_dep());
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:649 IncrementalInMemoryNodeEntryTest.getAllDirectDepsForIncompleteNode_notYetEvaluating
#[test]
fn get_all_direct_deps_for_incomplete_node_not_yet_evaluating() {
    let mut entry = create_entry();
    assert!(entry.get_all_direct_deps_for_incomplete_node().is_empty());
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:655 IncrementalInMemoryNodeEntryTest.getAllDirectDepsForIncompleteNode_initialBuild
#[test]
fn get_all_direct_deps_for_incomplete_node_initial_build() {
    let mut entry = create_entry();
    entry.add_reverse_dep_and_check_if_done(None);
    entry.mark_rebuilding();

    let dep1 = key("dep1");
    let dep2 = key("dep2");
    entry.add_temporary_direct_dep_group(vec![dep1.clone(), dep2.clone()]);

    assert_contains_exactly(
        &entry.get_all_direct_deps_for_incomplete_node(),
        &[dep1, dep2],
    );
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:668 IncrementalInMemoryNodeEntryTest.getAllDirectDepsForIncompleteNode_incrementalBuild
#[test]
fn get_all_direct_deps_for_incomplete_node_incremental_build() {
    let mut entry = create_entry();
    entry.add_reverse_dep_and_check_if_done(None);
    entry.mark_rebuilding();

    // Dep added on initial build that stays on incremental build.
    let old_and_new_dep = key("oldAndNewDep");
    entry.add_singleton_temporary_direct_dep(old_and_new_dep.clone());
    entry.signal_dep(initial_version(), Some(&old_and_new_dep));

    // Dep added on initial build that is removed on incremental build.
    let old_dep = key("oldDep");
    entry.add_singleton_temporary_direct_dep(old_dep.clone());
    entry.signal_dep(initial_version(), Some(&old_dep));

    // Initial build completes.
    set_value(
        &mut entry,
        Some(integer_value(1)),
        /* error_info= */ None,
        initial_version(),
    );

    // Start of incremental build.
    entry.mark_dirty(DirtyType::Dirty);
    entry.add_reverse_dep_and_check_if_done(None);

    // First dep changed, causes rebuild.
    assert_contains_exactly(
        &entry.get_next_dirty_direct_deps(),
        std::slice::from_ref(&old_and_new_dep),
    );
    entry.add_singleton_temporary_direct_dep(old_and_new_dep.clone());
    entry.signal_dep(incremental_version(), Some(&old_and_new_dep));
    entry.mark_rebuilding();

    // New dep requested.
    let new_dep = key("newDep");
    entry.add_singleton_temporary_direct_dep(new_dep.clone());

    assert_contains_exactly(
        &entry.get_all_direct_deps_for_incomplete_node(),
        &[old_dep, old_and_new_dep, new_dep],
    );
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:705 IncrementalInMemoryNodeEntryTest.getAllDirectDepsForIncompleteNode_afterReset
#[test]
fn get_all_direct_deps_for_incomplete_node_after_reset() {
    let mut entry = create_entry();
    entry.add_reverse_dep_and_check_if_done(None);
    entry.mark_rebuilding();

    let dep = key("dep");
    entry.add_singleton_temporary_direct_dep(dep.clone());
    entry.signal_dep(initial_version(), Some(&dep));
    entry.reset_evaluation_from_scratch();

    assert_contains_exactly(
        &entry
            .get_all_direct_deps_for_incomplete_node()
            .into_iter()
            .collect::<Vec<_>>(),
        &[dep],
    );
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:719 IncrementalInMemoryNodeEntryTest.resetOnDirtyNode
// (@TestParameter boolean valueChanges: both values)
#[test]
fn reset_on_dirty_node() {
    for value_changes in [false, true] {
        let mut entry = create_entry();
        entry.add_reverse_dep_and_check_if_done(None); // Start evaluation.
        entry.mark_rebuilding();

        // Rdep added on initial build that stays on incremental build.
        let old_and_new_parent = key("oldAndNewParent");
        entry.add_reverse_dep_and_check_if_done(Some(old_and_new_parent.clone()));

        // Rdep added on initial build that is removed on incremental build.
        let old_parent = key("oldParent");
        entry.add_reverse_dep_and_check_if_done(Some(old_parent.clone()));

        // Dep added on initial build that stays on incremental build.
        let old_and_new_dep = key("oldAndNewDep");
        entry.add_singleton_temporary_direct_dep(old_and_new_dep.clone());
        entry.signal_dep(initial_version(), Some(&old_and_new_dep));

        // Dep added on initial build that is removed on incremental build.
        let old_dep = key("oldDep");
        entry.add_singleton_temporary_direct_dep(old_dep.clone());
        entry.signal_dep(initial_version(), Some(&old_dep));

        // Initial build completes.
        set_value(&mut entry, Some(integer_value(1)), None, initial_version());

        // Start of incremental build.
        entry.mark_dirty(DirtyType::Dirty);

        // One rdep added, one rdep stays, one rdep removed.
        let new_parent = key("newParent");
        entry.add_reverse_dep_and_check_if_done(Some(new_parent.clone()));
        entry.check_if_done_for_dirty_reverse_dep(old_and_new_parent.clone());
        entry.remove_reverse_dep(old_parent.clone());

        // Old dep added before reset, triggers rebuild.
        assert_contains_exactly(
            &entry.get_next_dirty_direct_deps(),
            std::slice::from_ref(&old_and_new_dep),
        );
        entry.add_singleton_temporary_direct_dep(old_and_new_dep.clone());
        entry.signal_dep(incremental_version(), Some(&old_and_new_dep));
        entry.mark_rebuilding();
        assert!(entry.get_reset_direct_deps().is_empty());

        // Reset clears temporary direct deps, but preserves the dirty value.
        entry.reset_evaluation_from_scratch();
        assert_eq!(entry.get_lifecycle_state(), LifecycleState::Rebuilding);
        assert!(temporary_direct_deps(&mut entry).is_empty());
        assert_eq!(as_integer(entry.to_value()), Some(1));

        // Add back same dep.
        entry.add_singleton_temporary_direct_dep(old_and_new_dep.clone());
        assert!(entry.signal_dep(incremental_version(), Some(&old_and_new_dep)));
        assert_eq!(
            temporary_direct_deps(&mut entry),
            vec![old_and_new_dep.clone()]
        );

        // New dep added after reset.
        let new_dep = key("newDep");
        entry.add_singleton_temporary_direct_dep(new_dep.clone());
        assert!(entry.signal_dep(incremental_version(), Some(&new_dep)));
        assert_eq!(
            temporary_direct_deps(&mut entry),
            vec![old_and_new_dep.clone(), new_dep.clone()]
        );

        // Check dep accounting.
        assert_contains_exactly(
            &entry
                .get_reset_direct_deps()
                .into_iter()
                .collect::<Vec<_>>(),
            std::slice::from_ref(&old_and_new_dep),
        );
        assert_contains_exactly(
            &entry
                .get_all_remaining_dirty_direct_deps()
                .into_iter()
                .collect::<Vec<_>>(),
            &[old_dep],
        );

        // Set value and check that new parents will be signaled.
        let new_value = if value_changes { 2 } else { 1 };
        assert_contains_exactly(
            &set_value(
                &mut entry,
                Some(integer_value(new_value)),
                None,
                incremental_version(),
            )
            .into_iter()
            .collect::<Vec<_>>(),
            &[old_and_new_parent, new_parent],
        );

        if value_changes {
            assert_eq!(entry.get_version(), incremental_version());
        } else {
            assert_eq!(entry.get_version(), initial_version()); // Change pruning.
        }
    }
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:799 IncrementalInMemoryNodeEntryTest.rewindOnIncrementalBuild
// (@TestParameter boolean valueChanges: both values)
#[test]
fn rewind_on_incremental_build() {
    for value_changes in [false, true] {
        let mut entry = create_entry();

        // Initial build.
        entry.add_reverse_dep_and_check_if_done(None); // Start evaluation.
        entry.mark_rebuilding();
        set_value(&mut entry, Some(integer_value(1)), None, initial_version());

        // Rdeps that exhibit various behavior and timing on the incremental build.
        let early_old_parent = key("earlyOldParent");
        let late_old_parent = key("lateOldParent");
        let clean_parent = key("cleanParent");
        let reset_dirty_parent = key("resetDirtyParent");
        let early_dirty_parent = key("earlyDirtyParent");
        let late_dirty_parent = key("lateDirtyParent");
        for parent in [
            &early_old_parent,
            &late_old_parent,
            &clean_parent,
            &reset_dirty_parent,
            &early_dirty_parent,
            &late_dirty_parent,
        ] {
            entry.add_reverse_dep_and_check_if_done(Some(parent.clone()));
        }

        // Start of incremental build.

        // Rdep removed before rewinding.
        entry.remove_reverse_dep(early_old_parent.clone());

        // Dirty rdep registered before rewinding.
        assert_eq!(
            entry.check_if_done_for_dirty_reverse_dep(early_dirty_parent.clone()),
            DependencyState::Done
        );

        // New rdep registered before rewinding.
        let early_new_parent = key("earlyNewParent");
        assert_eq!(
            entry.add_reverse_dep_and_check_if_done(Some(early_new_parent.clone())),
            DependencyState::Done
        );

        // Rewinding initiated.
        assert_eq!(
            entry.check_if_done_for_dirty_reverse_dep(reset_dirty_parent.clone()),
            DependencyState::Done
        );
        assert!(entry.mark_dirty(DirtyType::Rewind).is_some());
        assert_eq!(as_integer(entry.to_value()), Some(1));

        // Parent declares dep again after resetting.
        assert_eq!(
            entry.check_if_done_for_dirty_reverse_dep(reset_dirty_parent.clone()),
            DependencyState::NeedsScheduling
        );
        assert_eq!(entry.get_lifecycle_state(), LifecycleState::NeedsRebuilding);
        entry.mark_rebuilding();
        assert_eq!(entry.get_lifecycle_state(), LifecycleState::Rebuilding);

        // Rdep removed after rewinding was initiated.
        entry.remove_reverse_dep(late_old_parent.clone());

        // Dirty rdep registered after rewinding was initiated.
        assert_eq!(
            entry.check_if_done_for_dirty_reverse_dep(late_dirty_parent.clone()),
            DependencyState::AlreadyEvaluating
        );

        // New rdep registered after rewinding was initiated.
        let late_new_parent = key("lateNewParent");
        assert_eq!(
            entry.add_reverse_dep_and_check_if_done(Some(late_new_parent.clone())),
            DependencyState::AlreadyEvaluating
        );

        // Rewound evaluation completes. Only parents that are waiting on the node (registered
        // after rewinding was initiated) are signalled.
        let new_value = if value_changes { 2 } else { 1 };
        assert_contains_exactly(
            &set_value(
                &mut entry,
                Some(integer_value(new_value)),
                None,
                incremental_version(),
            )
            .into_iter()
            .collect::<Vec<_>>(),
            &[
                reset_dirty_parent.clone(),
                late_dirty_parent.clone(),
                late_new_parent.clone(),
            ],
        );
        assert_eq!(as_integer(entry.get_value()), Some(new_value));
        if value_changes {
            assert_eq!(entry.get_version(), incremental_version());
        } else {
            assert_eq!(entry.get_version(), initial_version()); // Change pruning.
        }

        // Check rdep accounting.
        assert_contains_exactly(
            &entry.get_reverse_deps_for_done_entry(),
            &[
                clean_parent,
                early_dirty_parent,
                early_new_parent,
                reset_dirty_parent,
                late_dirty_parent,
                late_new_parent,
            ],
        );
    }
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:885 IncrementalInMemoryNodeEntryTest.rewindOnDirtyNodeIgnored
// (@TestParameter boolean valueChanges: both values)
#[test]
fn rewind_on_dirty_node_ignored() {
    for value_changes in [false, true] {
        let mut entry = create_entry();
        entry.add_reverse_dep_and_check_if_done(None); // Start evaluation.
        entry.mark_rebuilding();

        // Dep added on initial build that stays on incremental build.
        let dep = key("dep");
        entry.add_singleton_temporary_direct_dep(dep.clone());
        entry.signal_dep(initial_version(), Some(&dep));

        // Initial build completes.
        set_value(&mut entry, Some(integer_value(1)), None, initial_version());

        // Start of incremental build.
        entry.mark_dirty(DirtyType::Dirty);
        entry.add_reverse_dep_and_check_if_done(None);
        assert_eq!(
            entry.get_lifecycle_state(),
            LifecycleState::CheckDependencies
        );

        // Attempt to rewind node while it is dirty. Its lifecycle state does not change.
        entry.mark_dirty(DirtyType::Rewind);
        assert_eq!(
            entry.get_lifecycle_state(),
            LifecycleState::CheckDependencies
        );

        // Add back same dep.
        entry.add_singleton_temporary_direct_dep(dep.clone());
        assert!(entry.signal_dep(incremental_version(), Some(&dep)));
        entry.mark_rebuilding();
        assert_eq!(temporary_direct_deps(&mut entry), vec![dep]);

        // Set value and check version.
        let new_value = if value_changes { 2 } else { 1 };
        set_value(
            &mut entry,
            Some(integer_value(new_value)),
            None,
            incremental_version(),
        );

        if value_changes {
            assert_eq!(entry.get_version(), incremental_version());
        } else {
            assert_eq!(entry.get_version(), initial_version()); // Change pruning.
        }
    }
}

// ---------------------------------------------------------------------------------------------
// isPartialReevaluation = true runs of IncrementalInMemoryNodeEntryTest's own cases.

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:52 IncrementalInMemoryNodeEntryTest.dirtyLifecycle
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn dirty_lifecycle_partial_reevaluation() {
    unimplemented!("not ported: IncrementalInMemoryNodeEntryTest.dirtyLifecycle")
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:97 IncrementalInMemoryNodeEntryTest.changedLifecycle
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn changed_lifecycle_partial_reevaluation() {
    unimplemented!("not ported: IncrementalInMemoryNodeEntryTest.changedLifecycle")
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:135 IncrementalInMemoryNodeEntryTest.markDirtyThenChanged
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn mark_dirty_then_changed_partial_reevaluation() {
    unimplemented!("not ported: IncrementalInMemoryNodeEntryTest.markDirtyThenChanged")
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:161 IncrementalInMemoryNodeEntryTest.markChangedThenDirty
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn mark_changed_then_dirty_partial_reevaluation() {
    unimplemented!("not ported: IncrementalInMemoryNodeEntryTest.markChangedThenDirty")
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:187 IncrementalInMemoryNodeEntryTest.crashOnTwiceMarkedChanged
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn crash_on_twice_marked_changed_partial_reevaluation() {
    unimplemented!("not ported: IncrementalInMemoryNodeEntryTest.crashOnTwiceMarkedChanged")
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:202 IncrementalInMemoryNodeEntryTest.crashOnTwiceMarkedDirty
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn crash_on_twice_marked_dirty_partial_reevaluation() {
    unimplemented!("not ported: IncrementalInMemoryNodeEntryTest.crashOnTwiceMarkedDirty")
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:218 IncrementalInMemoryNodeEntryTest.forceRebuildAfterTransientError
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn force_rebuild_after_transient_error_partial_reevaluation() {
    unimplemented!("not ported: IncrementalInMemoryNodeEntryTest.forceRebuildAfterTransientError")
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:274 IncrementalInMemoryNodeEntryTest.crashOnAddReverseDepTwiceAfterDone
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn crash_on_add_reverse_dep_twice_after_done_partial_reevaluation() {
    unimplemented!(
        "not ported: IncrementalInMemoryNodeEntryTest.crashOnAddReverseDepTwiceAfterDone"
    )
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:290 IncrementalInMemoryNodeEntryTest.crashOnAddReverseDepBeforeAfterDone
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn crash_on_add_reverse_dep_before_after_done_partial_reevaluation() {
    unimplemented!(
        "not ported: IncrementalInMemoryNodeEntryTest.crashOnAddReverseDepBeforeAfterDone"
    )
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:306 IncrementalInMemoryNodeEntryTest.pruneBeforeBuild
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn prune_before_build_partial_reevaluation() {
    unimplemented!("not ported: IncrementalInMemoryNodeEntryTest.pruneBeforeBuild")
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:338 IncrementalInMemoryNodeEntryTest.pruneAfterBuild
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn prune_after_build_partial_reevaluation() {
    unimplemented!("not ported: IncrementalInMemoryNodeEntryTest.pruneAfterBuild")
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:362 IncrementalInMemoryNodeEntryTest.noPruneWhenDetailsChange
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn no_prune_when_details_change_partial_reevaluation() {
    unimplemented!("not ported: IncrementalInMemoryNodeEntryTest.noPruneWhenDetailsChange")
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:402 IncrementalInMemoryNodeEntryTest.pruneWhenDepGroupReordered
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn prune_when_dep_group_reordered_partial_reevaluation() {
    unimplemented!("not ported: IncrementalInMemoryNodeEntryTest.pruneWhenDepGroupReordered")
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:445 IncrementalInMemoryNodeEntryTest.errorInfoCannotBePruned
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn error_info_cannot_be_pruned_partial_reevaluation() {
    unimplemented!("not ported: IncrementalInMemoryNodeEntryTest.errorInfoCannotBePruned")
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:473 IncrementalInMemoryNodeEntryTest.getDependencyGroup
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn get_dependency_group_partial_reevaluation() {
    unimplemented!("not ported: IncrementalInMemoryNodeEntryTest.getDependencyGroup")
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:498 IncrementalInMemoryNodeEntryTest.maintainDependencyGroupAfterRemoval
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn maintain_dependency_group_after_removal_partial_reevaluation() {
    unimplemented!(
        "not ported: IncrementalInMemoryNodeEntryTest.maintainDependencyGroupAfterRemoval"
    )
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:529 IncrementalInMemoryNodeEntryTest.pruneWhenDepsChange
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn prune_when_deps_change_partial_reevaluation() {
    unimplemented!("not ported: IncrementalInMemoryNodeEntryTest.pruneWhenDepsChange")
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:554 IncrementalInMemoryNodeEntryTest.checkDepsOneByOne
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn check_deps_one_by_one_partial_reevaluation() {
    unimplemented!("not ported: IncrementalInMemoryNodeEntryTest.checkDepsOneByOne")
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:582 IncrementalInMemoryNodeEntryTest.signalOnlyNewParents
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn signal_only_new_parents_partial_reevaluation() {
    unimplemented!("not ported: IncrementalInMemoryNodeEntryTest.signalOnlyNewParents")
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:598 IncrementalInMemoryNodeEntryTest.getCompressedDirectDepsForDoneEntry
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn get_compressed_direct_deps_for_done_entry_partial_reevaluation() {
    unimplemented!(
        "not ported: IncrementalInMemoryNodeEntryTest.getCompressedDirectDepsForDoneEntry"
    )
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:623 IncrementalInMemoryNodeEntryTest.hasAtLeastOneDep_true
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn has_at_least_one_dep_true_partial_reevaluation() {
    unimplemented!("not ported: IncrementalInMemoryNodeEntryTest.hasAtLeastOneDep_true")
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:637 IncrementalInMemoryNodeEntryTest.hasAtLeastOneDep_false
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn has_at_least_one_dep_false_partial_reevaluation() {
    unimplemented!("not ported: IncrementalInMemoryNodeEntryTest.hasAtLeastOneDep_false")
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:649 IncrementalInMemoryNodeEntryTest.getAllDirectDepsForIncompleteNode_notYetEvaluating
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn get_all_direct_deps_for_incomplete_node_not_yet_evaluating_partial_reevaluation() {
    unimplemented!(
        "not ported: IncrementalInMemoryNodeEntryTest.getAllDirectDepsForIncompleteNode_notYetEvaluating"
    )
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:655 IncrementalInMemoryNodeEntryTest.getAllDirectDepsForIncompleteNode_initialBuild
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn get_all_direct_deps_for_incomplete_node_initial_build_partial_reevaluation() {
    unimplemented!(
        "not ported: IncrementalInMemoryNodeEntryTest.getAllDirectDepsForIncompleteNode_initialBuild"
    )
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:668 IncrementalInMemoryNodeEntryTest.getAllDirectDepsForIncompleteNode_incrementalBuild
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn get_all_direct_deps_for_incomplete_node_incremental_build_partial_reevaluation() {
    unimplemented!(
        "not ported: IncrementalInMemoryNodeEntryTest.getAllDirectDepsForIncompleteNode_incrementalBuild"
    )
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:705 IncrementalInMemoryNodeEntryTest.getAllDirectDepsForIncompleteNode_afterReset
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn get_all_direct_deps_for_incomplete_node_after_reset_partial_reevaluation() {
    unimplemented!(
        "not ported: IncrementalInMemoryNodeEntryTest.getAllDirectDepsForIncompleteNode_afterReset"
    )
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:719 IncrementalInMemoryNodeEntryTest.resetOnDirtyNode
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn reset_on_dirty_node_partial_reevaluation() {
    unimplemented!("not ported: IncrementalInMemoryNodeEntryTest.resetOnDirtyNode")
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:799 IncrementalInMemoryNodeEntryTest.rewindOnIncrementalBuild
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn rewind_on_incremental_build_partial_reevaluation() {
    unimplemented!("not ported: IncrementalInMemoryNodeEntryTest.rewindOnIncrementalBuild")
}

// Java: src/test/java/com/google/devtools/build/skyframe/IncrementalInMemoryNodeEntryTest.java:885 IncrementalInMemoryNodeEntryTest.rewindOnDirtyNodeIgnored
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn rewind_on_dirty_node_ignored_partial_reevaluation() {
    unimplemented!("not ported: IncrementalInMemoryNodeEntryTest.rewindOnDirtyNodeIgnored")
}
