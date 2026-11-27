//! Port of `analysis/test/TestRunnerAction.java` (the analysis-time parts: construction, key,
//! outputs, environment, arguments and the fields aquery and test execution read) and
//! `TestStrategy.expandedArgsFromAction`.

use std::any::Any;
use std::fmt;
use std::sync::{Arc, OnceLock};

use blaze_actions::actions::action::{CommandLineError, merge_maps};
use blaze_actions::{
    AbstractAction, Action, ActionEnvironment, ActionKeyContext, ActionOwner, Artifact,
    CommandAction, ComputeKeyError, InputMetadataProvider,
};
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_build_config::fragments::test_configuration::TestConfiguration;
use blaze_cmdline::Label;
use blaze_collect::NestedSet;
use blaze_optdefs::fragment::run_under::RunUnder;
use blaze_optdefs::fragment::test_configuration::TestConfigurationTestOptionsCancelConcurrentTests;
use blaze_shell::util::os::OS;
use blaze_util::{Fingerprint, check_state};
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};

use crate::providers::file_provider::FilesToRunProvider;
use crate::providers::package_specification_provider::PackageSpecificationProvider;
use crate::rule::action_environments::to_actions_env;
use crate::rule::config_util;
use crate::test::test_provider::TestOutput;
use crate::test::test_target_properties::{TestTargetExecutionSettings, TestTargetProperties};

// Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:104 TestRunnerAction.COVERAGE_TMP_ROOT
pub const COVERAGE_TMP_ROOT: &str = "_coverage";
// Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:106 TestRunnerAction.UNDECLARED_OUTPUTS_ZIP_NAME
const UNDECLARED_OUTPUTS_ZIP_NAME: &str = "outputs.zip";
// Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:111 TestRunnerAction.GUID
const GUID: &str = "cc41f9d0-47a6-11e7-8726-eb6ce83a8cc8";
// Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:112 TestRunnerAction.MNEMONIC
pub const MNEMONIC: &str = "TestRunner";

/// The constructor arguments of a [`TestRunnerAction`] (Java's 26 positional parameters).
pub struct TestRunnerActionArgs {
    pub owner: ActionOwner,
    pub inputs: NestedSet<Artifact>,
    pub runfiles_tree: Artifact,
    /// Must be in inputs.
    pub test_setup_script: Artifact,
    /// Must be in inputs.
    pub test_xml_generator_script: Artifact,
    /// filesToRun must be in input, if not null.
    pub collect_coverage_script: Option<Arc<FilesToRunProvider>>,
    pub test_log: Artifact,
    /// See `TestActionBuilder.TEST_XML_IS_ACTION_OUTPUT`: always an artifact in Bazel.
    pub test_xml: Artifact,
    pub cache_status: Artifact,
    pub coverage_artifact: Option<Artifact>,
    pub coverage_directory: Option<Artifact>,
    pub undeclared_outputs_dir: Artifact,
    pub test_properties: TestTargetProperties,
    /// Java `ImmutableMap.copyOf(TreeMap)`: sorted by key.
    pub coverage_env: IndexMap<String, String>,
    pub extra_test_env: ActionEnvironment,
    pub execution_settings: TestTargetExecutionSettings,
    pub shard_num: i32,
    pub run_number: i32,
    pub configuration: Arc<BuildConfigurationValue>,
    pub workspace_name: String,
    pub sh_executable: Option<PathFragment>,
    pub cancel_concurrent_tests: TestConfigurationTestOptionsCancelConcurrentTests,
    pub split_coverage_post_processing: bool,
    pub lcov_merger_files_to_run: NestedSet<Artifact>,
    pub network_allowlist: Arc<PackageSpecificationProvider>,
    pub unrunnable_reason: Option<String>,
}

