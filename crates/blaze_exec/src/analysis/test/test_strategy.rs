//! Port of `analysis/test/TestStrategy.java`: the base of the strategies executing
//! `TestRunnerAction`s.
//!
//! PORT: the abstract class is the [`TestStrategy`] struct, embedded by implementations (such as
//! `StandaloneTestStrategy`), which forward the `TestActionContext` methods it implements.

use std::collections::HashMap;
use std::io;
use std::sync::Arc;

use blaze_actions::ActionOwner;
use blaze_actions::actions::action::{Action, CommandLineError};
use blaze_actions::actions::fingerprint_util::FingerprintPathExt;
use blaze_cmdline::Label;
use blaze_events::{Event, EventKind};
use blaze_optdefs::command::execution_options::{ExecutionOptions, TestSummaryFormat};
use blaze_optdefs::command::test_summary_options::TestSummaryOptions;
use blaze_optdefs::fragment::run_under::RunUnder;
use blaze_protos::blaze::{BlazeTestStatus, TestCase, TestResultData};
use blaze_protos::failure_details::{self, FailureDetail, failure_detail};
use blaze_shell::termination_status::TerminationStatus;
use blaze_util::Fingerprint;
use blaze_vfs::os_path_policy::HOST_POLICY;
use blaze_vfs::{Path, PathFragment};
use indexmap::IndexMap;
use parking_lot::Mutex;

use super::test_action_context::AttemptGroup;
use super::test_result::TestResult;
use super::test_runner_action::TestRunnerAction;
use crate::actions::{ExecException, UserExecException};
use crate::exec::action_execution_context::ActionExecutionContext;
use crate::exec::test_log_helper;

/// `TestStrategy.TEST_TMP_ROOT`.
// Java: com/google/devtools/build/lib/analysis/test/TestStrategy.java:154 TestStrategy.TEST_TMP_ROOT
pub const TEST_TMP_ROOT: &str = "_tmp";

// Java: com/google/devtools/build/lib/analysis/test/TestStrategy.java:72 TestStrategy.AttemptGroupImpl
#[derive(Default)]
struct AttemptGroupImpl {
    inner: Mutex<AttemptGroupState>,
}

#[derive(Default)]
struct AttemptGroupState {
    cancelled: bool,
    running_threads: Vec<std::thread::ThreadId>,
}

