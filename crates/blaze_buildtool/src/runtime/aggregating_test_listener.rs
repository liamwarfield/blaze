//! Port of `runtime/AggregatingTestListener.java`: aggregates and reports target-wide test
//! statuses in real-time.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use blaze_cmdline::Label;
use blaze_events::EventBus;
use blaze_exec::analysis::test::test_result::TestResult;
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_optdefs::command::test_summary_options::TestSummaryOptions;
use blaze_protos::blaze::BlazeTestStatus;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use blaze_util::{check_state, unported};
use parking_lot::Mutex;

use super::terminal_test_result_notifier::TestResultNotifier;
use super::test_result_aggregator::{AggregationPolicy, TestResultAggregator};
use super::test_summary::{ConfigurationView, TestSummary};
use crate::buildtool::build_result::{
    BuildResult, ConfiguredTargetRef, DetailedExitCodeFactory, DetailedExitCodeRef,
    choose_more_important_with_first_if_tie, contains_target,
};

/// `ConfiguredTargetKey` as this listener uses it: the label and the configuration checksum.
///
/// PORT: `ConfiguredTargetKey` is ported in `blaze_analysis`, above this crate; the listener only
/// needs its identity (label + configuration key).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ConfiguredTargetKey {
    pub label: Label,
    pub configuration_checksum: Option<String>,
}

/// `AliasProvider.isAlias(target)`.
///
/// PORT: `ConfiguredTargetView` has no `AliasProvider`; an alias is a target whose original
/// label differs from its label.
// Java: com/google/devtools/build/lib/analysis/AliasProvider.java:154 AliasProvider.isAlias
fn is_alias(target: &ConfiguredTargetRef) -> bool {
    target.get_original_label() != target.get_label()
}

/// Aggregates and reports target-wide test statuses in real-time.
///
/// PORT: Java's `@Subscribe` methods take the event objects (`TestFilteringCompleteEvent`,
/// `TestAnalyzedEvent`, `BuildCompleteEvent`, `AnalysisFailureEvent`, `BuildInterruptedEvent`,
/// `TargetCompleteEvent`); here they are plain methods taking what the listener reads from them,
/// and the build driver's event bus calls them.
// Java: com/google/devtools/build/lib/runtime/AggregatingTestListener.java:55 AggregatingTestListener
pub struct AggregatingTestListener {
    summary_options: Arc<TestSummaryOptions>,
    execution_options: Arc<ExecutionOptions>,
    event_bus: Arc<dyn EventBus>,
    exit_codes: Arc<dyn DetailedExitCodeFactory>,
    blaze_halted: AtomicBool,

    // Store information about potential failures in the presence of --nokeep_going or
    // --notest_keep_going.
    skip_targets_on_failure: AtomicBool,

    aggregators: Mutex<HashMap<ConfiguredTargetKey, Arc<TestResultAggregator>>>,
}

impl AggregatingTestListener {
    /// `TESTS_FAILED_DETAILED_CODE`.
    // Java: com/google/devtools/build/lib/runtime/AggregatingTestListener.java:57 AggregatingTestListener.TESTS_FAILED_DETAILED_CODE
    fn tests_failed_detailed_code(&self) -> DetailedExitCodeRef {
        self.exit_codes.of(FailureDetail {
            message: "tests failed".to_string(),
            category: Some(Category::TestCommand(failure_details::TestCommand {
                code: failure_details::test_command::Code::TestsFailed as i32,
            })),
        })
    }

    /// PORT: `exit_codes` stands in for the static `DetailedExitCode` constructors (see
    /// [`DetailedExitCodeFactory`]).
    // Java: com/google/devtools/build/lib/runtime/AggregatingTestListener.java:75 AggregatingTestListener.<init>
    pub fn new(
        summary_options: Arc<TestSummaryOptions>,
        execution_options: Arc<ExecutionOptions>,
        event_bus: Arc<dyn EventBus>,
        exit_codes: Arc<dyn DetailedExitCodeFactory>,
    ) -> AggregatingTestListener {
        AggregatingTestListener {
            summary_options,
            execution_options,
            event_bus,
            exit_codes,
            blaze_halted: AtomicBool::new(false),
            skip_targets_on_failure: AtomicBool::new(false),
            aggregators: Mutex::new(HashMap::new()),
        }
    }

