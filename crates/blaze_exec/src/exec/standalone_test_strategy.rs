//! Port of `exec/StandaloneTestStrategy.java` (runs `TestRunnerAction`s) and
//! `exec/StandaloneTestResult.java`.
//!
//! PORT: `ActionExecutionContext.withFileOutErr` / `withOutputsAsInputs` return a new context in
//! Java; here [`DerivedActionExecutionContext`] wraps the action's context and replaces the
//! `FileOutErr` and/or adds outputs to the input metadata. `SpawnStrategyResolver` is stateless,
//! so it is constructed instead of looked up with `getContext(SpawnStrategyResolver.class)`.
//! `getPathResolver().convertPath(path)` is `ActionExecutionContext::convert_path` (the path on
//! the action file system when there is one).

use std::any::Any;
use std::collections::{BTreeMap, HashSet};
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use blaze_actions::ActionOwner;
use blaze_actions::actions::action::{
    Action, DEFAULT_RESOURCE_SET_CPU, DEFAULT_RESOURCE_SET_MEMORY_MB,
};
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_events::{Clock, ExtendedEventHandler};
use blaze_optdefs::command::execution_options::{ExecutionOptions, TestOutputFormat};
use blaze_optdefs::command::test_summary_options::TestSummaryOptions;
use blaze_protos::blaze::{BlazeTestStatus, TestResultData};
use blaze_protos::build_event_stream::test_result::ExecutionInfo;
use blaze_protos::build_event_stream::test_result::execution_info::TimingBreakdown;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail};
use blaze_vfs::{FileSystemRef, Path, PathFragment, Symlinks};
use indexmap::IndexMap;
use parking_lot::Mutex;

use crate::actions::action_adapters::{ActionMetadataSnapshot, artifact_input};
use crate::actions::execution_requirements as er;
use crate::actions::{
    ActionExecutionMetadata, ActionInput, ActionInputRef, EnvironmentalExecException,
    ExecException, FileArtifactValue, FilesetOutputTree, InputMetadataProvider, PathMapper,
    ResourceSet, ResourceSetOrBuilder, RunfilesArtifactValue, SimpleSpawn, Spawn, SpawnResult,
    TreeArtifactValue,
};
use crate::analysis::test::test_action_context::{
    AttemptGroup, ProcessedAttemptResult, TestActionContext, TestAttemptResult,
    TestAttemptResultResult, TestOutputs, TestRunnerSpawn, TestRunnerSpawnError,
};
use crate::analysis::test::test_attempt::{TestAttempt, duration_from_millis};
use crate::analysis::test::test_file_name_constants as names;
use crate::analysis::test::test_result::TestResult;
use crate::analysis::test::test_runner_action::{
    TestRunnerAction, remove_extension, test_exec_exception, touch_file,
};
use crate::analysis::test::test_strategy::TestStrategy;
use crate::exec::action_execution_context::{
    ActionContextRegistry, ActionExecutionContext, SpawnCache,
};
use crate::exec::spawn_log_context::SpawnLogContext;
use crate::exec::spawn_runner::ProgressStatus;
use crate::exec::spawn_strategy_registry::SpawnStrategyRegistry;
use crate::exec::spawn_strategy_resolver::SpawnStrategyResolver;
use crate::exec::streamed_test_output::StreamedTestOutput;
use crate::exec::test_policy::{self, TestPolicy};
use crate::util::FileOutErr;

const TEST_NAME_ENV: &str = "TEST_NAME";

// Java: com/google/devtools/build/lib/exec/StandaloneTestStrategy.java:77 StandaloneTestStrategy.ENV_VARS
fn env_vars() -> IndexMap<String, String> {
    let mut m = IndexMap::new();
    m.insert("TZ".to_string(), "UTC".to_string());
    m.insert(
        "TEST_SRCDIR".to_string(),
        test_policy::RUNFILES_DIR.to_string(),
    );
    // TODO(lberki): Remove JAVA_RUNFILES and PYTHON_RUNFILES.
    m.insert(
        "JAVA_RUNFILES".to_string(),
        test_policy::RUNFILES_DIR.to_string(),
    );
    m.insert(
        "PYTHON_RUNFILES".to_string(),
        test_policy::RUNFILES_DIR.to_string(),
    );
    m.insert(
        "RUNFILES_DIR".to_string(),
        test_policy::RUNFILES_DIR.to_string(),
    );
    m.insert(
        "TEST_TMPDIR".to_string(),
        test_policy::TEST_TMP_DIR.to_string(),
    );
    m.insert("RUN_UNDER_RUNFILES".to_string(), "1".to_string());
    m
}

/// `StandaloneTestStrategy.DEFAULT_LOCAL_POLICY`.
// Java: com/google/devtools/build/lib/exec/StandaloneTestStrategy.java:89 StandaloneTestStrategy.DEFAULT_LOCAL_POLICY
pub fn default_local_policy() -> TestPolicy {
    TestPolicy::new(env_vars())
}

/// Contains information about the results of test execution.
///
/// `spawn_results`: the SpawnResults created by the test, if any. `test_result_data_builder`:
/// the TestResultData for the test (a builder: later steps mutate it in place).
// Java: com/google/devtools/build/lib/exec/StandaloneTestResult.java:33 StandaloneTestResult
pub struct StandaloneTestResult {
    spawn_results: Vec<SpawnResult>,
    test_result_data_builder: Mutex<TestResultData>,
    execution_info: ExecutionInfo,
}

impl StandaloneTestResult {
    // Java: com/google/devtools/build/lib/exec/StandaloneTestResult.java:53 StandaloneTestResult.builder
    pub fn new(
        spawn_results: Vec<SpawnResult>,
        test_result_data_builder: TestResultData,
        execution_info: ExecutionInfo,
    ) -> StandaloneTestResult {
        StandaloneTestResult {
            spawn_results,
            test_result_data_builder: Mutex::new(test_result_data_builder),
            execution_info,
        }
    }

    /// `testResultDataBuilder()`.
    pub fn test_result_data_builder(&self) -> parking_lot::MutexGuard<'_, TestResultData> {
        self.test_result_data_builder.lock()
    }

    /// `executionInfo()`.
    pub fn execution_info(&self) -> &ExecutionInfo {
        &self.execution_info
    }
}

impl TestAttemptResult for StandaloneTestResult {
    // Java: com/google/devtools/build/lib/exec/StandaloneTestResult.java:45 StandaloneTestResult.result
    fn result(&self) -> TestAttemptResultResult {
        // TODO(b/148785690): Establish proper retry policy for flaky tests in
        // StandaloneTestStrategy.
        if self.test_result_data_builder().status() == BlazeTestStatus::Passed {
            TestAttemptResultResult::Passed
        } else {
            TestAttemptResultResult::FailedCanRetry
        }
    }

