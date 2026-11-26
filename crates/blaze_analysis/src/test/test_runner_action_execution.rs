//! The execution half of `analysis/test/TestRunnerAction.java` for the analysis-time
//! [`TestRunnerAction`]: `prepare` (`getAdditionalPathOutputsToDelete` /
//! `getDirectoryOutputsToDelete`), `execute` (`TestActionContext` / `StandaloneTestStrategy`) and
//! `executeUnconditionally` (the cached `TestResultData`).
//!
//! PORT: Java has one `TestRunnerAction` class. Blaze ported it twice: the analysis-time half here
//! (built by `TestActionBuilder`, read by aquery and the action key) and the execution half in
//! `blaze_exec::analysis::test::test_runner_action` (read by `StandaloneTestStrategy`,
//! `TestResult`, `TestAttempt`, the test summary classes), validated by `blaze-test-replay`.
//! Moving the execution behaviour here would duplicate that validated code, so the analysis action
//! delegates instead: on first use it builds the execution `TestRunnerAction` from its own fields
//! (owner, inputs, outputs, test properties, execution settings, run and shard numbers, the test
//! environment and the configuration values `TestRunnerAction` reads), never from defaults, and
//! `ExecutableAction` forwards `prepare` and `execute` to it. Both halves run the same Java
//! constructor on the same arguments, so they compute the same outputs, paths and environment.

use std::sync::Arc;

use blaze_actions::Action;
use blaze_build_config::fragments::test_configuration::TestConfiguration;
use blaze_exec::actions::{
    ActionExecutionException, ActionInputRef, ActionResult, ExecutableAction,
    register_executable_action,
};
use blaze_exec::analysis::test::test_runner_action::{
    TestRunnerAction as ExecTestRunnerAction, TestRunnerActionConfiguration,
};
use blaze_exec::analysis::test::test_target_execution_settings::TestTargetExecutionSettings as ExecTestTargetExecutionSettings;
use blaze_exec::analysis::test::test_target_properties::TestTargetProperties as ExecTestTargetProperties;
use blaze_exec::exec::action_execution_context::ActionExecutionContext;
use blaze_shell::util::os::OS;

use crate::rule::action_environments::to_actions_env;
use crate::rule::config_util;
use crate::test::test_runner_action::TestRunnerAction;

/// Registers the `ExecutableAction` implementations of `lib/analysis/test` (the analysis
/// [`TestRunnerAction`] and the execution half it delegates to) and the
/// `(TestRunnerAction) action` view `TestCompletionFunction` uses. Idempotent.
pub fn register_executable_actions() {
    register_executable_action::<TestRunnerAction>();
    register_executable_action::<
        crate::test::instrumented_file_manifest_action::InstrumentedFileManifestAction,
    >();
    register_executable_action::<crate::test::baseline_coverage_action::BaselineCoverageAction>();
    register_executable_action::<
        crate::bazel::coverage::coverage_report_action_builder::CoverageReportAction,
    >();
    // The extra-action classes (`lib/analysis/extra`, `PseudoAction`).
    crate::extra::execution::register_executable_actions();
    blaze_exec::analysis::test::register();
    blaze_exec::analysis::test::register_test_runner_action_view::<TestRunnerAction>(|a| {
        execution_action(a).clone()
    });
}

/// The execution half of `action`, built on first use.
pub fn execution_action(action: &TestRunnerAction) -> &Arc<ExecTestRunnerAction> {
    action
        .execution_cell()
        .get_or_init(|| create_execution_action(action))
}

