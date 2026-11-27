//! The result handling of `runtime/commands/TestCommand.java` (`exec` / `doTest` /
//! `analyzeTestResults` / `makeTestLogPathFormatter`) around an [`AggregatingTestListener`],
//! for the `test` command driver.
//!
//! PORT: `TestCommand` itself (building the request, `BuildTool.processRequest`) is the command
//! driver's; this module holds what `TestCommand` does with the listener and the
//! `BuildResult`. Java registers the listener on the command's event bus (Guava `@Subscribe`);
//! here [`TestCommandSupport`] is an [`EventBus`] the driver forwards every posted event to, and
//! it dispatches the ones `AggregatingTestListener` subscribes to.
//!
//! Use, in this order:
//! 1. [`TestCommandSupport::new`] before the build (prints the `--test_output=streamed` warning).
//! 2. Forward every event posted during the build to [`TestCommandSupport::subscriber`]
//!    (`TestAnalyzedEvent`, `TestResult`, `TargetCompleteEvent` are the ones it reads).
//! 3. [`TestCommandSupport::build_complete_event`] where Bazel posts `BuildCompleteEvent`
//!    (`BuildTool.stopRequest`), with the final `BuildResult`.
//! 4. [`TestCommandSupport::finish`]: prints the summary and returns the command's exit code.

use std::any::Any;
use std::sync::Arc;

use blaze_cmdline::RepositoryMapping;
use blaze_events::{Event, EventBus, EventHandler, Mode, Postable};
use blaze_exec::analysis::test::test_result::TestResult;
use blaze_optdefs::command::execution_options::{ExecutionOptions, TestOutputFormat};
use blaze_optdefs::command::test_summary_options::TestSummaryOptions;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use blaze_sky_exec::events::{TargetCompleteEvent, TestAnalyzedEvent, downcast_event};
use blaze_util::{DetailedExitCode, ExitCode};
use blaze_vfs::{Path, PathFragment};
use indexmap::IndexMap;

use super::aggregating_test_listener::{AggregatingTestListener, ConfiguredTargetKey};
use super::terminal_test_result_notifier::{TerminalTestResultNotifier, TestResultNotifier};
use super::test_summary::ConfigurationView;
use super::test_summary_printer::{TerminalPrinter, TestLogPathFormatter};
use crate::buildtool::build_result::{
    BuildResult, DetailedExitCodeFactory, DetailedExitCodeRef, DetailedExitCodeView,
    choose_more_important_with_first_if_tie,
};
use crate::buildtool::path_pretty_printer::PathPrettyPrinter;

impl DetailedExitCodeView for DetailedExitCode {
    // Java: com/google/devtools/build/lib/util/DetailedExitCode.java:58 DetailedExitCode.isSuccess
    fn is_success(&self) -> bool {
        DetailedExitCode::is_success(self)
    }

    // Java: com/google/devtools/build/lib/util/ExitCode.java:205 ExitCode.isInfrastructureFailure
    fn is_infrastructure_failure(&self) -> bool {
        self.get_exit_code().is_infrastructure_failure()
    }