    fn spawn_results(&self) -> &[SpawnResult] {
        &self.spawn_results
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// Java: com/google/devtools/build/lib/exec/StandaloneTestStrategy.java:551 StandaloneTestStrategy.StandaloneProcessedAttemptResult
pub struct StandaloneProcessedAttemptResult {
    pub test_result_data: TestResultData,
    pub test_outputs: TestOutputs,
}

impl ProcessedAttemptResult for StandaloneProcessedAttemptResult {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The resources of a test spawn: `(os, inputsSize) -> action.getTestProperties()
/// .getLocalResourceUsage(label, executionOptions.usingLocalTestJobs())`.
#[derive(Debug)]
struct TestLocalResources(ResourceSet);

impl ResourceSetOrBuilder for TestLocalResources {
    fn build_resource_set(&self, _inputs_size: i32) -> ResourceSet {
        self.0.clone()
    }
}

/// `ResourceSetOrBuilder.ignoringOverrides(resources)`.
// Java: com/google/devtools/build/lib/actions/ResourceSetOrBuilder.java:52 ResourceSetOrBuilder.ignoringOverrides
#[derive(Debug)]
struct IgnoringOverrides(ResourceSet);

impl ResourceSetOrBuilder for IgnoringOverrides {
    fn build_resource_set(&self, _inputs_size: i32) -> ResourceSet {
        self.0.clone()
    }
    fn build_local_resources(
        &self,
        _inputs_size: i32,
        _execution_info: &IndexMap<String, String>,
        _exec_properties: &IndexMap<String, String>,
    ) -> Result<ResourceSet, ExecException> {
        Ok(self.0.clone())
    }
}

/// `SpawnAction.DEFAULT_RESOURCE_SET` (`AbstractAction.DEFAULT_RESOURCE_SET`).
fn spawn_action_default_resource_set() -> ResourceSet {
    ResourceSet::create_with_ram_cpu(DEFAULT_RESOURCE_SET_MEMORY_MB, DEFAULT_RESOURCE_SET_CPU)
}

/// `NestedSet<Artifact>` as the spawn's `NestedSet<ActionInput>`.
fn to_action_inputs(set: &NestedSet<blaze_actions::Artifact>) -> NestedSet<ActionInputRef> {
    NestedSetBuilder::wrap(set.get_order(), set.to_list().iter().map(artifact_input))
}

/// `StaticInputMetadataProvider` + `DelegatingPairInputMetadataProvider` (what
/// `withOutputsAsInputs` installs): the given outputs' metadata first, then the action's.
struct OutputsAsInputsMetadataProvider<'a> {
    additional_inputs: IndexMap<PathFragment, (ActionInputRef, FileArtifactValue)>,
    delegate: &'a dyn InputMetadataProvider,
}

impl InputMetadataProvider for OutputsAsInputsMetadataProvider<'_> {
    fn get_input_metadata(&self, input: &ActionInputRef) -> io::Result<Option<FileArtifactValue>> {
        if let Some((_, v)) = self.additional_inputs.get(input.get_exec_path()) {
            return Ok(Some(v.clone()));
        }
        self.delegate.get_input_metadata(input)
    }
    fn get_tree_metadata(&self, input: &ActionInputRef) -> Option<Arc<TreeArtifactValue>> {
        self.delegate.get_tree_metadata(input)
    }
    fn get_fileset(&self, input: &ActionInputRef) -> Option<Arc<FilesetOutputTree>> {
        self.delegate.get_fileset(input)
    }
    fn get_runfiles_metadata(&self, input: &ActionInputRef) -> Option<Arc<RunfilesArtifactValue>> {
        self.delegate.get_runfiles_metadata(input)
    }
    fn get_input(&self, exec_path: &PathFragment) -> Option<ActionInputRef> {
        if let Some((input, _)) = self.additional_inputs.get(exec_path) {
            return Some(input.clone());
        }
        self.delegate.get_input(exec_path)
    }
}

/// `actionExecutionContext.withFileOutErr(fileOutErr)` and
/// `.withOutputsAsInputs(additionalInputs)`: the action's context with another `FileOutErr`
/// and/or more input metadata. Every other method forwards to the action's context.
pub struct DerivedActionExecutionContext<'a> {
    inner: &'a dyn ActionExecutionContext,
    file_out_err: Option<FileOutErr>,
    input_metadata_provider: Option<OutputsAsInputsMetadataProvider<'a>>,
}

impl<'a> DerivedActionExecutionContext<'a> {
    /// `withFileOutErr(fileOutErr)`.
    // Java: com/google/devtools/build/lib/actions/ActionExecutionContext.java:597 ActionExecutionContext.withFileOutErr
    pub fn with_file_out_err(
        inner: &'a dyn ActionExecutionContext,
        file_out_err: FileOutErr,
    ) -> DerivedActionExecutionContext<'a> {
        DerivedActionExecutionContext {
            inner,
            file_out_err: Some(file_out_err),
            input_metadata_provider: None,
        }
    }

    /// `withFileOutErr(fileOutErr).withOutputsAsInputs(additionalInputs)`.
    // Java: com/google/devtools/build/lib/actions/ActionExecutionContext.java:570 ActionExecutionContext.withOutputsAsInputs
    pub fn with_file_out_err_and_outputs_as_inputs(
        inner: &'a dyn ActionExecutionContext,
        file_out_err: FileOutErr,
        additional_inputs: &[ActionInputRef],
    ) -> io::Result<DerivedActionExecutionContext<'a>> {
        let mut additional_input_map = IndexMap::new();
        for input in additional_inputs {
            let value = inner.get_output_metadata(&*input.0)?;
            // ImmutableMap.Builder.buildOrThrow: duplicate keys throw.
            if additional_input_map
                .insert(input.get_exec_path().clone(), (input.clone(), value))
                .is_some()
            {
                panic!("IllegalArgumentException: Multiple entries with same key");
            }
        }
        Ok(DerivedActionExecutionContext {
            inner,
            file_out_err: Some(file_out_err),
            input_metadata_provider: Some(OutputsAsInputsMetadataProvider {
                additional_inputs: additional_input_map,
                delegate: inner.get_input_metadata_provider(),
            }),
        })
    }
}

impl ActionContextRegistry for DerivedActionExecutionContext<'_> {
    fn get_spawn_strategy_registry(&self) -> Option<Arc<SpawnStrategyRegistry>> {
        self.inner.get_spawn_strategy_registry()
    }
    fn get_spawn_log_context(&self) -> Option<Arc<dyn SpawnLogContext>> {
        self.inner.get_spawn_log_context()
    }
    fn get_spawn_cache(&self) -> Option<Arc<dyn SpawnCache>> {
        self.inner.get_spawn_cache()
    }
    fn get_context_any(
        &self,
        type_id: std::any::TypeId,
    ) -> Option<crate::actions::action_execution::ActionContextRef> {
        self.inner.get_context_any(type_id)
    }
}

