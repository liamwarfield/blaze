//! Port of `runtime/TestSummary.java`.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_cmdline::Label;
use blaze_events::{Mode, Postable};
use blaze_protos::blaze::test_case::{Status as TestCaseStatus, Type as TestCaseType};
use blaze_protos::blaze::{BlazeTestStatus, FailedTestCasesStatus, TestCase};
use blaze_sky_exec::analysis_seam::TestParams;
use blaze_util::{check_state, unported};
use blaze_vfs::Path;

use crate::buildtool::build_result::{
    ConfiguredTargetRef, DetailedExitCodeRef, choose_more_important_with_first_if_tie,
};

/// The parts of `BuildConfigurationValue` the test summary classes read.
///
/// PORT: a trait so that tests can stand in for a configuration, as Java's tests mock it.
pub trait ConfigurationView: Send + Sync + fmt::Debug {
    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:814 BuildConfigurationValue.checksum
    fn checksum(&self) -> String;
    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:500 BuildConfigurationValue.getMnemonic
    fn get_mnemonic(&self) -> String;
}

impl ConfigurationView for BuildConfigurationValue {
    fn checksum(&self) -> String {
        BuildConfigurationValue::checksum(self).to_string()
    }

    fn get_mnemonic(&self) -> String {
        BuildConfigurationValue::get_mnemonic(self).to_string()
    }
}

/// `AliasProvider.getDependencyLabel(target)`: the first label of the alias chain, which is the
/// target's original label.
// Java: com/google/devtools/build/lib/analysis/AliasProvider.java:79 AliasProvider.getDependencyLabel
pub fn get_dependency_label(target: &ConfiguredTargetRef) -> Label {
    target.get_original_label()
}

/// `target.getProvider(TestProvider.class).getTestParams()`, which must exist.
// Java: com/google/devtools/build/lib/runtime/TestSummary.java:693 TestSummary.getTestParams
pub fn get_test_params(target: &ConfiguredTargetRef) -> TestParams {
    match target.get_test_params() {
        Some(p) => p,
        None => panic!("NullPointerException: {:?}", target),
    }
}

/// Builder class responsible for creating and altering TestSummary objects.
// Java: com/google/devtools/build/lib/runtime/TestSummary.java:68 TestSummary.Builder
pub struct TestSummaryBuilder {
    summary: Arc<TestSummary>,
    built: bool,
}

impl TestSummaryBuilder {
    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:72 Builder.<init>
    fn new(target: ConfiguredTargetRef) -> TestSummaryBuilder {
        TestSummaryBuilder {
            summary: Arc::new(TestSummary::new(target)),
            built: false,
        }
    }

