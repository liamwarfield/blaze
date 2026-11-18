//! Ports of `analysis/test/TestTargetProperties.java` and
//! `analysis/test/TestTargetExecutionSettings.java`.

use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_actions::actions::command_line::CommandLineRef;
use blaze_actions::actions::execution_requirements::{
    LOCAL, NO_REMOTE, NO_REMOTE_EXEC, NO_TESTLOASD,
};
use blaze_actions::analysis::runfiles::Runfiles;
use blaze_actions::analysis::runfiles_support::RunfilesSupport;
use blaze_build_config::fragments::test_configuration::TestConfiguration;
use blaze_optdefs::fragment::run_under::RunUnder;
use blaze_optdefs::fragment::test_size::TestSize;
use blaze_optdefs::fragment::test_timeout::TestTimeout;
use blaze_packages::attribute::{AttributeMap, NonconfigurableAttributeMapper};
use blaze_packages::target::target_utils;
use blaze_packages::types::Type;
use blaze_shell::util::os::OS;
use blaze_util::check_argument;
use blaze_vfs::Path;
use indexmap::IndexMap;

use crate::platform::platform_info::PlatformInfo;
use crate::providers::file_provider::FilesToRunProvider;
use crate::rule::config_util;
use crate::rule::rule_context::RuleContext;
use crate::test::execution_info::ExecutionInfo;

/// Container for test target properties available to the TestRunnerAction instance.
// Java: com/google/devtools/build/lib/analysis/test/TestTargetProperties.java:38 TestTargetProperties
#[derive(Clone, Debug)]
pub struct TestTargetProperties {
    size: TestSize,
    timeout: TestTimeout,
    tags: Vec<String>,
    is_remotable: bool,
    is_flaky: bool,
    is_external: bool,
    language: String,
    execution_info: IndexMap<String, String>,
}