impl ActionExecutionContext for DerivedActionExecutionContext<'_> {
    fn get_exec_root(&self) -> &Path {
        self.inner.get_exec_root()
    }
    fn get_input_metadata_provider(&self) -> &dyn InputMetadataProvider {
        match &self.input_metadata_provider {
            Some(p) => p,
            None => self.inner.get_input_metadata_provider(),
        }
    }
    fn get_action_file_system(&self) -> Option<FileSystemRef> {
        self.inner.get_action_file_system()
    }
    fn get_remote_action_file_system(
        &self,
    ) -> Option<Arc<crate::exec::remote_action_file_system::RemoteActionFileSystem>> {
        self.inner.get_remote_action_file_system()
    }
    fn get_file_out_err(&self) -> &FileOutErr {
        match &self.file_out_err {
            Some(f) => f,
            None => self.inner.get_file_out_err(),
        }
    }
    fn get_client_env(&self) -> &IndexMap<String, String> {
        self.inner.get_client_env()
    }
    fn get_event_handler(&self) -> &dyn ExtendedEventHandler {
        self.inner.get_event_handler()
    }
    fn get_clock(&self) -> &dyn Clock {
        self.inner.get_clock()
    }
    fn maybe_report_subcommand(&self, spawn: &dyn Spawn, spawn_runner: Option<&str>) {
        self.inner.maybe_report_subcommand(spawn, spawn_runner)
    }
    fn post_progress(&self, action: &dyn ActionExecutionMetadata, progress: &ProgressStatus) {
        self.inner.post_progress(action, progress)
    }
    fn prefetch_files(
        &self,
        owner: &dyn ActionExecutionMetadata,
        inputs: &[ActionInputRef],
        input_metadata_provider: &dyn InputMetadataProvider,
    ) -> Result<(), ExecException> {
        self.inner
            .prefetch_files(owner, inputs, input_metadata_provider)
    }
    fn is_rewinding_enabled(&self) -> bool {
        self.inner.is_rewinding_enabled()
    }
    fn get_input_path(&self, input: &dyn ActionInput) -> Path {
        self.inner.get_input_path(input)
    }
    fn get_verbose_failures(&self) -> bool {
        self.inner.get_verbose_failures()
    }
    fn set_rich_artifact_data(&self, data: Arc<RunfilesArtifactValue>) {
        self.inner.set_rich_artifact_data(data)
    }
    fn check_for_lost_inputs(&self) -> Result<(), ExecException> {
        self.inner.check_for_lost_inputs()
    }
    fn get_output_metadata(&self, output: &dyn ActionInput) -> io::Result<FileArtifactValue> {
        self.inner.get_output_metadata(output)
    }
    fn get_tree_artifact_value(
        &self,
        tree_artifact: &ActionInputRef,
    ) -> io::Result<crate::actions::TreeArtifactValue> {
        self.inner.get_tree_artifact_value(tree_artifact)
    }
}

/// Runs TestRunnerAction actions.
// Java: com/google/devtools/build/lib/exec/StandaloneTestStrategy.java:75 StandaloneTestStrategy
pub struct StandaloneTestStrategy {
    base: TestStrategy,
    tmp_dir_root: Path,
}

impl StandaloneTestStrategy {
    // Java: com/google/devtools/build/lib/exec/StandaloneTestStrategy.java:93 StandaloneTestStrategy.<init>
    pub fn new(
        execution_options: Arc<ExecutionOptions>,
        test_summary_options: Arc<TestSummaryOptions>,
        tmp_dir_root: Path,
    ) -> StandaloneTestStrategy {
        StandaloneTestStrategy {
            base: TestStrategy::new(execution_options, test_summary_options),
            tmp_dir_root,
        }
    }

    /// The `TestStrategy` this strategy extends.
    pub fn base(&self) -> &TestStrategy {
        &self.base
    }

    // Java: com/google/devtools/build/lib/exec/StandaloneTestStrategy.java:195 StandaloneTestStrategy.processFailedTestAttempt
    fn process_failed_test_attempt(
        &self,
        attempt_id: i32,
        action_execution_context: &dyn ActionExecutionContext,
        action: &TestRunnerAction,
        result: &StandaloneTestResult,
    ) -> io::Result<StandaloneProcessedAttemptResult> {
        self.process_test_attempt(
            attempt_id,
            /* is_last_attempt= */ false,
            action_execution_context,
            action,
            result,
        )
    }

    // Java: com/google/devtools/build/lib/exec/StandaloneTestStrategy.java:205 StandaloneTestStrategy.finalizeTest
    fn finalize_test(
        &self,
        action: &TestRunnerAction,
        action_execution_context: &dyn ActionExecutionContext,
        standalone_test_result: &StandaloneTestResult,
        failed_attempts: &[Box<dyn ProcessedAttemptResult>],
    ) -> io::Result<()> {
        let last_attempt = self.process_test_attempt(
            failed_attempts.len() as i32 + 1,
            /* is_last_attempt= */ true,
            action_execution_context,
            action,
            standalone_test_result,
        )?;

        let data = {
            let mut data_builder = standalone_test_result.test_result_data_builder();
            for failed_attempt in failed_attempts {
                let failed_attempt_data = &failed_attempt
                    .as_any()
                    .downcast_ref::<StandaloneProcessedAttemptResult>()
                    .expect("ClassCastException: StandaloneProcessedAttemptResult")
                    .test_result_data;
                data_builder
                    .failed_logs
                    .extend(failed_attempt_data.failed_logs.iter().cloned());
                data_builder
                    .test_times
                    .push(failed_attempt_data.test_times[0]);
                data_builder
                    .test_process_times
                    .extend(failed_attempt_data.test_process_times.iter().copied());
            }
            if data_builder.status() == BlazeTestStatus::Passed && !failed_attempts.is_empty() {
                data_builder.set_status(BlazeTestStatus::Flaky);
            }
            data_builder.clone()
        };
        let result = TestResult::new(
            action.arc(),
            data,
            last_attempt.test_outputs,
            false,
            None,
            standalone_test_result.primary_system_failure(),
        );
        self.base.post_test_result(action_execution_context, result)
    }

    // Java: com/google/devtools/build/lib/exec/StandaloneTestStrategy.java:212 StandaloneTestStrategy.processTestAttempt
    fn process_test_attempt(
        &self,
        attempt_id: i32,
        is_last_attempt: bool,
        action_execution_context: &dyn ActionExecutionContext,
        action: &TestRunnerAction,
        result: &StandaloneTestResult,
    ) -> io::Result<StandaloneProcessedAttemptResult> {
        let action_file_system = action_execution_context.get_action_file_system();
        let mut test_outputs = action.get_test_outputs_mapping(
            action_file_system.as_ref(),
            Some(action_execution_context.get_exec_root()),
        )?;
        if !is_last_attempt {
            test_outputs =
                rename_outputs(action_execution_context, action, test_outputs, attempt_id)?;
        }

        // Recover the test log path, which may have been renamed, and add it to the data builder.
        let mut renamed_test_log: Option<Path> = None;
        for (key, value) in &test_outputs {
            if key == names::TEST_LOG {
                blaze_util::check_state!(renamed_test_log.is_none(), "multiple test_log matches");
                renamed_test_log = Some(value.clone());
            }
        }

        let data = {
            let mut data_builder = result.test_result_data_builder();
            // If the test log path does not exist, mark the test as incomplete
            if renamed_test_log.is_none() {
                data_builder.set_status(BlazeTestStatus::Incomplete);
            }

            if data_builder.status() == BlazeTestStatus::Passed {
                data_builder.passed_log = Some(
                    renamed_test_log
                        .as_ref()
                        .unwrap()
                        .get_path_string()
                        .to_string(),
                );
            } else if data_builder.status() != BlazeTestStatus::Incomplete {
                data_builder.failed_logs.push(
                    renamed_test_log
                        .as_ref()
                        .unwrap()
                        .get_path_string()
                        .to_string(),
                );
            }

            if !result.spawn_results().is_empty() {
                data_builder.exit_code = Some(result.spawn_results()[0].exit_code());
            }

            // Add the test log to the output
            data_builder.clone()
        };
        action_execution_context.get_event_handler().post(Arc::new(
            TestAttempt::for_executed_test_result(
                action.arc(),
                &data,
                attempt_id,
                test_outputs.clone(),
                result.execution_info().clone(),
                is_last_attempt,
            ),
        ));
        self.base.process_test_output(
            action_execution_context,
            &data,
            &action.get_test_name(),
            renamed_test_log.as_ref(),
        )?;
        Ok(StandaloneProcessedAttemptResult {
            test_result_data: data,
            test_outputs,
        })
    }