    fn new_policy(&self) -> AggregationPolicy {
        AggregationPolicy::new(
            self.event_bus.clone(),
            self.execution_options.test_check_up_to_date,
            self.summary_options.test_verbose_timeout_warnings,
            self.exit_codes.clone(),
        )
    }

    /// Populates the test summary map as soon as test filtering is complete. This is the earliest
    /// at which the final set of targets to test is known.
    ///
    /// This is used in the non-Skymeld case.
    ///
    /// PORT: takes `TestFilteringCompleteEvent`'s `getTestTargets()`,
    /// `getConfigurationForTarget` and `getSkippedTests()`.
    // Java: com/google/devtools/build/lib/runtime/AggregatingTestListener.java:92 AggregatingTestListener.populateTests
    pub fn populate_tests(
        &self,
        test_targets: &[ConfiguredTargetRef],
        configuration_for_target: &dyn Fn(&ConfiguredTargetRef) -> Arc<dyn ConfigurationView>,
        skipped_tests: &[ConfiguredTargetRef],
    ) {
        let policy = self.new_policy();
        // Add all target runs to the map, assuming 1:1 status artifact <-> result.
        for target in test_targets {
            if is_alias(target) {
                // It is safe to skip aliases because the actual target will be in
                // event.getTestTargets().
                continue;
            }
            let aggregator = Arc::new(TestResultAggregator::new(
                target.clone(),
                configuration_for_target(target),
                policy.clone(),
                contains_target(skipped_tests, target),
            ));
            let old_aggregator = self.aggregators.lock().insert(as_key(target), aggregator);
            check_state!(
                old_aggregator.is_none(),
                "target: {:?}, values: {} {}",
                target,
                "<old aggregator>",
                "<aggregator>"
            );
        }
    }

    /// Creates the `TestResultAggregator` for the analyzed test target.
    ///
    /// Since the event is fired from within a SkyFunction, it is possible to receive duplicate
    /// events. In case of duplication, simply return without creating any new aggregator.
    ///
    /// This is used in the Skymeld case.
    ///
    /// PORT: takes `TestAnalyzedEvent`'s `configuredTarget()`, `buildConfigurationValue()` and
    /// `isSkipped()`.
    // Java: com/google/devtools/build/lib/runtime/AggregatingTestListener.java:126 AggregatingTestListener.populateTest
    pub fn populate_test(
        &self,
        target: &ConfiguredTargetRef,
        build_configuration_value: Arc<dyn ConfigurationView>,
        is_skipped: bool,
    ) {
        // Even if target is an alias, we still need to ensure that there's an aggregator present.
        // Nothing guarantees that the actual target's TestAnalyzedEvent is posted before the
        // alias completes the test (b/419325593). Using computeIfAbsent ensures that we have a
        // single aggregator, as this method can be called concurrently for an alias and its
        // actual target.
        let mut aggregators = self.aggregators.lock();
        aggregators.entry(as_key(target)).or_insert_with(|| {
            if is_alias(target) {
                unported!("ConfiguredTarget.getActual");
            }
            Arc::new(TestResultAggregator::new(
                target.clone(), // In case target is an alias.
                build_configuration_value,
                self.new_policy(),
                is_skipped,
            ))
        });
    }

    /// Records a new test run result and incrementally updates the target status. This event is
    /// sent upon completion of executed test runs.
    // Java: com/google/devtools/build/lib/runtime/AggregatingTestListener.java:151 AggregatingTestListener.testEvent
    pub fn test_event(&self, result: &TestResult) {
        let test_action = result.get_test_action();
        let key = ConfiguredTargetKey {
            label: test_action
                .get_owner_label()
                .expect("TestRunnerAction owner label")
                .clone(),
            configuration_checksum: Some(test_action.get_configuration().checksum.clone()),
        };
        let aggregator = match self.aggregators.lock().get(&key) {
            Some(a) => a.clone(),
            None => panic!("NullPointerException: Missing aggregator for {:?}", key),
        };
        aggregator.test_event(result);
    }

    // Java: com/google/devtools/build/lib/runtime/AggregatingTestListener.java:163 AggregatingTestListener.targetFailure
    fn target_failure(&self, configured_target_key: &ConfiguredTargetKey) {
        let aggregator = self.aggregators.lock().get(configured_target_key).cloned();
        if let Some(aggregator) = aggregator {
            aggregator.target_failure(
                self.blaze_halted.load(Ordering::SeqCst),
                self.skip_targets_on_failure.load(Ordering::SeqCst),
            );
        }
    }