    // Java: com/google/devtools/build/lib/util/DetailedExitCode.java:54 DetailedExitCode.getFailureDetail
    fn get_failure_detail(&self) -> Option<&FailureDetail> {
        DetailedExitCode::get_failure_detail(self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The `DetailedExitCode` constructors, with `blaze_util`'s [`DetailedExitCode`] (the shared
/// port of `lib/util/DetailedExitCode`).
pub struct UtilDetailedExitCodes;

impl DetailedExitCodeFactory for UtilDetailedExitCodes {
    // Java: com/google/devtools/build/lib/util/DetailedExitCode.java:86 DetailedExitCode.of
    fn of(&self, failure_detail: FailureDetail) -> DetailedExitCodeRef {
        Arc::new(DetailedExitCode::of(failure_detail))
    }

    // Java: com/google/devtools/build/lib/util/DetailedExitCode.java:63 DetailedExitCode.success
    fn success(&self) -> DetailedExitCodeRef {
        Arc::new(DetailedExitCode::success())
    }
}

/// `detailedExitCode.getExitCode()` of a [`DetailedExitCodeRef`].
///
/// PORT: a [`DetailedExitCodeRef`] made by [`UtilDetailedExitCodes`] (or holding a
/// `blaze_util::DetailedExitCode`) carries its `ExitCode`; any other implementation must carry a
/// `FailureDetail` (whose metadata gives the code) or be a success.
// Java: com/google/devtools/build/lib/util/DetailedExitCode.java:40 DetailedExitCode.getExitCode
pub fn get_exit_code(code: &DetailedExitCodeRef) -> ExitCode {
    if let Some(code) = code.as_any().downcast_ref::<DetailedExitCode>() {
        return code.get_exit_code();
    }
    if let Some(failure_detail) = code.get_failure_detail() {
        return DetailedExitCode::get_exit_code_of(failure_detail);
    }
    if code.is_success() {
        return ExitCode::SUCCESS;
    }
    blaze_util::unported!("DetailedExitCode.justExitCode (exit code of a foreign DetailedExitCode)")
}

/// `BuildConfigurationValue` as `TestAnalyzedEvent` carries it (see
/// `blaze_sky_exec::events::TestAnalyzedEvent`).
#[derive(Debug)]
struct AnalyzedConfiguration {
    checksum: String,
    mnemonic: String,
}

impl ConfigurationView for AnalyzedConfiguration {
    fn checksum(&self) -> String {
        self.checksum.clone()
    }

    fn get_mnemonic(&self) -> String {
        self.mnemonic.clone()
    }
}

/// `buildtool/buildevent/TestingCompleteEvent`: posted when the test command finishes.
// Java: com/google/devtools/build/lib/buildtool/buildevent/TestingCompleteEvent.java:29 TestingCompleteEvent
#[derive(Clone, Debug)]
pub struct TestingCompleteEvent {
    pub exit_code: ExitCode,
    pub finish_time_millis: i64,
}

impl Postable for TestingCompleteEvent {}

/// `runtime/commands/NoTestsFound`: posted when testing was requested but no test targets were
/// found.
// Java: com/google/devtools/build/lib/runtime/commands/NoTestsFound.java:23 NoTestsFound
#[derive(Clone, Debug)]
pub struct NoTestsFound {
    pub exit_code: ExitCode,
    pub finish_time_millis: i64,
}

impl Postable for NoTestsFound {}

/// The `test` command's `AggregatingTestListener` and the result handling of
/// `TestCommand.doTest`. See the module documentation for the call order.
// Java: com/google/devtools/build/lib/runtime/commands/TestCommand.java:75 TestCommand
pub struct TestCommandSupport {
    listener: AggregatingTestListener,
    execution_options: Arc<ExecutionOptions>,
    summary_options: Arc<TestSummaryOptions>,
    event_bus: Arc<dyn EventBus>,
    exit_codes: Arc<dyn DetailedExitCodeFactory>,
}

impl TestCommandSupport {
    /// The start of `TestCommand.exec`: warns about streamed test output and creates the
    /// `AggregatingTestListener` on `event_bus` (the command's event bus, which receives the
    /// `TestSummary`/`TestAttempt` events the listener posts).
    // Java: com/google/devtools/build/lib/runtime/commands/TestCommand.java:98 TestCommand.exec
    pub fn new(
        execution_options: Arc<ExecutionOptions>,
        summary_options: Arc<TestSummaryOptions>,
        event_bus: Arc<dyn EventBus>,
        reporter: &dyn EventHandler,
    ) -> Arc<TestCommandSupport> {
        if execution_options.test_output == TestOutputFormat::STREAMED {
            reporter.handle(&Event::warn(
                "Streamed test output requested. All tests will be run without sharding, \
                 one at a time",
            ));
        }
        let exit_codes: Arc<dyn DetailedExitCodeFactory> = Arc::new(UtilDetailedExitCodes);
        // Initialize test handler.
        let listener = AggregatingTestListener::new(
            summary_options.clone(),
            execution_options.clone(),
            event_bus.clone(),
            exit_codes.clone(),
        );
        Arc::new(TestCommandSupport {
            listener,
            execution_options,
            summary_options,
            event_bus,
            exit_codes,
        })
    }

    /// The listener, for the `@Subscribe` methods without a Blaze event type
    /// (`analysisFailure`, `buildInterrupted`, `populateTests`).
    pub fn listener(&self) -> &AggregatingTestListener {
        &self.listener
    }

    /// `env.getEventBus().register(testListener)`: the subscriber to add to the build's event
    /// fan-out (e.g. `ExecutionEventCollector::forward`).
    pub fn subscriber(self: &Arc<Self>) -> Arc<dyn EventBus> {
        self.clone()
    }

    /// `AggregatingTestListener.buildCompleteEvent`, called where Bazel posts
    /// `BuildCompleteEvent` (before [`Self::finish`]).
    // Java: com/google/devtools/build/lib/runtime/AggregatingTestListener.java:207 AggregatingTestListener.buildCompleteEvent
    pub fn build_complete_event(&self, result: &BuildResult) {
        self.listener.build_complete_event(result);
    }

    /// The rest of `TestCommand.doTest` after `BuildTool.processRequest`: reports a build that
    /// could not start or found no tests, otherwise prints the test summary through a
    /// `TerminalTestResultNotifier` on `printer` and combines the build's and the tests' exit
    /// codes. Posts `TestingCompleteEvent` / `NoTestsFound` on the event bus.
    ///
    /// `printer` is `new AnsiTerminalPrinter(reporter.getOutErr().getOutputStream(),
    /// uiOptions.useColor())` (stdout); `test_log_path_formatter` comes from
    /// [`make_test_log_path_formatter`]; `validated_targets` is `None` unless
    /// `BuildRequest.useValidationAspect()` (`--experimental_use_validation_aspect`), when it holds
    /// the base targets of the successful validation aspects.
    ///
    /// Returns the command's `DetailedExitCode` (`get_exit_code(..).get_numeric_exit_code()` is
    /// the process exit code: 0, 1 build failure, 3 `TESTS_FAILED`, 4 `NO_TESTS_FOUND`, ...).
    // Java: com/google/devtools/build/lib/runtime/commands/TestCommand.java:124 TestCommand.doTest
    pub fn finish(
        &self,
        build_result: &BuildResult,
        reporter: &dyn EventHandler,
        printer: Arc<dyn TerminalPrinter>,
        test_log_path_formatter: Arc<dyn TestLogPathFormatter>,
        main_repo_mapping: RepositoryMapping,
        validated_targets: Option<&[ConfiguredTargetKey]>,
    ) -> DetailedExitCodeRef {
        // TODO(bazel-team): don't handle isEmpty here or fix up a bunch of tests
        if build_result.get_successful_targets().is_none() {
            // This can happen if there were errors in the target parsing or loading phase
            // (original exitcode=BUILD_FAILURE) or if there weren't but --noanalyze was given
            // (original exitcode=SUCCESS).
            let message = "Couldn't start the build. Unable to run tests";
            reporter.handle(&Event::error(message));
            let detailed_exit_code = if build_result.get_success() {
                self.exit_codes.of(FailureDetail {
                    message: message.to_string(),
                    category: Some(Category::TestCommand(failure_details::TestCommand {
                        code: failure_details::test_command::Code::TestWithNoanalyze as i32,
                    })),
                })
            } else {
                build_result.get_detailed_exit_code(self.exit_codes.as_ref())
            };
            self.event_bus.post(Arc::new(TestingCompleteEvent {
                exit_code: get_exit_code(&detailed_exit_code),
                finish_time_millis: build_result.get_stop_time(),
            }));
            return detailed_exit_code;
        }
        let test_targets = build_result
            .get_test_targets()
            .unwrap_or_else(|| panic!("NullPointerException: buildResult.getTestTargets()"));
        // TODO(bazel-team): the check above shadows NO_TESTS_FOUND, but switching the conditions
        // breaks more tests
        if test_targets.is_empty() {
            let message = "No test targets were found, yet testing was requested";
            reporter.handle(&Event::error(message));

            let detailed_exit_code = if build_result.get_success() {
                self.exit_codes.of(FailureDetail {
                    message: message.to_string(),
                    category: Some(Category::TestCommand(failure_details::TestCommand {
                        code: failure_details::test_command::Code::NoTestTargets as i32,
                    })),
                })
            } else {
                build_result.get_detailed_exit_code(self.exit_codes.as_ref())
            };
            self.event_bus.post(Arc::new(NoTestsFound {
                exit_code: get_exit_code(&detailed_exit_code),
                finish_time_millis: build_result.get_stop_time(),
            }));
            return detailed_exit_code;
        }

        let test_results = self.analyze_test_results(
            build_result,
            printer.clone(),
            test_log_path_formatter,
            main_repo_mapping,
            validated_targets,
        );

        if test_results.is_success() && !build_result.get_success() {
            // If all tests run successfully, test summary should include warning if
            // there were build errors not associated with the test targets.
            printer.print_ln(&format!(
                "{}All tests passed but there were other errors during the build.\n{}",
                Mode::Error,
                Mode::Default
            ));
        }

        let detailed_exit_code = choose_more_important_with_first_if_tie(
            Some(build_result.get_detailed_exit_code(self.exit_codes.as_ref())),
            Some(test_results),
        )
        .expect("chooseMoreImportantWithFirstIfTie of two codes");
        self.event_bus.post(Arc::new(TestingCompleteEvent {
            exit_code: get_exit_code(&detailed_exit_code),
            finish_time_millis: build_result.get_stop_time(),
        }));
        detailed_exit_code
    }

    /// Analyzes test results and prints summary information. Returns a `DetailedExitCode`
    /// summarizing those test results.
    // Java: com/google/devtools/build/lib/runtime/commands/TestCommand.java:237 TestCommand.analyzeTestResults
    fn analyze_test_results(
        &self,
        build_result: &BuildResult,
        printer: Arc<dyn TerminalPrinter>,
        test_log_path_formatter: Arc<dyn TestLogPathFormatter>,
        main_repo_mapping: RepositoryMapping,
        validated_targets: Option<&[ConfiguredTargetKey]>,
    ) -> DetailedExitCodeRef {
        let notifier: Box<dyn TestResultNotifier> = Box::new(TerminalTestResultNotifier::new(
            printer,
            test_log_path_formatter,
            self.execution_options.clone(),
            self.summary_options.clone(),
            main_repo_mapping,
        ));
        self.listener.differential_analyze_and_report(
            build_result
                .get_test_targets()
                .unwrap_or_else(|| panic!("NullPointerException: buildResult.getTestTargets()")),
            build_result
                .get_skipped_targets()
                .unwrap_or_else(|| panic!("NullPointerException: buildResult.getSkippedTargets()")),
            validated_targets,
            &*notifier,
        )
    }
}

impl EventBus for TestCommandSupport {
    /// Dispatches the events `AggregatingTestListener` subscribes to.
    fn post(&self, event: Arc<dyn Postable>) {
        if let Some(e) = downcast_event::<TestResult>(&event) {
            // Java: com/google/devtools/build/lib/runtime/AggregatingTestListener.java:151 AggregatingTestListener.testEvent
            self.listener.test_event(e);
        } else if let Some(e) = downcast_event::<TestAnalyzedEvent>(&event) {
            // Java: com/google/devtools/build/lib/runtime/AggregatingTestListener.java:126 AggregatingTestListener.populateTest
            self.listener.populate_test(
                &e.configured_target,
                Arc::new(AnalyzedConfiguration {
                    checksum: e.configuration_checksum.clone(),
                    mnemonic: e.configuration_mnemonic.clone(),
                }),
                e.is_skipped,
            );
        } else if let Some(e) = downcast_event::<TargetCompleteEvent>(&event) {
            // Java: com/google/devtools/build/lib/runtime/AggregatingTestListener.java:234 AggregatingTestListener.targetComplete
            self.listener.target_complete(
                e.failed(),
                &ConfiguredTargetKey {
                    label: e.label.clone(),
                    configuration_checksum: e.configuration_checksum.clone(),
                },
            );
        }
    }
}

/// `TestCommand.makeTestLogPathFormatter`: absolute paths, or with `--print_relative_test_log_paths`
/// paths relative to the working directory through the convenience symlinks.
///
/// `relative_working_directory` is `env.getRelativeWorkingDirectory()`; `symlink_prefix` is
/// `BuildRequestOptions.getSymlinkPrefix(productName)`.
// Java: com/google/devtools/build/lib/runtime/commands/TestCommand.java:266 TestCommand.makeTestLogPathFormatter
pub fn make_test_log_path_formatter(
    convenience_symlinks: &IndexMap<PathFragment, PathFragment>,
    summary_options: &TestSummaryOptions,
    relative_working_directory: &PathFragment,
    symlink_prefix: &str,
) -> Arc<dyn TestLogPathFormatter> {
    if !summary_options.print_relative_test_log_paths {
        return Arc::new(|path: &Path| path.get_path_string().to_string());
    }
    let path_pretty_printer = PathPrettyPrinter::new(
        relative_working_directory,
        symlink_prefix,
        convenience_symlinks.clone(),
    );
    Arc::new(move |path: &Path| {
        path_pretty_printer
            .get_pretty_path(path.as_fragment())
            .get_path_string()
            .to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // TestCommand has no JUnit test at the pin; these check the exit codes doTest returns
    // (FailureDetails metadata: TESTS_FAILED=3, NO_TESTS_FOUND=4).
    #[test]
    fn no_test_targets_exit_code_is_no_tests_found() {
        let code = UtilDetailedExitCodes.of(FailureDetail {
            message: "No test targets were found, yet testing was requested".to_string(),
            category: Some(Category::TestCommand(failure_details::TestCommand {
                code: failure_details::test_command::Code::NoTestTargets as i32,
            })),
        });
        assert_eq!(get_exit_code(&code), ExitCode::NO_TESTS_FOUND);
        assert_eq!(get_exit_code(&code).get_numeric_exit_code(), 4);
    }

    #[test]
    fn tests_failed_exit_code() {
        let code = UtilDetailedExitCodes.of(FailureDetail {
            message: "tests failed".to_string(),
            category: Some(Category::TestCommand(failure_details::TestCommand {
                code: failure_details::test_command::Code::TestsFailed as i32,
            })),
        });
        assert_eq!(get_exit_code(&code), ExitCode::TESTS_FAILED);
        assert_eq!(get_exit_code(&code).get_numeric_exit_code(), 3);
        assert!(!code.is_success());
    }

    #[test]
    fn success_exit_code() {
        let code = UtilDetailedExitCodes.success();
        assert!(code.is_success());
        assert_eq!(get_exit_code(&code), ExitCode::SUCCESS);
    }
}