    // Java: com/google/devtools/build/lib/exec/StandaloneTestStrategy.java:307 StandaloneTestStrategy.beginTestAttempt
    fn begin_test_attempt(
        &self,
        test_action: &TestRunnerAction,
        spawn: &SimpleSpawn,
        action_execution_context: &dyn ActionExecutionContext,
        exec_root: &Path,
        child_out_err_count: &AtomicI32,
    ) -> Result<StandaloneTestResult, TestRunnerSpawnError> {
        let resolved_paths = test_action.resolve(exec_root);
        let out = action_execution_context.get_input_path(test_action.get_test_log());
        let err = resolved_paths.get_test_stderr();
        let test_out_err = FileOutErr::new(out.clone(), err);
        // PORT: the watcher thread of the streamed output borrows the event handler, so it runs
        // in a scope around the attempt.
        std::thread::scope(|scope| {
            let mut streamed: Option<StreamedTestOutput<'_>> = None;
            if self.base.execution_options.test_output == TestOutputFormat::STREAMED {
                streamed = Some(self.base.create_streamed_test_output(
                    scope,
                    action_execution_context.get_event_handler(),
                    out.clone(),
                )?);
            }

            let start_time_millis = action_execution_context.get_clock().current_time_millis();
            let resolver = SpawnStrategyResolver::new();

            self.run_test_attempt(
                test_action,
                action_execution_context,
                spawn,
                &resolver,
                &resolved_paths,
                test_out_err,
                streamed,
                start_time_millis,
                child_out_err_count,
            )
        })
    }

