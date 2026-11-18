//! Port of `analysis/test/TestRunnerAction.java`: an action representing a test with the
//! associated environment (runfiles, environment variables, test result, etc). It consumes test
//! executable and runfiles artifacts and produces test result and test status artifacts.
//!
//! PORT: `BuildConfigurationValue` and its `TestConfiguration` fragment live in a crate above this
//! one; the action reads them through [`TestRunnerActionConfiguration`], a plain snapshot of the
//! values it uses. `FilesToRunProvider collectCoverageScript` is kept as its executable artifact.
//! `PackageSpecificationProvider networkAllowlist` is not kept: nothing in test execution reads it.

use std::any::Any;
use std::collections::BTreeMap;
use std::fmt;
use std::io::{self, Read, Write};
use std::sync::{Arc, Weak};
use std::time::Duration;

use blaze_actions::actions::action::{
    AbstractAction, Action, CommandAction, CommandLineError, ComputeKeyError,
    InputMetadataProvider as KeyInputMetadataProvider, merge_maps,
};
use blaze_actions::{ActionEnvironment, ActionKeyContext, ActionOwner, Artifact};
use blaze_cmdline::Label;
use blaze_collect::NestedSet;
use blaze_events::{Event, Postable};
use blaze_optdefs::common::test_timeout::TestTimeout;
use blaze_optdefs::fragment::test_configuration::TestConfigurationTestOptionsCancelConcurrentTests as CancelConcurrentTests;
use blaze_options::TriState;
use blaze_options::value::java_duration_to_string;
use blaze_protos::JavaMessage;
use blaze_protos::blaze::TestResultData;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail};
use blaze_util::Fingerprint;
use blaze_vfs::{Path, PathFragment, Symlinks};
use indexmap::{IndexMap, IndexSet};
use parking_lot::Mutex;
use prost::Message;

use super::test_action_context::{
    NextTestRunnerSpawn, ProcessedAttemptResult, TestActionContext, TestAttemptResultResult,
    TestOutputs, TestRunnerSpawn, TestRunnerSpawnError, attempt_group_noop,
};
use super::test_file_name_constants as names;
use super::test_strategy::TestStrategy;
use super::test_target_execution_settings::TestTargetExecutionSettings;
use super::test_target_properties::TestTargetProperties;
use crate::actions::action_adapters::as_blaze_artifact;
use crate::actions::{
    ActionExecutionException, ActionInputRef, ActionResult, BasicActionInput,
    EnvironmentalExecException, ExecException, ExecutableAction, SpawnResult, UserExecException,
};
use crate::exec::action_execution_context::{ActionExecutionContext, get_context};

/// The parts of `BuildConfigurationValue` (and of its `TestConfiguration` fragment) that
/// `TestRunnerAction` reads.
#[derive(Clone, Debug)]
pub struct TestRunnerActionConfiguration {
    /// `checksum()` (the BEP configuration id).
    pub checksum: String,
    /// `getActionEnvironment()`.
    pub action_environment: Arc<ActionEnvironment>,
    /// `getTestActionEnvironment()`.
    pub test_action_environment: Arc<ActionEnvironment>,
    /// `isCodeCoverageEnabled()`.
    pub code_coverage_enabled: bool,
    /// `runfilesEnabled()`.
    pub runfiles_enabled: bool,
    /// `TestConfiguration.getTestTimeout()` (`--test_timeout`).
    pub test_timeout: BTreeMap<TestTimeout, Duration>,
    /// `TestConfiguration.cacheTestResults()` (`--cache_test_results`).
    pub cache_test_results: TriState,
    /// `TestConfiguration.getZipUndeclaredTestOutputs()`.
    pub zip_undeclared_test_outputs: bool,
    /// `TestConfiguration.getTestRunnerFailFast()`.
    pub test_runner_fail_fast: bool,
    /// `TestConfiguration.checkShardingSupport()`.
    pub check_sharding_support: bool,
    /// `TestConfiguration.allowLocalTests()`.
    pub allow_local_tests: bool,
}

/// `SpawnExecutedEvent.ChangePhase`: posted when a test attempt is rerun.
// Java: com/google/devtools/build/lib/actions/SpawnExecutedEvent.java:105 SpawnExecutedEvent.ChangePhase
#[derive(Debug, Clone)]
pub struct SpawnExecutedEventChangePhase {
    action: Arc<TestRunnerAction>,
}

impl SpawnExecutedEventChangePhase {
    pub fn get_action(&self) -> &Arc<TestRunnerAction> {
        &self.action
    }
}

impl Postable for SpawnExecutedEventChangePhase {}

pub const COVERAGE_TMP_ROOT: &str = "_coverage";

const UNDECLARED_OUTPUTS_ZIP_NAME: &str = "outputs.zip";

// Used for selecting subset of testcase / testmethods.
const TEST_BRIDGE_TEST_FILTER_ENV: &str = "TESTBRIDGE_TEST_ONLY";

const GUID: &str = "cc41f9d0-47a6-11e7-8726-eb6ce83a8cc8";
pub const MNEMONIC: &str = "TestRunner";

/// An Action representing a test with the associated environment (runfiles, environment
/// variables, test result, etc). It consumes test executable and runfiles artifacts and produces
/// test result and test status artifacts.
// Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:101 TestRunnerAction
pub struct TestRunnerAction {
    abstract_action: AbstractAction,
    self_ref: Weak<TestRunnerAction>,

    runfiles_tree: Artifact,
    test_setup_script: Artifact,
    test_xml_generator_script: Artifact,
    collect_coverage_script: Option<Artifact>,
    configuration: Arc<TestRunnerActionConfiguration>,
    test_log: Artifact,
    test_xml: ActionInputRef,
    cache_status: Artifact,
    test_warnings_path: PathFragment,
    unused_runfiles_log_path: PathFragment,
    sh_executable: Option<PathFragment>,
    split_logs_path: PathFragment,
    split_logs_dir: PathFragment,
    undeclared_outputs_annotations_dir: PathFragment,
    undeclared_outputs_manifest_path: PathFragment,
    undeclared_outputs_annotations_path: PathFragment,
    undeclared_outputs_annotations_pb_path: PathFragment,
    test_shard: Option<PathFragment>,
    test_exit_safe: PathFragment,
    test_stderr: PathFragment,
    test_infrastructure_failure: PathFragment,
    base_dir: PathFragment,

    files_to_delete_before_execution: IndexSet<PathFragment>,
    directories_to_delete_before_execution: IndexSet<PathFragment>,

    coverage_data: Option<Artifact>,
    coverage_directory: Option<Artifact>,
    undeclared_outputs_dir: Artifact,
    test_properties: TestTargetProperties,
    execution_settings: TestTargetExecutionSettings,
    shard_num: i32,
    run_number: i32,
    workspace_name: String,

    /// Cached test result status used to minimize disk accesses. This field is set when test
    /// status is retrieved from disk or saved to disk. This field is `None` if it has not been
    /// set yet. This field is `Some(None)` when the file was not present on disk or there was a
    /// failure to read it.
    cached_test_result_data: Mutex<Option<Option<TestResultData>>>,

    /// Environment variables specific to running code coverage.
    coverage_env: IndexMap<String, String>,

    /// Any extra environment variables (and values) added by the rule that created this action.
    extra_test_env: Arc<ActionEnvironment>,

    /// The set of environment variables that are inherited from the client environment. These
    /// are handled explicitly by the ActionCacheChecker and so don't have to be included in the
    /// cache key.
    required_client_env_variables: Vec<String>,

    cancel_concurrent_tests: CancelConcurrentTests,

    split_coverage_post_processing: bool,
    lcov_merger_files_to_run: NestedSet<Artifact>,

    /// If not `None`, the reason why this test can't be run in the current build, to be reported
    /// when the action is executed. The action is still created so that the test target can be
    /// built.
    unrunnable_reason: Option<String>,
}

impl fmt::Debug for TestRunnerAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TestRunnerAction")
            .field("owner", &self.get_owner().get_label())
            .field("test_log", &self.test_log)
            .field("shard_num", &self.shard_num)
            .field("run_number", &self.run_number)
            .finish()
    }
}