impl TestTargetProperties {
    /// Creates test target properties instance. Constructor expects that it will be called only
    /// for test configured targets.
    // Java: com/google/devtools/build/lib/analysis/test/TestTargetProperties.java:70 TestTargetProperties.<init>
    pub fn new(
        rule_context: &RuleContext,
        execution_requirements: Option<&ExecutionInfo>,
    ) -> TestTargetProperties {
        let rule = rule_context.get_rule();
        blaze_util::check_state!(target_utils::is_test_rule(rule_context.get_target()));
        // Java: TestSize.getTestSize(rule) (null for an unrecognized size).
        let size_attr = NonconfigurableAttributeMapper::of(rule.as_ref())
            .get("size", Type::String)
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        let size = TestSize::get_test_size(&size_attr).unwrap_or_else(|| {
            panic!("NullPointerException: unrecognized test size '{size_attr}'")
        });
        let timeout = rule
            .get_test_timeout()
            .expect("NullPointerException: test timeout");
        let tags: Vec<String> = rule_context
            .attributes()
            .get("tags", Type::StringList)
            .and_then(|v| v.as_string_list().map(|s| s.to_vec()))
            .unwrap_or_default();

        // We need to use method on ruleConfiguredTarget to perform validation.
        let is_flaky = rule_context
            .attributes()
            .get("flaky", Type::Boolean)
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let is_external = target_utils::is_external_test_rule(rule);

        let mut execution_info: IndexMap<String, String> = IndexMap::new();
        for (k, v) in target_utils::get_execution_info(rule) {
            execution_info.insert(k, v);
        }

        let mut incompatible_exclusive_test_sandboxed = false;
        if let Some(test_configuration) = rule_context
            .get_configuration()
            .get_fragment::<TestConfiguration>()
        {
            incompatible_exclusive_test_sandboxed =
                test_configuration.incompatible_exclusive_test_sandboxed();
        }

        if incompatible_exclusive_test_sandboxed {
            if target_utils::is_local_test_rule(rule) {
                execution_info.insert(LOCAL.to_owned(), String::new());
            } else if target_utils::is_exclusive_test_rule(rule) {
                execution_info.insert(NO_REMOTE_EXEC.to_owned(), String::new());
            }
        } else if target_utils::is_local_test_rule(rule)
            || target_utils::is_exclusive_test_rule(rule)
        {
            execution_info.insert(LOCAL.to_owned(), String::new());
        }

        if target_utils::is_no_testloasd_test_rule(rule) {
            execution_info.insert(LOCAL.to_owned(), String::new());
            execution_info.insert(NO_TESTLOASD.to_owned(), String::new());
        }

        if let Some(requirements) = execution_requirements {
            // This will overwrite whatever TargetUtils put there, which might be confusing.
            for (k, v) in requirements.get_execution_info() {
                execution_info.insert(k.clone(), v.clone());
            }
        }
        config_util::modify_execution_info(
            rule_context.get_configuration(),
            &mut execution_info,
            crate::test::test_runner_action::MNEMONIC,
        );

        let is_remotable = !execution_info.contains_key(LOCAL)
            && !execution_info.contains_key(NO_REMOTE)
            && !execution_info.contains_key(NO_REMOTE_EXEC);

        let language = target_utils::get_rule_language(rule.get_rule_class()).to_owned();
        TestTargetProperties {
            size,
            timeout,
            tags,
            is_remotable,
            is_flaky,
            is_external,
            language,
            execution_info,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetProperties.java:131 TestTargetProperties.getSize
    pub fn get_size(&self) -> TestSize {
        self.size
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetProperties.java:135 TestTargetProperties.getTimeout
    pub fn get_timeout(&self) -> TestTimeout {
        self.timeout
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetProperties.java:139 TestTargetProperties.getTags
    pub fn get_tags(&self) -> &[String] {
        &self.tags
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetProperties.java:143 TestTargetProperties.isRemotable
    pub fn is_remotable(&self) -> bool {
        self.is_remotable
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetProperties.java:147 TestTargetProperties.isFlaky
    pub fn is_flaky(&self) -> bool {
        self.is_flaky
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetProperties.java:151 TestTargetProperties.isExternal
    pub fn is_external(&self) -> bool {
        self.is_external
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetProperties.java:169 TestTargetProperties.getExecutionInfo
    pub fn get_execution_info(&self) -> &IndexMap<String, String> {
        &self.execution_info
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetProperties.java:173 TestTargetProperties.getLanguage
    pub fn get_language(&self) -> &str {
        &self.language
    }
}

/// Container for common test execution settings shared by all TestRunnerAction instances for
/// the given test target.
// Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:38 TestTargetExecutionSettings
#[derive(Clone, Debug)]
pub struct TestTargetExecutionSettings {
    /// Java `CommandLine.concat(targetArgs, testConfig.getTestArguments())`.
    target_args: CommandLineRef,
    test_arguments: Vec<String>,
    test_filter: Option<String>,
    total_shards: i32,
    total_runs: i32,
    run_under: Option<RunUnder>,
    run_under_executable: Option<Artifact>,
    executable: Artifact,
    runfiles_symlinks_created: bool,
    runfiles_dir: Option<Path>,
    runfiles: Runfiles,
    runfiles_input_manifest: Option<Artifact>,
    instrumented_file_manifest: Option<Artifact>,
    test_runner_fail_fast: bool,
    execution_os: OS,
}

impl TestTargetExecutionSettings {
    // Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:56 TestTargetExecutionSettings.<init>
    pub fn new(
        rule_context: &RuleContext,
        runfiles_support: &Arc<RunfilesSupport>,
        executable: Artifact,
        instrumented_file_manifest: Option<Artifact>,
        shards: i32,
        runs: i32,
        execution_platform: Option<&PlatformInfo>,
    ) -> TestTargetExecutionSettings {
        check_argument!(target_utils::is_test_rule(rule_context.get_target()));
        check_argument!(shards >= 0);
        let config = rule_context.get_configuration();
        let test_config = config
            .get_fragment::<TestConfiguration>()
            .expect("NullPointerException: TestConfiguration");

        TestTargetExecutionSettings {
            target_args: runfiles_support.get_args().clone(),
            test_arguments: test_config.get_test_arguments().to_vec(),
            total_shards: shards,
            total_runs: runs,
            run_under: config_util::get_run_under(config).cloned(),
            run_under_executable: Self::get_run_under_executable(rule_context),
            test_filter: test_config.get_test_filter().map(str::to_owned),
            test_runner_fail_fast: test_config.get_test_runner_fail_fast(),
            executable,
            runfiles_symlinks_created: runfiles_support
                .get_runfiles_tree()
                .is_build_runfile_links(),
            runfiles_dir: Some(runfiles_support.get_runfiles_directory()),
            runfiles: runfiles_support.get_runfiles().clone(),
            runfiles_input_manifest: runfiles_support.get_runfiles_input_manifest().cloned(),
            instrumented_file_manifest,
            execution_os: crate::platform::constraint_constants::get_os_from_constraints_or_host(
                execution_platform,
            ),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:89 TestTargetExecutionSettings.getRunUnderExecutable
    fn get_run_under_executable(rule_context: &RuleContext) -> Option<Artifact> {
        let run_under_target = rule_context.get_run_under_prerequisite()?;
        crate::configuredtargets::get_provider::<FilesToRunProvider>(&run_under_target)
            .expect("FilesToRunProvider")
            .get_executable()
            .cloned()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:97 TestTargetExecutionSettings.getRunUnderExecutable
    pub fn get_run_under_executable_artifact(&self) -> Option<&Artifact> {
        self.run_under_executable.as_ref()
    }

    /// Java `getArgs().arguments()`.
    // Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:101 TestTargetExecutionSettings.getArgs
    pub fn get_args_arguments(
        &self,
    ) -> Result<Vec<String>, blaze_collect::command_line_item::MapFnError> {
        let mut args = self.target_args.arguments()?;
        args.extend(self.test_arguments.iter().cloned());
        Ok(args)
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:105 TestTargetExecutionSettings.getTestFilter
    pub fn get_test_filter(&self) -> Option<&str> {
        self.test_filter.as_deref()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:109 TestTargetExecutionSettings.getTestRunnerFailFast
    pub fn get_test_runner_fail_fast(&self) -> bool {
        self.test_runner_fail_fast
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:113 TestTargetExecutionSettings.getTotalShards
    pub fn get_total_shards(&self) -> i32 {
        self.total_shards
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:117 TestTargetExecutionSettings.getTotalRuns
    pub fn get_total_runs(&self) -> i32 {
        self.total_runs
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:121 TestTargetExecutionSettings.getRunUnder
    pub fn get_run_under(&self) -> Option<&RunUnder> {
        self.run_under.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:125 TestTargetExecutionSettings.getExecutable
    pub fn get_executable(&self) -> &Artifact {
        &self.executable
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:129 TestTargetExecutionSettings.getRunfilesSymlinksCreated
    pub fn get_runfiles_symlinks_created(&self) -> bool {
        self.runfiles_symlinks_created
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:134 TestTargetExecutionSettings.getRunfilesDir
    pub fn get_runfiles_dir(&self) -> Option<&Path> {
        self.runfiles_dir.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:138 TestTargetExecutionSettings.getRunfiles
    pub fn get_runfiles(&self) -> &Runfiles {
        &self.runfiles
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:142 TestTargetExecutionSettings.getInputManifest
    pub fn get_input_manifest(&self) -> Option<&Artifact> {
        self.runfiles_input_manifest.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:146 TestTargetExecutionSettings.getInstrumentedFileManifest
    pub fn get_instrumented_file_manifest(&self) -> Option<&Artifact> {
        self.instrumented_file_manifest.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:150 TestTargetExecutionSettings.getExecutionOs
    pub fn get_execution_os(&self) -> OS {
        self.execution_os
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:154 TestTargetExecutionSettings.needsShell
    pub fn needs_shell(&self) -> bool {
        match &self.run_under {
            Some(RunUnder::CommandRunUnder { command, .. }) => {
                // --run_under commands that do not contain '/' are either shell built-ins or
                // need to be located on the PATH env, so we wrap them in a shell invocation.
                // Note that we shell-tokenize the --run_under parameter and getCommand only
                // returns the first such token.
                !command.contains('/')
                    && (self.execution_os != OS::Windows || !command.contains('\\'))
            }
            _ => false,
        }
    }
}

/// Keeps `AttributeMap` in scope for trait method calls.
#[allow(unused)]
fn _uses(_: &dyn AttributeMap) {}