    // Java: com/google/devtools/build/lib/runtime/AggregatingTestListener.java:170 AggregatingTestListener.targetSkipped
    fn target_skipped(&self, configured_target_key: &ConfiguredTargetKey) {
        let aggregator = self.aggregators.lock().get(configured_target_key).cloned();
        if let Some(aggregator) = aggregator {
            aggregator.target_skipped();
        }
    }

    // Java: com/google/devtools/build/lib/runtime/AggregatingTestListener.java:178 AggregatingTestListener.buildComplete
    pub fn build_complete(
        &self,
        actual_targets: Option<&[ConfiguredTargetRef]>,
        skipped_targets: Option<&[ConfiguredTargetRef]>,
        successful_targets: Option<&[ConfiguredTargetRef]>,
    ) {
        let (Some(actual_targets), Some(successful_targets)) = (actual_targets, successful_targets)
        else {
            return;
        };
        let skipped_targets = match skipped_targets {
            Some(s) => s,
            None => panic!("NullPointerException: skippedTargets"),
        };

        // Sets.difference(ImmutableSet.copyOf(actualTargets), ImmutableSet.copyOf(successfulTargets))
        let mut non_successful_targets: Vec<ConfiguredTargetRef> = Vec::new();
        for target in actual_targets {
            if !contains_target(successful_targets, target)
                && !contains_target(&non_successful_targets, target)
            {
                non_successful_targets.push(target.clone());
            }
        }
        for target in &non_successful_targets {
            if contains_target(skipped_targets, target) {
                continue;
            }
            if is_alias(target) {
                continue;
            }
            self.target_failure(&as_key(target));
        }

        for target in skipped_targets {
            if is_alias(target) {
                continue;
            }
            self.target_skipped(&as_key(target));
        }
    }

    // Java: com/google/devtools/build/lib/runtime/AggregatingTestListener.java:207 AggregatingTestListener.buildCompleteEvent
    pub fn build_complete_event(&self, result: &BuildResult) {
        if result.was_catastrophe() {
            self.blaze_halted.store(true, Ordering::SeqCst);
        }
        self.skip_targets_on_failure
            .store(result.get_stop_on_first_failure(), Ordering::SeqCst);
        self.build_complete(
            result.get_actual_targets(),
            result.get_skipped_targets(),
            result.get_successful_targets(),
        );
    }

    /// PORT: takes `AnalysisFailureEvent.getFailedTarget()`.
    // Java: com/google/devtools/build/lib/runtime/AggregatingTestListener.java:218 AggregatingTestListener.analysisFailure
    pub fn analysis_failure(&self, failed_target: &ConfiguredTargetKey) {
        self.target_failure(failed_target);
    }

    // Java: com/google/devtools/build/lib/runtime/AggregatingTestListener.java:224 AggregatingTestListener.buildInterrupted
    pub fn build_interrupted(&self) {
        self.blaze_halted.store(true, Ordering::SeqCst);
    }

    /// Called when a build action is not executed (e.g. because a dependency failed to build). We
    /// want to catch such events in order to determine when a test target has failed to build.
    ///
    /// PORT: takes `TargetCompleteEvent.failed()` and `getConfiguredTargetKey()`.
    // Java: com/google/devtools/build/lib/runtime/AggregatingTestListener.java:234 AggregatingTestListener.targetComplete
    pub fn target_complete(&self, failed: bool, configured_target_key: &ConfiguredTargetKey) {
        if failed {
            self.target_failure(configured_target_key);
        }
    }

