//! Port of `analysis/test/TestAttempt.java`: the build event raised whenever an individual test
//! attempt is completed.
//!
//! PORT: the `BuildEvent` / `BuildEventWithOrderConstraint` interfaces belong to the build event
//! stream, which is not ported yet; their methods are inherent methods here, building the same
//! protos (`getEventId`, `postedAfter`, `getChildrenEvents`, `referencedLocalFiles`,
//! `asTestResult`). The `PathConverter` of `BuildEventContext` is a closure.

use std::sync::Arc;

use blaze_events::Postable;
use blaze_protos::blaze::{BlazeTestStatus, TestResultData};
use blaze_protos::build_event_stream::build_event_id::{
    self, ConfigurationId, TargetCompletedId, TestResultId,
};
use blaze_protos::build_event_stream::test_result::ExecutionInfo;
use blaze_protos::build_event_stream::{self as bes, BuildEventId, TestStatus, file};
use blaze_protos::google::protobuf::{Duration as ProtoDuration, Timestamp};
use blaze_vfs::Path;

use super::test_action_context::TestOutputs;
use super::test_runner_action::TestRunnerAction;

/// This event is raised whenever an individual test attempt is completed.
// Java: com/google/devtools/build/lib/analysis/test/TestAttempt.java:43 TestAttempt
#[derive(Debug, Clone)]
pub struct TestAttempt {
    test_action: Arc<TestRunnerAction>,
    status: TestStatus,
    status_details: String,
    cached_locally: bool,
    attempt: i32,
    last_attempt: bool,
    files: TestOutputs,
    test_warnings: Vec<String>,
    duration_millis: i64,
    start_time_millis: i64,
    execution_info: ExecutionInfo,
}

/// `BuildEvent.LocalFile.LocalFileType` values a test attempt references.
// Java: com/google/devtools/build/lib/buildeventstream/BuildEvent.java:53 BuildEvent.LocalFile.LocalFileType
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestAttemptLocalFileType {
    SuccessfulTestOutput,
    FailedTestOutput,
}

/// `BuildEventStreamerUtils.bepStatus`.
// Java: com/google/devtools/build/lib/runtime/BuildEventStreamerUtils.java:30 BuildEventStreamerUtils.bepStatus
pub fn bep_status(status: BlazeTestStatus) -> TestStatus {
    match status {
        BlazeTestStatus::NoStatus => TestStatus::NoStatus,
        BlazeTestStatus::Passed => TestStatus::Passed,
        BlazeTestStatus::Flaky => TestStatus::Flaky,
        BlazeTestStatus::Failed => TestStatus::Failed,
        BlazeTestStatus::Timeout => TestStatus::Timeout,
        BlazeTestStatus::FailedToBuild => TestStatus::FailedToBuild,
        BlazeTestStatus::Incomplete => TestStatus::Incomplete,
        BlazeTestStatus::RemoteFailure => TestStatus::RemoteFailure,
        BlazeTestStatus::BlazeHaltedBeforeTesting => TestStatus::ToolHaltedBeforeTesting,
    }
}

/// `Timestamps.fromMillis`.
pub fn timestamp_from_millis(millis: i64) -> Timestamp {
    Timestamp {
        seconds: millis.div_euclid(1000),
        nanos: (millis.rem_euclid(1000) * 1_000_000) as i32,
    }
}

/// `Durations.fromMillis`.
pub fn duration_from_millis(millis: i64) -> ProtoDuration {
    // Durations.normalizedDuration(seconds, nanos): the sign of nanos follows the sign of seconds.
    ProtoDuration {
        seconds: millis / 1000,
        nanos: ((millis % 1000) * 1_000_000) as i32,
    }
}

/// `BuildConfigurationValue.configurationId(configuration)` (`getEventId()`).
fn configuration_id(test_action: &TestRunnerAction) -> ConfigurationId {
    ConfigurationId {
        id: test_action.get_configuration().checksum.clone(),
    }
}