    // Java: com/google/devtools/build/lib/exec/StandaloneTestStrategy.java:634 StandaloneTestStrategy.runTestAttempt
    fn run_test_attempt(
        &self,
        test_action: &TestRunnerAction,
        action_execution_context: &dyn ActionExecutionContext,
        spawn: &SimpleSpawn,
        resolver: &SpawnStrategyResolver,
        resolved_paths: &crate::analysis::test::test_runner_action::ResolvedPaths<'_>,
        file_out_err: FileOutErr,
        // PORT: closed on every early return by its Drop (Java's closeSuppressed).
        streamed: Option<StreamedTestOutput<'_>>,
        start_time_millis: i64,
        child_out_err_count: &AtomicI32,
    ) -> Result<StandaloneTestResult, TestRunnerSpawnError> {
        // We have two protos to represent test attempts:
        // 1. com.google.devtools.build.lib.view.test.TestStatus.TestResultData represents both
        //    failed attempts and finished tests. Bazel stores this to disk to persist cached test
        //    result information across server restarts.
        // 2. com.google.devtools.build.lib.buildeventstream.BuildEventStreamProtos.TestResult
        //    represents only individual attempts (failed or not). Bazel reports this as an event
        //    to the Build Event Protocol, but never saves it to disk.
        //
        // The TestResult proto is always constructed from a TestResultData instance, either one
        // that is created right here, or one that is read back from disk.
        let mut spawn_results: Vec<SpawnResult>;
        let mut test_result_data_builder = TestResultData::default();
        let test_context = DerivedActionExecutionContext::with_file_out_err(
            action_execution_context,
            file_out_err.clone(),
        );
        match resolver.exec(spawn, &test_context) {
            Ok(results) => {
                spawn_results = results;
                test_result_data_builder.cachable = Some(true);
                test_result_data_builder.test_passed = Some(true);
                test_result_data_builder.set_status(BlazeTestStatus::Passed);
            }
            Err(ExecException::Spawn(e)) => {
                if e.catastrophe {
                    return Err(ExecException::Spawn(e).into());
                }
                if !e.get_spawn_result().setup_success() {
                    // Rethrow as the test could not be run and thus there's no point in retrying.
                    return Err(ExecException::Spawn(e).into());
                }
                spawn_results = vec![e.get_spawn_result().clone()];
                test_result_data_builder.cachable =
                    Some(e.get_spawn_result().status().is_considered_user_error());
                test_result_data_builder.test_passed = Some(false);
                test_result_data_builder.set_status(if e.has_timed_out() {
                    BlazeTestStatus::Timeout
                } else {
                    BlazeTestStatus::Failed
                });
            }
            Err(e) => return Err(e.into()),
        }
        let end_time_millis = action_execution_context.get_clock().current_time_millis();

        // Check TEST_PREMATURE_EXIT_FILE file (and always delete it)
        // Java: com/google/devtools/build/lib/exec/StandaloneTestStrategy.java:687 StandaloneTestStrategy.runTestAttempt
        if action_execution_context
            .convert_path(&resolved_paths.get_exit_safe_file())
            .delete()?
            && test_result_data_builder.test_passed()
        {
            test_result_data_builder.cachable = Some(false);
            test_result_data_builder.test_passed = Some(false);
            test_result_data_builder.set_status(BlazeTestStatus::Failed);
            file_out_err
                .write_err(b"-- Test exited prematurely (TEST_PREMATURE_EXIT_FILE exists) --\n")?;
        }

        // Do not override a more informative test failure with a generic failure due to the
        // missing shard file, which may have been caused by the test failing before the runner
        // had a chance to touch the file
        if test_result_data_builder.test_passed()
            && test_action.is_sharded()
            && test_action.check_sharding_support()
            // Java: com/google/devtools/build/lib/exec/StandaloneTestStrategy.java:706 StandaloneTestStrategy.runTestAttempt
            && !action_execution_context
                .convert_path(&resolved_paths.get_test_shard().unwrap())
                .exists()
        {
            return Err(create_test_exec_exception(
                failure_details::test_action::Code::LocalTestPrereqUnmet,
                "Sharding requested, but the test runner did not advertise support for it by \
                 touching TEST_SHARD_STATUS_FILE. Either remove the 'shard_count' attribute, use \
                 a test runner that supports sharding or temporarily disable this check via \
                 --noincompatible_check_sharding_support."
                    .to_string(),
            )
            .into());
        }

        // SpawnActionContext guarantees the first entry to correspond to the spawn passed in
        // (there may be additional entries due to tree artifact handling).
        let primary_result = &spawn_results[0];

        // The SpawnResult of a remotely cached or remotely executed action may not have walltime
        // set. We fall back to the time measured here for backwards compatibility.
        let mut duration_millis = end_time_millis - start_time_millis;
        duration_millis = if primary_result.get_wall_time_in_ms() != 0 {
            primary_result.get_wall_time_in_ms() as i64
        } else {
            duration_millis
        };

        test_result_data_builder.start_time_millis_epoch = Some(start_time_millis);
        test_result_data_builder.test_times.push(duration_millis);
        test_result_data_builder
            .test_process_times
            .push(duration_millis);
        test_result_data_builder.run_duration_millis = Some(duration_millis);
        test_result_data_builder.has_coverage = Some(test_action.is_coverage_mode());

        if test_action.is_coverage_mode() && test_action.get_split_coverage_post_processing() {
            if test_action.get_coverage_directory_tree_artifact().is_none() {
                // Otherwise we'll get a NPE https://github.com/bazelbuild/bazel/issues/13185
                return Err(create_test_exec_exception(
                    failure_details::test_action::Code::LocalTestPrereqUnmet,
                    "coverageDirectoryTreeArtifact is null: \
                     --experimental_split_coverage_postprocessing depends on \
                     --experimental_fetch_all_coverage_outputs being enabled"
                        .to_string(),
                )
                .into());
            }
            let coverage_directory = artifact_input(
                test_action
                    .get_coverage_directory_tree_artifact()
                    .expect("checked above"),
            );
            // var unused = getOutputMetadataStore().getOutputMetadata(coverageDirectory)
            let _ = action_execution_context.get_output_metadata(&*coverage_directory.0)?;

            let expanded_coverage_dir: Vec<ActionInputRef> = action_execution_context
                .get_tree_artifact_value(&coverage_directory)?
                .get_children();
            let mut coverage_spawn_metadata: Vec<ActionInputRef> = expanded_coverage_dir.clone();
            coverage_spawn_metadata.push(coverage_directory.clone());

            let coverage_post_processing_spawn = create_coverage_post_processing_spawn(
                action_execution_context,
                test_action,
                &expanded_coverage_dir,
                &self.tmp_dir_root,
            );

            let test_root = action_execution_context
                .get_input_path(test_action.get_test_log())
                .get_parent_directory()
                .unwrap();

            let out = test_root.get_child("coverage.log");
            let err = test_root.get_child("coverage.err");
            let coverage_out_err = FileOutErr::new(out, err);
            let coverage_action_execution_context =
                DerivedActionExecutionContext::with_file_out_err_and_outputs_as_inputs(
                    action_execution_context,
                    coverage_out_err.clone(),
                    &coverage_spawn_metadata,
                )?;

            match resolver.exec(
                &coverage_post_processing_spawn,
                &coverage_action_execution_context,
            ) {
                Ok(_) => {}
                Err(ExecException::Spawn(e)) => {
                    if e.catastrophe {
                        return Err(ExecException::Spawn(e).into());
                    }
                    if !e.get_spawn_result().setup_success() {
                        // Rethrow as the test could not be run and thus there's no point in
                        // retrying.
                        return Err(ExecException::Spawn(e).into());
                    }
                    test_result_data_builder.cachable =
                        Some(e.get_spawn_result().status().is_considered_user_error());
                    test_result_data_builder.test_passed = Some(false);
                    test_result_data_builder.set_status(if e.has_timed_out() {
                        BlazeTestStatus::Timeout
                    } else {
                        BlazeTestStatus::Failed
                    });
                }
                Err(e) => return Err(e.into()),
            }

            // Append all output from the coverage spawn to the test log.
            // coverageOutErr.close()
            write_out_file(
                coverage_out_err.get_error_path(),
                coverage_out_err.get_output_path(),
            )?;
            append_coverage_log(&coverage_out_err, &file_out_err)?;
        }

        if test_action.is_coverage_mode() && test_action.get_split_coverage_post_processing() {
            // Verify.verify(...): the post-processing spawn wrote the coverage data.
            assert!(
                action_execution_context
                    .get_input_path(
                        test_action
                            .get_coverage_data()
                            .expect("NullPointerException: getCoverageData()")
                    )
                    .exists(),
                "VerifyException"
            );
        }

        let finish = (|| -> io::Result<()> {
            if !file_out_err.has_recorded_output() {
                // Make sure that the test.log exists.
                touch_file(file_out_err.get_output_path())?;
            }
            // Append any error output to the test.log. This is very rare.
            //
            // Only write after the error output stream has been closed. Otherwise, Bazel cannot
            // delete test.err file on Windows. See https://github.com/bazelbuild/bazel/issues/20741.
            write_out_file(
                file_out_err.get_error_path(),
                file_out_err.get_output_path(),
            )?;
            if let Some(streamed) = streamed {
                streamed.close()?;
            }
            Ok(())
        })();
        if let Err(e) = finish {
            return Err(ExecException::Environmental(environmental_exec_exception(
                &e,
                failure_details::execution::Code::TestOutErrIoException,
            ))
            .into());
        }

        let xml_output_path = resolved_paths.get_xml_output_path();

        // If the test did not create a test.xml, then we run a separate action to create a
        // test.xml from test.log. We do this as a spawn rather than doing it locally in-process,
        // as the test.log file may only exist remotely (when remote execution is enabled), and we
        // do not want to have to download it.
        // `resolvedPaths` and the test's FileOutErr are resolved against
        // `actionExecutionContext.getExecRoot()`, which is on the action file system when there
        // is one: the existence checks see a `test.xml` (or `test.log`) that a cached or remote
        // test produced but that was not downloaded.
        // Java: com/google/devtools/build/lib/exec/StandaloneTestStrategy.java:857 StandaloneTestStrategy.runTestAttempt
        if file_out_err.get_output_path().exists() && !xml_output_path.exists() {
            let xml_generating_spawn = create_xml_generating_spawn(
                test_action,
                spawn.get_environment(),
                &spawn_results[0],
            );
            // We treat all failures to generate the test.xml here as catastrophic, and won't
            // rerun the test if this fails. We redirect the output to a temporary file.
            let xml_spawn_out_err = child_out_err(
                action_execution_context.get_file_out_err(),
                child_out_err_count,
            );

            let xml_action_execution_context =
                DerivedActionExecutionContext::with_file_out_err_and_outputs_as_inputs(
                    action_execution_context,
                    xml_spawn_out_err,
                    &[artifact_input(test_action.get_test_log())],
                )?;
            let xml_spawn_results =
                resolver.exec(&xml_generating_spawn, &xml_action_execution_context)?;
            spawn_results.extend(xml_spawn_results);
        }

        if let Some(details) = self.base.parse_test_result(&xml_output_path) {
            test_result_data_builder.test_case = Some(details);
        }

        let execution_info =
            extract_execution_info(&spawn_results[0], &mut test_result_data_builder);
        Ok(StandaloneTestResult::new(
            spawn_results,
            // We return the TestResultData.Builder rather than the finished TestResultData
            // instance, as we may have to rename the output files in case the test needs to be
            // rerun (if it failed here _and_ is marked flaky _and_ the number of flaky attempts
            // is larger than 1).
            test_result_data_builder,
            execution_info,
        ))
    }
}