    /// Prints out the results of the given tests, and returns a `DetailedExitCode` summarizing
    /// those test results. Posts any targets which weren't already completed by the listener to
    /// the EventBus. Reports all targets on the console via the given notifier. Run at the end of
    /// the build, run only once.
    ///
    /// `test_targets`: the list of targets being run; `validated_targets`: targets with
    /// ValidateTarget aspect success or `None` if aspect not used; `notifier`: a console notifier
    /// to echo results to.
    // Java: com/google/devtools/build/lib/runtime/AggregatingTestListener.java:251 AggregatingTestListener.differentialAnalyzeAndReport
    pub fn differential_analyze_and_report(
        &self,
        test_targets: &[ConfiguredTargetRef],
        skipped_targets: &[ConfiguredTargetRef],
        validated_targets: Option<&[ConfiguredTargetKey]>,
        notifier: &dyn TestResultNotifier,
    ) -> DetailedExitCodeRef {
        let skip_targets_on_failure = self.skip_targets_on_failure.load(Ordering::SeqCst);
        // The natural ordering of the summaries defines their output order.
        // PORT: Java's TreeSet; a summary comparing equal to one already present is dropped.
        let mut summaries: Vec<Arc<TestSummary>> = Vec::new();

        let mut total_run = 0; // Number of targets running at least one non-cached test.
        let mut pass_count = 0;

        let mut system_failure: Option<DetailedExitCodeRef> = None;
        for test_target in test_targets {
            let key = as_key(test_target);
            let aggregator = match self.aggregators.lock().get(&key) {
                Some(a) => a.clone(),
                None => panic!(
                    "NullPointerException: Missing aggregator (key={:?}, testTarget={:?})",
                    key, test_target
                ),
            };
            let mut summary = if is_alias(test_target) {
                let mut summary_builder = TestSummary::new_builder(test_target.clone());
                summary_builder
                    .merge_from(&aggregator.aggregate_and_report_summary(skip_targets_on_failure));
                summary_builder.build()
            } else {
                aggregator.aggregate_and_report_summary(skip_targets_on_failure)
            };

            if let Some(validated_targets) = validated_targets
                && summary.get_status() != BlazeTestStatus::NoStatus
                && !validated_targets.contains(&key)
            {
                // Approximate what targetFailure() would do for test targets that failed
                // validation for the purposes of printing test results to console only. Note that
                // absent -k, targetFailure() ends up marking one test as FAILED_TO_BUILD before
                // buildComplete() marks the remaining targets NO_STATUS. While we could
                // approximate that, for simplicity, we just use NO_STATUS for all tests with
                // failed validations for simplicity here (absent -k). Events published on BEP are
                // not affected by this, but validation failures are published as separate events
                // and are additionally accounted in TargetSummary BEP messages.
                let mut summary_builder = TestSummary::new_builder(summary.get_target().clone());
                summary_builder.merge_from(&summary);
                summary_builder.set_status(if skip_targets_on_failure {
                    BlazeTestStatus::NoStatus
                } else {
                    TestResultAggregator::aggregate_status(
                        summary.get_status(),
                        BlazeTestStatus::FailedToBuild,
                    )
                });
                summary = summary_builder.build();
            }

            if let Err(pos) = summaries.binary_search_by(|s| s.compare_to(&summary)) {
                summaries.insert(pos, summary.clone());
            }

            // Finished aggregating; build the final console output.
            if summary.action_ran() {
                total_run += 1;
            }

            if TestResult::is_blaze_test_status_passed(summary.get_status()) {
                pass_count += 1;
            }

            system_failure = choose_more_important_with_first_if_tie(
                system_failure,
                summary.get_system_failure().cloned(),
            );
        }

        let summary_size = summaries.len();
        let test_targets_size = test_targets.len();
        check_state!(
            summary_size == test_targets_size,
            "Unequal sizes: {} vs {} ({:?} and {:?})",
            summary_size,
            test_targets_size,
            summaries,
            test_targets
        );

        notifier.notify(&summaries, total_run);

        if let Some(system_failure) = system_failure {
            return system_failure;
        }

        // skipped targets are not in passCount since they have NO_STATUS
        let mut not_skipped: Vec<ConfiguredTargetRef> = Vec::new();
        for target in test_targets {
            if !contains_target(skipped_targets, target) && !contains_target(&not_skipped, target) {
                not_skipped.push(target.clone());
            }
        }

        if pass_count == not_skipped.len() {
            self.exit_codes.success()
        } else {
            self.tests_failed_detailed_code()
        }
    }
}

// Java: com/google/devtools/build/lib/runtime/AggregatingTestListener.java:341 AggregatingTestListener.asKey
pub fn as_key(target: &ConfiguredTargetRef) -> ConfiguredTargetKey {
    ConfiguredTargetKey {
        label: target.get_label(),
        configuration_checksum: target.get_configuration_checksum(),
    }
}
