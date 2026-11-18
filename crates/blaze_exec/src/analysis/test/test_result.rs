//! Port of `analysis/test/TestResult.java`: the event passed from the test strategies to the
//! test listeners upon test completion.

use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_cmdline::Label;
use blaze_events::Postable;
use blaze_protos::blaze::{BlazeTestStatus, TestResultData};
use blaze_protos::build_event_stream::test_result::ExecutionInfo;
use blaze_protos::failure_details::FailureDetail;
use blaze_vfs::Path;

use super::test_action_context::TestOutputs;
use super::test_attempt::TestAttempt;
use super::test_runner_action::TestRunnerAction;

/// This is the event passed from the various test strategies to the `RecordingTestListener` upon
/// test completion.
// Java: com/google/devtools/build/lib/analysis/test/TestResult.java:42 TestResult
#[derive(Debug, Clone)]
pub struct TestResult {
    test_action: Arc<TestRunnerAction>,
    data: TestResultData,
    test_outputs: TestOutputs,
    cached: bool,
    exec_root: Option<Path>,
    /// `DetailedExitCode`, kept as its `FailureDetail`.
    system_failure: Option<FailureDetail>,
}

impl TestResult {
    /// Construct the TestResult for the given test / status.
    ///
    /// - `test_action`: The test that was run.
    /// - `data`: test result protobuffer.
    /// - `cached`: true if this is a locally cached test result.
    /// - `exec_root`: The execution root in which the action was carried out; can be `None`, in
    ///   which case everything depending on the execution root is ignored.
    /// - `system_failure`: Description of the system failure responsible for the test not
    ///   succeeding; `None` if no such failure occurred.
    // Java: com/google/devtools/build/lib/analysis/test/TestResult.java:62 TestResult.<init>
    pub fn new(
        test_action: Arc<TestRunnerAction>,
        data: TestResultData,
        test_outputs: TestOutputs,
        cached: bool,
        exec_root: Option<Path>,
        system_failure: Option<FailureDetail>,
    ) -> TestResult {
        TestResult {
            test_action,
            data,
            test_outputs,
            cached,
            exec_root,
            system_failure,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestResult.java:86 TestResult.isBlazeTestStatusPassed
    pub fn is_blaze_test_status_passed(status: BlazeTestStatus) -> bool {
        status == BlazeTestStatus::Passed || status == BlazeTestStatus::Flaky
    }

    /// Returns the test action.
    // Java: com/google/devtools/build/lib/analysis/test/TestResult.java:91 TestResult.getTestAction
    pub fn get_test_action(&self) -> &Arc<TestRunnerAction> {
        &self.test_action
    }

    /// Returns the test log path. Note, that actual log file may no longer correspond to this
    /// artifact. Use getActualLogPath() method if you need log location.
    // Java: com/google/devtools/build/lib/analysis/test/TestResult.java:99 TestResult.getTestLogPath
    pub fn get_test_log_path(&self) -> Path {
        let test_log_path = self.test_action.get_test_log().get_path();
        // If we have an exec root we'll use its fileSystem
        if let Some(exec_root) = &self.exec_root {
            let file_system = exec_root.get_file_system();
            return Path::create(test_log_path.as_fragment().clone(), file_system);
        }
        test_log_path
    }

    /// Returns whether the result was loaded from local action cache.
    // Java: com/google/devtools/build/lib/analysis/test/TestResult.java:110 TestResult.isCached
    pub fn is_cached(&self) -> bool {
        self.cached
    }

    /// Returns the list of locally cached test attempts. This method must only be called if
    /// [`Self::is_cached`] returns `true`.
    // Java: com/google/devtools/build/lib/analysis/test/TestResult.java:118 TestResult.getCachedTestAttempts
    pub fn get_cached_test_attempts(&self) -> Vec<TestAttempt> {
        blaze_util::check_state!(self.is_cached());
        vec![TestAttempt::from_cached_test_result(
            self.test_action.clone(),
            &self.data,
            1,
            self.test_outputs.clone(),
            ExecutionInfo::default(),
            /* last_attempt= */ true,
        )]
    }

    /// Returns the coverage data artifact, if available, and `None` otherwise.
    // Java: com/google/devtools/build/lib/analysis/test/TestResult.java:132 TestResult.getCoverageData
    pub fn get_coverage_data(&self) -> Option<Path> {
        if self.data.has_coverage() {
            return Some(
                self.test_action
                    .get_coverage_data()
                    .expect("NullPointerException: getCoverageData()")
                    .get_path(),
            );
        }
        None
    }

    /// Returns the test status artifact.
    // Java: com/google/devtools/build/lib/analysis/test/TestResult.java:140 TestResult.getTestStatusArtifact
    pub fn get_test_status_artifact(&self) -> &Artifact {
        // these artifacts are used to keep track of the number of pending and completed tests.
        self.test_action.get_cache_status_artifact()
    }

    /// Returns the test name in a user-friendly format. Will generally include the target name
    /// and shard number, if applicable.
    // Java: com/google/devtools/build/lib/analysis/test/TestResult.java:149 TestResult.getTestName
    pub fn get_test_name(&self) -> String {
        self.test_action.get_test_name()
    }

    /// Returns the test label.
    // Java: com/google/devtools/build/lib/analysis/test/TestResult.java:154 TestResult.getLabel
    pub fn get_label(&self) -> String {
        Label::print(self.test_action.get_owner_label())
    }

    /// Returns the test shard number.
    // Java: com/google/devtools/build/lib/analysis/test/TestResult.java:159 TestResult.getShardNum
    pub fn get_shard_num(&self) -> i32 {
        self.test_action.get_shard_num()
    }

    /// Returns the total number of test shards. 0 means no sharding, whereas 1 means degenerate
    /// sharding.
    // Java: com/google/devtools/build/lib/analysis/test/TestResult.java:167 TestResult.getTotalShards
    pub fn get_total_shards(&self) -> i32 {
        self.test_action.get_execution_settings().get_total_shards()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestResult.java:171 TestResult.getData
    pub fn get_data(&self) -> &TestResultData {
        &self.data
    }

    /// The test output files (`testOutputs`).
    pub fn get_test_outputs(&self) -> &TestOutputs {
        &self.test_outputs
    }

    /// Returns the description of the system failure responsible for the test not succeeding or
    /// `None` if no such failure occurred.
    // Java: com/google/devtools/build/lib/analysis/test/TestResult.java:180 TestResult.getSystemFailure
    pub fn get_system_failure(&self) -> Option<&FailureDetail> {
        self.system_failure.as_ref()
    }
}

impl Postable for TestResult {}