/// `TestRunnerAction.nonNullAsSet`.
fn non_null_as_set(artifacts: Vec<Option<Artifact>>) -> Vec<Artifact> {
    let mut builder: IndexSet<Artifact> = IndexSet::new();
    for artifact in artifacts.into_iter().flatten() {
        builder.insert(artifact);
    }
    builder.into_iter().collect()
}

/// `FileSystemUtils.touchFile`.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:269 FileSystemUtils.touchFile
pub(crate) fn touch_file(path: &Path) -> io::Result<()> {
    if path.exists() {
        // Path.NOW_SENTINEL_TIME
        path.set_last_modified_time(-1)
    } else {
        // FileSystemUtils.createEmptyFile
        path.get_output_stream()?.flush()
    }
}

/// `FileSystemUtils.removeExtension(String)`.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:128 FileSystemUtils.removeExtension
pub(crate) fn remove_extension(filename: &str) -> &str {
    let Some(last_dot_index) = filename.rfind('.') else {
        return filename;
    };
    if filename
        .rfind('/')
        .is_some_and(|last_slash_index| last_slash_index > last_dot_index)
    {
        return filename;
    }
    &filename[..last_dot_index]
}

impl TestRunnerAction {
    /// Create new TestRunnerAction instance. Should not be called directly. Use
    /// `TestActionBuilder` instead.
    ///
    /// `shard_num`: The shard number. Must be 0 if totalShards == 0 (no sharding). Otherwise,
    /// must be >= 0 and < totalShards. `run_number`: test run number.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:203 TestRunnerAction.<init>
    pub fn new(
        owner: ActionOwner,
        inputs: NestedSet<Artifact>,
        runfiles_tree: Artifact,
        test_setup_script: Artifact,               // Must be in inputs
        test_xml_generator_script: Artifact,       // Must be in inputs
        collect_coverage_script: Option<Artifact>, // filesToRun must be in input, if not null
        test_log: Artifact,
        test_xml: ActionInputRef,
        cache_status: Artifact,
        coverage_artifact: Option<Artifact>,
        coverage_directory: Option<Artifact>,
        undeclared_outputs_dir: Artifact,
        test_properties: TestTargetProperties,
        coverage_env: IndexMap<String, String>,
        extra_test_env: Arc<ActionEnvironment>,
        execution_settings: TestTargetExecutionSettings,
        shard_num: i32,
        run_number: i32,
        configuration: Arc<TestRunnerActionConfiguration>,
        workspace_name: String,
        sh_executable: Option<PathFragment>,
        cancel_concurrent_tests: CancelConcurrentTests,
        split_coverage_post_processing: bool,
        lcov_merger_files_to_run: NestedSet<Artifact>,
        unrunnable_reason: Option<String>,
    ) -> Arc<TestRunnerAction> {
        let test_xml_artifact = as_blaze_artifact(&test_xml).cloned();
        let outputs = non_null_as_set(vec![
            Some(test_log.clone()),
            // See TestActionBuilder.TEST_XML_IS_ACTION_OUTPUT for details.
            test_xml_artifact.clone(),
            Some(cache_status.clone()),
            coverage_artifact.clone(),
            coverage_directory.clone(),
            Some(undeclared_outputs_dir.clone()),
        ]);
        let abstract_action = AbstractAction::new(owner, inputs, outputs);
        blaze_util::check_state!(collect_coverage_script.is_none() == coverage_artifact.is_none());

        let base_dir = cache_status
            .get_exec_path()
            .get_parent_directory()
            .expect("cache status has a parent directory");

        let total_shards = execution_settings.get_total_shards();
        blaze_util::check_state!(
            (total_shards == 0 && shard_num == 0)
                || (total_shards > 0 && 0 <= shard_num && shard_num < total_shards)
        );
        let test_exit_safe = base_dir.get_child("test.exited_prematurely");
        // testShard Path should be set only if sharding is enabled.
        let test_shard = if total_shards > 1 {
            Some(base_dir.get_child("test.shard"))
        } else {
            None
        };
        let test_warnings_path = base_dir.get_child("test.warnings");
        let unused_runfiles_log_path = base_dir.get_child("test.unused_runfiles_log");
        let test_stderr = base_dir.get_child("test.err");
        let split_logs_dir = base_dir.get_child("test.raw_splitlogs");
        // See note in {@link #getSplitLogsPath} on the choice of file name.
        let split_logs_path = split_logs_dir.get_child("test.splitlogs");
        let undeclared_outputs_annotations_dir = base_dir.get_child("test.outputs_manifest");
        let undeclared_outputs_manifest_path =
            undeclared_outputs_annotations_dir.get_child("MANIFEST");
        let undeclared_outputs_annotations_path =
            undeclared_outputs_annotations_dir.get_child("ANNOTATIONS");
        let undeclared_outputs_annotations_pb_path =
            undeclared_outputs_annotations_dir.get_child("ANNOTATIONS.pb");
        let test_infrastructure_failure = base_dir.get_child("test.infrastructure_failure");

        // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:1356 TestRunnerAction.LazySetConcatenation
        let required_client_env_variables: Vec<String> = configuration
            .action_environment
            .get_inherited_env()
            .into_iter()
            .chain(configuration.test_action_environment.get_inherited_env())
            .chain(extra_test_env.get_inherited_env())
            .collect();

        // Mark all possible test outputs for deletion before test execution.
        // TestRunnerAction potentially can create many more non-declared outputs - xml output,
        // coverage data file and logs for failed attempts. All those outputs are uniquely
        // identified by the test log base name with arbitrary prefix and extension.

        // We need to remove *.(xml|data|shard|warnings|zip) files if they are present.
        let mut files_to_delete: IndexSet<PathFragment> = IndexSet::new();
        files_to_delete.insert(test_warnings_path.clone());
        files_to_delete.insert(unused_runfiles_log_path.clone());
        files_to_delete.insert(test_stderr.clone());
        files_to_delete.insert(test_exit_safe.clone());
        files_to_delete.insert(test_infrastructure_failure.clone());
        // We cannot use coverageData artifact since it may be null. Generate coverage name
        // instead.
        files_to_delete.insert(base_dir.get_child("coverage.dat"));
        files_to_delete.insert(base_dir.get_child("test.zip")); // Delete files fetched from remote execution.
        if test_xml_artifact.is_none() {
            files_to_delete.insert(test_xml.get_exec_path().clone());
        }
        if let Some(test_shard) = &test_shard {
            files_to_delete.insert(test_shard.clone());
        }
        let mut directories_to_delete: IndexSet<PathFragment> = IndexSet::new();
        // Note that splitLogsPath points to a file inside the splitLogsDir so it's not necessary
        // to delete it explicitly.
        directories_to_delete.insert(split_logs_dir.clone());
        directories_to_delete.insert(undeclared_outputs_dir.get_exec_path().clone());
        directories_to_delete.insert(undeclared_outputs_annotations_dir.clone());
        directories_to_delete.insert(base_dir.get_relative_str("test_attempts"));

        Arc::new_cyclic(|self_ref| TestRunnerAction {
            abstract_action,
            self_ref: self_ref.clone(),
            runfiles_tree,
            test_setup_script,
            test_xml_generator_script,
            collect_coverage_script,
            configuration,
            test_log,
            test_xml,
            cache_status,
            test_warnings_path,
            unused_runfiles_log_path,
            sh_executable,
            split_logs_path,
            split_logs_dir,
            undeclared_outputs_annotations_dir,
            undeclared_outputs_manifest_path,
            undeclared_outputs_annotations_path,
            undeclared_outputs_annotations_pb_path,
            test_shard,
            test_exit_safe,
            test_stderr,
            test_infrastructure_failure,
            base_dir,
            files_to_delete_before_execution: files_to_delete,
            directories_to_delete_before_execution: directories_to_delete,
            coverage_data: coverage_artifact,
            coverage_directory,
            undeclared_outputs_dir,
            test_properties,
            execution_settings,
            shard_num,
            run_number,
            workspace_name,
            cached_test_result_data: Mutex::new(None),
            coverage_env,
            extra_test_env,
            required_client_env_variables,
            cancel_concurrent_tests,
            split_coverage_post_processing,
            lcov_merger_files_to_run,
            unrunnable_reason,
        })
    }

