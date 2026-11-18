//! Port of `runtime/TestResultAggregator.java`: aggregates and reports target-wide test statuses
//! in real-time.

use std::sync::Arc;

use blaze_events::EventBus;
use blaze_exec::analysis::test::test_result::TestResult;
use blaze_optdefs::common::test_size::TestSize;
use blaze_optdefs::common::test_timeout::TestTimeout;
use blaze_protos::blaze::BlazeTestStatus;
use parking_lot::Mutex;

use super::test_summary::{
    ConfigurationView, TestSummary, TestSummaryBuilder, get_dependency_label, get_test_params,
};
use super::test_summary_printer::java_format_fixed;
use crate::buildtool::build_result::{ConfiguredTargetRef, DetailedExitCodeFactory};

/// Settings for the aggregator; there are usually many aggregator instances with the same set of
/// settings, so we move them to a separate object.
///
/// PORT: also carries the `DetailedExitCode` factory (see [`DetailedExitCodeFactory`]) that turns
/// a `TestResult`'s system failure (kept as its `FailureDetail`) back into a `DetailedExitCode`.
// Java: com/google/devtools/build/lib/runtime/TestResultAggregator.java:43 TestResultAggregator.AggregationPolicy
#[derive(Clone)]
pub struct AggregationPolicy {
    event_bus: Arc<dyn EventBus>,
    test_check_up_to_date: bool,
    test_verbose_timeout_warnings: bool,
    exit_codes: Arc<dyn DetailedExitCodeFactory>,
}

impl AggregationPolicy {
    // Java: com/google/devtools/build/lib/runtime/TestResultAggregator.java:48 AggregationPolicy.<init>
    pub fn new(
        event_bus: Arc<dyn EventBus>,
        test_check_up_to_date: bool,
        test_verbose_timeout_warnings: bool,
        exit_codes: Arc<dyn DetailedExitCodeFactory>,
    ) -> AggregationPolicy {
        AggregationPolicy {
            event_bus,
            test_check_up_to_date,
            test_verbose_timeout_warnings,
            exit_codes,
        }
    }
}

struct AggregatorState {
    summary: TestSummaryBuilder,
    remaining_runs: i32,
    summary_posted: bool,
}

/// This class aggregates and reports target-wide test statuses in real-time.
///
/// PORT: Java's `synchronized` methods lock a mutex around the mutable state.
// Java: com/google/devtools/build/lib/runtime/TestResultAggregator.java:38 TestResultAggregator
pub struct TestResultAggregator {
    policy: AggregationPolicy,
    state: Mutex<AggregatorState>,
}

impl TestResultAggregator {
    // Java: com/google/devtools/build/lib/runtime/TestResultAggregator.java:61 TestResultAggregator.<init>
    pub fn new(
        target: ConfiguredTargetRef,
        configuration: Arc<dyn ConfigurationView>,
        policy: AggregationPolicy,
        skipped_this_test: bool,
    ) -> TestResultAggregator {
        let remaining_runs = get_test_params(&target).test_status_artifacts.len() as i32;
        let mut summary = TestSummary::new_builder(target);
        summary
            .set_configuration(configuration)
            .set_status(BlazeTestStatus::NoStatus)
            .set_skipped(skipped_this_test);
        TestResultAggregator {
            policy,
            state: Mutex::new(AggregatorState {
                summary,
                remaining_runs,
                summary_posted: false,
            }),
        }
    }

    /// Records a new test run result and incrementally updates the target status. This event is
    /// sent upon completion of executed test runs.
    // Java: com/google/devtools/build/lib/runtime/TestResultAggregator.java:79 TestResultAggregator.testEvent
    pub fn test_event(&self, result: &TestResult) {
        let mut st = self.state.lock();
        // If a test result was cached, then post the cached attempts to the event bus.
        if result.is_cached() {
            for attempt in result.get_cached_test_attempts() {
                self.policy.event_bus.post(Arc::new(attempt));
            }
        }

        st.remaining_runs -= 1;
        if st.summary_posted {
            // This can happen if a buildCompleteEvent() was processed before this event reached
            // us. This situation is likely to happen if --notest_keep_going is set with multiple
            // targets.
            return;
        }

        self.incremental_analyze(&mut st, result);

        // If all runs are processed, the target is finished and ready to report.
        if st.remaining_runs == 0 {
            self.post_summary(&mut st);
        }
    }

    // Java: com/google/devtools/build/lib/runtime/TestResultAggregator.java:102 TestResultAggregator.postSummary
    fn post_summary(&self, st: &mut AggregatorState) -> Arc<TestSummary> {
        let result = st.summary.build();
        self.policy.event_bus.post(result.clone());
        st.summary_posted = true;
        result
    }