/// The configuration values the execution half reads (`BuildConfigurationValue` and its
/// `TestConfiguration` fragment), taken from the action's configuration.
fn execution_configuration(action: &TestRunnerAction) -> TestRunnerActionConfiguration {
    let configuration = action.get_configuration();
    let test_configuration = configuration
        .get_fragment::<TestConfiguration>()
        .expect("NullPointerException: TestConfiguration");
    TestRunnerActionConfiguration {
        // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:621 BuildConfigurationValue.checksum
        checksum: configuration.checksum().to_owned(),
        // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:508 BuildConfigurationValue.getActionEnvironment
        action_environment: Arc::new(to_actions_env(configuration.get_action_environment())),
        // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:567 BuildConfigurationValue.getTestActionEnvironment
        test_action_environment: Arc::new(to_actions_env(
            configuration.get_test_action_environment(),
        )),
        code_coverage_enabled: config_util::is_code_coverage_enabled(configuration),
        runfiles_enabled: configuration.runfiles_enabled(),
        test_timeout: test_configuration
            .get_test_timeout()
            .expect("NullPointerException: TestConfiguration.getTestTimeout()")
            .clone(),
        cache_test_results: test_configuration.cache_test_results(),
        zip_undeclared_test_outputs: test_configuration.get_zip_undeclared_test_outputs(),
        test_runner_fail_fast: test_configuration.get_test_runner_fail_fast(),
        check_sharding_support: test_configuration.check_sharding_support(),
        allow_local_tests: test_configuration.allow_local_tests(),
    }
}

/// Builds the execution `TestRunnerAction` with the constructor arguments `TestActionBuilder`
/// passed to `action`.
// Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:203 TestRunnerAction.<init>
fn create_execution_action(action: &TestRunnerAction) -> Arc<ExecTestRunnerAction> {
    let configuration = action.get_configuration();
    let test_configuration = configuration
        .get_fragment::<TestConfiguration>()
        .expect("NullPointerException: TestConfiguration");

    let properties = action.get_test_properties();
    // Java: TestTargetProperties keeps the rule's TestConfiguration for getLocalResourceUsage
    // (`testConfiguration.getTestResources(size)`); the action's configuration is the rule's.
    let test_properties = ExecTestTargetProperties::new(
        properties.get_size(),
        properties.get_timeout(),
        properties.get_tags().to_vec(),
        properties.is_flaky(),
        properties.is_external(),
        properties.get_language().to_owned(),
        properties.get_execution_info().clone(),
        Some(test_configuration.get_test_resources(properties.get_size())),
    );

    let settings = action.get_execution_settings();
    if settings.get_execution_os() != OS::Linux {
        // The execution half assumes a Linux execution platform
        // (`TestTargetExecutionSettings.getExecutionOs`).
        blaze_util::unported!("TestTargetExecutionSettings.getExecutionOs (non-Linux)");
    }
    let execution_settings = ExecTestTargetExecutionSettings::new(
        settings.get_args(),
        settings.get_test_filter().map(str::to_owned),
        settings.get_total_shards(),
        settings.get_total_runs(),
        settings.get_run_under().cloned(),
        settings.get_run_under_executable_artifact().cloned(),
        settings.get_executable().clone(),
        settings.get_runfiles_symlinks_created(),
        settings.get_runfiles_dir().cloned(),
        settings.get_input_manifest().cloned(),
        settings.get_instrumented_file_manifest().cloned(),
        settings.get_test_runner_fail_fast(),
    );

    // Java keeps the `FilesToRunProvider`; the execution half reads its executable only.
    let collect_coverage_script = action.get_collect_coverage_script().map(|files_to_run| {
        files_to_run
            .get_executable()
            .cloned()
            .expect("NullPointerException: collectCoverageScript.getExecutable()")
    });

    ExecTestRunnerAction::new(
        action.get_owner().clone(),
        action.get_inputs(),
        action.get_runfiles_tree().clone(),
        action.get_test_setup_script().clone(),
        action.get_test_xml_generator_script().clone(),
        collect_coverage_script,
        action.get_test_log().clone(),
        // See TestActionBuilder.TEST_XML_IS_ACTION_OUTPUT: always an artifact in Bazel.
        ActionInputRef::new(action.get_test_xml().clone()),
        action.get_cache_status_artifact().clone(),
        action.get_coverage_data().cloned(),
        action.get_coverage_directory_tree_artifact().cloned(),
        action.get_undeclared_outputs_dir().clone(),
        test_properties,
        action.get_coverage_env().clone(),
        Arc::new(action.get_extra_test_env().clone()),
        execution_settings,
        action.get_shard_num(),
        action.get_run_number(),
        Arc::new(execution_configuration(action)),
        action.get_runfiles_prefix().to_owned(),
        action.get_sh_executable_maybe().cloned(),
        action.get_cancel_concurrent_tests(),
        action.get_split_coverage_post_processing(),
        action.get_lcov_merger_files_to_run().clone(),
        action.get_unrunnable_reason().map(str::to_owned),
    )
}