/// An Action representing a test with the associated environment (runfiles, environment
/// variables, test result, etc). It consumes test executable and runfiles artifacts and produces
/// test result and test status artifacts.
// Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:101 TestRunnerAction
pub struct TestRunnerAction {
    abstract_action: AbstractAction,
    runfiles_tree: Artifact,
    test_setup_script: Artifact,
    test_xml_generator_script: Artifact,
    collect_coverage_script: Option<Arc<FilesToRunProvider>>,
    configuration: Arc<BuildConfigurationValue>,
    test_log: Artifact,
    test_xml: Artifact,
    cache_status: Artifact,
    test_warnings_path: PathFragment,
    unused_runfiles_log_path: PathFragment,
    sh_executable: Option<PathFragment>,
    split_logs_path: PathFragment,
    // PORT: read by `TestRunnerAction.ResolvedPaths.getSplitLogsDir` (test execution, not
    // ported yet).
    #[allow(dead_code)]
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
    /// Environment variables specific to running code coverage.
    coverage_env: IndexMap<String, String>,
    /// Any extra environment variables (and values) added by the rule that created this action.
    extra_test_env: ActionEnvironment,
    /// The set of environment variables that are inherited from the client environment.
    required_client_env_variables: Vec<String>,
    cancel_concurrent_tests: TestConfigurationTestOptionsCancelConcurrentTests,
    split_coverage_post_processing: bool,
    lcov_merger_files_to_run: NestedSet<Artifact>,
    // TODO(b/192694287): Remove once we migrate all tests from the allowlist.
    network_allowlist: Arc<PackageSpecificationProvider>,
    /// If not null, the reason why this test can't be run in the current build.
    unrunnable_reason: Option<String>,
    /// PORT: the execution half of this action (`blaze_exec`'s `TestRunnerAction`), built on
    /// first use from the fields above; see `test_runner_action_execution`.
    execution: OnceLock<Arc<blaze_exec::analysis::test::test_runner_action::TestRunnerAction>>,
}

