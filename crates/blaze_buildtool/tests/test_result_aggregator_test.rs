//! Port of `src/test/java/com/google/devtools/build/lib/runtime/TestResultAggregatorTest.java`.
//!
//! Every case feeds `TestResult`s built around a Mockito-mocked `TestRunnerAction`; blaze_exec's
//! `TestRunnerAction` has no test double yet, so they are ignored.

#[test]
#[ignore = "needs a TestRunnerAction test double (Java mocks it)"]
fn non_cached_result_sets_action_ran_true() {}

#[test]
#[ignore = "needs a TestRunnerAction test double (Java mocks it)"]
fn locally_cached_test_sets_action_ran_false() {}

#[test]
#[ignore = "needs a TestRunnerAction test double (Java mocks it)"]
fn remotely_cached_test_sets_action_ran_false() {}

#[test]
#[ignore = "needs a TestRunnerAction test double (Java mocks it)"]
fn new_cached_result_keeps_action_ran_true_when_already_true() {}

#[test]
#[ignore = "needs a TestRunnerAction test double (Java mocks it)"]
fn timing_aggregation() {}

#[test]
#[ignore = "needs a TestRunnerAction test double (Java mocks it)"]
fn attempt_count_aggregates_single_shard_single_attempt() {}

#[test]
#[ignore = "needs a TestRunnerAction test double (Java mocks it)"]
fn attempt_count_aggregates_single_shard_multiple_attempts() {}

#[test]
#[ignore = "needs a TestRunnerAction test double (Java mocks it)"]
fn attempt_count_aggregates_multiple_shards_multiple_attempts() {}

#[test]
#[ignore = "needs a TestRunnerAction test double (Java mocks it)"]
fn attempt_count_aggregates_multiple_shards_single_shard_has_most_attempts() {}

#[test]
#[ignore = "needs a TestRunnerAction test double (Java mocks it)"]
fn cancel_concurrent_tests_cancellation_after_pass_ignored() {}

#[test]
#[ignore = "needs a TestRunnerAction test double (Java mocks it)"]
fn not_all_test_runs_reported_skip_targets_on_failure_no_status() {}

#[test]
#[ignore = "needs a TestRunnerAction test double (Java mocks it)"]
fn not_all_test_runs_reported_no_skip_targets_on_failure_incomplete() {}