impl ExecutableAction for TestRunnerAction {
    /// `AbstractAction.prepare` → `deleteOutputs`, with `TestRunnerAction`'s
    /// `getAdditionalPathOutputsToDelete` and `getDirectoryOutputsToDelete` overrides.
    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:547 AbstractAction.prepare
    fn prepare(&self, exec_root: &blaze_vfs::Path) -> std::io::Result<()> {
        ExecutableAction::prepare(&**execution_action(self), exec_root)
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestRunnerAction.java:1027 TestRunnerAction.execute
    fn execute(
        &self,
        action_execution_context: &dyn ActionExecutionContext,
    ) -> Result<ActionResult, ActionExecutionException> {
        ExecutableAction::execute(&**execution_action(self), action_execution_context)
    }
}

// Java: com/google/devtools/build/lib/analysis/test/InstrumentedFileManifestAction.java:55 InstrumentedFileManifestAction.newDeterministicWriter
impl ExecutableAction
    for crate::test::instrumented_file_manifest_action::InstrumentedFileManifestAction
{
    // Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:60 AbstractFileWriteAction.execute
    fn execute(
        &self,
        ctx: &dyn ActionExecutionContext,
    ) -> Result<ActionResult, ActionExecutionException> {
        blaze_exec::analysis::actions::abstract_file_write_action::execute_file_write(
            self,
            ctx,
            || file_write_writer(self),
        )
    }
}

// Java: com/google/devtools/build/lib/analysis/test/BaselineCoverageAction.java:69 BaselineCoverageAction.newDeterministicWriter
impl ExecutableAction for crate::test::baseline_coverage_action::BaselineCoverageAction {
    // Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:60 AbstractFileWriteAction.execute
    fn execute(
        &self,
        ctx: &dyn ActionExecutionContext,
    ) -> Result<ActionResult, ActionExecutionException> {
        blaze_exec::analysis::actions::abstract_file_write_action::execute_file_write(
            self,
            ctx,
            || file_write_writer(self),
        )
    }
}

/// `newDeterministicWriter(ctx)` of the coverage file writes (their writers do not read the
/// context and throw nothing).
fn file_write_writer<
    A: blaze_actions::analysis::actions::abstract_file_write_action::AbstractFileWriteAction,
>(
    action: &A,
) -> Result<
    Box<dyn blaze_actions::analysis::actions::abstract_file_write_action::DeterministicWriter + '_>,
    blaze_exec::actions::ExecException,
> {
    use blaze_actions::analysis::actions::abstract_file_write_action::NewDeterministicWriterError;
    action.new_deterministic_writer().map_err(|e| match e {
        NewDeterministicWriterError::Interrupted => blaze_exec::actions::ExecException::Interrupted,
        NewDeterministicWriterError::Exec(m) => panic!("unexpected ExecException: {m}"),
    })
}

#[cfg(test)]
mod tests {
    // TestRunnerActionTest (src/test/java/com/google/devtools/build/lib/analysis/test) needs a
    // configured cc_test; the execution half is validated end to end by blaze-test-replay.
    #[test]
    #[ignore = "needs analysis of a cc_test (TestRunnerActionTest.testExecuteUnconditionally*)"]
    fn execution_action_matches_analysis_action() {}
}