impl TestRunnerAction {
    /// Create new TestRunnerAction instance. Should not be called directly. Use
    /// `TestActionBuilder` instead.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:203 TestRunnerAction.<init>
    pub fn new(args: TestRunnerActionArgs) -> TestRunnerAction {
        // Java: nonNullAsSet(testLog, testXml, cacheStatus, coverageArtifact, coverageDirectory,
        // undeclaredOutputsDir).
        let mut outputs: Vec<Artifact> = vec![
            args.test_log.clone(),
            args.test_xml.clone(),
            args.cache_status.clone(),
        ];
        if let Some(c) = &args.coverage_artifact {
            outputs.push(c.clone());
        }
        if let Some(d) = &args.coverage_directory {
            outputs.push(d.clone());
        }
        outputs.push(args.undeclared_outputs_dir.clone());
        let abstract_action = AbstractAction::new(args.owner, args.inputs, outputs);
        check_state!(args.collect_coverage_script.is_none() == args.coverage_artifact.is_none());
        let _ = args
            .configuration
            .get_fragment::<TestConfiguration>()
            .expect("NullPointerException: TestConfiguration");

        let base_dir = args
            .cache_status
            .get_exec_path()
            .get_parent_directory()
            .expect("cache status has a parent");

        let total_shards = args.execution_settings.get_total_shards();
        check_state!(
            (total_shards == 0 && args.shard_num == 0)
                || (total_shards > 0 && 0 <= args.shard_num && args.shard_num < total_shards)
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
        // See note in getSplitLogsPath on the choice of file name.
        let split_logs_path = split_logs_dir.get_child("test.splitlogs");
        let undeclared_outputs_annotations_dir = base_dir.get_child("test.outputs_manifest");
        let undeclared_outputs_manifest_path =
            undeclared_outputs_annotations_dir.get_child("MANIFEST");
        let undeclared_outputs_annotations_path =
            undeclared_outputs_annotations_dir.get_child("ANNOTATIONS");
        let undeclared_outputs_annotations_pb_path =
            undeclared_outputs_annotations_dir.get_child("ANNOTATIONS.pb");
        let test_infrastructure_failure = base_dir.get_child("test.infrastructure_failure");

        // Java: LazySetConcatenation.from(config action env inherited, test action env
        // inherited, extra test env inherited).
        let mut required_client_env_variables: Vec<String> = Vec::new();
        required_client_env_variables.extend(
            args.configuration
                .get_action_environment()
                .get_inherited_env()
                .iter()
                .cloned(),
        );
        required_client_env_variables.extend(
            args.configuration
                .get_test_action_environment()
                .get_inherited_env()
                .iter()
                .cloned(),
        );
        required_client_env_variables.extend(args.extra_test_env.get_inherited_env());

        // Mark all possible test outputs for deletion before test execution.
        let mut files_to_delete: IndexSet<PathFragment> = IndexSet::new();
        files_to_delete.insert(test_warnings_path.clone());
        files_to_delete.insert(unused_runfiles_log_path.clone());
        files_to_delete.insert(test_stderr.clone());
        files_to_delete.insert(test_exit_safe.clone());
        files_to_delete.insert(test_infrastructure_failure.clone());
        // We cannot use coverageData artifact since it may be null. Generate coverage name
        // instead.
        files_to_delete.insert(base_dir.get_child("coverage.dat"));
        // Delete files fetched from remote execution.
        files_to_delete.insert(base_dir.get_child("test.zip"));
        if let Some(s) = &test_shard {
            files_to_delete.insert(s.clone());
        }
        let mut directories_to_delete: IndexSet<PathFragment> = IndexSet::new();
        // Note that splitLogsPath points to a file inside the splitLogsDir so it's not necessary
        // to delete it explicitly.
        directories_to_delete.insert(split_logs_dir.clone());
        directories_to_delete.insert(args.undeclared_outputs_dir.get_exec_path().clone());
        directories_to_delete.insert(undeclared_outputs_annotations_dir.clone());
        directories_to_delete.insert(base_dir.get_relative(&PathFragment::create("test_attempts")));

        TestRunnerAction {
            abstract_action,
            runfiles_tree: args.runfiles_tree,
            test_setup_script: args.test_setup_script,
            test_xml_generator_script: args.test_xml_generator_script,
            collect_coverage_script: args.collect_coverage_script,
            configuration: args.configuration,
            test_log: args.test_log,
            test_xml: args.test_xml,
            cache_status: args.cache_status,
            test_warnings_path,
            unused_runfiles_log_path,
            sh_executable: args.sh_executable,
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
            coverage_data: args.coverage_artifact,
            coverage_directory: args.coverage_directory,
            undeclared_outputs_dir: args.undeclared_outputs_dir,
            test_properties: args.test_properties,
            execution_settings: args.execution_settings,
            shard_num: args.shard_num,
            run_number: args.run_number,
            workspace_name: args.workspace_name,
            coverage_env: args.coverage_env,
            extra_test_env: args.extra_test_env,
            required_client_env_variables,
            cancel_concurrent_tests: args.cancel_concurrent_tests,
            split_coverage_post_processing: args.split_coverage_post_processing,
            lcov_merger_files_to_run: args.lcov_merger_files_to_run,
            network_allowlist: args.network_allowlist,
            unrunnable_reason: args.unrunnable_reason,
            execution: OnceLock::new(),
        }
    }

    /// The cell holding the execution half of this action (see `test_runner_action_execution`).
    pub(crate) fn execution_cell(
        &self,
    ) -> &OnceLock<Arc<blaze_exec::analysis::test::test_runner_action::TestRunnerAction>> {
        &self.execution
    }

    /// Environment variables specific to running code coverage (`coverageEnv`).
    pub fn get_coverage_env(&self) -> &IndexMap<String, String> {
        &self.coverage_env
    }

    fn test_configuration(&self) -> &TestConfiguration {
        self.configuration
            .get_fragment::<TestConfiguration>()
            .expect("TestConfiguration")
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:332 TestRunnerAction.allowLocalTests
    pub fn allow_local_tests(&self) -> bool {
        self.test_configuration().allow_local_tests()
    }

    /// Returns the reason why this test can't be run in the current build, or `None` if it can
    /// be run.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:342 TestRunnerAction.getUnrunnableReason
    pub fn get_unrunnable_reason(&self) -> Option<&str> {
        self.unrunnable_reason.as_deref()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:356 TestRunnerAction.getRunfilesTree
    pub fn get_runfiles_tree(&self) -> &Artifact {
        &self.runfiles_tree
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:365 TestRunnerAction.getConfiguration
    pub fn get_configuration(&self) -> &Arc<BuildConfigurationValue> {
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
        self.test_configuration().check_sharding_support()
    }

    /// The outputs of the test spawn (Java `getSpawnOutputs()`).
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:394 TestRunnerAction.getSpawnOutputs
    pub fn get_spawn_outputs(&self) -> Vec<TestOutput> {
        let mut outputs = vec![
            TestOutput::Artifact(self.test_xml.clone()),
            TestOutput::Path(self.test_exit_safe.clone()),
        ];
        if let Some(s) = &self.test_shard {
            outputs.push(TestOutput::Path(s.clone()));
        }
        outputs.push(TestOutput::Path(self.test_warnings_path.clone()));
        outputs.push(TestOutput::Path(self.split_logs_path.clone()));
        outputs.push(TestOutput::Path(self.unused_runfiles_log_path.clone()));
        outputs.push(TestOutput::Path(self.test_infrastructure_failure.clone()));
        if self.test_configuration().get_zip_undeclared_test_outputs() {
            outputs.push(TestOutput::Path(self.get_undeclared_outputs_zip_path()));
        } else {
            outputs.push(TestOutput::Artifact(self.undeclared_outputs_dir.clone()));
        }
        outputs.push(TestOutput::Path(
            self.undeclared_outputs_manifest_path.clone(),
        ));
        outputs.push(TestOutput::Path(
            self.undeclared_outputs_annotations_path.clone(),
        ));
        outputs.push(TestOutput::Path(
            self.undeclared_outputs_annotations_pb_path.clone(),
        ));
        if self.is_coverage_mode() {
            if !self.split_coverage_post_processing {
                outputs.push(TestOutput::Artifact(self.coverage_data.clone().unwrap()));
            }
            if let Some(d) = &self.coverage_directory {
                outputs.push(TestOutput::Artifact(d.clone()));
            }
        }
        outputs
    }

    /// Gets the test name in a user-friendly format. Will generally include the target name and
    /// run/shard numbers, if applicable.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:833 TestRunnerAction.getTestName
    pub fn get_test_name(&self) -> String {
        let suffix = self.get_test_suffix();
        let label = Label::print(self.get_owner().get_label());
        if suffix.is_empty() {
            label
        } else {
            format!("{label} {suffix}")
        }
    }

    /// Gets the test suffix in a user-friendly format, eg "(shard 1 of 7)".
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:843 TestRunnerAction.getTestSuffix
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
    pub fn get_timeout(&self) -> std::time::Duration {
        *self
            .test_configuration()
            .get_test_timeout()
            .expect("test timeouts")
            .get(&self.test_properties.get_timeout())
            .expect("timeout for every TestTimeout")
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:864 TestRunnerAction.getTestLog
    pub fn get_test_log(&self) -> &Artifact {
        &self.test_log
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:868 TestRunnerAction.getTestXml
    pub fn get_test_xml(&self) -> &Artifact {
        &self.test_xml
    }

    /// Returns all environment variables which must be set in order to run this test.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:873 TestRunnerAction.getExtraTestEnv
    pub fn get_extra_test_env(&self) -> &ActionEnvironment {
        &self.extra_test_env
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:886 TestRunnerAction.getCacheStatusArtifact
    pub fn get_cache_status_artifact(&self) -> &Artifact {
        &self.cache_status
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:890 TestRunnerAction.getTestStderrPath
    pub fn get_test_stderr_path(&self) -> &PathFragment {
        &self.test_stderr
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:894 TestRunnerAction.getTestWarningsPath
    pub fn get_test_warnings_path(&self) -> &PathFragment {
        &self.test_warnings_path
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:898 TestRunnerAction.getUnusedRunfilesLogPath
    pub fn get_unused_runfiles_log_path(&self) -> &PathFragment {
        &self.unused_runfiles_log_path
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:902 TestRunnerAction.getSplitLogsPath
    pub fn get_split_logs_path(&self) -> &PathFragment {
        &self.split_logs_path
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:906 TestRunnerAction.getUndeclaredOutputsDir
    pub fn get_undeclared_outputs_dir(&self) -> &Artifact {
        &self.undeclared_outputs_dir
    }

    /// Returns path to the optional zip file of undeclared test outputs.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:911 TestRunnerAction.getUndeclaredOutputsZipPath
    pub fn get_undeclared_outputs_zip_path(&self) -> PathFragment {
        self.undeclared_outputs_dir
            .get_exec_path()
            .get_child(UNDECLARED_OUTPUTS_ZIP_NAME)
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:916 TestRunnerAction.getUndeclaredOutputsManifestPath
    pub fn get_undeclared_outputs_manifest_path(&self) -> &PathFragment {
        &self.undeclared_outputs_manifest_path
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:920 TestRunnerAction.getUndeclaredOutputsAnnotationsDir
    pub fn get_undeclared_outputs_annotations_dir(&self) -> &PathFragment {
        &self.undeclared_outputs_annotations_dir
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:925 TestRunnerAction.getUndeclaredOutputsAnnotationsPath
    pub fn get_undeclared_outputs_annotations_path(&self) -> &PathFragment {
        &self.undeclared_outputs_annotations_path
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:930 TestRunnerAction.getUndeclaredOutputsAnnotationsPbPath
    pub fn get_undeclared_outputs_annotations_pb_path(&self) -> &PathFragment {
        &self.undeclared_outputs_annotations_pb_path
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:934 TestRunnerAction.getTestShard
    pub fn get_test_shard(&self) -> Option<&PathFragment> {
        self.test_shard.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:938 TestRunnerAction.getExitSafeFile
    pub fn get_exit_safe_file(&self) -> &PathFragment {
        &self.test_exit_safe
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:942 TestRunnerAction.getInfrastructureFailureFile
    pub fn get_infrastructure_failure_file(&self) -> &PathFragment {
        &self.test_infrastructure_failure
    }

    /// Returns coverage data artifact or `None` if code coverage was not requested.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:948 TestRunnerAction.getCoverageData
    pub fn get_coverage_data(&self) -> Option<&Artifact> {
        self.coverage_data.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:953 TestRunnerAction.getCoverageManifest
    pub fn get_coverage_manifest(&self) -> Option<&Artifact> {
        self.execution_settings.get_instrumented_file_manifest()
    }

    /// Returns true if coverage data should be gathered.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:958 TestRunnerAction.isCoverageMode
    pub fn is_coverage_mode(&self) -> bool {
        self.coverage_data.is_some()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:986 TestRunnerAction.getTestProperties
    pub fn get_test_properties(&self) -> &TestTargetProperties {
        &self.test_properties
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:995 TestRunnerAction.getExecutionSettings
    pub fn get_execution_settings(&self) -> &TestTargetExecutionSettings {
        &self.execution_settings
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:999 TestRunnerAction.isSharded
    pub fn is_sharded(&self) -> bool {
        self.test_shard.is_some()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:1007 TestRunnerAction.getShardNum
    pub fn get_shard_num(&self) -> i32 {
        self.shard_num
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:1012 TestRunnerAction.getRunNumber
    pub fn get_run_number(&self) -> i32 {
        self.run_number
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:1017 TestRunnerAction.getRunfilesPrefix
    pub fn get_runfiles_prefix(&self) -> &str {
        &self.workspace_name
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:1021 TestRunnerAction.getNetworkAllowlist
    pub fn get_network_allowlist(&self) -> &Arc<PackageSpecificationProvider> {
        &self.network_allowlist
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:1115 TestRunnerAction.getTestSetupScript
    pub fn get_test_setup_script(&self) -> &Artifact {
        &self.test_setup_script
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:1119 TestRunnerAction.getTestXmlGeneratorScript
    pub fn get_test_xml_generator_script(&self) -> &Artifact {
        &self.test_xml_generator_script
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:1124 TestRunnerAction.getCollectCoverageScript
    pub fn get_collect_coverage_script(&self) -> Option<&Arc<FilesToRunProvider>> {
        self.collect_coverage_script.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:1129 TestRunnerAction.getShExecutableMaybe
    pub fn get_sh_executable_maybe(&self) -> Option<&PathFragment> {
        self.sh_executable.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:730 TestRunnerAction.getAdditionalPathOutputsToDelete
    pub fn get_additional_path_outputs_to_delete(&self) -> &IndexSet<PathFragment> {
        &self.files_to_delete_before_execution
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:735 TestRunnerAction.getDirectoryOutputsToDelete
    pub fn get_directory_outputs_to_delete(&self) -> &IndexSet<PathFragment> {
        &self.directories_to_delete_before_execution
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:497 TestRunnerAction.cancelConcurrentTests
    pub fn get_cancel_concurrent_tests(&self) -> TestConfigurationTestOptionsCancelConcurrentTests {
        self.cancel_concurrent_tests
    }
}

/// `PathFragment.getCallablePathStringForOs(os)`.
// Java: com/google/devtools/build/lib/vfs/PathFragment.java:806 PathFragment.getCallablePathStringForOs
fn callable_path_string_for_os(path: &PathFragment, os: OS) -> String {
    if os == OS::Windows {
        // PORT: Windows only.
        blaze_util::unported!("PathFragment.getCallablePathStringForOs(WINDOWS)");
    }
    path.get_callable_path_string()
}

/// For an given environment, returns a subset containing all variables in the given list if
/// they are defined in the given environment.
// Java: com/google/devtools/build/lib/analysis/test/TestStrategy.java:211 TestStrategy.expandedArgsFromAction
pub fn expanded_args_from_action(
    test_action: &TestRunnerAction,
) -> Result<Vec<String>, CommandLineError> {
    let mut args: Vec<String> = Vec::new();
    let execution_os = test_action.get_execution_settings().get_execution_os();

    let test_setup = test_action.get_test_setup_script();
    args.push(callable_path_string_for_os(
        test_setup.get_exec_path(),
        execution_os,
    ));

    if test_action.is_coverage_mode() {
        args.push(callable_path_string_for_os(
            test_action
                .get_collect_coverage_script()
                .expect("coverage script")
                .get_executable()
                .expect("coverage script executable")
                .get_exec_path(),
            execution_os,
        ));
    }

    let exec_settings = test_action.get_execution_settings();

    // Insert the command prefix specified by the "--run_under=<command-prefix>" option, if any.
    if exec_settings.get_run_under().is_some() {
        add_run_under_args(test_action, &mut args);
    }

    // Execute the test using the alias in the runfiles tree, as mandated by the Test
    // Encyclopedia. Do not use getCallablePathStringForOs as tw.exe expects a path with forward
    // slashes.
    args.push(
        exec_settings
            .get_executable()
            .get_runfiles_path()
            .get_callable_path_string(),
    );
    args.extend(exec_settings.get_args_arguments()?);
    Ok(args)
}

// Java: com/google/devtools/build/lib/analysis/test/TestStrategy.java:243 TestStrategy.addRunUnderArgs
fn add_run_under_args(test_action: &TestRunnerAction, args: &mut Vec<String>) {
    let exec_settings = test_action.get_execution_settings();
    let execution_os = exec_settings.get_execution_os();
    let run_under = exec_settings.get_run_under().expect("run under");
    match run_under {
        RunUnder::LabelRunUnder { .. } => {
            args.push(callable_path_string_for_os(
                &exec_settings
                    .get_run_under_executable_artifact()
                    .expect("run under executable")
                    .get_runfiles_path(),
                execution_os,
            ));
        }
        RunUnder::CommandRunUnder { command, .. } => {
            if exec_settings.needs_shell() {
                // TestActionBuilder constructs TestRunnerAction with a 'null' shell only when
                // none is required. Something clearly went wrong.
                let sh = test_action
                    .get_sh_executable_maybe()
                    .unwrap_or_else(|| panic!("NullPointerException: {test_action:?}"));
                let shell_executable = callable_path_string_for_os(sh, execution_os);
                args.push(shell_executable.clone());
                args.push("-c".to_owned());
                args.push("\"$@\"".to_owned());
                args.push(shell_executable); // Sets $0.
            }
            args.push(command.clone());
        }
    }
    args.extend(run_under.options().iter().cloned());
}

impl fmt::Debug for TestRunnerAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_string_java())
    }
}

impl Action for TestRunnerAction {
    fn java_class_name(&self) -> Option<&'static str> {
        Some("com.google.devtools.build.lib.analysis.test.TestRunnerAction")
    }

    fn abstract_action(&self) -> &AbstractAction {
        &self.abstract_action
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:1106 TestRunnerAction.getMnemonic
    fn get_mnemonic(&self) -> String {
        MNEMONIC.to_owned()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:526 TestRunnerAction.computeKey
    fn compute_key(
        &self,
        _action_key_context: &ActionKeyContext,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        // TODO(b/150305897): use addUUID?
        fp.add_string(GUID);
        fp.add_iterable_strings(self.execution_settings.get_args_arguments()?);
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
        to_actions_env(self.configuration.get_action_environment()).add_to(fp);
        to_actions_env(self.configuration.get_test_action_environment()).add_to(fp);
        // The 'requiredClientEnvVariables' are handled by Skyframe and don't need to be added
        // here.
        fp.add_string(&self.test_properties.get_size().to_string());
        fp.add_string(&self.test_properties.get_timeout().to_string());
        fp.add_string(&blaze_options::value::java_duration_to_string(
            self.get_timeout(),
        ));
        fp.add_strings(self.test_properties.get_tags());
        fp.add_boolean(self.test_properties.is_remotable());
        fp.add_int(self.shard_num);
        fp.add_int(self.execution_settings.get_total_shards());
        fp.add_int(self.run_number);
        fp.add_int(self.execution_settings.get_total_runs());
        fp.add_boolean(config_util::is_code_coverage_enabled(&self.configuration));
        fp.add_boolean(self.test_configuration().get_zip_undeclared_test_outputs());
        let execution_info = self.get_execution_info();
        fp.add_string_map(execution_info.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        fp.add_nullable_string(self.unrunnable_reason.as_deref());
        Ok(())
    }

    // Test actions are always distinguished by their target name, which must be unique.
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:521 TestRunnerAction.isShareable
    fn is_shareable(&self) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:565 TestRunnerAction.executeUnconditionally
    fn execute_unconditionally(&self) -> bool {
        // Note: isVolatile must return true if executeUnconditionally can ever return true
        // for this instance.
        // PORT: reads the previous test status from disk (`maybeReadCacheStatus`), which the
        // execution half of this action holds.
        Action::execute_unconditionally(
            &**crate::test::test_runner_action_execution::execution_action(self),
        )
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:603 TestRunnerAction.isVolatile
    fn is_volatile(&self) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:361 TestRunnerAction.getEnvironment
    fn get_environment(&self) -> Arc<ActionEnvironment> {
        Arc::new(to_actions_env(self.configuration.get_action_environment()))
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:878 TestRunnerAction.getClientEnvironmentVariables
    fn get_client_environment_variables(&self) -> Vec<String> {
        self.required_client_env_variables.clone()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:386 TestRunnerAction.showsOutputUnconditionally
    fn shows_output_unconditionally(&self) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:725 TestRunnerAction.getRawProgressMessage
    fn get_raw_progress_message(&self) -> Option<String> {
        Some(format!("Testing {}", self.get_test_name()))
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:991 TestRunnerAction.getExecutionInfo
    fn get_execution_info(&self) -> IndexMap<String, String> {
        merge_maps(
            &self.get_exec_properties(),
            self.test_properties.get_execution_info(),
        )
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:1111 TestRunnerAction.getMandatoryOutputs
    fn get_mandatory_outputs(&self) -> Vec<Artifact> {
        self.get_outputs()
    }

    fn as_command_action(&self) -> Option<&dyn CommandAction> {
        Some(self)
    }
}

impl CommandAction for TestRunnerAction {
    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:1134 TestRunnerAction.getArguments
    fn get_arguments(&self) -> Result<Vec<String>, CommandLineError> {
        expanded_args_from_action(self)
    }
}

#[cfg(test)]
mod tests {
    // Ports of src/test/java/com/google/devtools/build/lib/analysis/test/TestRunnerActionTest.java
    // need a configured test target.
    #[test]
    #[ignore = "needs analysis of a cc_test (TestRunnerActionTest)"]
    fn test_runner_action_key_matches_oracle() {}

    #[test]
    #[ignore = "needs analysis of a sharded test (TestActionBuilderTest.testShardedTestOutputs)"]
    fn test_sharded_test_outputs() {}
}
