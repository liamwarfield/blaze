//! Port of `analysis/test/TestTargetExecutionSettings.java`.
//!
//! PORT: the Java constructor reads the `RuleContext`, `RunfilesSupport` and execution platform
//! during analysis; here the struct is built from those already-computed values. `getRunfiles()`
//! (the `Runfiles` object) is not kept: nothing in test execution reads it. The execution OS is
//! always Linux (`getOsFromConstraintsOrHost`), so `getExecutionOs()` is not stored.

use blaze_actions::Artifact;
use blaze_actions::actions::command_line::CommandLineRef;
use blaze_optdefs::fragment::run_under::RunUnder;
use blaze_vfs::Path;

/// Container for common test execution settings shared by all `TestRunnerAction` instances for
/// the given test target.
// Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:41 TestTargetExecutionSettings
#[derive(Clone, Debug)]
pub struct TestTargetExecutionSettings {
    test_arguments: CommandLineRef,
    test_filter: Option<String>,
    total_shards: i32,
    total_runs: i32,
    run_under: Option<RunUnder>,
    run_under_executable: Option<Artifact>,
    executable: Artifact,
    runfiles_symlinks_created: bool,
    runfiles_dir: Option<Path>,
    runfiles_input_manifest: Option<Artifact>,
    instrumented_file_manifest: Option<Artifact>,
    test_runner_fail_fast: bool,
}

impl TestTargetExecutionSettings {
    /// `test_arguments` is `CommandLine.concat(runfilesSupport.getArgs(),
    /// testConfig.getTestArguments())`; `run_under` is `config.getRunUnder()`;
    /// `run_under_executable` the `FilesToRunProvider` executable of the `--run_under` target;
    /// `runfiles_symlinks_created` is `runfilesSupport.getRunfilesTree().isBuildRunfileLinks()`;
    /// `runfiles_dir` is `runfilesSupport.getRunfilesDirectory()`.
    // Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:58 TestTargetExecutionSettings.<init>
    pub fn new(
        test_arguments: CommandLineRef,
        test_filter: Option<String>,
        shards: i32,
        runs: i32,
        run_under: Option<RunUnder>,
        run_under_executable: Option<Artifact>,
        executable: Artifact,
        runfiles_symlinks_created: bool,
        runfiles_dir: Option<Path>,
        runfiles_input_manifest: Option<Artifact>,
        instrumented_file_manifest: Option<Artifact>,
        test_runner_fail_fast: bool,
    ) -> TestTargetExecutionSettings {
        blaze_util::check_argument!(shards >= 0);
        TestTargetExecutionSettings {
            test_arguments,
            test_filter,
            total_shards: shards,
            total_runs: runs,
            run_under,
            run_under_executable,
            executable,
            runfiles_symlinks_created,
            runfiles_dir,
            runfiles_input_manifest,
            instrumented_file_manifest,
            test_runner_fail_fast,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:78 TestTargetExecutionSettings.getRunUnderExecutable
    pub fn get_run_under_executable(&self) -> Option<&Artifact> {
        self.run_under_executable.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:103 TestTargetExecutionSettings.getArgs
    pub fn get_args(&self) -> &CommandLineRef {
        &self.test_arguments
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:107 TestTargetExecutionSettings.getTestFilter
    pub fn get_test_filter(&self) -> Option<&str> {
        self.test_filter.as_deref()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:111 TestTargetExecutionSettings.getTestRunnerFailFast
    pub fn get_test_runner_fail_fast(&self) -> bool {
        self.test_runner_fail_fast
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:115 TestTargetExecutionSettings.getTotalShards
    pub fn get_total_shards(&self) -> i32 {
        self.total_shards
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:119 TestTargetExecutionSettings.getTotalRuns
    pub fn get_total_runs(&self) -> i32 {
        self.total_runs
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:123 TestTargetExecutionSettings.getRunUnder
    pub fn get_run_under(&self) -> Option<&RunUnder> {
        self.run_under.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:127 TestTargetExecutionSettings.getExecutable
    pub fn get_executable(&self) -> &Artifact {
        &self.executable
    }

    /// Returns whether or not the runfiles symlinks were created.
    // Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:132 TestTargetExecutionSettings.getRunfilesSymlinksCreated
    pub fn get_runfiles_symlinks_created(&self) -> bool {
        self.runfiles_symlinks_created
    }

    /// Returns the directory of the runfiles.
    // Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:138 TestTargetExecutionSettings.getRunfilesDir
    pub fn get_runfiles_dir(&self) -> Option<&Path> {
        self.runfiles_dir.as_ref()
    }

    /// Returns the input runfiles manifest for this test.
    ///
    /// This always returns the input manifest outside of the runfiles tree.
    // Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:154 TestTargetExecutionSettings.getInputManifest
    pub fn get_input_manifest(&self) -> Option<&Artifact> {
        self.runfiles_input_manifest.as_ref()
    }

    /// Returns instrumented file manifest or null if code coverage is not collected.
    // Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:159 TestTargetExecutionSettings.getInstrumentedFileManifest
    pub fn get_instrumented_file_manifest(&self) -> Option<&Artifact> {
        self.instrumented_file_manifest.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetExecutionSettings.java:167 TestTargetExecutionSettings.needsShell
    pub fn needs_shell(&self) -> bool {
        if let Some(RunUnder::CommandRunUnder { command, .. }) = self.get_run_under() {
            // --run_under commands that do not contain '/' are either shell built-ins or need to
            // be located on the PATH env, so we wrap them in a shell invocation. Note that we
            // shell-tokenize the --run_under parameter and getCommand only returns the first such
            // token.
            // PORT: skipped, Windows only (the `\\` check for a Windows execution OS).
            !command.contains('/')
        } else {
            false
        }
    }
}
