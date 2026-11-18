//! Port of `analysis/test/TestActionBuilder.java`.

use std::collections::BTreeMap;
use std::sync::Arc;

use blaze_actions::analysis::runfiles_support::RunfilesSupport;
use blaze_actions::{ActionOwner, Artifact};
use blaze_build_config::fragments::test_configuration::TestConfiguration;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_optdefs::fragment::test_configuration::TestConfigurationTestOptionsCancelConcurrentTests;
use blaze_optdefs::fragment::test_sharding_strategy::TestShardingStrategy;
use blaze_packages::attribute::AttributeMap;
use blaze_packages::starlark_defs::builtin_providers::INSTRUMENTED_FILES_INFO;
use blaze_packages::types::Type;
use blaze_shell::util::os::OS;
use blaze_starlark::label::LabelValue;
use blaze_vfs::PathFragment;
use indexmap::IndexMap;
use starlark::values::OwnedFrozenValue;

use crate::info::downcast_info;
use crate::platform::platform_info::PlatformInfo;
use crate::providers::file_provider::FilesToRunProvider;
use crate::providers::package_specification_provider::PackageSpecificationProvider;
use crate::rule::config_util;
use crate::rule::prerequisite_artifacts::PrerequisiteArtifacts;
use crate::rule::rule_configured_target_builder::allowlist;
use crate::rule::rule_context::{DEFAULT_TEST_RUNNER_EXEC_GROUP_NAME, RuleContext};
use crate::rule::rule_error_consumer::RuleErrorConsumer;
use crate::test::execution_info::ExecutionInfo;
use crate::test::instrumented_files_info::InstrumentedFilesInfo;
use crate::test::test_provider::{CoverageParams, TestOutput, TestParams};
use crate::test::test_runner_action::{TestRunnerAction, TestRunnerActionArgs};
use crate::test::test_target_properties::{TestTargetExecutionSettings, TestTargetProperties};
use crate::toolchains::toolchain_context::ToolchainContext;

/// Whether the test.xml is a declared output of this action rather than just an output of the
/// test spawn. True for Bazel so that it behaves properly with Build without the Bytes.
// Java: com/google/devtools/build/lib/analysis/test/TestActionBuilder.java:65 TestActionBuilder.TEST_XML_IS_ACTION_OUTPUT
const TEST_XML_IS_ACTION_OUTPUT: bool = true;

// Java: com/google/devtools/build/lib/analysis/test/TestActionBuilder.java:67 TestActionBuilder.CC_CODE_COVERAGE_SCRIPT
// PORT: used by the coverage branch of createTestAction (not ported yet).
#[allow(dead_code)]
const CC_CODE_COVERAGE_SCRIPT: &str = "CC_CODE_COVERAGE_SCRIPT";
// Java: com/google/devtools/build/lib/analysis/test/TestActionBuilder.java:68 TestActionBuilder.LCOV_MERGER
// PORT: used by the coverage branch of createTestAction (not ported yet).
#[allow(dead_code)]
const LCOV_MERGER: &str = "LCOV_MERGER";
// The coverage tool Bazel uses to generate a code coverage report for C++.
// Java: com/google/devtools/build/lib/analysis/test/TestActionBuilder.java:70 TestActionBuilder.BAZEL_CC_COVERAGE_TOOL
// PORT: used by the coverage branch of createTestAction (not ported yet).
#[allow(dead_code)]
const BAZEL_CC_COVERAGE_TOOL: &str = "BAZEL_CC_COVERAGE_TOOL";
// Java: com/google/devtools/build/lib/analysis/test/TestActionBuilder.java:71 TestActionBuilder.GCOV_TOOL
// PORT: used by the coverage branch of createTestAction (not ported yet).
#[allow(dead_code)]
const GCOV_TOOL: &str = "GCOV";
// Java: com/google/devtools/build/lib/analysis/test/TestActionBuilder.java:75 TestActionBuilder.COVERAGE_REPORTED_TO_ACTUAL_SOURCES_FILE
// PORT: used by the coverage branch of createTestAction (not ported yet).
#[allow(dead_code)]
const COVERAGE_REPORTED_TO_ACTUAL_SOURCES_FILE: &str = "COVERAGE_REPORTED_TO_ACTUAL_SOURCES_FILE";