/// `new EnvironmentalExecException(IOException cause, Execution.Code code)`.
///
/// PORT: Java's message is the cause's `toString()`; the Rust I/O error's message stands for it.
fn environmental_exec_exception(
    cause: &io::Error,
    code: failure_details::execution::Code,
) -> EnvironmentalExecException {
    EnvironmentalExecException {
        message: cause.to_string(),
        failure_detail: FailureDetail {
            message: String::new(),
            category: Some(failure_detail::Category::Execution(
                failure_details::Execution { code: code as i32 },
            )),
        },
        lost_inputs: None,
    }
}

/// `FileOutErr.childOutErr()`: `<out>-<n>` / `<err>-<n>`, numbered per parent.
// Java: com/google/devtools/build/lib/util/io/FileOutErr.java:232 FileOutErr.childOutErr
fn child_out_err(parent: &FileOutErr, child_count: &AtomicI32) -> FileOutErr {
    let index = child_count.fetch_add(1, Ordering::SeqCst);
    let out_path = parent.get_output_path();
    let err_path = parent.get_error_path();
    FileOutErr::new(
        out_path
            .get_parent_directory()
            .unwrap()
            .get_relative_str(&format!("{}-{}", out_path.get_base_name(), index)),
        err_path
            .get_parent_directory()
            .unwrap()
            .get_relative_str(&format!("{}-{}", err_path.get_base_name(), index)),
    )
}

// Java: com/google/devtools/build/lib/exec/StandaloneTestStrategy.java:148 StandaloneTestStrategy.renameOutputs
fn rename_outputs(
    action_execution_context: &dyn ActionExecutionContext,
    action: &TestRunnerAction,
    test_outputs: TestOutputs,
    attempt_id: i32,
) -> io::Result<TestOutputs> {
    // Rename outputs
    let name_prefix = remove_extension(action.get_test_log().get_exec_path().get_base_name());
    let test_root = action_execution_context
        .get_input_path(action.get_test_log())
        .get_parent_directory()
        .unwrap();
    let attempts_dir = test_root.get_child(&format!("{name_prefix}_attempts"));
    attempts_dir.create_directory()?;
    let attempt_prefix = format!("attempt_{attempt_id}");
    let test_log = attempts_dir.get_child(&format!("{attempt_prefix}.log"));

    // Get the normal test output paths, and then update them to use "attempt_N" names, and
    // attemptDir, before adding them to the outputs.
    let mut test_outputs_builder: TestOutputs = Vec::new();
    for (key, test_output_path) in test_outputs {
        // e.g. /testRoot/test.dir/file, an example we follow throughout this loop's comments.
        let destination_path = if key == names::TEST_LOG {
            // The rename rules for the test log are different than for all the other files.
            test_log.clone()
        } else {
            // e.g. test.dir/file
            let relative_to_test_directory = test_output_path.relative_to(&test_root);

            // e.g. attempt_1.dir/file
            let destination_path_fragment_str = relative_to_test_directory
                .get_safe_path_string()
                .replacen("test", &attempt_prefix, 1);
            let destination_path_fragment = PathFragment::create(&destination_path_fragment_str);

            // e.g. /attemptsDir/attempt_1.dir/file
            let destination_path = attempts_dir.get_relative(&destination_path_fragment);
            destination_path
                .get_parent_directory()
                .unwrap()
                .create_directory_and_parents()?;
            destination_path
        };

        // Move to the destination.
        test_output_path.rename_to(&destination_path)?;

        test_outputs_builder.push((key, destination_path));
    }
    Ok(test_outputs_builder)
}

// Java: com/google/devtools/build/lib/exec/StandaloneTestStrategy.java:291 StandaloneTestStrategy.setupEnvironment
fn setup_environment(
    action: &TestRunnerAction,
    client_env: &IndexMap<String, String>,
    exec_root: &Path,
    runfiles_dir: &Path,
    tmp_dir: &Path,
) -> IndexMap<String, String> {
    let relative_tmp_dir = if tmp_dir.starts_with(exec_root) {
        tmp_dir.relative_to(exec_root)
    } else {
        tmp_dir.as_fragment().clone()
    };
    default_local_policy().compute_test_environment(
        action,
        client_env,
        &runfiles_dir.relative_to(exec_root),
        &relative_tmp_dir,
    )
}

// Java: com/google/devtools/build/lib/exec/StandaloneTestStrategy.java:338 StandaloneTestStrategy.appendCoverageLog
fn append_coverage_log(coverage_out_err: &FileOutErr, out_err: &FileOutErr) -> io::Result<()> {
    write_out_file(coverage_out_err.get_error_path(), out_err.get_output_path())?;
    write_out_file(
        coverage_out_err.get_output_path(),
        out_err.get_output_path(),
    )
}

// Java: com/google/devtools/build/lib/exec/StandaloneTestStrategy.java:340 StandaloneTestStrategy.writeOutFile
fn write_out_file(in_file_path: &Path, out_file_path: &Path) -> io::Result<()> {
    let Some(stat) = in_file_path.stat_nullable(Symlinks::Follow) else {
        return Ok(());
    };
    let result = (|| {
        if stat.get_size() > 0 {
            if out_file_path.exists() {
                out_file_path.set_writable(true)?;
            }
            let mut out = out_file_path.get_output_stream_with(true, false)?;
            let mut input = in_file_path.get_input_stream()?;
            let mut buf = Vec::new();
            input.read_to_end(&mut buf)?;
            out.write_all(&buf)?;
            out.flush()?;
        }
        Ok(())
    })();
    // finally
    in_file_path.delete()?;
    result
}

// Java: com/google/devtools/build/lib/exec/StandaloneTestStrategy.java:363 StandaloneTestStrategy.extractExecutionInfo
#[allow(clippy::field_reassign_with_default)]
fn extract_execution_info(
    spawn_result: &SpawnResult,
    result: &mut TestResultData,
) -> ExecutionInfo {
    let mut execution_info = ExecutionInfo::default();

    // The return of `SpawnResult#exitCode()` is noted to only be meaningful if the subprocess
    // actually executed. In this position, `spawnResult.exitCode()` is always meaningful,
    // because the code only runs if `spawnResult.setupSuccess()` is previously verified to
    // be `true`.
    execution_info.exit_code = spawn_result.exit_code();

    if spawn_result.is_cache_hit() {
        result.remotely_cached = Some(true);
        execution_info.cached_remotely = true;
    }

    let strategy = spawn_result.get_runner_name();
    execution_info.strategy = strategy.to_string();
    result.is_remote_strategy = Some(strategy == "remote");

    if let Some(host) = spawn_result.get_executor_host_name() {
        execution_info.hostname = host.to_string();
    }

    let sm = spawn_result.get_metrics();
    let child = |name: &str, ms: i32| TimingBreakdown {
        name: name.to_string(),
        time: Some(to_proto_duration(ms)),
        ..Default::default()
    };
    execution_info.timing_breakdown = Some(TimingBreakdown {
        name: "totalTime".to_string(),
        time: Some(to_proto_duration(sm.total_time_in_ms)),
        child: vec![
            child("parseTime", sm.parse_time_in_ms),
            child("fetchTime", sm.fetch_time_in_ms),
            child("queueTime", sm.queue_time_in_ms),
            child("uploadTime", sm.upload_time_in_ms),
            child("setupTime", sm.setup_time_in_ms),
            child("executionWallTime", sm.execution_wall_time_in_ms),
            child("processOutputsTime", sm.process_outputs_time_in_ms),
            child("networkTime", sm.network_time_in_ms),
        ],
        ..Default::default()
    });

    execution_info
}

