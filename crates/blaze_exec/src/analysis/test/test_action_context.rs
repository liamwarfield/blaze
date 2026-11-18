//! Port of `analysis/test/TestActionContext.java`: the context for the execution of test actions
//! (`TestRunnerAction`), with its nested `AttemptGroup`, `TestAttemptResult`,
//! `ProcessedAttemptResult` and `TestRunnerSpawn` interfaces.

use std::any::Any;
use std::io;
use std::sync::Arc;

use blaze_actions::ActionOwner;
use blaze_protos::blaze::TestResultData;
use blaze_protos::failure_details::FailureDetail;
use blaze_vfs::Path;

use super::test_result::TestResult;
use super::test_runner_action::TestRunnerAction;
use crate::actions::{ExecException, SpawnResult, Status};
use crate::exec::action_execution_context::ActionExecutionContext;

/// `ImmutableMultimap<String, Path>` of test output files (name constant to path), in insertion
/// order.
pub type TestOutputs = Vec<(String, Path)>;

/// The `throws IOException, ExecException, InterruptedException` of the `TestRunnerSpawn`
/// methods. `InterruptedException` is [`ExecException::Interrupted`].
#[derive(Debug)]
pub enum TestRunnerSpawnError {
    Exec(ExecException),
    Io(io::Error),
}

impl From<ExecException> for TestRunnerSpawnError {
    fn from(e: ExecException) -> Self {
        TestRunnerSpawnError::Exec(e)
    }
}

impl From<io::Error> for TestRunnerSpawnError {
    fn from(e: io::Error) -> Self {
        TestRunnerSpawnError::Io(e)
    }
}

/// A group of attempts for a single test shard, ran either sequentially or in parallel.
///
/// When one attempt matches the result specified by `--experimental_cancel_concurrent_tests`,
/// threads running the other attempts get an `InterruptedException` and `cancelled()` will in
/// the future return true. When a thread joins an attempt group that is already cancelled,
/// `InterruptedException` will be thrown on the call to `register()`.
// Java: com/google/devtools/build/lib/analysis/test/TestActionContext.java:43 TestActionContext.AttemptGroup
pub trait AttemptGroup: Send + Sync {
    /// Registers a thread to the attempt group.
    ///
    /// If the attempt group is already cancelled, throw `InterruptedException`
    /// ([`ExecException::Interrupted`]).
    fn register(&self) -> Result<(), ExecException>;

    /// Unregisters a thread from the attempt group.
    fn unregister(&self);

    /// Signal that the attempt run by this thread has the desired result and cancel all the
    /// others.
    fn cancel_others(&self);

    /// Whether the attempt group has been cancelled.
    fn cancelled(&self) -> bool;
}

/// A dummy attempt group used when no flaky test attempt cancellation is done.
// Java: com/google/devtools/build/lib/analysis/test/TestActionContext.java:64 TestActionContext.AttemptGroup.NOOP
#[derive(Debug, Default)]
pub struct NoopAttemptGroup;

impl AttemptGroup for NoopAttemptGroup {
    fn register(&self) -> Result<(), ExecException> {
        Ok(())
    }
    fn unregister(&self) {}
    fn cancel_others(&self) {}
    fn cancelled(&self) -> bool {
        false
    }
}

/// `AttemptGroup.NOOP`.
pub fn attempt_group_noop() -> Arc<dyn AttemptGroup> {
    static NOOP: std::sync::OnceLock<Arc<dyn AttemptGroup>> = std::sync::OnceLock::new();
    NOOP.get_or_init(|| Arc::new(NoopAttemptGroup)).clone()
}