    /// The summary under construction. Only called after [`Self::check_mutation`], so this
    /// builder holds the only reference.
    fn summary_mut(&mut self) -> &mut TestSummary {
        Arc::get_mut(&mut self.summary).expect("TestSummary mutated after build()")
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:77 Builder.mergeFrom
    pub fn merge_from(&mut self, existing_summary: &TestSummary) -> &mut Self {
        self.check_mutation();
        // Yuck, manually fill in fields.
        {
            let summary = self.summary_mut();
            for i in 0..existing_summary.shard_run_statuses.len() {
                summary.shard_run_statuses[i]
                    .extend(existing_summary.shard_run_statuses[i].iter().copied());
            }
            summary.first_start_time_millis = existing_summary.first_start_time_millis;
            summary.last_stop_time_millis = existing_summary.last_stop_time_millis;
            summary.total_run_duration_millis = existing_summary.total_run_duration_millis;
        }
        match &existing_summary.configuration {
            Some(c) => self.set_configuration(c.clone()),
            None => panic!("NullPointerException: configuration"),
        };
        match existing_summary.status {
            Some(s) => self.set_status(s),
            None => panic!("NullPointerException: status"),
        };
        self.add_coverage_files(&existing_summary.coverage_files);
        self.add_passed_logs(&existing_summary.passed_logs);
        self.add_failed_logs(&existing_summary.failed_logs);
        {
            let summary = self.summary_mut();
            summary.total_test_cases += existing_summary.total_test_cases;
            summary.total_unknown_test_cases += existing_summary.total_unknown_test_cases;
        }

        if let Some(status) = existing_summary.failed_test_cases_status {
            self.add_failed_test_cases(existing_summary.get_failed_test_cases(), status);
        }

        self.add_test_times(&existing_summary.test_times);
        self.add_warnings(&existing_summary.warnings);
        self.set_action_ran(existing_summary.action_ran);
        self.set_num_cached(existing_summary.num_cached);
        self.set_ran_remotely(existing_summary.ran_remotely);
        self.set_was_unreported_wrong_size(existing_summary.was_unreported_wrong_size);
        self.merge_system_failure(existing_summary.get_system_failure().cloned());
        self
    }

    // Implements copy on write logic, allowing reuse of the same builder.
    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:121 TestSummary.Builder.checkMutation
    fn check_mutation(&mut self) {
        // If mutating the builder after an object was built, create another copy.
        if self.built {
            self.built = false;
            let last_summary = self.summary.clone();
            self.summary = Arc::new(TestSummary::new(last_summary.target.clone()));
            self.merge_from(&last_summary);
        }
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:127 Builder.setConfiguration
    pub fn set_configuration(&mut self, configuration: Arc<dyn ConfigurationView>) -> &mut Self {
        self.check_mutation();
        self.summary_mut().configuration = Some(configuration);
        self
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:134 Builder.setStatus
    pub fn set_status(&mut self, status: BlazeTestStatus) -> &mut Self {
        self.check_mutation();
        self.summary_mut().status = Some(status);
        self
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:141 Builder.setSkipped
    pub fn set_skipped(&mut self, skipped: bool) -> &mut Self {
        self.check_mutation();
        self.summary_mut().skipped = skipped;
        self
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:148 Builder.addCoverageFiles
    pub fn add_coverage_files(&mut self, coverage_files: &[Path]) -> &mut Self {
        self.check_mutation();
        self.summary_mut()
            .coverage_files
            .extend(coverage_files.iter().cloned());
        self
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:155 Builder.addPassedLogs
    pub fn add_passed_logs(&mut self, passed_logs: &[Path]) -> &mut Self {
        self.check_mutation();
        self.summary_mut()
            .passed_logs
            .extend(passed_logs.iter().cloned());
        self
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:162 Builder.addPassedLog
    pub fn add_passed_log(&mut self, passed_log: Path) -> &mut Self {
        self.check_mutation();
        self.summary_mut().passed_logs.push(passed_log);
        self
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:169 Builder.addFailedLogs
    pub fn add_failed_logs(&mut self, failed_logs: &[Path]) -> &mut Self {
        self.check_mutation();
        self.summary_mut()
            .failed_logs
            .extend(failed_logs.iter().cloned());
        self
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:176 Builder.addFailedLog
    pub fn add_failed_log(&mut self, failed_log: Path) -> &mut Self {
        self.check_mutation();
        self.summary_mut().failed_logs.push(failed_log);
        self
    }

    /// PORT: Java mutates the summary without `checkMutation()`, which would also change a
    /// summary already returned by `build()`; a built summary is immutable here, so this copies
    /// it first.
    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:183 Builder.collectTestCases
    pub fn collect_test_cases(&mut self, test_case: Option<&TestCase>) -> &mut Self {
        self.check_mutation();
        // Maintain the invariant: failedTestCases + totalUnknownTestCases <= totalTestCases
        match test_case {
            None => {
                // If we don't have test case information, count each test as one case with
                // unknown status.
                let summary = self.summary_mut();
                summary.failed_test_cases_status = Some(FailedTestCasesStatus::NotAvailable);
                summary.total_test_cases += 1;
                summary.total_unknown_test_cases += 1;
            }
            Some(test_case) => {
                self.summary_mut().failed_test_cases_status = Some(FailedTestCasesStatus::Full);
                let n = self.traverse_test_cases(test_case);
                self.summary_mut().total_test_cases += n;
            }
        }
        self
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:197 Builder.traverseTestCases
    fn traverse_test_cases(&mut self, test_case: &TestCase) -> i32 {
        if !test_case.child.is_empty() {
            // This is a non-leaf result. Traverse its children, but do not add its
            // name to the output list. It should not contain any 'failure' or
            // 'error' tags, but we want to be lax here, because the syntax of the
            // test.xml file is also lax.
            // don't count container of test cases as test
            let mut res = 0;
            for child in &test_case.child {
                res += self.traverse_test_cases(child);
            }
            return res;
        } else if test_case.r#type() != TestCaseType::TestCase {
            return 0;
        }

        // This is a leaf result.
        if !test_case.run() {
            // Don't count test cases that were not run.
            return 0;
        }
        let summary = self.summary_mut();
        match test_case.status() {
            TestCaseStatus::Passed => summary.passed_test_cases.push(test_case.clone()),
            TestCaseStatus::Skipped => summary.skipped_test_cases.push(test_case.clone()),
            _ => summary.failed_test_cases.push(test_case.clone()),
        }

        1
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:227 Builder.addPassedTestCases
    pub fn add_passed_test_cases(&mut self, test_cases: &[TestCase]) -> &mut Self {
        self.check_mutation();
        self.summary_mut()
            .passed_test_cases
            .extend(test_cases.iter().cloned());
        self
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:234 Builder.addFailedTestCases
    pub fn add_failed_test_cases(
        &mut self,
        test_cases: &[TestCase],
        status: FailedTestCasesStatus,
    ) -> &mut Self {
        self.check_mutation();
        let summary = self.summary_mut();

        match summary.failed_test_cases_status {
            None => summary.failed_test_cases_status = Some(status),
            Some(s) if s != status => {
                summary.failed_test_cases_status = Some(FailedTestCasesStatus::Partial)
            }
            Some(_) => {}
        }

        if test_cases.is_empty() {
            return self;
        }

        // union of summary.failedTestCases, testCases
        // PORT: Java's TreeMap<String> orders by UTF-16 code units; test case names are compared
        // by bytes here, which agrees for the BMP below the surrogates.
        let mut all_cases: BTreeMap<String, TestCase> = BTreeMap::new();
        for detail in &summary.failed_test_cases {
            all_cases.insert(
                format!("{}.{}", detail.class_name(), detail.name()),
                detail.clone(),
            );
        }
        for detail in test_cases {
            all_cases.insert(
                format!("{}.{}", detail.class_name(), detail.name()),
                detail.clone(),
            );
        }

        summary.failed_test_cases = all_cases.into_values().collect();
        self
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:264 Builder.addTestTimes
    pub fn add_test_times(&mut self, test_times: &[i64]) -> &mut Self {
        self.check_mutation();
        self.summary_mut().test_times.extend_from_slice(test_times);
        self
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:271 Builder.mergeTiming
    pub fn merge_timing(&mut self, start_time_millis: i64, run_duration_millis: i64) -> &mut Self {
        self.check_mutation();
        let summary = self.summary_mut();
        summary.first_start_time_millis = summary.first_start_time_millis.min(start_time_millis);
        summary.last_stop_time_millis = summary
            .last_stop_time_millis
            .max(start_time_millis.wrapping_add(run_duration_millis));
        summary.total_run_duration_millis = summary
            .total_run_duration_millis
            .wrapping_add(run_duration_millis);
        self
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:281 Builder.addWarnings
    pub fn add_warnings(&mut self, warnings: &[String]) -> &mut Self {
        self.check_mutation();
        self.summary_mut().warnings.extend(warnings.iter().cloned());
        self
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:288 Builder.setActionRan
    pub fn set_action_ran(&mut self, action_ran: bool) -> &mut Self {
        self.check_mutation();
        self.summary_mut().action_ran = action_ran;
        self
    }

    /// Set the number of results cached, locally or remotely.
    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:301 Builder.setNumCached
    pub fn set_num_cached(&mut self, num_cached: i32) -> &mut Self {
        self.check_mutation();
        self.summary_mut().num_cached = num_cached;
        self
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:308 Builder.setNumLocalActionCached
    pub fn set_num_local_action_cached(&mut self, num_local_action_cached: i32) -> &mut Self {
        self.check_mutation();
        self.summary_mut().num_local_action_cached = num_local_action_cached;
        self
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:315 Builder.setRanRemotely
    pub fn set_ran_remotely(&mut self, ran_remotely: bool) -> &mut Self {
        self.check_mutation();
        self.summary_mut().ran_remotely = ran_remotely;
        self
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:322 Builder.setWasUnreportedWrongSize
    pub fn set_was_unreported_wrong_size(&mut self, was_unreported_wrong_size: bool) -> &mut Self {
        self.check_mutation();
        self.summary_mut().was_unreported_wrong_size = was_unreported_wrong_size;
        self
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:329 Builder.mergeSystemFailure
    pub fn merge_system_failure(
        &mut self,
        system_failure: Option<DetailedExitCodeRef>,
    ) -> &mut Self {
        self.check_mutation();
        let summary = self.summary_mut();
        summary.system_failure =
            choose_more_important_with_first_if_tie(summary.system_failure.take(), system_failure);
        self
    }

    /// Records a new result for the given shard of the test.
    ///
    /// Returns an immutable view of the statuses associated with the shard, with the new element.
    ///
    /// PORT: copies a built summary first (see [`Self::collect_test_cases`]).
    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:342 Builder.addShardStatus
    pub fn add_shard_status(
        &mut self,
        shard_number: i32,
        status: BlazeTestStatus,
    ) -> Vec<BlazeTestStatus> {
        self.check_mutation();
        let statuses = &mut self.summary_mut().shard_run_statuses[shard_number as usize];
        statuses.push(status);
        statuses.clone()
    }

    /// Records new attempts for the given shard of the target.
    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:350 Builder.addShardAttempts
    pub fn add_shard_attempts(&mut self, shard_number: i32, new_atttempts: i32) -> &mut Self {
        self.check_mutation();
        let summary = self.summary_mut();
        summary.shard_attempts[shard_number as usize] += new_atttempts;
        self
    }

    /// Returns the created TestSummary object. Any actions following a build() will create
    /// another copy of the same values. Since no mutators are provided directly by TestSummary, a
    /// copy will not be produced if two builds are invoked in a row without calling a setter.
    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:362 Builder.build
    pub fn build(&mut self) -> Arc<TestSummary> {
        self.peek();
        if !self.built {
            self.make_summary_immutable();
            // else: it is already immutable.
        }
        check_state!(self.built, "Built flag was not set");
        self.summary.clone()
    }

    /// Within-package, it is possible to read directly from an incompletely-built TestSummary.
    /// Used to pass Builders around directly.
    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:376 Builder.peek
    pub fn peek(&self) -> &TestSummary {
        if self.summary.status.is_none() {
            panic!("NullPointerException: Status cannot be null");
        }
        &self.summary
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:382 Builder.makeSummaryImmutable
    fn make_summary_immutable(&mut self) {
        // Once finalized, the list types are immutable.
        self.built = true;
    }
}

/// Test summary entry. Stores summary information for a single test rule. Also used to sort
/// summary output by status.
///
/// Invariant: All TestSummary mutations should be performed through the Builder. No direct
/// TestSummary methods (except the constructor) may mutate the object.
///
/// PORT: the `BuildEventWithOrderConstraint` side (`getEventId`, `postedAfter`,
/// `referencedLocalFiles`, `asStreamProto`) waits for the BEP port.
// Java: com/google/devtools/build/lib/runtime/TestSummary.java:64 TestSummary
#[derive(Clone)]
pub struct TestSummary {
    target: ConfiguredTargetRef,
    // Currently only populated if --runs_per_test_detects_flakes is enabled.
    shard_run_statuses: Vec<Vec<BlazeTestStatus>>,

    configuration: Option<Arc<dyn ConfigurationView>>,
    status: Option<BlazeTestStatus>,
    skipped: bool,
    shard_attempts: Vec<i32>,
    num_cached: i32,
    num_local_action_cached: i32,
    action_ran: bool,
    ran_remotely: bool,
    was_unreported_wrong_size: bool,
    failed_test_cases: Vec<TestCase>,
    passed_test_cases: Vec<TestCase>,
    skipped_test_cases: Vec<TestCase>,
    passed_logs: Vec<Path>,
    failed_logs: Vec<Path>,
    warnings: Vec<String>,
    coverage_files: Vec<Path>,
    test_times: Vec<i64>,
    total_run_duration_millis: i64,
    first_start_time_millis: i64,
    last_stop_time_millis: i64,
    failed_test_cases_status: Option<FailedTestCasesStatus>,
    total_test_cases: i32,
    total_unknown_test_cases: i32,
    system_failure: Option<DetailedExitCodeRef>,
}

impl TestSummary {
    // Don't allow public instantiation; go through the Builder.
    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:424 TestSummary.<init>
    fn new(target: ConfiguredTargetRef) -> TestSummary {
        let test_params = get_test_params(&target);
        let sz = test_params.shards.max(1) as usize;
        TestSummary {
            target,
            shard_run_statuses: Self::create_and_initialize(if test_params.runs_detects_flakes {
                sz
            } else {
                0
            }),
            configuration: None,
            status: None,
            skipped: false,
            shard_attempts: vec![0; sz],
            num_cached: 0,
            num_local_action_cached: 0,
            action_ran: false,
            ran_remotely: false,
            was_unreported_wrong_size: false,
            failed_test_cases: Vec::new(),
            passed_test_cases: Vec::new(),
            skipped_test_cases: Vec::new(),
            passed_logs: Vec::new(),
            failed_logs: Vec::new(),
            warnings: Vec::new(),
            coverage_files: Vec::new(),
            test_times: Vec::new(),
            total_run_duration_millis: 0,
            first_start_time_millis: i64::MAX,
            last_stop_time_millis: i64::MIN,
            failed_test_cases_status: None,
            total_test_cases: 0,
            total_unknown_test_cases: 0,
            system_failure: None,
        }
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:432 TestSummary.createAndInitialize
    fn create_and_initialize(sz: usize) -> Vec<Vec<BlazeTestStatus>> {
        (0..sz).map(|_| Vec::with_capacity(1)).collect()
    }

    /// Creates a new Builder allowing construction of a new TestSummary object.
    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:439 TestSummary.newBuilder
    pub fn new_builder(target: ConfiguredTargetRef) -> TestSummaryBuilder {
        TestSummaryBuilder::new(target)
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:443 TestSummary.getLabel
    pub fn get_label(&self) -> Label {
        get_dependency_label(&self.target)
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:447 TestSummary.getTarget
    pub fn get_target(&self) -> &ConfiguredTargetRef {
        &self.target
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:451 TestSummary.getConfiguration
    pub fn get_configuration(&self) -> Option<&Arc<dyn ConfigurationView>> {
        self.configuration.as_ref()
    }

    /// PORT: Java returns null before `setStatus`; every built summary has a status (`peek`
    /// checks it), so this panics like the NPE that would follow.
    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:455 TestSummary.getStatus
    pub fn get_status(&self) -> BlazeTestStatus {
        self.status.expect("NullPointerException: status")
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:459 TestSummary.isSkipped
    pub fn is_skipped(&self) -> bool {
        self.skipped
    }

    /// Whether or not any results associated with this test were cached locally or remotely.
    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:468 TestSummary.isCached
    pub fn is_cached(&self) -> bool {
        self.num_cached > 0
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:472 TestSummary.isLocalActionCached
    pub fn is_local_action_cached(&self) -> bool {
        self.num_local_action_cached > 0
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:476 TestSummary.numLocalActionCached
    pub fn num_local_action_cached(&self) -> i32 {
        self.num_local_action_cached
    }

    /// Number of results that were cached locally or remotely.
    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:483 TestSummary.numCached
    pub fn num_cached(&self) -> i32 {
        self.num_cached
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:487 TestSummary.numUncached
    fn num_uncached(&self) -> i32 {
        self.total_runs() - self.num_cached
    }

    /// Whether or not any action was taken for this test, that is there was some result that was
    /// *not cached*.
    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:497 TestSummary.actionRan
    pub fn action_ran(&self) -> bool {
        self.action_ran
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:501 TestSummary.ranRemotely
    pub fn ran_remotely(&self) -> bool {
        self.ran_remotely
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:505 TestSummary.wasUnreportedWrongSize
    pub fn was_unreported_wrong_size(&self) -> bool {
        self.was_unreported_wrong_size
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:509 TestSummary.getTotalTestCases
    pub fn get_total_test_cases(&self) -> i32 {
        self.total_test_cases
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:513 TestSummary.getUnknownTestCases
    pub fn get_unknown_test_cases(&self) -> i32 {
        self.total_unknown_test_cases
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:517 TestSummary.getFailedTestCases
    pub fn get_failed_test_cases(&self) -> &[TestCase] {
        &self.failed_test_cases
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:521 TestSummary.getSkippedTestCases
    pub fn get_skipped_test_cases(&self) -> &[TestCase] {
        &self.skipped_test_cases
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:525 TestSummary.getPassedTestCases
    pub fn get_passed_test_cases(&self) -> &[TestCase] {
        &self.passed_test_cases
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:529 TestSummary.getCoverageFiles
    pub fn get_coverage_files(&self) -> &[Path] {
        &self.coverage_files
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:533 TestSummary.getPassedLogs
    pub fn get_passed_logs(&self) -> &[Path] {
        &self.passed_logs
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:537 TestSummary.getFailedLogs
    pub fn get_failed_logs(&self) -> &[Path] {
        &self.failed_logs
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:541 TestSummary.getFailedTestCasesStatus
    pub fn get_failed_test_cases_status(&self) -> Option<FailedTestCasesStatus> {
        self.failed_test_cases_status
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:546 TestSummary.getSystemFailure
    pub fn get_system_failure(&self) -> Option<&DetailedExitCodeRef> {
        self.system_failure.as_ref()
    }

    /// Returns an immutable view of the warnings associated with this test.
    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:553 TestSummary.getWarnings
    pub fn get_warnings(&self) -> &[String] {
        &self.warnings
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:557 TestSummary.getSortKey
    fn get_sort_key(status: BlazeTestStatus) -> i32 {
        if status == BlazeTestStatus::Passed {
            -1
        } else {
            status as i32
        }
    }

    /// `compareTo`: the natural order of summaries, which is their output order.
    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:562 TestSummary.compareTo
    pub fn compare_to(&self, that: &TestSummary) -> Ordering {
        // compareTrueFirst
        that.is_cached()
            .cmp(&self.is_cached())
            .then_with(|| self.num_uncached().cmp(&that.num_uncached()))
            .then_with(|| {
                Self::get_sort_key(self.get_status()).cmp(&Self::get_sort_key(that.get_status()))
            })
            .then_with(|| self.get_label().cmp(&that.get_label()))
            .then_with(|| {
                let a = self.get_target().get_configuration_checksum();
                let b = that.get_target().get_configuration_checksum();
                match (a, b) {
                    (Some(a), Some(b)) => a.cmp(&b),
                    _ => panic!("NullPointerException: configuration checksum"),
                }
            })
            .then_with(|| {
                self.get_total_test_cases()
                    .cmp(&that.get_total_test_cases())
            })
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:587 TestSummary.getTestTimes
    pub fn get_test_times(&self) -> &[i64] {
        // The return result is unmodifiable (UnmodifiableList instance)
        &self.test_times
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:592 TestSummary.getNumAttempts
    pub fn get_num_attempts(&self) -> i32 {
        *self.shard_attempts.iter().max().unwrap()
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:596 TestSummary.getNumCached
    pub fn get_num_cached(&self) -> i32 {
        self.num_cached
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:600 TestSummary.totalRuns
    pub fn total_runs(&self) -> i32 {
        self.test_times.len() as i32
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:604 TestSummary.getTotalRunDurationMillis
    pub fn get_total_run_duration_millis(&self) -> i64 {
        self.total_run_duration_millis
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:608 TestSummary.getFirstStartTimeMillis
    pub fn get_first_start_time_millis(&self) -> i64 {
        self.first_start_time_millis
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:612 TestSummary.getLastStopTimeMillis
    pub fn get_last_stop_time_millis(&self) -> i64 {
        self.last_stop_time_millis
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:616 TestSummary.getStatusMode
    pub fn get_status_mode(&self) -> Mode {
        if self.skipped {
            return Mode::Warning;
        }
        if self.get_status() == BlazeTestStatus::Passed {
            Mode::Info
        } else if self.get_status() == BlazeTestStatus::Flaky {
            Mode::Warning
        } else {
            Mode::Error
        }
    }

    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:661 TestSummary.asStreamProto
    pub fn as_stream_proto(&self) -> ! {
        unported!("TestSummary.asStreamProto")
    }
}

impl fmt::Debug for TestSummary {
    // Java: com/google/devtools/build/lib/runtime/TestSummary.java:576 TestSummary.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TestSummary")
            .field("target", &self.target)
            .field("status", &self.status)
            .field("numCached", &self.num_cached)
            .field("numLocalActionCached", &self.num_local_action_cached)
            .field("actionRan", &self.action_ran)
            .field("ranRemotely", &self.ran_remotely)
            .finish()
    }
}

/// Posted on the event bus by `TestResultAggregator` (as the Java object itself).
impl Postable for TestSummary {}