// Java: com/google/devtools/build/lib/exec/StandaloneTestStrategy.java:439 StandaloneTestStrategy.toProtoDuration
fn to_proto_duration(time_in_ms: i32) -> blaze_protos::google::protobuf::Duration {
    duration_from_millis(time_in_ms as i64)
}

/// A spawn to generate a test.xml file from the test log. This is only used if the test does not
/// generate a test.xml file itself.
// Java: com/google/devtools/build/lib/exec/StandaloneTestStrategy.java:447 StandaloneTestStrategy.createXmlGeneratingSpawn
pub fn create_xml_generating_spawn(
    action: &TestRunnerAction,
    test_env: &IndexMap<String, String>,
    result: &SpawnResult,
) -> SimpleSpawn {
    let args = vec![
        action
            .get_test_xml_generator_script()
            .get_exec_path()
            .get_callable_path_string_for_os(&blaze_vfs::os_path_policy::HOST_POLICY),
        action.get_test_log().get_exec_path_string().to_string(),
        action.get_test_xml().get_exec_path_string(),
        (result.get_wall_time_in_ms() / 1000).to_string(),
        result.exit_code().to_string(),
    ];
    let mut env_builder: IndexMap<String, String> = IndexMap::new();
    // "PATH" and "TEST_BINARY" are also required, they should always be set in testEnv.
    blaze_util::check_argument!(test_env.contains_key("PATH"));
    blaze_util::check_argument!(test_env.contains_key("TEST_BINARY"));
    let mut put = |k: &str, v: String| {
        // ImmutableMap.Builder.buildOrThrow: duplicate keys throw.
        if env_builder.insert(k.to_string(), v).is_some() {
            panic!("IllegalArgumentException: Multiple entries with same key: {k}");
        }
    };
    for (k, v) in test_env {
        put(k, v.clone());
    }
    put(TEST_NAME_ENV, action.get_test_name());
    // testEnv only contains TEST_SHARD_INDEX and TEST_TOTAL_SHARDS if the test action is
    // sharded, we need to set the default value when the action isn't sharded.
    if !action.is_sharded() {
        put("TEST_SHARD_INDEX", "0".to_string());
        put("TEST_TOTAL_SHARDS", "0".to_string());
    }
    SimpleSpawn::new(
        ActionMetadataSnapshot::of(action),
        args,
        env_builder,
        // Pass the execution info of the action which is identical to the supported tags set on
        // the test target. In particular, this does not set the test timeout on the spawn.
        action.get_execution_info(),
        /* inputs= */
        NestedSetBuilder::create(
            Order::STABLE_ORDER,
            [
                artifact_input(action.get_test_xml_generator_script()),
                artifact_input(action.get_test_log()),
            ],
        ),
        /* tools= */ NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        /* outputs= */ vec![action.get_test_xml().clone()],
        /* mandatory_outputs= */ None,
        // The resources are fixed because the execution info above carries the test target's
        // `resources:` tags (and, via TestTargetProperties, its exec_properties), which describe
        // the test process, not this script. Letting them override the default would make a
        // log-to-XML conversion book the whole test's CPU/memory/custom resources and queue
        // behind unrelated actions.
        Arc::new(IgnoringOverrides(spawn_action_default_resource_set())),
        PathMapper::NOOP,
    )
}

// Java: com/google/devtools/build/lib/exec/StandaloneTestStrategy.java:490 StandaloneTestStrategy.createCoveragePostProcessingSpawn
fn create_coverage_post_processing_spawn(
    action_execution_context: &dyn ActionExecutionContext,
    action: &TestRunnerAction,
    expanded_coverage_dir: &[ActionInputRef],
    tmp_dir_root: &Path,
) -> SimpleSpawn {
    let args = vec![
        action
            .get_collect_coverage_script()
            .expect("NullPointerException: getCollectCoverageScript()")
            .get_exec_path_string()
            .to_string(),
    ];

    let mut test_environment = create_environment(action_execution_context, action, tmp_dir_root);

    test_environment.insert(
        "TEST_SHARD_INDEX".to_string(),
        action.get_shard_num().to_string(),
    );
    test_environment.insert(
        "TEST_TOTAL_SHARDS".to_string(),
        action
            .get_execution_settings()
            .get_total_shards()
            .to_string(),
    );
    test_environment.insert(TEST_NAME_ENV.to_string(), action.get_test_name());
    test_environment.insert("IS_COVERAGE_SPAWN".to_string(), "1".to_string());
    // Let the coverage script locate its own runfiles tree, which is separate from the test
    // runfiles.
    test_environment.shift_remove("RUNFILES_DIR");
    test_environment.shift_remove("JAVA_RUNFILES");
    test_environment.shift_remove("PYTHON_RUNFILES");

    let mut inputs: NestedSetBuilder<ActionInputRef> = NestedSetBuilder::compile_order();
    inputs.add_transitive(&to_action_inputs(&action.get_inputs()));
    inputs.add_all(expanded_coverage_dir.iter().cloned());
    inputs.add(artifact_input(
        action
            .get_coverage_manifest()
            .expect("NullPointerException: getCoverageManifest()"),
    ));
    SimpleSpawn::new(
        ActionMetadataSnapshot::of(action),
        args,
        test_environment,
        action.get_execution_info(),
        /* inputs= */ inputs.build(),
        /* tools= */ NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        /* outputs= */
        vec![artifact_input(
            action
                .get_coverage_data()
                .expect("NullPointerException: getCoverageData()"),
        )],
        /* mandatory_outputs= */ None,
        // As in createXmlGeneratingSpawn: the test target's `resources:` entries describe the
        // test process, not this post-processing step.
        Arc::new(IgnoringOverrides(spawn_action_default_resource_set())),
        PathMapper::NOOP,
    )
}

// Java: com/google/devtools/build/lib/exec/StandaloneTestStrategy.java:104 StandaloneTestStrategy.createEnvironment
fn create_environment(
    action_execution_context: &dyn ActionExecutionContext,
    action: &TestRunnerAction,
    tmp_dir_root: &Path,
) -> IndexMap<String, String> {
    let exec_root = action_execution_context.get_exec_root();
    // ArtifactPathResolver pathResolver = actionExecutionContext.getPathResolver()
    let runfiles_dir = action_execution_context.convert_path(
        action
            .get_execution_settings()
            .get_runfiles_dir()
            .expect("NullPointerException: getRunfilesDir()"),
    );
    let tmp_dir = action_execution_context
        .convert_path(&tmp_dir_root.get_child(&TestStrategy::get_tmp_dir_name(action)));
    setup_environment(
        action,
        action_execution_context.get_client_env(),
        exec_root,
        &runfiles_dir,
        &tmp_dir,
    )
}