    /// The `Arc` this action lives in (Java passes `this` around).
    pub fn arc(&self) -> Arc<TestRunnerAction> {
        self.self_ref
            .upgrade()
            .expect("TestRunnerAction is always held in an Arc")
    }

    /// `getOwner().getLabel()`.
    pub fn get_owner_label(&self) -> Option<&Label> {
        self.get_owner().get_label()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:332 TestRunnerAction.allowLocalTests
    pub fn allow_local_tests(&self) -> bool {
        self.configuration.allow_local_tests
    }

    /// Returns the reason why this test can't be run in the current build, or `None` if it can
    /// be run.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:342 TestRunnerAction.getUnrunnableReason
    pub fn get_unrunnable_reason(&self) -> Option<&str> {
        self.unrunnable_reason.as_deref()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:347 TestRunnerAction.mayModifySpawnOutputsAfterExecution
    pub fn may_modify_spawn_outputs_after_execution(&self) -> bool {
        // Test actions modify test spawn outputs after execution:
        // - if there are multiple attempts (unavoidable);
        // - in all cases due to appending any stray stderr output to the test log in
        //   StandaloneTestStrategy.
        // TODO: Get rid of the second case and only return true if there are multiple attempts.
        true
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:356 TestRunnerAction.getRunfilesTree
    pub fn get_runfiles_tree(&self) -> &Artifact {
        &self.runfiles_tree
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:365 TestRunnerAction.getConfiguration
    pub fn get_configuration(&self) -> &Arc<TestRunnerActionConfiguration> {
        &self.configuration
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:369 TestRunnerAction.getBaseDir
    pub fn get_base_dir(&self) -> &PathFragment {
        &self.base_dir
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:373 TestRunnerAction.getSplitCoveragePostProcessing
    pub fn get_split_coverage_post_processing(&self) -> bool {
        self.split_coverage_post_processing
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:377 TestRunnerAction.getLcovMergerFilesToRun
    pub fn get_lcov_merger_files_to_run(&self) -> &NestedSet<Artifact> {
        &self.lcov_merger_files_to_run
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:381 TestRunnerAction.getCoverageDirectoryTreeArtifact
    pub fn get_coverage_directory_tree_artifact(&self) -> Option<&Artifact> {
        self.coverage_directory.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:390 TestRunnerAction.checkShardingSupport
    pub fn check_sharding_support(&self) -> bool {
        self.configuration.check_sharding_support
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:394 TestRunnerAction.getSpawnOutputs
    pub fn get_spawn_outputs(&self) -> Vec<ActionInputRef> {
        let from_path =
            |p: &PathFragment| ActionInputRef::new(BasicActionInput::from_path_fragment(p.clone()));
        let mut outputs: Vec<ActionInputRef> = Vec::new();
        outputs.push(self.test_xml.clone());
        outputs.push(from_path(self.get_exit_safe_file()));
        if self.is_sharded() {
            outputs.push(from_path(self.get_test_shard().unwrap()));
        }
        outputs.push(from_path(self.get_test_warnings_path()));
        outputs.push(from_path(self.get_split_logs_path()));
        outputs.push(from_path(self.get_unused_runfiles_log_path()));
        outputs.push(from_path(self.get_infrastructure_failure_file()));
        if self.configuration.zip_undeclared_test_outputs {
            outputs.push(from_path(&self.get_undeclared_outputs_zip_path()));
        } else {
            outputs.push(ActionInputRef::new(self.undeclared_outputs_dir.clone()));
        }
        outputs.push(from_path(self.get_undeclared_outputs_manifest_path()));
        outputs.push(from_path(self.get_undeclared_outputs_annotations_path()));
        outputs.push(from_path(self.get_undeclared_outputs_annotations_pb_path()));
        if self.is_coverage_mode() {
            if !self.split_coverage_post_processing {
                outputs.push(ActionInputRef::new(self.coverage_data.clone().unwrap()));
            }
            if let Some(coverage_directory) = &self.coverage_directory {
                outputs.push(ActionInputRef::new(coverage_directory.clone()));
            }
        }
        outputs
    }

    /// Returns the list of mappings from file name constants to output files. This method checks
    /// the file system for existence of these output files, so it must only be used after test
    /// execution.
    ///
    /// PORT: the `ArtifactPathResolver` is always `IDENTITY` (no action file system), so
    /// `resolver.toPath(artifact)` is `artifact.getPath()`.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:429 TestRunnerAction.getTestOutputsMapping
    pub fn get_test_outputs_mapping(&self, exec_root: Option<&Path>) -> io::Result<TestOutputs> {
        // TODO(tjgq): The existence checks below will incorrectly return false if the test action
        // was reconstructed from the action cache, as we don't populate the output filesystem on
        // an action cache hit. This is difficult to fix because some of the files below are
        // produced by test spawns, but not declared as action outputs, and only the latter are
        // stored in the action cache.
        let mut builder: TestOutputs = Vec::new();
        if self.get_test_log().get_path().exists() {
            builder.push((names::TEST_LOG.to_string(), self.get_test_log().get_path()));
        }
        if let Some(coverage_data) = self.get_coverage_data()
            && coverage_data.get_path().exists()
        {
            builder.push((names::TEST_COVERAGE.to_string(), coverage_data.get_path()));
        }
        if let Some(exec_root) = exec_root {
            let resolved_paths = self.resolve(exec_root);
            if resolved_paths.get_test_stderr().exists() {
                builder.push((
                    names::TEST_STDERR.to_string(),
                    resolved_paths.get_test_stderr(),
                ));
            }
            if resolved_paths.get_xml_output_path().exists() {
                builder.push((
                    names::TEST_XML.to_string(),
                    resolved_paths.get_xml_output_path(),
                ));
            }
            if resolved_paths.get_split_logs_path().exists() {
                builder.push((
                    names::SPLIT_LOGS.to_string(),
                    resolved_paths.get_split_logs_path(),
                ));
            }
            if resolved_paths.get_test_warnings_path().exists() {
                builder.push((
                    names::TEST_WARNINGS.to_string(),
                    resolved_paths.get_test_warnings_path(),
                ));
            }
            if self.configuration.zip_undeclared_test_outputs
                && resolved_paths.get_undeclared_outputs_zip_path().exists()
            {
                builder.push((
                    names::UNDECLARED_OUTPUTS_ZIP.to_string(),
                    resolved_paths.get_undeclared_outputs_zip_path(),
                ));
            }
            if !self.configuration.zip_undeclared_test_outputs
                && resolved_paths.get_undeclared_outputs_dir().exists()
            {
                add_all_files_in_undeclared_outputs_directory(
                    &mut builder,
                    &resolved_paths.get_undeclared_outputs_dir(),
                )?;
            }
            if resolved_paths
                .get_undeclared_outputs_manifest_path()
                .exists()
            {
                builder.push((
                    names::UNDECLARED_OUTPUTS_MANIFEST.to_string(),
                    resolved_paths.get_undeclared_outputs_manifest_path(),
                ));
            }
            if resolved_paths
                .get_undeclared_outputs_annotations_path()
                .exists()
            {
                builder.push((
                    names::UNDECLARED_OUTPUTS_ANNOTATIONS.to_string(),
                    resolved_paths.get_undeclared_outputs_annotations_path(),
                ));
            }
            if resolved_paths
                .get_undeclared_outputs_annotations_pb_path()
                .exists()
            {
                builder.push((
                    names::UNDECLARED_OUTPUTS_ANNOTATIONS_PB.to_string(),
                    resolved_paths.get_undeclared_outputs_annotations_pb_path(),
                ));
            }
            if resolved_paths.get_unused_runfiles_log_path().exists() {
                builder.push((
                    names::UNUSED_RUNFILES_LOG.to_string(),
                    resolved_paths.get_unused_runfiles_log_path(),
                ));
            }
            if resolved_paths.get_infrastructure_failure_file().exists() {
                builder.push((
                    names::TEST_INFRASTRUCTURE_FAILURE.to_string(),
                    resolved_paths.get_infrastructure_failure_file(),
                ));
            }
        }
        Ok(builder)
    }

    /// Returns whether the test should be executed unconditionally based on the test
    /// configuration, the test properties, and the previous test result when known.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:565 TestRunnerAction.executeUnconditionally
    pub fn execute_unconditionally_static(
        cache_test_results: TriState,
        prev_status: &dyn Fn() -> Option<TestResultData>, // lazy to avoid I/O if possible
        is_external: bool,
        runs_per_test: i32,
    ) -> bool {
        if !Self::should_accept_cached_result_static(cache_test_results, is_external, runs_per_test)
        {
            return true;
        }
        let Some(status) = prev_status() else {
            // Execute unconditionally if a previous test result is not available.
            return true;
        };
        if !status.cachable() {
            // Execute unconditionally if the previous test result was marked non-cacheable.
            // It seems that this can only happen with --experimental_cancel_concurrent_tests.
            return true;
        }
        if cache_test_results == TriState::AUTO && !status.test_passed() {
            // Execute unconditionally if the previous test result was a failure, as otherwise we
            // can get stuck forever in the event of a flaky failure.
            return true;
        }
        false
    }

    /// Saves cache status to disk.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:608 TestRunnerAction.saveCacheStatus
    pub fn save_cache_status(
        &self,
        action_execution_context: &dyn ActionExecutionContext,
        data: &TestResultData,
    ) -> io::Result<()> {
        let result = (|| {
            let mut out = action_execution_context
                .get_input_path(&self.cache_status)
                .get_output_stream()?;
            out.write_all(&data.to_byte_array())?;
            out.flush()
        })();
        match result {
            Ok(()) => {
                // set unconditionally at the end of test action execution
                *self.cached_test_result_data.lock() = Some(Some(data.clone()));
                Ok(())
            }
            Err(e) => {
                *self.cached_test_result_data.lock() = Some(None);
                Err(e)
            }
        }
    }

    /// Sets cachedTestResultData, if not already set, to the cached status from disk or empty
    /// optional if the file doesn't exist or if there is an error. Then returns
    /// cachedTestResultData.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:625 TestRunnerAction.maybeReadCacheStatus
    pub fn maybe_read_cache_status(&self) -> Option<TestResultData> {
        let mut cached = self.cached_test_result_data.lock();
        if cached.is_none() {
            match self.read_cache_status() {
                Ok(testing) => *cached = Some(Some(testing)),
                // FileNotFoundException and other IOExceptions ("Unexpected IOException thrown
                // while reading cached status." is logged at INFO level in Java).
                Err(_) => *cached = Some(None),
            }
        }
        cached.clone().expect("cachedTestResultData")
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:628 TestRunnerAction.readCacheStatus
    pub fn read_cache_status(&self) -> io::Result<TestResultData> {
        let mut bytes = Vec::new();
        self.cache_status
            .get_path()
            .get_input_stream()?
            .read_to_end(&mut bytes)?;
        TestResultData::decode(bytes.as_slice())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
    }

    /// Returns whether a cached result should be accepted from a disk/remote cache, depending on
    /// the test configuration and test properties.
    ///
    /// This should *not* be used to determine whether to accept a cached result from the action
    /// cache. Call `executeUnconditionally` instead.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:665 TestRunnerAction.shouldAcceptCachedResult
    pub fn should_accept_cached_result(&self) -> bool {
        Self::should_accept_cached_result_static(
            self.configuration.cache_test_results,
            self.test_properties.is_external(),
            self.execution_settings.get_total_runs(),
        )
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:665 TestRunnerAction.shouldAcceptCachedResult
    pub fn should_accept_cached_result_static(
        cache_test_results: TriState,
        is_external: bool,
        runs_per_test: i32,
    ) -> bool {
        if is_external || cache_test_results == TriState::NO {
            return false;
        }
        if cache_test_results == TriState::AUTO && runs_per_test > 1 {
            return false;
        }
        true
    }

    /// `NotifyOnActionCacheHit.actionCacheHit`: called by the action executor when the action
    /// cache hits; posts the cached `TestResult`. Returns false if the cached status could not be
    /// read (the action is then executed).
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:685 TestRunnerAction.actionCacheHit
    pub fn action_cache_hit(
        &self,
        event_handler: &dyn blaze_events::ExtendedEventHandler,
        exec_root: &Path,
        test_action_context: &dyn TestActionContext,
    ) -> bool {
        let Some(cached) = self.maybe_read_cache_status() else {
            event_handler.handle(&Event::warn(
                self.get_error_message_on_cached_test_result_error(),
            ));
            return false;
        };
        let posted = self
            .get_test_outputs_mapping(Some(exec_root))
            .and_then(|test_outputs| {
                test_action_context.new_cached_test_result(exec_root, self, cached, test_outputs)
            });
        match posted {
            Ok(result) => {
                event_handler.post(Arc::new(result));
                true
            }
            Err(e) => {
                event_handler.handle(&Event::warn(
                    self.get_error_message_on_new_cached_test_result_error(&e.to_string()),
                ));
                false
            }
        }
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:702 TestRunnerAction.getErrorMessageOnNewCachedTestResultError
    pub fn get_error_message_on_new_cached_test_result_error(&self, exception_msg: &str) -> String {
        format!(
            "{}: {}",
            self.get_error_message_on_cached_test_result_error(),
            exception_msg
        )
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:717 TestRunnerAction.getErrorMessageOnCachedTestResultError
    pub fn get_error_message_on_cached_test_result_error(&self) -> String {
        format!(
            "Cached test status was unexpectedly unavailable on disk: could be result of expired \
             authentication, bad disk, or modifications in the output tree. From {}",
            self.describe()
        )
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:730 TestRunnerAction.getAdditionalPathOutputsToDelete
    pub fn get_additional_path_outputs_to_delete(&self) -> &IndexSet<PathFragment> {
        &self.files_to_delete_before_execution
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:735 TestRunnerAction.getDirectoryOutputsToDelete
    pub fn get_directory_outputs_to_delete(&self) -> &IndexSet<PathFragment> {
        &self.directories_to_delete_before_execution
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:739 TestRunnerAction.createEmptyOutputs
    fn create_empty_outputs(&self, context: &dyn ActionExecutionContext) -> io::Result<()> {
        for output in self.get_outputs() {
            touch_file(&context.get_input_path(&output))?;
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:745 TestRunnerAction.setupEnvVariables
    pub fn setup_env_variables(&self, env: &mut IndexMap<String, String>) {
        // Allow --test_env and rules to overwite these values
        for (k, v) in &self.coverage_env {
            env.entry(k.clone()).or_insert_with(|| v.clone());
        }

        env.insert(
            "TEST_TARGET".to_string(),
            Label::print(self.get_owner().get_label()),
        );
        env.insert(
            "TEST_SIZE".to_string(),
            self.get_test_properties().get_size().to_string(),
        );
        env.insert(
            "TEST_TIMEOUT".to_string(),
            self.get_timeout().as_secs().to_string(),
        );
        env.insert(
            "TEST_WORKSPACE".to_string(),
            self.get_runfiles_prefix().to_string(),
        );
        env.insert(
            "TEST_BINARY".to_string(),
            self.get_execution_settings()
                .get_executable()
                .get_runfiles_path()
                .get_callable_path_string_for_os(&blaze_vfs::os_path_policy::HOST_POLICY),
        );

        // When we run test multiple times, set different TEST_RANDOM_SEED values for each run.
        // Don't override any previous setting.
        if self.execution_settings.get_total_runs() > 1 && !env.contains_key("TEST_RANDOM_SEED") {
            env.insert(
                "TEST_RANDOM_SEED".to_string(),
                (self.get_run_number() + 1).to_string(),
            );
        }
        // TODO(b/184206260): Actually set TEST_RANDOM_SEED with random seed.
        // The above TEST_RANDOM_SEED has historically been set with the run number, but we should
        // explicitly set TEST_RUN_NUMBER to indicate the run number and actually set
        // TEST_RANDOM_SEED with a random seed. However, much code has come to depend on it being
        // set to the run number and this is an externally documented behavior. Modifying
        // TEST_RANDOM_SEED should be done carefully.
        if self.execution_settings.get_total_runs() > 1 && !env.contains_key("TEST_RUN_NUMBER") {
            env.insert(
                "TEST_RUN_NUMBER".to_string(),
                (self.get_run_number() + 1).to_string(),
            );
        }

        if let Some(test_filter) = self.get_execution_settings().get_test_filter() {
            env.insert(
                TEST_BRIDGE_TEST_FILTER_ENV.to_string(),
                test_filter.to_string(),
            );
        }
        if self.configuration.test_runner_fail_fast {
            env.insert(
                "TESTBRIDGE_TEST_RUNNER_FAIL_FAST".to_string(),
                "1".to_string(),
            );
        }

        env.insert(
            "TEST_WARNINGS_OUTPUT_FILE".to_string(),
            self.get_test_warnings_path().get_path_string().to_string(),
        );
        env.insert(
            "TEST_UNUSED_RUNFILES_LOG_FILE".to_string(),
            self.get_unused_runfiles_log_path()
                .get_path_string()
                .to_string(),
        );

        env.insert(
            "TEST_LOGSPLITTER_OUTPUT_FILE".to_string(),
            self.get_split_logs_path().get_path_string().to_string(),
        );

        if self.configuration.zip_undeclared_test_outputs {
            env.insert(
                "TEST_UNDECLARED_OUTPUTS_ZIP".to_string(),
                self.get_undeclared_outputs_zip_path()
                    .get_path_string()
                    .to_string(),
            );
        }

        env.insert(
            "TEST_UNDECLARED_OUTPUTS_DIR".to_string(),
            self.undeclared_outputs_dir
                .get_exec_path_string()
                .to_string(),
        );
        env.insert(
            "TEST_UNDECLARED_OUTPUTS_MANIFEST".to_string(),
            self.get_undeclared_outputs_manifest_path()
                .get_path_string()
                .to_string(),
        );
        env.insert(
            "TEST_UNDECLARED_OUTPUTS_ANNOTATIONS".to_string(),
            self.get_undeclared_outputs_annotations_path()
                .get_path_string()
                .to_string(),
        );
        env.insert(
            "TEST_UNDECLARED_OUTPUTS_ANNOTATIONS_DIR".to_string(),
            self.get_undeclared_outputs_annotations_dir()
                .get_path_string()
                .to_string(),
        );

        env.insert(
            "TEST_PREMATURE_EXIT_FILE".to_string(),
            self.get_exit_safe_file().get_path_string().to_string(),
        );
        env.insert(
            "TEST_INFRASTRUCTURE_FAILURE_FILE".to_string(),
            self.get_infrastructure_failure_file()
                .get_path_string()
                .to_string(),
        );

        if self.is_sharded() {
            env.insert(
                "TEST_SHARD_INDEX".to_string(),
                self.get_shard_num().to_string(),
            );
            env.insert(
                "TEST_TOTAL_SHARDS".to_string(),
                self.get_execution_settings().get_total_shards().to_string(),
            );
            env.insert(
                "TEST_SHARD_STATUS_FILE".to_string(),
                self.get_test_shard().unwrap().get_path_string().to_string(),
            );
        }
        env.insert(
            "XML_OUTPUT_FILE".to_string(),
            self.test_xml.get_exec_path_string(),
        );

        if !self.configuration.runfiles_enabled {
            // If runfiles are disabled, tell remote-runtest.sh/local-runtest.sh about that.
            env.insert("RUNFILES_MANIFEST_ONLY".to_string(), "1".to_string());
        }

        if self.is_coverage_mode() {
            // Instruct remote-runtest.sh/local-runtest.sh not to cd into the runfiles directory.
            // TODO(ulfjack): Find a way to avoid setting this variable.
            env.insert("RUNTEST_PRESERVE_CWD".to_string(), "1".to_string());

            env.insert(
                "COVERAGE_MANIFEST".to_string(),
                self.get_coverage_manifest()
                    .expect("NullPointerException: getCoverageManifest()")
                    .get_exec_path_string()
                    .to_string(),
            );
            env.insert(
                "COVERAGE_DIR".to_string(),
                self.get_coverage_directory().get_path_string().to_string(),
            );
            env.insert(
                "COVERAGE_OUTPUT_FILE".to_string(),
                self.get_coverage_data()
                    .unwrap()
                    .get_exec_path_string()
                    .to_string(),
            );
            env.insert(
                "SPLIT_COVERAGE_POST_PROCESSING".to_string(),
                if self.split_coverage_post_processing {
                    "1"
                } else {
                    "0"
                }
                .to_string(),
            );
            env.insert("IS_COVERAGE_SPAWN".to_string(), "0".to_string());
        }
    }

    /// Gets the test name in a user-friendly format. Will generally include the target name and
    /// run/shard numbers, if applicable.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:726 TestRunnerAction.getTestName
    pub fn get_test_name(&self) -> String {
        let suffix = self.get_test_suffix();
        let label = Label::print(self.get_owner().get_label());
        if suffix.is_empty() {
            label
        } else {
            format!("{label} {suffix}")
        }
    }

    /// Gets the test suffix in a user-friendly format, eg "(shard 1 of 7)". Will include the
    /// target name and run/shard numbers, if applicable.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:834 TestRunnerAction.getTestSuffix
    pub fn get_test_suffix(&self) -> String {
        let total_shards = self.execution_settings.get_total_shards();
        // Use a 1-based index for user friendliness.
        let runs_per_test = self.execution_settings.get_total_runs();
        if total_shards > 1 && runs_per_test > 1 {
            format!(
                "(shard {} of {}, run {} of {})",
                self.shard_num + 1,
                total_shards,
                self.run_number + 1,
                runs_per_test
            )
        } else if total_shards > 1 {
            format!("(shard {} of {})", self.shard_num + 1, total_shards)
        } else if runs_per_test > 1 {
            format!("(run {} of {})", self.run_number + 1, runs_per_test)
        } else {
            String::new()
        }
    }

    /// Returns the timeout for this test action, respecting the value of `--test_timeout`.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:860 TestRunnerAction.getTimeout
    pub fn get_timeout(&self) -> Duration {
        *self
            .configuration
            .test_timeout
            .get(&self.test_properties.get_timeout())
            .expect("NullPointerException: TestConfiguration.getTestTimeout().get(timeout)")
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:864 TestRunnerAction.getTestLog
    pub fn get_test_log(&self) -> &Artifact {
        &self.test_log
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:868 TestRunnerAction.getTestXml
    pub fn get_test_xml(&self) -> &ActionInputRef {
        &self.test_xml
    }

    /// Returns all environment variables which must be set in order to run this test.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:873 TestRunnerAction.getExtraTestEnv
    pub fn get_extra_test_env(&self) -> &Arc<ActionEnvironment> {
        &self.extra_test_env
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:444 TestRunnerAction.resolve
    pub fn resolve(&self, exec_root: &Path) -> ResolvedPaths<'_> {
        ResolvedPaths {
            action: self,
            exec_root: exec_root.clone(),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:886 TestRunnerAction.getCacheStatusArtifact
    pub fn get_cache_status_artifact(&self) -> &Artifact {
        &self.cache_status
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:890 TestRunnerAction.getTestStderrPath
    pub fn get_test_stderr_path(&self) -> &PathFragment {
        &self.test_stderr
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:783 TestRunnerAction.getTestWarningsPath
    pub fn get_test_warnings_path(&self) -> &PathFragment {
        &self.test_warnings_path
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:784 TestRunnerAction.getUnusedRunfilesLogPath
    pub fn get_unused_runfiles_log_path(&self) -> &PathFragment {
        &self.unused_runfiles_log_path
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:786 TestRunnerAction.getSplitLogsPath
    pub fn get_split_logs_path(&self) -> &PathFragment {
        &self.split_logs_path
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:906 TestRunnerAction.getUndeclaredOutputsDir
    pub fn get_undeclared_outputs_dir(&self) -> &Artifact {
        &self.undeclared_outputs_dir
    }

    /// Returns path to the optional zip file of undeclared test outputs.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:789 TestRunnerAction.getUndeclaredOutputsZipPath
    pub fn get_undeclared_outputs_zip_path(&self) -> PathFragment {
        self.undeclared_outputs_dir
            .get_exec_path()
            .get_child(UNDECLARED_OUTPUTS_ZIP_NAME)
    }

    /// Returns path to the undeclared output manifest file.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:793 TestRunnerAction.getUndeclaredOutputsManifestPath
    pub fn get_undeclared_outputs_manifest_path(&self) -> &PathFragment {
        &self.undeclared_outputs_manifest_path
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:799 TestRunnerAction.getUndeclaredOutputsAnnotationsDir
    pub fn get_undeclared_outputs_annotations_dir(&self) -> &PathFragment {
        &self.undeclared_outputs_annotations_dir
    }

    /// Returns path to the undeclared output annotations file.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:796 TestRunnerAction.getUndeclaredOutputsAnnotationsPath
    pub fn get_undeclared_outputs_annotations_path(&self) -> &PathFragment {
        &self.undeclared_outputs_annotations_path
    }

    /// Returns path to the undeclared output annotations file.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:930 TestRunnerAction.getUndeclaredOutputsAnnotationsPbPath
    pub fn get_undeclared_outputs_annotations_pb_path(&self) -> &PathFragment {
        &self.undeclared_outputs_annotations_pb_path
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:807 TestRunnerAction.getTestShard
    pub fn get_test_shard(&self) -> Option<&PathFragment> {
        self.test_shard.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:801 TestRunnerAction.getExitSafeFile
    pub fn get_exit_safe_file(&self) -> &PathFragment {
        &self.test_exit_safe
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:802 TestRunnerAction.getInfrastructureFailureFile
    pub fn get_infrastructure_failure_file(&self) -> &PathFragment {
        &self.test_infrastructure_failure
    }

    /// Returns coverage data artifact or `None` if code coverage was not requested.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:823 TestRunnerAction.getCoverageData
    pub fn get_coverage_data(&self) -> Option<&Artifact> {
        self.coverage_data.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:821 TestRunnerAction.getCoverageManifest
    pub fn get_coverage_manifest(&self) -> Option<&Artifact> {
        self.get_execution_settings()
            .get_instrumented_file_manifest()
    }

    /// Returns true if coverage data should be gathered.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:958 TestRunnerAction.isCoverageMode
    pub fn is_coverage_mode(&self) -> bool {
        self.coverage_data.is_some()
    }

    /// Returns a directory to temporarily store coverage results for the given action relative to
    /// the execution root. This directory is used to store all coverage results related to the
    /// test execution with exception of the locally generated *.gcda files. Those are stored
    /// separately using relative path within coverage directory.
    ///
    /// If the coverageDirectory field is set, then its exec path is returned. This is a tree
    /// artifact, meaning that all files in the corresponding directories are returned from
    /// sandboxed or remote execution.
    ///
    /// Otherwise, the directory name for the given test runner action is constructed as:
    /// `[blaze-out/.../testlogs/]_coverage/target_path/test_log_name` where `test_log_name` is
    /// usually a target name but potentially can include extra suffix, such as a shard number (if
    /// test execution was sharded).
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:822 TestRunnerAction.getCoverageDirectory
    pub fn get_coverage_directory(&self) -> PathFragment {
        if let Some(coverage_directory) = &self.coverage_directory {
            return coverage_directory.get_exec_path().clone();
        }
        let coverage_root = self
            .get_test_log()
            .get_root()
            .get_exec_path()
            .get_relative_str(COVERAGE_TMP_ROOT);
        // FileSystemUtils.removeExtension(PathFragment): replaceName(removeExtension(baseName)).
        let root_relative = self.get_test_log().get_root_relative_path();
        let without_extension = root_relative
            .replace_name(remove_extension(root_relative.get_base_name()))
            .expect("test log has a name");
        coverage_root.get_relative(&without_extension)
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:750 TestRunnerAction.getTestProperties
    pub fn get_test_properties(&self) -> &TestTargetProperties {
        &self.test_properties
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:755 TestRunnerAction.getExecutionSettings
    pub fn get_execution_settings(&self) -> &TestTargetExecutionSettings {
        &self.execution_settings
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:999 TestRunnerAction.isSharded
    pub fn is_sharded(&self) -> bool {
        self.test_shard.is_some()
    }

    /// Returns the shard number for this action. If getTotalShards() > 0, must be >= 0 and <
    /// getTotalShards(). Otherwise, must be 0.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:1007 TestRunnerAction.getShardNum
    pub fn get_shard_num(&self) -> i32 {
        self.shard_num
    }

    /// Returns run number.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:1012 TestRunnerAction.getRunNumber
    pub fn get_run_number(&self) -> i32 {
        self.run_number
    }

    /// Returns the workspace name.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:752 TestRunnerAction.getRunfilesPrefix
    pub fn get_runfiles_prefix(&self) -> &str {
        &self.workspace_name
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:1115 TestRunnerAction.getTestSetupScript
    pub fn get_test_setup_script(&self) -> &Artifact {
        &self.test_setup_script
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:1119 TestRunnerAction.getTestXmlGeneratorScript
    pub fn get_test_xml_generator_script(&self) -> &Artifact {
        &self.test_xml_generator_script
    }

    /// `getCollectCoverageScript().getExecutable()`.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:1124 TestRunnerAction.getCollectCoverageScript
    pub fn get_collect_coverage_script(&self) -> Option<&Artifact> {
        self.collect_coverage_script.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:1129 TestRunnerAction.getShExecutableMaybe
    pub fn get_sh_executable_maybe(&self) -> Option<&PathFragment> {
        self.sh_executable.as_ref()
    }

    /// `execute(actionExecutionContext, testActionContext)`.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:1027 TestRunnerAction.execute
    #[allow(clippy::result_large_err)]
    pub fn execute_with_context(
        &self,
        action_execution_context: &dyn ActionExecutionContext,
        test_action_context: &dyn TestActionContext,
    ) -> Result<ActionResult, ActionExecutionException> {
        if let Some(unrunnable_reason) = &self.unrunnable_reason {
            let failure_detail = FailureDetail {
                message: unrunnable_reason.clone(),
                category: Some(failure_detail::Category::Toolchain(
                    failure_details::Toolchain {
                        code: failure_details::toolchain::Code::NoMatchingToolchain as i32,
                    },
                )),
            };
            return Err(ActionExecutionException::new(
                unrunnable_reason.clone(),
                Some(self),
                /* catastrophe= */ false,
                Some(failure_detail),
            ));
        }

        let mut spawn_results: Vec<SpawnResult> = Vec::new();
        let mut failed_attempts: Vec<Box<dyn ProcessedAttemptResult>> = Vec::new();

        let outcome: Result<ActionResult, TestRunnerSpawnError> = (|| {
            let test_runner_spawn =
                test_action_context.create_test_runner_spawn(self, action_execution_context)?;
            let attempt_group = if self.cancel_concurrent_tests != CancelConcurrentTests::NEVER {
                test_action_context.get_attempt_group(self.get_owner(), self.shard_num)
            } else {
                attempt_group_noop()
            };
            let cancel_on_result = match self.cancel_concurrent_tests {
                CancelConcurrentTests::NEVER => None,
                CancelConcurrentTests::ON_FAILED => Some(TestAttemptResultResult::FailedCanRetry),
                CancelConcurrentTests::ON_PASSED => Some(TestAttemptResultResult::Passed),
            };
            let result = (|| {
                attempt_group.register()?;
                // PORT: Java then checks `Thread.interrupted()` for an attempt cancelled after it
                // finished; Blaze has no thread interruption.
                self.execute_all_attempts(
                    &*test_runner_spawn,
                    test_action_context.is_test_keep_going(),
                    cancel_on_result,
                    &*attempt_group,
                    &mut spawn_results,
                    &mut failed_attempts,
                )
            })();
            attempt_group.unregister();
            match result {
                Err(TestRunnerSpawnError::Exec(ExecException::Interrupted)) => {
                    if !attempt_group.cancelled() {
                        return Err(TestRunnerSpawnError::Exec(ExecException::Interrupted));
                    }

                    test_runner_spawn.finalize_cancelled_test(&failed_attempts)?;
                    self.create_empty_outputs(test_runner_spawn.get_action_execution_context())?;
                    Ok(ActionResult::create(spawn_results.clone()))
                }
                other => other,
            }
        })();

        match outcome {
            Ok(r) => Ok(r),
            Err(TestRunnerSpawnError::Exec(e)) => Err(
                ActionExecutionException::from_exec_exception(&e, None, self),
            ),
            Err(TestRunnerSpawnError::Io(e)) => {
                // PORT: `new EnvironmentalExecException(IOException, Code)` takes the cause's
                // `toString()` as its message; the Rust I/O error's message stands for it.
                let e = ExecException::Environmental(EnvironmentalExecException {
                    message: e.to_string(),
                    failure_detail: FailureDetail {
                        message: String::new(),
                        category: Some(failure_detail::Category::Execution(
                            failure_details::Execution {
                                code: failure_details::execution::Code::TestRunnerIoException
                                    as i32,
                            },
                        )),
                    },
                });
                Err(ActionExecutionException::from_exec_exception(
                    &e, None, self,
                ))
            }
        }
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:1068 TestRunnerAction.executeAllAttempts
    pub fn execute_all_attempts<'a>(
        &self,
        test_runner_spawn: &(dyn TestRunnerSpawn<'a> + 'a),
        keep_going: bool,
        cancel_on_result: Option<TestAttemptResultResult>,
        attempt_group: &dyn super::test_action_context::AttemptGroup,
        spawn_results: &mut Vec<SpawnResult>,
        failed_attempts: &mut Vec<Box<dyn ProcessedAttemptResult>>,
    ) -> Result<ActionResult, TestRunnerSpawnError> {
        let mut max_attempts = 0;
        let mut owned_runner: Option<Box<dyn TestRunnerSpawn<'a> + 'a>> = None;

        loop {
            let current: &(dyn TestRunnerSpawn<'a> + 'a) =
                owned_runner.as_deref().unwrap_or(test_runner_spawn);
            let result = current.execute()?;
            let actual_max_attempts = if failed_attempts.is_empty() {
                current.get_max_attempts(&*result)
            } else {
                max_attempts
            };
            blaze_util::check_state!(actual_max_attempts != 0);

            spawn_results.extend(result.spawn_results().iter().cloned());
            let test_result = result.result();
            if Some(test_result) == cancel_on_result {
                attempt_group.cancel_others();
            }
            if test_result != TestAttemptResultResult::Passed {
                let next_runner_and_attempts = compute_next_runner_and_max_attempts(
                    test_result,
                    current,
                    (failed_attempts.len() + 1) as i32,
                    actual_max_attempts,
                    spawn_results,
                )?;
                if let Some((next_runner, next_max_attempts)) = next_runner_and_attempts {
                    failed_attempts.push(current.finalize_failed_test_attempt(
                        &*result,
                        (failed_attempts.len() + 1) as i32,
                    )?);

                    // Change the phase here because we are executing a rerun of the failed
                    // attempt.
                    current
                        .get_action_execution_context()
                        .get_event_handler()
                        .post(Arc::new(SpawnExecutedEventChangePhase {
                            action: self.arc(),
                        }));

                    if let NextTestRunnerSpawn::Other(next) = next_runner {
                        owned_runner = Some(next);
                    }
                    max_attempts = next_max_attempts;
                    continue;
                }
            }
            current.finalize_test(&*result, failed_attempts)?;

            if !keep_going && test_result != TestAttemptResultResult::Passed {
                if let Some(system_failure) = result.primary_system_failure() {
                    return Err(TestRunnerSpawnError::Exec(test_exec_exception(
                        format!(
                            "Test failed (system error), aborting: {}",
                            system_failure.message
                        ),
                        system_failure,
                    )));
                }
                let error_message = "Test failed, aborting";
                return Err(TestRunnerSpawnError::Exec(test_exec_exception(
                    error_message.to_string(),
                    FailureDetail {
                        message: error_message.to_string(),
                        category: Some(failure_detail::Category::TestAction(
                            failure_details::TestAction {
                                code: failure_details::test_action::Code::NoKeepGoingTestFailure
                                    as i32,
                            },
                        )),
                    },
                )));
            }
            return Ok(ActionResult::create(spawn_results.clone()));
        }
    }
}

/// `new TestExecException(message, failureDetail)`.
///
/// PORT: `TestExecException` behaves as a `UserExecException` with its own message (not
/// catastrophic, `getFailureDetail(message)` replaces the detail's message), so it is one,
/// marked `test_exec_exception` for the `instanceof TestExecException` of
/// `SkyframeErrorProcessor.rethrow` (see
/// [`ActionExecutionException::is_caused_by_test_exec_exception`]).
// Java: com/google/devtools/build/lib/actions/TestExecException.java:24 TestExecException.<init>
pub fn test_exec_exception(message: String, failure_detail: FailureDetail) -> ExecException {
    ExecException::User(UserExecException {
        message,
        failure_detail,
        test_exec_exception: true,
    })
}

/// Method used to compute next runner and max attempts. Returns `None` if there if there is no
/// remaining attempts (including fallback runner).
// Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:1270 TestRunnerAction.computeNextRunnerAndMaxAttempts
pub fn compute_next_runner_and_max_attempts<'a>(
    result: TestAttemptResultResult,
    test_runner_spawn: &(dyn TestRunnerSpawn<'a> + 'a),
    num_attempts: i32,
    max_attempts: i32,
    results: &[SpawnResult],
) -> Result<Option<(NextTestRunnerSpawn<'a>, i32)>, ExecException> {
    blaze_util::check_state!(
        result != TestAttemptResultResult::Passed,
        "Should not compute retry runner if last result passed"
    );
    if result.can_retry() && num_attempts < max_attempts {
        if let Some(next_runner) = test_runner_spawn.get_flaky_retry_runner(results)? {
            return Ok(Some((next_runner, max_attempts)));
        }
    } else if let Some(next_runner) = test_runner_spawn.get_fallback_runner()? {
        // We only support one level of fallback, in which case maxAttempts gets *added* once. We
        // don't support a different number of max attempts for the fallback strategy.
        return Ok(Some((
            NextTestRunnerSpawn::Other(next_runner),
            num_attempts + max_attempts,
        )));
    }
    Ok(None)
}

// Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:465 TestRunnerAction.addAllFilesInUndeclaredOutputsDirectory
fn add_all_files_in_undeclared_outputs_directory(
    builder: &mut TestOutputs,
    undeclared_outputs_dir: &Path,
) -> io::Result<()> {
    let mut dirs_to_visit: std::collections::VecDeque<Path> = std::collections::VecDeque::new();
    dirs_to_visit.push_back(undeclared_outputs_dir.clone());
    // PORT: Java's ArrayDeque.add appends and pop removes the head (FIFO).
    while let Some(dir) = dirs_to_visit.pop_front() {
        let mut sorted_entries = dir.readdir(Symlinks::Follow)?;
        sorted_entries.sort_by(|a, b| a.get_name().cmp(b.get_name()));
        for dirent in sorted_entries {
            let child = dir.get_child(dirent.get_name());
            match dirent.get_type() {
                blaze_vfs::dirent::DirentType::Directory => dirs_to_visit.push_back(child),
                blaze_vfs::dirent::DirentType::File => {
                    let name = format!(
                        "{}/{}",
                        names::UNDECLARED_OUTPUTS_DIR,
                        child.relative_to(undeclared_outputs_dir)
                    );
                    builder.push((name, child));
                }
                _ => {}
            }
        }
    }
    Ok(())
}

/// The same set of paths as the parent test action, resolved against a given exec root.
// Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:1150 TestRunnerAction.ResolvedPaths
pub struct ResolvedPaths<'a> {
    action: &'a TestRunnerAction,
    exec_root: Path,
}

impl ResolvedPaths<'_> {
    fn get_path(&self, relative_path: &PathFragment) -> Path {
        self.exec_root.get_relative(relative_path)
    }

    pub fn get_base_dir(&self) -> Path {
        self.get_path(&self.action.base_dir)
    }

    /// In rare cases, error messages will be printed to stderr instead of stdout. The test action
    /// is responsible for appending anything in the stderr file to the real test.log.
    pub fn get_test_stderr(&self) -> Path {
        self.get_path(&self.action.test_stderr)
    }

    pub fn get_test_warnings_path(&self) -> Path {
        self.get_path(&self.action.test_warnings_path)
    }

    pub fn get_split_logs_path(&self) -> Path {
        self.get_path(&self.action.split_logs_path)
    }

    pub fn get_unused_runfiles_log_path(&self) -> Path {
        self.get_path(&self.action.unused_runfiles_log_path)
    }

    /// Returns path to the directory containing the split logs (raw and proto file).
    pub fn get_split_logs_dir(&self) -> Path {
        self.get_path(&self.action.split_logs_dir)
    }

    /// Returns path to the optional zip file of undeclared test outputs.
    pub fn get_undeclared_outputs_zip_path(&self) -> Path {
        self.get_undeclared_outputs_dir()
            .get_child(UNDECLARED_OUTPUTS_ZIP_NAME)
    }

    /// Returns path to the directory to hold undeclared test outputs.
    pub fn get_undeclared_outputs_dir(&self) -> Path {
        self.get_path(self.action.undeclared_outputs_dir.get_exec_path())
    }

    /// Returns path to the directory to hold undeclared output annotations parts.
    pub fn get_undeclared_outputs_annotations_dir(&self) -> Path {
        self.get_path(&self.action.undeclared_outputs_annotations_dir)
    }

    /// Returns path to the undeclared output manifest file.
    pub fn get_undeclared_outputs_manifest_path(&self) -> Path {
        self.get_path(&self.action.undeclared_outputs_manifest_path)
    }

    /// Returns path to the undeclared output annotations file.
    pub fn get_undeclared_outputs_annotations_path(&self) -> Path {
        self.get_path(&self.action.undeclared_outputs_annotations_path)
    }

    /// Returns path to the undeclared output annotations pb file.
    pub fn get_undeclared_outputs_annotations_pb_path(&self) -> Path {
        self.get_path(&self.action.undeclared_outputs_annotations_pb_path)
    }

    pub fn get_test_shard(&self) -> Option<Path> {
        self.action.test_shard.as_ref().map(|s| self.get_path(s))
    }

    pub fn get_exit_safe_file(&self) -> Path {
        self.get_path(&self.action.test_exit_safe)
    }

    pub fn get_infrastructure_failure_file(&self) -> Path {
        self.get_path(&self.action.test_infrastructure_failure)
    }

    /// Returns path to the optionally created XML output file created by the test.
    pub fn get_xml_output_path(&self) -> Path {
        self.get_path(self.action.test_xml.get_exec_path())
    }

    pub fn get_coverage_directory(&self) -> Path {
        self.get_path(&self.action.get_coverage_directory())
    }

    pub fn get_coverage_data_path(&self) -> Path {
        self.get_path(self.action.get_coverage_data().unwrap().get_exec_path())
    }
}

impl Action for TestRunnerAction {
    fn abstract_action(&self) -> &AbstractAction {
        &self.abstract_action
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:1106 TestRunnerAction.getMnemonic
    fn get_mnemonic(&self) -> String {
        MNEMONIC.to_string()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:526 TestRunnerAction.computeKey
    fn compute_key(
        &self,
        _action_key_context: &ActionKeyContext,
        _input_metadata_provider: Option<&dyn KeyInputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        // TODO(b/150305897): use addUUID?
        fp.add_string(GUID);
        fp.add_iterable_strings(self.execution_settings.get_args().arguments()?);
        fp.add_string(self.execution_settings.get_test_filter().unwrap_or(""));
        fp.add_boolean(self.execution_settings.get_test_runner_fail_fast());
        let run_under = self.execution_settings.get_run_under();
        fp.add_string(run_under.map(|r| r.value()).unwrap_or(""));
        fp.add_string_map(
            self.coverage_env
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str())),
        );
        self.extra_test_env.add_to(fp);
        // TODO(ulfjack): It might be better for performance to hash the action and test envs in
        // config, and only add a hash here.
        self.configuration.action_environment.add_to(fp);
        self.configuration.test_action_environment.add_to(fp);
        // The 'requiredClientEnvVariables' are handled by Skyframe and don't need to be added
        // here.
        fp.add_string(&self.test_properties.get_size().to_string());
        fp.add_string(&self.test_properties.get_timeout().to_string());
        fp.add_string(&java_duration_to_string(self.get_timeout()));
        fp.add_strings(self.test_properties.get_tags().iter());
        fp.add_boolean(self.test_properties.is_remotable());
        fp.add_int(self.shard_num);
        fp.add_int(self.execution_settings.get_total_shards());
        fp.add_int(self.run_number);
        fp.add_int(self.execution_settings.get_total_runs());
        fp.add_boolean(self.configuration.code_coverage_enabled);
        fp.add_boolean(self.configuration.zip_undeclared_test_outputs);
        let execution_info = self.get_execution_info();
        fp.add_string_map(execution_info.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        fp.add_nullable_string(self.unrunnable_reason.as_deref());
        Ok(())
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:361 TestRunnerAction.getEnvironment
    fn get_environment(&self) -> Arc<ActionEnvironment> {
        self.configuration.action_environment.clone()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:878 TestRunnerAction.getClientEnvironmentVariables
    fn get_client_environment_variables(&self) -> Vec<String> {
        self.required_client_env_variables.clone()
    }

    /// Returns whether the test should be executed unconditionally based on the test
    /// configuration, the test properties, and the previous test result when known.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:565 TestRunnerAction.executeUnconditionally
    fn execute_unconditionally(&self) -> bool {
        // Note: isVolatile must return true if executeUnconditionally can ever return true
        // for this instance.
        Self::execute_unconditionally_static(
            self.configuration.cache_test_results,
            &|| self.maybe_read_cache_status(),
            self.test_properties.is_external(),
            self.execution_settings.get_total_runs(),
        )
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:603 TestRunnerAction.isVolatile
    fn is_volatile(&self) -> bool {
        true
    }

    // Test actions are always distinguished by their target name, which must be unique.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:521 TestRunnerAction.isShareable
    fn is_shareable(&self) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:386 TestRunnerAction.showsOutputUnconditionally
    fn shows_output_unconditionally(&self) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:725 TestRunnerAction.getRawProgressMessage
    fn get_raw_progress_message(&self) -> Option<String> {
        Some(format!("Testing {}", self.get_test_name()))
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:1111 TestRunnerAction.getMandatoryOutputs
    fn get_mandatory_outputs(&self) -> Vec<Artifact> {
        self.get_outputs()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:991 TestRunnerAction.getExecutionInfo
    fn get_execution_info(&self) -> IndexMap<String, String> {
        merge_maps(
            &self.get_exec_properties(),
            self.test_properties.get_execution_info(),
        )
    }

    fn as_command_action(&self) -> Option<&dyn CommandAction> {
        Some(self)
    }
}

impl CommandAction for TestRunnerAction {
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:1134 TestRunnerAction.getArguments
    fn get_arguments(&self) -> Result<Vec<String>, CommandLineError> {
        TestStrategy::expanded_args_from_action(self)
    }
}

impl ExecutableAction for TestRunnerAction {
    /// `AbstractAction.prepare` → `deleteOutputs`, with this class's
    /// `getAdditionalPathOutputsToDelete` and `getDirectoryOutputsToDelete` overrides.
    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:438 AbstractAction.deleteOutputs
    fn prepare(&self, exec_root: &Path) -> io::Result<()> {
        crate::actions::action_execution::delete_outputs(self, exec_root)?;

        for path in self.get_additional_path_outputs_to_delete() {
            crate::actions::action_execution::delete_output(
                &exec_root.get_relative(path),
                /* root= */ None,
            )?;
        }

        for path in self.get_directory_outputs_to_delete() {
            exec_root.get_relative(path).delete_tree()?;
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:1027 TestRunnerAction.execute
    fn execute(
        &self,
        action_execution_context: &dyn ActionExecutionContext,
    ) -> Result<ActionResult, ActionExecutionException> {
        let context = get_context::<Arc<dyn TestActionContext>>(action_execution_context)
            .expect("NullPointerException: getContext(TestActionContext.class)");
        self.execute_with_context(action_execution_context, &**context)
    }
}