    // Java: com/google/devtools/build/lib/runtime/TestResultAggregator.java:109 TestResultAggregator.targetFailure
    pub fn target_failure(&self, blaze_halted: bool, skip_targets_on_failure: bool) {
        let mut st = self.state.lock();
        if st.summary_posted {
            // Blaze does not guarantee that BuildResult.getSuccessfulTargets() and posted
            // TestResult events are in sync. Thus, it is possible that a test event was posted,
            // but the target is not present in the set of successful targets.
            return;
        }

        self.mark_unbuilt(&mut st, blaze_halted, skip_targets_on_failure);

        // These are never going to run; removing them marks the target complete.
        self.post_summary(&mut st);
    }

    // Java: com/google/devtools/build/lib/runtime/TestResultAggregator.java:123 TestResultAggregator.targetSkipped
    pub fn target_skipped(&self) {
        let mut st = self.state.lock();
        if st.summary_posted {
            // Blaze does not guarantee that BuildResult.getSuccessfulTargets() and posted
            // TestResult events are in sync. Thus, it is possible that a test event was posted,
            // but the target is not present in the set of successful targets.
            return;
        }

        st.summary.set_status(BlazeTestStatus::NoStatus);

        // These are never going to run; removing them marks the target complete.
        self.post_summary(&mut st);
    }

    // Java: com/google/devtools/build/lib/runtime/TestResultAggregator.java:137 TestResultAggregator.aggregateStatus
    pub fn aggregate_status(status: BlazeTestStatus, other: BlazeTestStatus) -> BlazeTestStatus {
        if status as i32 > other as i32 {
            status
        } else {
            other
        }
    }

    /// Helper for differential analysis which aggregates the TestSummary for an individual target,
    /// reporting runs on the EventBus if necessary.
    // Java: com/google/devtools/build/lib/runtime/TestResultAggregator.java:145 TestResultAggregator.aggregateAndReportSummary
    pub fn aggregate_and_report_summary(&self, skip_targets_on_failure: bool) -> Arc<TestSummary> {
        let mut st = self.state.lock();
        // If already reported by the listener, no work remains for this target.
        if st.summary_posted {
            return st.summary.build(); // Reuses the same summary if nothing has changed.
        }

        // Build may have been interrupted.
        if st.remaining_runs > 0 {
            Self::mark_incomplete(&mut st, skip_targets_on_failure);
        }

        self.post_summary(&mut st)
    }

    /// Incrementally updates a TestSummary given an existing summary and a new TestResult. Only
    /// call on built targets.
    // Java: com/google/devtools/build/lib/runtime/TestResultAggregator.java:165 TestResultAggregator.incrementalAnalyze
    fn incremental_analyze(&self, st: &mut AggregatorState, result: &TestResult) {
        let summary = &mut st.summary;
        // Cache retrieval should have been performed already.
        let existing_summary = summary.peek();

        let mut status = existing_summary.get_status();
        let mut num_cached = existing_summary.num_cached();
        let mut num_local_action_cached = existing_summary.num_local_action_cached();
        let target = existing_summary.get_target().clone();

        let data = result.get_data();
        // If a test was neither cached locally nor remotely we say action was taken.
        if !(result.is_cached() || data.remotely_cached()) {
            summary.set_action_ran(true);
        } else {
            num_cached += 1;
        }

        if result.is_cached() {
            num_local_action_cached += 1;
        }

        if let Some(coverage_data) = result.get_coverage_data() {
            summary.add_coverage_files(&[coverage_data]);
        }

        let test_params = get_test_params(&target);

        let shard_number = result.get_shard_num();
        summary.add_shard_attempts(shard_number, data.test_times.len() as i32);

        if !test_params.runs_detects_flakes {
            status = Self::aggregate_status(status, data.status());
        } else {
            let runs_per_test_for_label = test_params.runs;
            let single_shard_statuses = summary.add_shard_status(shard_number, data.status());
            if single_shard_statuses.len() as i32 == runs_per_test_for_label {
                // Aggregation is based on the order of status enums where larger values take
                // precedence over smaller ones (NO_STATUS = 0, PASSED = 1, etc.). However, there
                // are some special cases:
                // 1. Tests that have some passing, some not passing shard are marked as flaky
                // 2. The INCOMPLETE status is ignored - it is used for tests runs that are
                //    cancelled by Bazel if --cancel_concurrent_tests is set, otherwise INCOMPLETE
                //    is not used
                // 3. Individual test shards can be FLAKY if the test is marked flaky and
                //    --flaky_test_attempts is not zero for this test
                let mut shard_status = BlazeTestStatus::NoStatus;
                let mut passes = 0;
                let mut cancelled = 0;
                for run_status_for_shard in single_shard_statuses {
                    if run_status_for_shard == BlazeTestStatus::Incomplete {
                        // If runs_per_test_detects_flakes is enabled, then INCOMPLETE status
                        // indicates cancelled test runs. We count them separately so that they
                        // don't result in a flaky status below.
                        cancelled += 1;
                    } else {
                        shard_status = Self::aggregate_status(shard_status, run_status_for_shard);
                        if TestResult::is_blaze_test_status_passed(run_status_for_shard) {
                            passes += 1;
                        }
                    }
                }
                // Under the RunsPerTestDetectsFlakes option, return flaky if 0 < p < (n-cancelled)
                // shards pass. Otherwise, we aggregate the shardStatus.
                if passes == 0 || (passes + cancelled) == runs_per_test_for_label {
                    status = Self::aggregate_status(status, shard_status);
                } else {
                    status = Self::aggregate_status(status, BlazeTestStatus::Flaky);
                }
            }
        }

        if let Some(passed_log) = &data.passed_log {
            summary.add_passed_log(result.get_test_log_path().get_relative_str(passed_log));
        }
        for path in &data.failed_logs {
            summary.add_failed_log(result.get_test_log_path().get_relative_str(path));
        }

        summary
            .add_test_times(&data.test_times)
            .merge_timing(data.start_time_millis_epoch(), data.run_duration_millis())
            .add_warnings(&data.warning)
            .collect_test_cases(data.test_case.as_ref())
            .set_ran_remotely(data.is_remote_strategy());

        let mut warnings: Vec<String> = Vec::new();
        if status == BlazeTestStatus::Passed
            && Self::should_emit_test_size_warning_in_summary(
                self.policy.test_verbose_timeout_warnings,
                &mut warnings,
                &data.test_process_times,
                &target,
            )
        {
            summary.set_was_unreported_wrong_size(true);
        }

        summary
            .merge_system_failure(
                result
                    .get_system_failure()
                    .map(|f| self.policy.exit_codes.of(f.clone())),
            )
            .set_status(status)
            .set_num_cached(num_cached)
            .set_num_local_action_cached(num_local_action_cached)
            .add_warnings(&warnings);
    }

