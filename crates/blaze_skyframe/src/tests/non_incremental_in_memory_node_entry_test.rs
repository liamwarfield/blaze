//! Port of `NonIncrementalInMemoryNodeEntryTest`
//! (src/test/java/com/google/devtools/build/skyframe/NonIncrementalInMemoryNodeEntryTest.java):
//! the shared `InMemoryNodeEntryTest` cases run on a `NonIncrementalInMemoryNodeEntry` at
//! `Version.constant()`. The class adds no test of its own.

use super::in_memory_node_entry_test::{self as shared, Kind};

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:83 InMemoryNodeEntryTest.entryAtStartOfEvaluation
#[test]
fn entry_at_start_of_evaluation_non_incremental() {
    shared::entry_at_start_of_evaluation(&Kind::non_incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:83 InMemoryNodeEntryTest.entryAtStartOfEvaluation
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn entry_at_start_of_evaluation_non_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.entryAtStartOfEvaluation")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:101 InMemoryNodeEntryTest.signalEntry
#[test]
fn signal_entry_non_incremental() {
    shared::signal_entry(&Kind::non_incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:101 InMemoryNodeEntryTest.signalEntry
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn signal_entry_non_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.signalEntry")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:136 InMemoryNodeEntryTest.signalExternalDep
#[test]
#[ignore = "external deps (NodeEntry.addExternalDep, Environment.dependOnFuture) are not ported"]
fn signal_external_dep_non_incremental() {
    unimplemented!("not ported: InMemoryNodeEntryTest.signalExternalDep")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:136 InMemoryNodeEntryTest.signalExternalDep
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn signal_external_dep_non_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.signalExternalDep")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:156 InMemoryNodeEntryTest.reverseDeps
#[test]
fn reverse_deps_non_incremental() {
    shared::reverse_deps(&Kind::non_incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:156 InMemoryNodeEntryTest.reverseDeps
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn reverse_deps_non_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.reverseDeps")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:179 InMemoryNodeEntryTest.errorValue
#[test]
fn error_value_non_incremental() {
    shared::error_value(&Kind::non_incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:179 InMemoryNodeEntryTest.errorValue
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn error_value_non_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.errorValue")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:196 InMemoryNodeEntryTest.errorAndValue
#[test]
fn error_and_value_non_incremental() {
    shared::error_and_value(&Kind::non_incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:196 InMemoryNodeEntryTest.errorAndValue
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn error_and_value_non_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.errorAndValue")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:211 InMemoryNodeEntryTest.crashOnNullErrorAndValue
#[test]
fn crash_on_null_error_and_value_non_incremental() {
    shared::crash_on_null_error_and_value(&Kind::non_incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:211 InMemoryNodeEntryTest.crashOnNullErrorAndValue
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn crash_on_null_error_and_value_non_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.crashOnNullErrorAndValue")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:221 InMemoryNodeEntryTest.crashOnTooManySignals
#[test]
fn crash_on_too_many_signals_non_incremental() {
    shared::crash_on_too_many_signals(&Kind::non_incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:221 InMemoryNodeEntryTest.crashOnTooManySignals
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn crash_on_too_many_signals_non_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.crashOnTooManySignals")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:229 InMemoryNodeEntryTest.crashOnSetValueWhenDone
#[test]
fn crash_on_set_value_when_done_non_incremental() {
    shared::crash_on_set_value_when_done(&Kind::non_incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:229 InMemoryNodeEntryTest.crashOnSetValueWhenDone
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn crash_on_set_value_when_done_non_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.crashOnSetValueWhenDone")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:241 InMemoryNodeEntryTest.crashOnAddReverseDepTwice
#[test]
fn crash_on_add_reverse_dep_twice_non_incremental() {
    shared::crash_on_add_reverse_dep_twice(&Kind::non_incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:241 InMemoryNodeEntryTest.crashOnAddReverseDepTwice
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn crash_on_add_reverse_dep_twice_non_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.crashOnAddReverseDepTwice")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:280 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups
#[test]
fn add_temporary_direct_deps_in_groups_non_incremental() {
    shared::add_temporary_direct_deps_in_groups(&Kind::non_incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:280 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn add_temporary_direct_deps_in_groups_non_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:298 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_toleratesEmpty
#[test]
fn add_temporary_direct_deps_in_groups_tolerates_empty_non_incremental() {
    shared::add_temporary_direct_deps_in_groups_tolerates_empty(&Kind::non_incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:298 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_toleratesEmpty
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn add_temporary_direct_deps_in_groups_tolerates_empty_non_incremental_partial_reevaluation() {
    unimplemented!(
        "not ported: InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_toleratesEmpty"
    )
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:306 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_toleratesGroupSizeOfZero
#[test]
fn add_temporary_direct_deps_in_groups_tolerates_group_size_of_zero_non_incremental() {
    shared::add_temporary_direct_deps_in_groups_tolerates_group_size_of_zero(
        &Kind::non_incremental(),
    );
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:306 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_toleratesGroupSizeOfZero
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn add_temporary_direct_deps_in_groups_tolerates_group_size_of_zero_non_incremental_partial_reevaluation()
 {
    unimplemented!(
        "not ported: InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_toleratesGroupSizeOfZero"
    )
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:314 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_notEnoughGroups_throws
#[test]
fn add_temporary_direct_deps_in_groups_not_enough_groups_throws_non_incremental() {
    shared::add_temporary_direct_deps_in_groups_not_enough_groups_throws(&Kind::non_incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:314 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_notEnoughGroups_throws
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn add_temporary_direct_deps_in_groups_not_enough_groups_throws_non_incremental_partial_reevaluation()
 {
    unimplemented!(
        "not ported: InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_notEnoughGroups_throws"
    )
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:324 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_tooManyGroups_throws
#[test]
fn add_temporary_direct_deps_in_groups_too_many_groups_throws_non_incremental() {
    shared::add_temporary_direct_deps_in_groups_too_many_groups_throws(&Kind::non_incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:324 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_tooManyGroups_throws
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn add_temporary_direct_deps_in_groups_too_many_groups_throws_non_incremental_partial_reevaluation()
{
    unimplemented!(
        "not ported: InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_tooManyGroups_throws"
    )
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:332 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_depsLeftOver_throws
#[test]
fn add_temporary_direct_deps_in_groups_deps_left_over_throws_non_incremental() {
    shared::add_temporary_direct_deps_in_groups_deps_left_over_throws(&Kind::non_incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:332 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_depsLeftOver_throws
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn add_temporary_direct_deps_in_groups_deps_left_over_throws_non_incremental_partial_reevaluation()
{
    unimplemented!(
        "not ported: InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_depsLeftOver_throws"
    )
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:343 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_depsExhausted_throws
#[test]
fn add_temporary_direct_deps_in_groups_deps_exhausted_throws_non_incremental() {
    shared::add_temporary_direct_deps_in_groups_deps_exhausted_throws(&Kind::non_incremental());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:343 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_depsExhausted_throws
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn add_temporary_direct_deps_in_groups_deps_exhausted_throws_non_incremental_partial_reevaluation()
{
    unimplemented!(
        "not ported: InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_depsExhausted_throws"
    )
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:354 InMemoryNodeEntryTest.resetLifecycle
#[test]
#[ignore = "rewinding and resetEvaluationFromScratch are not ported (docs/decisions.md 2026-11-19)"]
fn reset_lifecycle_non_incremental() {
    unimplemented!("not ported: InMemoryNodeEntryTest.resetLifecycle")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:354 InMemoryNodeEntryTest.resetLifecycle
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn reset_lifecycle_non_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.resetLifecycle")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:399 InMemoryNodeEntryTest.resetTwice_moreDepsRequestedBeforeFirstReset
#[test]
#[ignore = "rewinding and resetEvaluationFromScratch are not ported (docs/decisions.md 2026-11-19)"]
fn reset_twice_more_deps_requested_before_first_reset_non_incremental() {
    unimplemented!("not ported: InMemoryNodeEntryTest.resetTwice_moreDepsRequestedBeforeFirstReset")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:399 InMemoryNodeEntryTest.resetTwice_moreDepsRequestedBeforeFirstReset
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn reset_twice_more_deps_requested_before_first_reset_non_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.resetTwice_moreDepsRequestedBeforeFirstReset")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:443 InMemoryNodeEntryTest.rewindingLifecycle
#[test]
#[ignore = "rewinding and resetEvaluationFromScratch are not ported (docs/decisions.md 2026-11-19)"]
fn rewinding_lifecycle_non_incremental() {
    unimplemented!("not ported: InMemoryNodeEntryTest.rewindingLifecycle")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:443 InMemoryNodeEntryTest.rewindingLifecycle
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn rewinding_lifecycle_non_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.rewindingLifecycle")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:491 InMemoryNodeEntryTest.resetAfterRewind
#[test]
#[ignore = "rewinding and resetEvaluationFromScratch are not ported (docs/decisions.md 2026-11-19)"]
fn reset_after_rewind_non_incremental() {
    unimplemented!("not ported: InMemoryNodeEntryTest.resetAfterRewind")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:491 InMemoryNodeEntryTest.resetAfterRewind
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn reset_after_rewind_non_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.resetAfterRewind")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:555 InMemoryNodeEntryTest.concurrentRewindingAllowed
#[test]
#[ignore = "rewinding and resetEvaluationFromScratch are not ported (docs/decisions.md 2026-11-19)"]
fn concurrent_rewinding_allowed_non_incremental() {
    unimplemented!("not ported: InMemoryNodeEntryTest.concurrentRewindingAllowed")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:555 InMemoryNodeEntryTest.concurrentRewindingAllowed
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn concurrent_rewinding_allowed_non_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.concurrentRewindingAllowed")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:571 InMemoryNodeEntryTest.rewindErrorfulNode_toleratedButNoOp
#[test]
#[ignore = "rewinding and resetEvaluationFromScratch are not ported (docs/decisions.md 2026-11-19)"]
fn rewind_errorful_node_tolerated_but_no_op_non_incremental() {
    unimplemented!("not ported: InMemoryNodeEntryTest.rewindErrorfulNode_toleratedButNoOp")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:571 InMemoryNodeEntryTest.rewindErrorfulNode_toleratedButNoOp
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn rewind_errorful_node_tolerated_but_no_op_non_incremental_partial_reevaluation() {
    unimplemented!("not ported: InMemoryNodeEntryTest.rewindErrorfulNode_toleratedButNoOp")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:592 InMemoryNodeEntryTest.skipsBatchPrefetch_testTemporaryDepsContainsHashSet
#[test]
#[ignore = "SkyKey.skipsBatchPrefetch and GroupedDeps.WithHashSet are not ported (partial reevaluation)"]
fn skips_batch_prefetch_test_temporary_deps_contains_hash_set_non_incremental() {
    unimplemented!(
        "not ported: InMemoryNodeEntryTest.skipsBatchPrefetch_testTemporaryDepsContainsHashSet"
    )
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:592 InMemoryNodeEntryTest.skipsBatchPrefetch_testTemporaryDepsContainsHashSet
#[test]
#[ignore = "partial reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)"]
fn skips_batch_prefetch_test_temporary_deps_contains_hash_set_non_incremental_partial_reevaluation()
{
    unimplemented!(
        "not ported: InMemoryNodeEntryTest.skipsBatchPrefetch_testTemporaryDepsContainsHashSet"
    )
}