/// Helper class to create test actions.
// Java: com/google/devtools/build/lib/analysis/test/TestActionBuilder.java:58 TestActionBuilder
pub struct TestActionBuilder<'a> {
    rule_context: &'a RuleContext,
    additional_tools: Vec<Artifact>,
    runfiles_support: Option<Arc<RunfilesSupport>>,
    executable: Option<Artifact>,
    execution_requirements: Option<OwnedFrozenValue>,
    instrumented_files: Option<OwnedFrozenValue>,
}

impl<'a> TestActionBuilder<'a> {
    // Java: com/google/devtools/build/lib/analysis/test/TestActionBuilder.java:86 TestActionBuilder.<init>
    pub fn new(rule_context: &'a RuleContext) -> TestActionBuilder<'a> {
        TestActionBuilder {
            rule_context,
            additional_tools: Vec::new(),
            runfiles_support: None,
            executable: None,
            execution_requirements: None,
            instrumented_files: None,
        }
    }

    /// Creates the test actions and artifacts using the previously set parameters.
    // Java: com/google/devtools/build/lib/analysis/test/TestActionBuilder.java:114 TestActionBuilder.build
    pub fn build(&self) -> TestParams {
        blaze_util::check_state!(self.runfiles_support.is_some());
        self.create_test_action()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestActionBuilder.java:120 TestActionBuilder.setFilesToRunProvider
    pub fn set_files_to_run_provider(&mut self, provider: &FilesToRunProvider) -> &mut Self {
        self.runfiles_support = Some(
            provider
                .get_runfiles_support()
                .expect("NullPointerException: runfiles support")
                .clone(),
        );
        self.executable = Some(
            provider
                .get_executable()
                .expect("NullPointerException: executable")
                .clone(),
        );
        self
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestActionBuilder.java:129 TestActionBuilder.addTools
    pub fn add_tools(&mut self, tools: &[Artifact]) -> &mut Self {
        self.additional_tools.extend(tools.iter().cloned());
        self
    }

    /// Set the instrumented files (an `InstrumentedFilesInfo` info).
    // Java: com/google/devtools/build/lib/analysis/test/TestActionBuilder.java:135 TestActionBuilder.setInstrumentedFiles
    pub fn set_instrumented_files(
        &mut self,
        instrumented_files: Option<OwnedFrozenValue>,
    ) -> &mut Self {
        self.instrumented_files = instrumented_files;
        self
    }

    /// Set the execution requirements (an `ExecutionInfo` info).
    // Java: com/google/devtools/build/lib/analysis/test/TestActionBuilder.java:141 TestActionBuilder.setExecutionRequirements
    pub fn set_execution_requirements(
        &mut self,
        execution_requirements: Option<OwnedFrozenValue>,
    ) -> &mut Self {
        self.execution_requirements = execution_requirements;
        self
    }

    fn execution_info(&self) -> Option<&ExecutionInfo> {
        self.execution_requirements
            .as_ref()
            .map(|v| downcast_info::<ExecutionInfo>(v).expect("ExecutionInfo"))
    }

    fn instrumented_files_info(&self) -> Option<&InstrumentedFilesInfo> {
        self.instrumented_files
            .as_ref()
            .map(|v| downcast_info::<InstrumentedFilesInfo>(v).expect("InstrumentedFilesInfo"))
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestActionBuilder.java:147 TestActionBuilder.getTestExecGroupName
    fn get_test_exec_group_name(&self) -> String {
        match self.execution_info() {
            Some(e) => e.get_exec_group().to_owned(),
            None => DEFAULT_TEST_RUNNER_EXEC_GROUP_NAME.to_owned(),
        }
    }

    /// Returns the test action owner and its execution platform (Java reads the platform back
    /// with `actionOwner.getExecutionPlatform()`).
    ///
    /// PORT: `ActionOwner` holds the platform as `blaze_actions::PlatformInfoRef`, which cannot be
    /// downcast to the analysis `PlatformInfo`; the platform is returned with the owner, chosen
    /// exactly as the owner is.
    // Java: com/google/devtools/build/lib/analysis/test/TestActionBuilder.java:153 TestActionBuilder.getTestActionOwner
    fn get_test_action_owner(
        &self,
        use_target_platform_for_tests: bool,
    ) -> (ActionOwner, Option<Arc<PlatformInfo>>) {
        let rule_context = self.rule_context;
        if use_target_platform_for_tests && self.execution_requirements.is_none() {
            return (
                rule_context.get_test_action_owner(),
                rule_context.get_test_execution_platform(),
            );
        }
        let exec_group = self.get_test_exec_group_name();
        if let Some(owner) = rule_context.get_action_owner_for(&exec_group) {
            return (owner, rule_context.get_execution_platform_for(&exec_group));
        }
        if use_target_platform_for_tests {
            (
                rule_context.get_test_action_owner(),
                rule_context.get_test_execution_platform(),
            )
        } else {
            (
                rule_context.get_action_owner(),
                rule_context.get_execution_platform(),
            )
        }
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestActionBuilder.java:166 TestActionBuilder.getUnrunnableReason
    fn get_unrunnable_reason(&self) -> Option<String> {
        let toolchain_contexts = self.rule_context.get_toolchain_contexts()?;
        let exec_group = self.get_test_exec_group_name();
        if !toolchain_contexts.has_toolchain_context(&exec_group) {
            return None;
        }
        let toolchain_context = toolchain_contexts
            .get_toolchain_context(&exec_group)
            .expect("toolchain context");
        let test_toolchain_type = toolchain_context
            .requested_toolchain_type_labels()
            .get(&crate::platform::platform_constants::default_test_toolchain_type())?;
        if toolchain_context
            .for_toolchain_type(test_toolchain_type.type_label())
            .is_some()
        {
            return None;
        }
        let main_repo_mapping = if self.rule_context.get_label().get_repository().is_main() {
            Some(
                &self
                    .rule_context
                    .get_rule()
                    .get_package_metadata()
                    .repository_mapping,
            )
        } else {
            None
        };
        Some(format!(
            "No matching toolchain found for type {}, which is required to run tests for target platform {}.{}\nTo debug, rerun with --toolchain_resolution_debug='{}'",
            test_toolchain_type.type_label().get_display_form(main_repo_mapping),
            toolchain_context
                .target_platform()
                .label()
                .get_display_form(main_repo_mapping),
            match test_toolchain_type.none_found_error() {
                Some(e) => format!(" {e}"),
                None => String::new(),
            },
            blaze_build_config::fragments::platform_configuration::PlatformConfiguration::toolchain_resolution_debug_filter(
                test_toolchain_type.type_label()
            )
        ))
    }

    /// Returns the number of shards for the test (explicit `shard_count` adjusted by
    /// `--test_sharding_strategy`).
    // Java: com/google/devtools/build/lib/analysis/test/TestActionBuilder.java:200 TestActionBuilder.getShardCount
    pub fn get_shard_count(rule_context: &RuleContext) -> i32 {
        let explicit_shard_count = rule_context
            .attributes()
            .get("shard_count", Type::Integer)
            .and_then(|v| v.as_int())
            .unwrap_or(0);
        let Some(test_configuration) = rule_context
            .get_configuration()
            .get_fragment::<TestConfiguration>()
        else {
            return explicit_shard_count;
        };
        let strategy: TestShardingStrategy = test_configuration.test_sharding_strategy();
        let result = strategy.get_number_of_shards(explicit_shard_count);
        blaze_util::check_state!(
            result >= 0,
            "{:?} returned negative shard count {}",
            strategy,
            result
        );
        result
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestActionBuilder.java:215 TestActionBuilder.getRunsPerTest
    pub fn get_runs_per_test(rule_context: &RuleContext) -> i32 {
        match rule_context
            .get_configuration()
            .get_fragment::<TestConfiguration>()
        {
            None => 1,
            Some(test_configuration) => {
                test_configuration.get_runs_per_test_for_label(rule_context.get_label())
            }
        }
    }

    /// Creates a test action and artifacts for the given rule. The test action will use the
    /// specified executable and runfiles.
    // Java: com/google/devtools/build/lib/analysis/test/TestActionBuilder.java:232 TestActionBuilder.createTestAction
    fn create_test_action(&self) -> TestParams {
        let rule_context = self.rule_context;
        let target_name = PathFragment::create(rule_context.get_label().get_name());
        let config = rule_context.get_configuration();
        let test_configuration = config
            .get_fragment::<TestConfiguration>()
            .expect("TestConfiguration");
        let env = rule_context.get_analysis_environment();
        let root = rule_context.get_test_logs_directory();
        let (action_owner, execution_platform) = self
            .get_test_action_owner(config_util::core_options(config).use_target_platform_for_tests);
        let unrunnable_reason = self.get_unrunnable_reason();
        let is_executed_on_windows =
            crate::platform::constraint_constants::get_os_from_constraints_or_host(
                execution_platform.as_deref(),
            ) == OS::Windows;
        let runfiles_support = self.runfiles_support.as_ref().unwrap();
        let executable = self.executable.clone().unwrap();

        let mut inputs_builder: NestedSetBuilder<Artifact> = NestedSetBuilder::stable_order();
        inputs_builder.add_transitive(&NestedSetBuilder::create(
            Order::STABLE_ORDER,
            [runfiles_support.get_runfiles_tree_artifact().clone()],
        ));

        if !is_executed_on_windows {
            let test_runtime = PrerequisiteArtifacts::nested_set(
                rule_context.get_rule_prerequisites_collection(),
                "$test_runtime",
            );
            inputs_builder.add_transitive(&test_runtime);
        }
        let test_properties = TestTargetProperties::new(rule_context, self.execution_info());

        // If the test rule does not provide InstrumentedFilesProvider, there's not much that we
        // can do.
        let collect_code_coverage =
            config_util::is_code_coverage_enabled(config) && self.instrumented_files.is_some();

        let test_action_executable = if is_executed_on_windows {
            rule_context.get_prerequisite_artifact("$test_wrapper")
        } else {
            rule_context.get_prerequisite_artifact("$test_setup_script")
        }
        .expect("test setup script");
        inputs_builder.add(test_action_executable.clone());
        let test_xml_generator_executable = if is_executed_on_windows {
            rule_context.get_prerequisite_artifact("$xml_writer")
        } else {
            rule_context.get_prerequisite_artifact("$xml_generator_script")
        }
        .expect("xml generator script");
        inputs_builder.add(test_xml_generator_executable.clone());

        let mut collect_coverage_script: Option<Arc<FilesToRunProvider>> = None;
        let coverage_test_env: BTreeMap<String, String> = BTreeMap::new();

        let runs_per_test = Self::get_runs_per_test(rule_context);
        let shard_count = Self::get_shard_count(rule_context);

        let lcov_merger_files_to_run: NestedSet<Artifact> =
            NestedSetBuilder::empty_set(Order::STABLE_ORDER);

        let execution_settings = if collect_code_coverage {
            let _ = (
                &mut collect_coverage_script,
                &coverage_test_env,
                self.instrumented_files_info(),
            );
            // PORT: coverage collection (collect_coverage.sh, lcov merger,
            // InstrumentedFileManifestAction, LazyWriteNestedSetOfTupleAction) is not ported.
            blaze_util::unported!("TestActionBuilder.createTestAction (--collect_code_coverage)");
        } else {
            TestTargetExecutionSettings::new(
                rule_context,
                runfiles_support,
                executable.clone(),
                None,
                shard_count,
                runs_per_test,
                execution_platform.as_deref(),
            )
        };

        if config_util::get_run_under(config).is_some() {
            if let Some(run_under_executable) =
                execution_settings.get_run_under_executable_artifact()
            {
                inputs_builder.add(run_under_executable.clone());
            }
        }

        let inputs = inputs_builder.build();
        let shard_runs = if shard_count > 0 { shard_count } else { 1 };
        let mut results: Vec<Artifact> =
            Vec::with_capacity((runs_per_test * shard_runs).max(0) as usize);
        let coverage_artifacts: Vec<Artifact> = Vec::new();
        let mut test_outputs: Vec<TestOutput> = Vec::new();

        // Use 1-based indices for user friendliness.
        for shard in 0..shard_runs {
            let shard_dir = if shard_runs > 1 {
                Some(format!("shard_{}_of_{}", shard + 1, shard_count))
            } else {
                None
            };
            for run in 0..runs_per_test {
                let dir = if runs_per_test > 1 {
                    let run_dir = format!("run_{}_of_{}", run + 1, runs_per_test);
                    match &shard_dir {
                        None => target_name.get_relative(&PathFragment::create(&run_dir)),
                        Some(s) => target_name
                            .get_relative(&PathFragment::create(&format!("{s}_{run_dir}"))),
                    }
                } else {
                    match &shard_dir {
                        None => target_name.clone(),
                        Some(s) => target_name.get_relative(&PathFragment::create(s)),
                    }
                };

                let test_log = rule_context.get_package_relative_artifact(
                    &dir.get_relative(&PathFragment::create("test.log")),
                    &root,
                );
                blaze_util::check_state!(TEST_XML_IS_ACTION_OUTPUT);
                let test_xml = rule_context.get_package_relative_artifact(
                    &dir.get_relative(&PathFragment::create("test.xml")),
                    &root,
                );
                let cache_status = rule_context.get_package_relative_artifact(
                    &dir.get_relative(&PathFragment::create("test.cache_status")),
                    &root,
                );

                let coverage_artifact: Option<Artifact> = None;
                let coverage_directory: Option<Artifact> = None;
                // (collect_code_coverage is false here; see above.)

                let undeclared_outputs_dir = rule_context.get_package_relative_tree_artifact(
                    &dir.get_relative(&PathFragment::create("test.outputs")),
                    &root,
                );

                let cancel_concurrent_tests = if test_configuration.runs_per_test_detects_flakes() {
                    test_configuration.cancel_concurrent_tests()
                } else {
                    TestConfigurationTestOptionsCancelConcurrentTests::NEVER
                };

                let split_coverage_post_processing =
                    test_configuration.split_coverage_post_processing();
                let sh_executable = if !is_executed_on_windows || execution_settings.needs_shell() {
                    Some(crate::rule::sh_toolchain::get_path_for_platform(
                        rule_context.get_configuration(),
                        execution_platform.as_ref(),
                    ))
                } else {
                    None
                };
                // Network allowlist only makes sense in workspaces which explicitly add it, use
                // an empty one as a fallback.
                let network_allowlist = allowlist::fetch_package_specification_provider_or_null(
                    rule_context,
                    "external_network",
                )
                .map(Arc::new)
                .unwrap_or_else(PackageSpecificationProvider::empty);
                let test_runner_action = TestRunnerAction::new(TestRunnerActionArgs {
                    owner: action_owner.clone(),
                    inputs: inputs.clone(),
                    runfiles_tree: runfiles_support.get_runfiles_tree_artifact().clone(),
                    test_setup_script: test_action_executable.clone(),
                    test_xml_generator_script: test_xml_generator_executable.clone(),
                    collect_coverage_script: collect_coverage_script.clone(),
                    test_log: test_log.clone(),
                    test_xml,
                    cache_status: cache_status.clone(),
                    coverage_artifact,
                    coverage_directory,
                    undeclared_outputs_dir,
                    test_properties: test_properties.clone(),
                    coverage_env: coverage_test_env
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect::<IndexMap<String, String>>(),
                    extra_test_env: runfiles_support.get_action_environment().clone(),
                    execution_settings: execution_settings.clone(),
                    shard_num: shard,
                    run_number: run,
                    configuration: config.clone(),
                    workspace_name: rule_context.get_workspace_name().to_owned(),
                    sh_executable,
                    cancel_concurrent_tests,
                    split_coverage_post_processing,
                    lcov_merger_files_to_run: lcov_merger_files_to_run.clone(),
                    network_allowlist,
                    unrunnable_reason: unrunnable_reason.clone(),
                });

                test_outputs.extend(test_runner_action.get_spawn_outputs());
                test_outputs.extend(
                    blaze_actions::Action::get_outputs(&test_runner_action)
                        .into_iter()
                        .map(TestOutput::Artifact),
                );

                env.register_action(Arc::new(test_runner_action));

                results.push(cache_status);
            }
        }
        let mut coverage_params: Option<CoverageParams> = None;
        if config_util::is_code_coverage_enabled(config) {
            // TODO(bazel-team): Passing the reportGenerator to every TestParams is a bit strange.
            let report_generator_target = rule_context
                .get_prerequisite(":coverage_report_generator")
                .expect("coverage report generator");
            let report_generator =
                crate::configuredtargets::get_files_to_run_provider(&report_generator_target)
                    .expect("FilesToRunProvider");
            if report_generator.get_executable().is_none() {
                rule_context.rule_error(
                    "--coverage_report_generator does not refer to an executable target",
                );
            }
            coverage_params = Some(CoverageParams {
                coverage_artifacts: coverage_artifacts.clone(),
                coverage_report_generator: report_generator,
                action_owner: action_owner.clone(),
            });
        }

        TestParams::new(
            runs_per_test,
            shard_count,
            test_configuration.runs_per_test_detects_flakes(),
            rule_context
                .get_rule()
                .get_test_timeout()
                .expect("test timeout"),
            rule_context.get_rule().get_rule_class().to_owned(),
            results,
            test_outputs,
            coverage_params,
        )
    }
}

/// Keeps `LabelValue` in scope for the unrunnable-reason message helpers.
#[allow(unused)]
fn _uses(_: &LabelValue) {}

/// Re-export so callers can name the instrumented files key.
pub fn instrumented_files_key() -> blaze_packages::rule_class::ProviderKey {
    INSTRUMENTED_FILES_INFO.key()
}