    // Java: com/google/devtools/build/lib/runtime/TestResultAggregator.java:270 TestResultAggregator.markIncomplete
    fn mark_incomplete(st: &mut AggregatorState, skip_targets_on_failure: bool) {
        // TODO(bazel-team): (2010) Make NotRunTestResult support both tests failed to built and
        // tests with no status and post it here.
        let peek_summary = st.summary.peek();
        let mut status = peek_summary.get_status();
        if skip_targets_on_failure {
            status = BlazeTestStatus::NoStatus;
        } else if status != BlazeTestStatus::NoStatus {
            status = Self::aggregate_status(status, BlazeTestStatus::Incomplete);
        }

        st.summary.set_status(status);
    }

    // Java: com/google/devtools/build/lib/runtime/TestResultAggregator.java:284 TestResultAggregator.markUnbuilt
    fn mark_unbuilt(
        &self,
        st: &mut AggregatorState,
        blaze_halted: bool,
        skip_targets_on_failure: bool,
    ) {
        let run_status = if blaze_halted {
            BlazeTestStatus::BlazeHaltedBeforeTesting
        } else if self.policy.test_check_up_to_date || skip_targets_on_failure {
            BlazeTestStatus::NoStatus
        } else {
            BlazeTestStatus::FailedToBuild
        };

        st.summary.set_status(run_status);
    }

    /// Checks whether the specified test timeout could have been smaller or is too small and
    /// adds a warning message if verbose is true.
    ///
    /// Returns true if there was a test with the wrong timeout, but if was not reported.
    // Java: com/google/devtools/build/lib/runtime/TestResultAggregator.java:301 TestResultAggregator.shouldEmitTestSizeWarningInSummary
    fn should_emit_test_size_warning_in_summary(
        verbose: bool,
        warnings: &mut Vec<String>,
        test_times: &[i64],
        target: &ConfiguredTargetRef,
    ) -> bool {
        let specified_timeout = get_test_params(target).timeout;
        let mut max_time_of_shard: i64 = 0;

        for &shard_time in test_times {
            max_time_of_shard = max_time_of_shard.max(shard_time);
        }

        let max_time_in_seconds = (max_time_of_shard / 1000) as i32;

        if !specified_timeout.is_in_range_fuzzy(max_time_in_seconds) {
            let expected_timeout: TestTimeout =
                TestTimeout::get_suggested_test_timeout(max_time_in_seconds)
                    .expect("TestTimeout.getSuggestedTestTimeout");
            let expected_size = TestSize::get_test_size_for_timeout(expected_timeout);
            if verbose {
                // PORT: `String.format` without a locale; Bazel runs with an English locale.
                let mut builder = format!(
                    "{}: Test execution time ({}s excluding execution overhead) outside of \
                     range for {} tests. Consider setting timeout=\"{}\"",
                    get_dependency_label(target),
                    java_format_fixed(max_time_of_shard as f64 / 1000.0, 1),
                    specified_timeout.pretty_print(),
                    expected_timeout
                );
                if let Some(expected_size) = expected_size {
                    builder.push_str(&format!(" or size=\"{}\"", expected_size));
                }
                builder.push('.');
                warnings.push(builder);
                return false;
            }
            true
        } else {
            false
        }
    }
}