impl AttemptGroup for AttemptGroupImpl {
    // Java: com/google/devtools/build/lib/analysis/test/TestStrategy.java:82 TestStrategy.AttemptGroupImpl.register
    fn register(&self) -> Result<(), ExecException> {
        let mut s = self.inner.lock();
        let me = std::thread::current().id();
        // Verify.verify(runningThreads.add(Thread.currentThread()))
        assert!(
            !s.running_threads.contains(&me),
            "VerifyException: runningThreads.add"
        );
        s.running_threads.push(me);

        if s.cancelled {
            return Err(ExecException::Interrupted);
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestStrategy.java:91 TestStrategy.AttemptGroupImpl.unregister
    fn unregister(&self) {
        let mut s = self.inner.lock();
        let me = std::thread::current().id();
        let pos = s.running_threads.iter().position(|t| *t == me);
        // Verify.verify(runningThreads.remove(Thread.currentThread()))
        let pos = pos.expect("VerifyException: runningThreads.remove");
        s.running_threads.remove(pos);
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestStrategy.java:96 TestStrategy.AttemptGroupImpl.cancelled
    fn cancelled(&self) -> bool {
        self.inner.lock().cancelled
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestStrategy.java:101 TestStrategy.AttemptGroupImpl.cancelOthers
    fn cancel_others(&self) {
        let mut s = self.inner.lock();
        if s.cancelled {
            return;
        }

        s.cancelled = true;

        let me = std::thread::current().id();
        if s.running_threads.iter().any(|t| *t != me) {
            // Java interrupts the other threads (thread.interrupt()); Blaze has no thread
            // interruption for running spawns.
            blaze_util::unported!("TestStrategy.AttemptGroupImpl.cancelOthers (Thread.interrupt)");
        }
    }
}

/// `TestStrategy.ShardKey`.
// Java: com/google/devtools/build/lib/analysis/test/TestStrategy.java:481 TestStrategy.ShardKey
// PORT: `ActionOwner.equals` is identity in Java; owners of one target compare equal by label
// and configuration here.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ShardKey {
    owner_label: Option<Label>,
    owner_configuration: String,
    shard: i32,
}

/// A strategy for executing a `TestRunnerAction`.
// Java: com/google/devtools/build/lib/analysis/test/TestStrategy.java:71 TestStrategy
pub struct TestStrategy {
    cancel_groups: Mutex<HashMap<ShardKey, Arc<AttemptGroupImpl>>>,
    /// Used for generating unique temporary directory names. Contains the next numeric index for
    /// every executable base name.
    tmp_index: Mutex<HashMap<String, i32>>,
    pub(crate) execution_options: Arc<ExecutionOptions>,
    pub(crate) test_summary_options: Arc<TestSummaryOptions>,
}

impl TestStrategy {
    // Java: com/google/devtools/build/lib/analysis/test/TestStrategy.java:162 TestStrategy.<init>
    pub fn new(
        execution_options: Arc<ExecutionOptions>,
        test_summary_options: Arc<TestSummaryOptions>,
    ) -> TestStrategy {
        TestStrategy {
            cancel_groups: Mutex::new(HashMap::new()),
            tmp_index: Mutex::new(HashMap::new()),
            execution_options,
            test_summary_options,
        }
    }

    pub fn get_execution_options(&self) -> &Arc<ExecutionOptions> {
        &self.execution_options
    }

    /// Ensures that all directories used to run test are in the correct state and their content
    /// will not result in stale files.
    // Java: com/google/devtools/build/lib/analysis/test/TestStrategy.java:123 TestStrategy.prepareFileSystem
    pub fn prepare_file_system(
        &self,
        test_action: &TestRunnerAction,
        exec_root: &Path,
        tmp_dir: Option<&Path>,
    ) -> io::Result<()> {
        if let Some(tmp_dir) = tmp_dir {
            recreate_directory(tmp_dir)?;
        }

        let resolved_paths = test_action.resolve(exec_root);
        if test_action.is_coverage_mode() {
            recreate_directory(&resolved_paths.get_coverage_directory())?;
        }

        resolved_paths
            .get_base_dir()
            .create_directory_and_parents()?;
        resolved_paths
            .get_undeclared_outputs_dir()
            .create_directory_and_parents()?;
        resolved_paths
            .get_undeclared_outputs_annotations_dir()
            .create_directory_and_parents()?;
        resolved_paths
            .get_split_logs_dir()
            .create_directory_and_parents()?;
        Ok(())
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestStrategy.java:168 TestStrategy.isTestKeepGoing
    pub fn is_test_keep_going(&self) -> bool {
        self.execution_options.test_keep_going
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestStrategy.java:173 TestStrategy.getAttemptGroup
    pub fn get_attempt_group(&self, owner: &ActionOwner, shard_num: i32) -> Arc<dyn AttemptGroup> {
        let key = ShardKey {
            owner_label: owner.get_label().cloned(),
            owner_configuration: owner.get_configuration_checksum().to_string(),
            shard: shard_num,
        };
        self.cancel_groups
            .lock()
            .entry(key)
            .or_insert_with(|| Arc::new(AttemptGroupImpl::default()))
            .clone()
    }

    /// Generates a command line to run for the test action, taking into account coverage and
    /// `--run_under` settings.
    ///
    /// Basically [`Self::expanded_args_from_action`], but throws `ExecException` instead. This
    /// should be used in action execution.
    // Java: com/google/devtools/build/lib/analysis/test/TestStrategy.java:189 TestStrategy.getArgs
    pub fn get_args(test_action: &TestRunnerAction) -> Result<Vec<String>, ExecException> {
        match Self::expanded_args_from_action(test_action) {
            Ok(args) => Ok(args),
            Err(CommandLineError::Interrupted(_)) => Err(ExecException::Interrupted),
            Err(CommandLineError::CommandLineExpansion(e)) => {
                Err(ExecException::User(UserExecException::with_cause(
                    &e,
                    FailureDetail {
                        message: e.to_string(),
                        category: Some(failure_detail::Category::TestAction(
                            failure_details::TestAction {
                                code:
                                    failure_details::test_action::Code::CommandLineExpansionFailure
                                        as i32,
                            },
                        )),
                    },
                )))
            }
        }
    }

    /// Generates a command line to run for the test action, taking into account coverage and
    /// `--run_under` settings.
    // Java: com/google/devtools/build/lib/analysis/test/TestStrategy.java:211 TestStrategy.expandedArgsFromAction
    pub fn expanded_args_from_action(
        test_action: &TestRunnerAction,
    ) -> Result<Vec<String>, CommandLineError> {
        let mut args: Vec<String> = Vec::new();
        // PORT: the execution OS is always Linux.
        let execution_os = &HOST_POLICY;

        let test_setup = test_action.get_test_setup_script();
        args.push(
            test_setup
                .get_exec_path()
                .get_callable_path_string_for_os(execution_os),
        );

        if test_action.is_coverage_mode() {
            args.push(
                test_action
                    .get_collect_coverage_script()
                    .expect("NullPointerException: getCollectCoverageScript()")
                    .get_exec_path()
                    .get_callable_path_string_for_os(execution_os),
            );
        }

        let exec_settings = test_action.get_execution_settings();

        // Insert the command prefix specified by the "--run_under=<command-prefix>" option, if
        // any.
        if exec_settings.get_run_under().is_some() {
            add_run_under_args(test_action, &mut args);
        }

        // Execute the test using the alias in the runfiles tree, as mandated by the Test
        // Encyclopedia. Do not use getCallablePathStringForOs as tw.exe expects a path with
        // forward slashes.
        args.push(
            exec_settings
                .get_executable()
                .get_runfiles_path()
                .get_callable_path_string(),
        );
        args.extend(exec_settings.get_args().arguments()?);
        Ok(args)
    }

    /// Returns the number of attempts specific test action can be retried.
    ///
    /// For rules with "flaky = 1" attribute, this method will return 3 unless
    /// --flaky_test_attempts option is given and specifies another value.
    // Java: com/google/devtools/build/lib/analysis/test/TestStrategy.java:279 TestStrategy.getTestAttempts
    pub fn get_test_attempts(&self, action: &TestRunnerAction) -> i32 {
        if action.get_test_properties().is_flaky() {
            self.get_test_attempts_for_flaky_test(action)
        } else {
            self.get_test_attempts_with_default(action, /* default_test_attempts= */ 1)
        }
    }

    fn get_test_attempts_with_default(
        &self,
        action: &TestRunnerAction,
        default_test_attempts: i32,
    ) -> i32 {
        let test_label = action
            .get_owner()
            .get_label()
            .expect("NullPointerException: getOwner().getLabel()");
        get_test_attempts_per_label(&self.execution_options, test_label, default_test_attempts)
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestStrategy.java:281 TestStrategy.getTestAttemptsForFlakyTest
    pub fn get_test_attempts_for_flaky_test(&self, action: &TestRunnerAction) -> i32 {
        self.get_test_attempts_with_default(action, /* default_test_attempts= */ 3)
    }

    /// Finalize test run: persist the result, and post on the event bus.
    // Java: com/google/devtools/build/lib/analysis/test/TestStrategy.java:312 TestStrategy.postTestResult
    pub fn post_test_result(
        &self,
        action_execution_context: &dyn ActionExecutionContext,
        result: TestResult,
    ) -> io::Result<()> {
        result
            .get_test_action()
            .save_cache_status(action_execution_context, result.get_data())?;
        action_execution_context
            .get_event_handler()
            .post(Arc::new(result));
        Ok(())
    }

    /// Returns a unique name for a temporary directory a test could use.
    ///
    /// Since each test within single Blaze run must have a unique TEST_TMPDIR, we will use rule
    /// name and a unique (within single Blaze request) number to generate directory name.
    ///
    /// This does not create the directory.
    // Java: com/google/devtools/build/lib/analysis/test/TestStrategy.java:326 TestStrategy.getTmpDirName
    pub fn get_tmp_dir_name_for_exec_path(&self, exec_path: &PathFragment) -> String {
        let basename = exec_path.get_base_name().to_string();

        let mut tmp_index = self.tmp_index.lock();
        let index = *tmp_index.get(&basename).unwrap_or(&1);
        tmp_index.insert(basename.clone(), index + 1);
        format!("{basename}_{index}")
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestStrategy.java:326 TestStrategy.getTmpDirName
    pub fn get_tmp_dir_name(action: &TestRunnerAction) -> String {
        let mut digest = Fingerprint::new();
        digest.add_path_fragment(
            action
                .get_execution_settings()
                .get_executable()
                .get_exec_path(),
        );
        digest.add_int(action.get_shard_num());
        digest.add_int(action.get_run_number());
        // Truncate the string to 32 character to avoid exceeding path length limit on Windows and
        // macOS
        digest.hex_digest_and_reset()[..32].to_string()
    }

    /// Parse a test result XML file into a `TestCase`.
    // Java: com/google/devtools/build/lib/analysis/test/TestStrategy.java:353 TestStrategy.parseTestResult
    pub fn parse_test_result(&self, _result_file: &Path) -> Option<TestCase> {
        // xml files. We avoid parsing it unnecessarily, since test results can potentially
        // consume a large amount of memory.
        // Java: TestStrategy.PARSE_TEST_RESULT_FORMATS
        if !matches!(
            self.execution_options.test_summary,
            TestSummaryFormat::DETAILED
                | TestSummaryFormat::DETAILED_UNCACHED
                | TestSummaryFormat::TESTCASE
        ) {
            return None;
        }

        blaze_util::unported!("TestXmlOutputParser.parseXmlIntoTestResult")
    }

    /// Outputs test result to the stdout after test has finished (e.g. for --test_output=all or
    /// --test_output=errors). Will also try to group output lines together (up to 10000 lines)
    /// so parallel test outputs will not get interleaved.
    // Java: com/google/devtools/build/lib/analysis/test/TestStrategy.java:372 TestStrategy.processTestOutput
    pub fn process_test_output(
        &self,
        action_execution_context: &dyn ActionExecutionContext,
        test_result_data: &TestResultData,
        test_name: &str,
        test_log: Option<&Path>,
    ) -> io::Result<()> {
        let is_passed = test_result_data.test_passed();
        let result = (|| {
            if test_result_data.status() != BlazeTestStatus::Incomplete
                && test_log_helper::should_output_test_log(
                    self.execution_options.test_output,
                    is_passed,
                )
            {
                let mut out: Vec<u8> = Vec::new();
                let written = test_log_helper::write_test_log(
                    test_log.expect("NullPointerException: testLog"),
                    test_name,
                    &mut out,
                    self.execution_options.max_test_output_bytes,
                );
                action_execution_context
                    .get_file_out_err()
                    .write_out(&out)?;
                written?;
            }
            Ok(())
        })();
        // finally
        let handler = action_execution_context.get_event_handler();
        if is_passed {
            handler.handle(&Event::of(EventKind::Pass, test_name));
        } else {
            let test_log_path_to_output: Option<PathFragment> = test_log.map(|test_log| {
                if self.test_summary_options.print_relative_test_log_paths {
                    test_log
                        .as_fragment()
                        .relative_to(action_execution_context.get_exec_root().as_fragment())
                } else {
                    test_log.as_fragment().clone()
                }
            });
            if test_result_data.status_details.is_some() {
                handler.handle(&Event::error(format!(
                    "{}: {}",
                    test_name,
                    test_result_data.status_details()
                )));
            }
            let see = test_log_path_to_output
                .map(|p| format!(" (see {p})"))
                .unwrap_or_default();
            if test_result_data.status() == BlazeTestStatus::Timeout {
                let message = format!("{test_name}{see}");
                handler.handle(&Event::of(EventKind::Timeout, message));
            } else if test_result_data.status() == BlazeTestStatus::Incomplete {
                handler.handle(&Event::of(EventKind::Cancelled, test_name));
            } else {
                let ts: TerminationStatus = TerminationStatus::builder()
                    .set_wait_response(test_result_data.exit_code())
                    .set_timed_out(test_result_data.status() == BlazeTestStatus::Timeout)
                    .build();
                let message = format!("{} ({}){}", test_name, ts.to_short_string(), see);
                handler.handle(&Event::of(EventKind::Fail, message));
            }
        }
        result
    }

    /// Returns a temporary directory for all tests in a workspace to use. Individual tests
    /// should create child directories to actually use.
    ///
    /// This either dynamically generates a directory name or uses the directory specified by
    /// --test_tmpdir. This does not create the directory.
    // Java: com/google/devtools/build/lib/analysis/test/TestStrategy.java:444 TestStrategy.getTmpRoot
    pub fn get_tmp_root(
        workspace: &Path,
        exec_root: &Path,
        execution_options: &ExecutionOptions,
    ) -> Path {
        match &execution_options.test_tmp_dir {
            Some(test_tmp_dir) => workspace
                .get_relative(test_tmp_dir.get())
                .get_relative_str(TEST_TMP_ROOT),
            None => exec_root.get_relative_str(TEST_TMP_ROOT),
        }
    }

    /// Returns a subset containing all variables in the given list if they are defined in the
    /// given environment.
    // Java: com/google/devtools/build/lib/analysis/test/TestStrategy.java:455 TestStrategy.getMapping
    pub fn get_mapping(
        variables: &[String],
        environment: &IndexMap<String, String>,
    ) -> HashMap<String, String> {
        let mut result = HashMap::new();
        for var in variables {
            if let Some(v) = environment.get(var) {
                result.insert(var.clone(), v.clone());
            }
        }
        result
    }

    /// `createStreamedTestOutput(outErr, testLogPath)` (`--test_output=streamed`).
    // Java: com/google/devtools/build/lib/analysis/test/TestStrategy.java:477 TestStrategy.createStreamedTestOutput
    pub fn create_streamed_test_output(&self, _test_log_path: &Path) {
        blaze_util::unported!("TestStrategy.createStreamedTestOutput (StreamedTestOutput)")
    }
}

/// Removes directory if it exists and recreates it.
// Java: com/google/devtools/build/lib/analysis/test/TestStrategy.java:126 TestStrategy.recreateDirectory
fn recreate_directory(directory: &Path) -> io::Result<()> {
    directory.delete_tree()?;
    directory.create_directory_and_parents()
}

// Java: com/google/devtools/build/lib/analysis/test/TestStrategy.java:232 TestStrategy.addRunUnderArgs
fn add_run_under_args(test_action: &TestRunnerAction, args: &mut Vec<String>) {
    let exec_settings = test_action.get_execution_settings();
    let execution_os = &HOST_POLICY;
    let run_under = exec_settings.get_run_under().unwrap();
    match run_under {
        RunUnder::LabelRunUnder { .. } => {
            args.push(
                exec_settings
                    .get_run_under_executable()
                    .expect("NullPointerException: getRunUnderExecutable()")
                    .get_runfiles_path()
                    .get_callable_path_string_for_os(execution_os),
            );
        }
        RunUnder::CommandRunUnder { command, .. } => {
            if exec_settings.needs_shell() {
                // TestActionBuilder constructs TestRunnerAction with a 'null' shell only when none
                // is required. Something clearly went wrong.
                let shell_executable = test_action
                    .get_sh_executable_maybe()
                    .unwrap_or_else(|| panic!("NullPointerException: {test_action:?}"))
                    .get_callable_path_string_for_os(execution_os);
                args.push(shell_executable.clone());
                args.push("-c".to_string());
                args.push("\"$@\"".to_string());
                args.push(shell_executable); // Sets $0.
            }
            args.push(command.clone());
        }
    }
    args.extend(run_under.options().iter().cloned());
}

// Java: com/google/devtools/build/lib/analysis/test/TestStrategy.java:294 TestStrategy.getTestAttemptsPerLabel
fn get_test_attempts_per_label(
    options: &ExecutionOptions,
    label: &Label,
    default_test_attempts: i32,
) -> i32 {
    // Check from the last provided, so that the last option provided takes precedence.
    for per_label_attempts in options.test_attempts.iter().rev() {
        if per_label_attempts.is_included(label) {
            let opts = per_label_attempts.get_options();
            if opts.len() != 1 {
                panic!(
                    "IllegalArgumentException: expected one element but was: <{}>",
                    opts.join(", ")
                );
            }
            let attempts = &opts[0];
            if attempts == "default" {
                return default_test_attempts;
            }
            return attempts.parse::<i32>().unwrap_or_else(|_| {
                panic!("NumberFormatException: For input string: \"{attempts}\"")
            });
        }
    }
    default_test_attempts
}