/// `BuildEventIdUtil.testResult(target, run, shard, attempt, configuration)`.
// Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:236 BuildEventIdUtil.testResult
pub fn test_result_event_id(
    target: &str,
    run: i32,
    shard: i32,
    attempt: i32,
    configuration: ConfigurationId,
) -> BuildEventId {
    BuildEventId {
        id: Some(build_event_id::Id::TestResult(TestResultId {
            label: target.to_string(),
            configuration: Some(configuration),
            run: run + 1,
            shard: shard + 1,
            attempt,
        })),
    }
}

impl TestAttempt {
    /// Construct the event given the test action and attempt number.
    ///
    /// `cached_locally`: True if the reported attempt is taken from the tool's local cache.
    // Java: com/google/devtools/build/lib/analysis/test/TestAttempt.java:64 TestAttempt.<init>
    fn new(
        cached_locally: bool,
        test_action: Arc<TestRunnerAction>,
        execution_info: ExecutionInfo,
        attempt: i32,
        status: BlazeTestStatus,
        status_details: String,
        start_time_millis: i64,
        duration_millis: i64,
        files: TestOutputs,
        test_warnings: Vec<String>,
        last_attempt: bool,
    ) -> TestAttempt {
        TestAttempt {
            test_action,
            execution_info,
            attempt,
            status: bep_status(status),
            status_details,
            cached_locally,
            start_time_millis,
            duration_millis,
            files,
            test_warnings,
            last_attempt,
        }
    }

    /// Creates a test attempt result instance for a test that was not locally cached; it may have
    /// been locally executed, remotely executed, or remotely cached.
    // Java: com/google/devtools/build/lib/analysis/test/TestAttempt.java:93 TestAttempt.forExecutedTestResult
    pub fn for_executed_test_result(
        test_action: Arc<TestRunnerAction>,
        attempt_data: &TestResultData,
        attempt: i32,
        files: TestOutputs,
        execution_info: ExecutionInfo,
        last_attempt: bool,
    ) -> TestAttempt {
        TestAttempt::new(
            false,
            test_action,
            execution_info,
            attempt,
            attempt_data.status(),
            attempt_data.status_details().to_string(),
            attempt_data.start_time_millis_epoch(),
            attempt_data.run_duration_millis(),
            files,
            attempt_data.warning.clone(),
            last_attempt,
        )
    }

    /// Creates a test attempt result from cached test data, providing a result while indicating
    /// to consumers that the test did not actually execute.
    // Java: com/google/devtools/build/lib/analysis/test/TestAttempt.java:118 TestAttempt.fromCachedTestResult
    pub fn from_cached_test_result(
        test_action: Arc<TestRunnerAction>,
        attempt_data: &TestResultData,
        attempt: i32,
        files: TestOutputs,
        execution_info: ExecutionInfo,
        last_attempt: bool,
    ) -> TestAttempt {
        TestAttempt::new(
            true,
            test_action,
            execution_info,
            attempt,
            attempt_data.status(),
            attempt_data.status_details().to_string(),
            attempt_data.start_time_millis_epoch(),
            attempt_data.run_duration_millis(),
            files,
            attempt_data.warning.clone(),
            last_attempt,
        )
    }

    /// Creates a test result for rare cases where the test itself was built, but the
    /// `TestRunnerAction` could not be started by a test strategy.
    ///
    /// This overload should be very rarely used, and in particular must not be used by an
    /// implementation of a `TestStrategy`.
    // Java: com/google/devtools/build/lib/analysis/test/TestAttempt.java:146 TestAttempt.forUnstartableTestResult
    pub fn for_unstartable_test_result(
        test_action: Arc<TestRunnerAction>,
        attempt_data: &TestResultData,
    ) -> TestAttempt {
        TestAttempt::new(
            false,
            test_action,
            ExecutionInfo::default(),
            /* attempt= */ 1,
            attempt_data.status(),
            attempt_data.status_details().to_string(),
            attempt_data.start_time_millis_epoch(),
            attempt_data.run_duration_millis(),
            /* files= */ Vec::new(),
            attempt_data.warning.clone(),
            /* last_attempt= */ true,
        )
    }