// Java: com/google/devtools/build/lib/exec/StandaloneTestStrategy.java:107 StandaloneTestStrategy.createTestExecException
fn create_test_exec_exception(
    error_code: failure_details::test_action::Code,
    error_message: String,
) -> ExecException {
    test_exec_exception(
        error_message.clone(),
        FailureDetail {
            message: error_message,
            category: Some(failure_detail::Category::TestAction(
                failure_details::TestAction {
                    code: error_code as i32,
                },
            )),
        },
    )
}

impl TestActionContext for StandaloneTestStrategy {
    // Java: com/google/devtools/build/lib/exec/StandaloneTestStrategy.java:100 StandaloneTestStrategy.createTestRunnerSpawn
    fn create_test_runner_spawn<'a>(
        &'a self,
        action: &'a TestRunnerAction,
        action_execution_context: &'a dyn ActionExecutionContext,
    ) -> Result<Box<dyn TestRunnerSpawn<'a> + 'a>, ExecException> {
        let test_environment =
            create_environment(action_execution_context, action, &self.tmp_dir_root);

        if let Some(test_name) = test_environment.get(TEST_NAME_ENV) {
            return Err(create_test_exec_exception(
                failure_details::test_action::Code::LocalTestPrereqUnmet,
                format!(
                    "cannot set env variable TEST_NAME={test_name} because TEST_NAME is reserved"
                ),
            ));
        }

        let mut execution_info: BTreeMap<String, String> =
            action.get_execution_info().into_iter().collect();
        if !action.should_accept_cached_result() {
            // TODO(tjgq): We want to reject a previously cached result, but not prevent the
            // result of the current execution from being uploaded. We should introduce a separate
            // execution requirement for this.
            execution_info.insert(er::NO_CACHE.to_string(), String::new());
        }
        execution_info.insert(
            er::TIMEOUT.to_string(),
            action.get_timeout().as_secs().to_string(),
        );

        let label = action
            .get_owner()
            .get_label()
            .expect("NullPointerException: getOwner().getLabel()");
        let local_resources =
            TestLocalResources(action.get_test_properties().get_local_resource_usage(
                label,
                self.base.execution_options.using_local_test_jobs(),
            ));

        let spawn = SimpleSpawn::new(
            ActionMetadataSnapshot::of(action),
            TestStrategy::get_args(action)?,
            test_environment,
            execution_info.into_iter().collect(),
            /* inputs= */ to_action_inputs(&action.get_inputs()),
            NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            action.get_spawn_outputs(),
            /* mandatory_outputs= */ Some(HashSet::new()),
            Arc::new(local_resources),
            PathMapper::NOOP,
        );
        let exec_root = action_execution_context.get_exec_root().clone();
        // ArtifactPathResolver pathResolver = actionExecutionContext.getPathResolver()
        let tmp_dir = action_execution_context.convert_path(
            &self
                .tmp_dir_root
                .get_child(&TestStrategy::get_tmp_dir_name(action)),
        );
        Ok(Box::new(StandaloneTestRunnerSpawn {
            strategy: self,
            test_action: action,
            action_execution_context,
            spawn,
            tmp_dir,
            exec_root,
            child_out_err_count: AtomicI32::new(0),
        }))
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestStrategy.java:168 TestStrategy.isTestKeepGoing
    fn is_test_keep_going(&self) -> bool {
        self.base.is_test_keep_going()
    }

    // Java: com/google/devtools/build/lib/exec/StandaloneTestStrategy.java:541 StandaloneTestStrategy.newCachedTestResult
    fn new_cached_test_result(
        &self,
        exec_root: &Path,
        action: &TestRunnerAction,
        cached_result: TestResultData,
        test_outputs: TestOutputs,
    ) -> io::Result<TestResult> {
        Ok(TestResult::new(
            action.arc(),
            cached_result,
            test_outputs,
            /* cached= */ true,
            Some(exec_root.clone()),
            /* system_failure= */ None,
        ))
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestStrategy.java:173 TestStrategy.getAttemptGroup
    fn get_attempt_group(&self, owner: &ActionOwner, shard_num: i32) -> Arc<dyn AttemptGroup> {
        self.base.get_attempt_group(owner, shard_num)
    }
}

// Java: com/google/devtools/build/lib/exec/StandaloneTestStrategy.java:555 StandaloneTestStrategy.StandaloneTestRunnerSpawn
struct StandaloneTestRunnerSpawn<'a> {
    strategy: &'a StandaloneTestStrategy,
    test_action: &'a TestRunnerAction,
    action_execution_context: &'a dyn ActionExecutionContext,
    spawn: SimpleSpawn,
    tmp_dir: Path,
    exec_root: Path,
    /// `FileOutErr.childCount` of the action's `FileOutErr`.
    child_out_err_count: AtomicI32,
}

fn as_standalone(r: &dyn TestAttemptResult) -> &StandaloneTestResult {
    r.as_any()
        .downcast_ref::<StandaloneTestResult>()
        .expect("ClassCastException: StandaloneTestResult")
}

impl<'a> TestRunnerSpawn<'a> for StandaloneTestRunnerSpawn<'a> {
    fn get_action_execution_context(&self) -> &dyn ActionExecutionContext {
        self.action_execution_context
    }

    fn execute(&self) -> Result<Box<dyn TestAttemptResult>, TestRunnerSpawnError> {
        self.strategy.base.prepare_file_system(
            self.test_action,
            &self.exec_root,
            Some(&self.tmp_dir),
        )?;
        Ok(Box::new(self.strategy.begin_test_attempt(
            self.test_action,
            &self.spawn,
            self.action_execution_context,
            &self.exec_root,
            &self.child_out_err_count,
        )?))
    }

    fn get_max_attempts(&self, _first_test_attempt_result: &dyn TestAttemptResult) -> i32 {
        self.strategy.base.get_test_attempts(self.test_action)
    }

    fn finalize_failed_test_attempt(
        &self,
        test_attempt_result: &dyn TestAttemptResult,
        attempt: i32,
    ) -> Result<Box<dyn ProcessedAttemptResult>, TestRunnerSpawnError> {
        Ok(Box::new(self.strategy.process_failed_test_attempt(
            attempt,
            self.action_execution_context,
            self.test_action,
            as_standalone(test_attempt_result),
        )?))
    }

    fn finalize_test(
        &self,
        final_result: &dyn TestAttemptResult,
        failed_attempts: &[Box<dyn ProcessedAttemptResult>],
    ) -> Result<(), TestRunnerSpawnError> {
        Ok(self.strategy.finalize_test(
            self.test_action,
            self.action_execution_context,
            as_standalone(final_result),
            failed_attempts,
        )?)
    }

    fn finalize_cancelled_test(
        &self,
        failed_attempts: &[Box<dyn ProcessedAttemptResult>],
    ) -> Result<(), TestRunnerSpawnError> {
        let mut builder = TestResultData {
            cachable: Some(false),
            test_passed: Some(false),
            ..Default::default()
        };
        builder.set_status(BlazeTestStatus::Incomplete);
        let standalone_test_result =
            StandaloneTestResult::new(Vec::new(), builder, ExecutionInfo::default());
        self.finalize_test(&standalone_test_result, failed_attempts)
    }
}