/// A context for the execution of test actions (`TestRunnerAction`).
///
/// Register it with the action context registry as `Arc<Arc<dyn TestActionContext>>` (that is,
/// `get_context::<Arc<dyn TestActionContext>>(ctx)` must find it).
// Java: com/google/devtools/build/lib/analysis/test/TestActionContext.java:32 TestActionContext
pub trait TestActionContext: Send + Sync {
    // Java: com/google/devtools/build/lib/analysis/test/TestActionContext.java:82 TestActionContext.createTestRunnerSpawn
    fn create_test_runner_spawn<'a>(
        &'a self,
        test_runner_action: &'a TestRunnerAction,
        action_execution_context: &'a dyn ActionExecutionContext,
    ) -> Result<Box<dyn TestRunnerSpawn<'a> + 'a>, ExecException>;

    /// Returns whether test_keep_going is enabled.
    // Java: com/google/devtools/build/lib/analysis/test/TestActionContext.java:87 TestActionContext.isTestKeepGoing
    fn is_test_keep_going(&self) -> bool;

    /// Returns `true` to indicate that exclusive tests should be treated as regular parallel
    /// tests.
    ///
    /// Returning `true` may make sense for certain forced remote test execution strategies where
    /// running tests in sequence would be wasteful.
    // Java: com/google/devtools/build/lib/analysis/test/TestActionContext.java:96 TestActionContext.forceExclusiveTestsInParallel
    fn force_exclusive_tests_in_parallel(&self) -> bool {
        false
    }

    /// Returns `true` to indicate that "exclusive-if-local" tests should be treated as regular
    /// parallel tests.
    ///
    /// Returning `true` may make sense for certain remote test execution strategies where running
    /// tests in sequence would be wasteful.
    // Java: com/google/devtools/build/lib/analysis/test/TestActionContext.java:107 TestActionContext.forceExclusiveIfLocalTestsInParallel
    fn force_exclusive_if_local_tests_in_parallel(&self) -> bool {
        false
    }

    /// Creates a cached test result.
    // Java: com/google/devtools/build/lib/analysis/test/TestActionContext.java:112 TestActionContext.newCachedTestResult
    fn new_cached_test_result(
        &self,
        exec_root: &Path,
        action: &TestRunnerAction,
        cached: TestResultData,
        test_outputs: TestOutputs,
    ) -> io::Result<TestResult>;

    /// Returns the attempt group associated with the given shard.
    // Java: com/google/devtools/build/lib/analysis/test/TestActionContext.java:120 TestActionContext.getAttemptGroup
    fn get_attempt_group(&self, owner: &ActionOwner, shard_num: i32) -> Arc<dyn AttemptGroup>;
}

/// Test attempt result classification, splitting failures into permanent vs retriable.
// Java: com/google/devtools/build/lib/analysis/test/TestActionContext.java:125 TestActionContext.TestAttemptResult.Result
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TestAttemptResultResult {
    /// Test attempt successful.
    Passed,
    /// Test failed, potentially due to test flakiness, can be retried.
    FailedCanRetry,
    /// Permanent failure.
    Failed,
}

impl TestAttemptResultResult {
    // Java: com/google/devtools/build/lib/analysis/test/TestActionContext.java:133 TestActionContext.TestAttemptResult.Result.canRetry
    pub fn can_retry(self) -> bool {
        self == TestAttemptResultResult::FailedCanRetry
    }
}

/// An individual test attempt result.
// Java: com/google/devtools/build/lib/analysis/test/TestActionContext.java:123 TestActionContext.TestAttemptResult
pub trait TestAttemptResult: Send + Sync + Any {
    /// Returns the overall test result.
    fn result(&self) -> TestAttemptResultResult;

    /// Returns a list of spawn results for this test attempt.
    fn spawn_results(&self) -> &[SpawnResult];

    /// Returns a description of the system failure associated with the primary spawn result, if
    /// any (`DetailedExitCode.of(failureDetail)`, kept as its `FailureDetail`).
    // Java: com/google/devtools/build/lib/analysis/test/TestActionContext.java:148 TestActionContext.TestAttemptResult.primarySystemFailure
    fn primary_system_failure(&self) -> Option<FailureDetail> {
        let spawn_results = self.spawn_results();
        if spawn_results.is_empty() {
            return None;
        }
        let primary_spawn_result = &spawn_results[0];
        if primary_spawn_result.status() == Status::Success {
            return None;
        }
        if primary_spawn_result.status().is_considered_user_error() {
            return None;
        }
        // PORT: DetailedExitCode.of(null) throws in Java; a non-success spawn always carries a
        // failure detail.
        Some(
            primary_spawn_result
                .failure_detail()
                .cloned()
                .expect("NullPointerException: SpawnResult.failureDetail()"),
        )
    }

    /// For downcasting to the implementation's class.
    fn as_any(&self) -> &dyn Any;
}

/// The result of passing a [`TestAttemptResult`] through
/// [`TestRunnerSpawn::finalize_failed_test_attempt`], which is later passed to
/// [`TestRunnerSpawn::finalize_test`] or [`TestRunnerSpawn::finalize_cancelled_test`].
///
/// This exists so that implementations may replace or augment the result with their own data.
// Java: com/google/devtools/build/lib/analysis/test/TestActionContext.java:170 TestActionContext.ProcessedAttemptResult
pub trait ProcessedAttemptResult: Send + Sync + Any {
    /// For downcasting to the implementation's class.
    fn as_any(&self) -> &dyn Any;
}