    /// The test action this attempt belongs to.
    pub fn get_test_action(&self) -> &Arc<TestRunnerAction> {
        &self.test_action
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestAttempt.java:163 TestAttempt.getTestStatusArtifact
    pub fn get_test_status_artifact(&self) -> &blaze_actions::Artifact {
        self.test_action.get_cache_status_artifact()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestAttempt.java:168 TestAttempt.getFiles
    pub fn get_files(&self) -> &TestOutputs {
        &self.files
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestAttempt.java:173 TestAttempt.getExecutionInfo
    pub fn get_execution_info(&self) -> &ExecutionInfo {
        &self.execution_info
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestAttempt.java:178 TestAttempt.getStatus
    pub fn get_status(&self) -> TestStatus {
        self.status
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestAttempt.java:183 TestAttempt.isCachedLocally
    pub fn is_cached_locally(&self) -> bool {
        self.cached_locally
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestAttempt.java:188 TestAttempt.getAttempt
    pub fn get_attempt(&self) -> i32 {
        self.attempt
    }

    /// Whether this is the last attempt of its test run.
    pub fn is_last_attempt(&self) -> bool {
        self.last_attempt
    }

    fn label_string(&self) -> String {
        self.test_action
            .get_owner_label()
            .map(|l| l.to_string())
            .unwrap_or_else(|| "null".to_string())
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestAttempt.java:193 TestAttempt.getEventId
    pub fn get_event_id(&self) -> BuildEventId {
        test_result_event_id(
            &self.label_string(),
            self.test_action.get_run_number(),
            self.test_action.get_shard_num(),
            self.attempt,
            configuration_id(&self.test_action),
        )
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestAttempt.java:203 TestAttempt.postedAfter
    pub fn posted_after(&self) -> Vec<BuildEventId> {
        // Java: com/google/devtools/build/lib/buildeventstream/BuildEventIdUtil.java:175 BuildEventIdUtil.targetCompleted
        vec![BuildEventId {
            id: Some(build_event_id::Id::TargetCompleted(TargetCompletedId {
                label: self.label_string(),
                configuration: Some(configuration_id(&self.test_action)),
                aspect: String::new(),
            })),
        }]
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestAttempt.java:210 TestAttempt.getChildrenEvents
    pub fn get_children_events(&self) -> Vec<BuildEventId> {
        if self.last_attempt {
            Vec::new()
        } else {
            vec![test_result_event_id(
                &self.label_string(),
                self.test_action.get_run_number(),
                self.test_action.get_shard_num(),
                self.attempt + 1,
                configuration_id(&self.test_action),
            )]
        }
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestAttempt.java:225 TestAttempt.referencedLocalFiles
    pub fn referenced_local_files(&self) -> Vec<(Path, TestAttemptLocalFileType)> {
        let local_file_type = if self.status == TestStatus::Passed {
            TestAttemptLocalFileType::SuccessfulTestOutput
        } else {
            TestAttemptLocalFileType::FailedTestOutput
        };
        // TODO(b/199940216): Can we populate metadata for these files?
        self.files
            .iter()
            .map(|(_, path)| (path.clone(), local_file_type))
            .collect()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestAttempt.java:246 TestAttempt.asTestResult
    pub fn as_test_result(
        &self,
        path_converter: &dyn Fn(&Path) -> Option<String>,
    ) -> bes::TestResult {
        let mut builder = bes::TestResult {
            status: self.status as i32,
            status_details: self.status_details.clone(),
            execution_info: Some(self.execution_info.clone()),
            cached_locally: self.cached_locally,
            ..Default::default()
        };
        if self.start_time_millis != 0 {
            builder.test_attempt_start = Some(timestamp_from_millis(self.start_time_millis));
        }
        #[allow(deprecated)]
        {
            builder.test_attempt_start_millis_epoch = self.start_time_millis;
        }
        if self.duration_millis != 0 {
            builder.test_attempt_duration = Some(duration_from_millis(self.duration_millis));
        }
        #[allow(deprecated)]
        {
            builder.test_attempt_duration_millis = self.duration_millis;
        }
        builder.warning.extend(self.test_warnings.iter().cloned());
        for (name, path) in &self.files {
            if let Some(uri) = path_converter(path) {
                builder.test_action_output.push(bes::File {
                    name: name.clone(),
                    file: Some(file::File::Uri(uri)),
                    ..Default::default()
                });
            }
        }
        builder
    }
}

impl Postable for TestAttempt {}