/// What [`TestRunnerSpawn::get_flaky_retry_runner`] returns: Java returns `this`, another runner,
/// or null.
pub enum NextTestRunnerSpawn<'a> {
    /// `this`.
    This,
    /// Another runner.
    Other(Box<dyn TestRunnerSpawn<'a> + 'a>),
}

/// A delegate to run a test. This may include running multiple spawns, renaming outputs, etc.
// Java: com/google/devtools/build/lib/analysis/test/TestActionContext.java:173 TestActionContext.TestRunnerSpawn
pub trait TestRunnerSpawn<'a> {
    // Java: com/google/devtools/build/lib/analysis/test/TestActionContext.java:174 TestActionContext.TestRunnerSpawn.getActionExecutionContext
    fn get_action_execution_context(&self) -> &dyn ActionExecutionContext;

    /// Run the test attempt. Blocks until the attempt is complete.
    // Java: com/google/devtools/build/lib/analysis/test/TestActionContext.java:177 TestActionContext.TestRunnerSpawn.execute
    fn execute(&self) -> Result<Box<dyn TestAttemptResult>, TestRunnerSpawnError>;

    /// After the first attempt has run, this method is called to determine the maximum number of
    /// attempts for this test.
    // Java: com/google/devtools/build/lib/analysis/test/TestActionContext.java:183 TestActionContext.TestRunnerSpawn.getMaxAttempts
    fn get_max_attempts(&self, first_test_attempt_result: &dyn TestAttemptResult) -> i32;

    /// Rename the output files if the test attempt failed, and post the test attempt result.
    // Java: com/google/devtools/build/lib/analysis/test/TestActionContext.java:186 TestActionContext.TestRunnerSpawn.finalizeFailedTestAttempt
    fn finalize_failed_test_attempt(
        &self,
        test_attempt_result: &dyn TestAttemptResult,
        attempt: i32,
    ) -> Result<Box<dyn ProcessedAttemptResult>, TestRunnerSpawnError>;

    /// Post the final test result based on the last attempt and the list of failed attempts.
    // Java: com/google/devtools/build/lib/analysis/test/TestActionContext.java:191 TestActionContext.TestRunnerSpawn.finalizeTest
    fn finalize_test(
        &self,
        last_test_attempt_result: &dyn TestAttemptResult,
        failed_attempts: &[Box<dyn ProcessedAttemptResult>],
    ) -> Result<(), TestRunnerSpawnError>;

    /// Post the final test result based on the last attempt and the list of failed attempts.
    // Java: com/google/devtools/build/lib/analysis/test/TestActionContext.java:196 TestActionContext.TestRunnerSpawn.finalizeCancelledTest
    fn finalize_cancelled_test(
        &self,
        failed_attempts: &[Box<dyn ProcessedAttemptResult>],
    ) -> Result<(), TestRunnerSpawnError>;

    /// Return a `TestRunnerSpawn` object if test fallback is enabled, or `None` otherwise. Test
    /// fallback is a feature to allow a test to run with one strategy until the max attempts are
    /// exhausted and then run with another strategy for another set of attempts. This is rarely
    /// used, and should ideally be removed.
    // Java: com/google/devtools/build/lib/analysis/test/TestActionContext.java:206 TestActionContext.TestRunnerSpawn.getFallbackRunner
    fn get_fallback_runner(
        &self,
    ) -> Result<Option<Box<dyn TestRunnerSpawn<'a> + 'a>>, ExecException> {
        Ok(None)
    }

    /// Return a `TestRunnerSpawn` object that is used on flaky retries. Flaky retry runner allows
    /// a test to run with a different strategy on flaky retries (for example, enabling test
    /// fail-fast mode to save up resources).
    // Java: com/google/devtools/build/lib/analysis/test/TestActionContext.java:215 TestActionContext.TestRunnerSpawn.getFlakyRetryRunner
    fn get_flaky_retry_runner(
        &self,
        _previous_attempt_results: &[SpawnResult],
    ) -> Result<Option<NextTestRunnerSpawn<'a>>, ExecException> {
        Ok(Some(NextTestRunnerSpawn::This))
    }
}
