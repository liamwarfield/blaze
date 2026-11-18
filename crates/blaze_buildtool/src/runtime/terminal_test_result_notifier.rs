//! Port of `runtime/TerminalTestResultNotifier.java` (prints the test results to a terminal) and
//! the interface it implements, `runtime/TestResultNotifier.java`.

use std::collections::HashSet;
use std::sync::Arc;

use blaze_cmdline::{Label, RepositoryMapping};
use blaze_events::Mode;
use blaze_optdefs::command::execution_options::{ExecutionOptions, TestSummaryFormat};
use blaze_optdefs::command::test_summary_options::TestSummaryOptions;
use blaze_protos::blaze::BlazeTestStatus;

use super::test_summary::TestSummary;
use super::test_summary_printer::{self, TerminalPrinter, TestLogPathFormatter};
use blaze_exec::analysis::test::test_result::TestResult;
use blaze_exec::exec::test_log_helper;

/// Used to notify interested parties of test results.
// Java: com/google/devtools/build/lib/runtime/TestResultNotifier.java:21 TestResultNotifier
pub trait TestResultNotifier {
    /// `summaries`: summary of all targets that were supposed to be tested (regardless whether
    /// they actually were executed), in their natural (output) order.
    /// `number_of_executed_targets`: the number of targets that were actually run. Must not
    /// exceed `summaries.len()`.
    // Java: com/google/devtools/build/lib/runtime/TestResultNotifier.java:30 TestResultNotifier.notify
    fn notify(&self, summaries: &[Arc<TestSummary>], number_of_executed_targets: i32);
}

/// Prints the test results to a terminal.
///
/// PORT: Java reads `ExecutionOptions` and `TestSummaryOptions` from an `OptionsParsingResult`;
/// they are passed directly.
// Java: com/google/devtools/build/lib/runtime/TerminalTestResultNotifier.java:46 TerminalTestResultNotifier
pub struct TerminalTestResultNotifier {
    printer: Arc<dyn TerminalPrinter>,
    test_log_path_formatter: Arc<dyn TestLogPathFormatter>,
    execution_options: Arc<ExecutionOptions>,
    summary_options: Arc<TestSummaryOptions>,
    main_repo_mapping: RepositoryMapping,
}

// The number of failed-to-build tests to report.
// (We do not want to report hundreds of failed-to-build tests as it would probably be caused
// by some intermediate target not related to tests themselves.)
// The total number of failed-to-build tests will be reported in any case.
// Java: com/google/devtools/build/lib/runtime/TerminalTestResultNotifier.java:51 TerminalTestResultNotifier.NUM_FAILED_TO_BUILD
pub const NUM_FAILED_TO_BUILD: i32 = 5;

// Java: com/google/devtools/build/lib/runtime/TerminalTestResultNotifier.java:53 TerminalTestResultNotifier.TestResultStats
#[derive(Default)]
struct TestResultStats {
    number_of_targets: i32,
    pass_count: i32,
    failed_to_build_count: i32,
    failed_count: i32,
    failed_remotely_count: i32,
    failed_locally_count: i32,
    no_status_count: i32,
    number_of_executed_targets: i32,
    was_unreported_wrong_size: bool,

    total_test_cases: i32,
    total_failed_test_cases: i32,
    total_skipped_test_cases: i32,
    total_unknown_test_cases: i32,
}

// Java: com/google/devtools/build/lib/runtime/TerminalTestResultNotifier.java:161 TerminalTestResultNotifier.SHOW_ALL_TESTS_FORMATS
const SHOW_ALL_TESTS_FORMATS: &[TestSummaryFormat] = &[
    TestSummaryFormat::DETAILED,
    TestSummaryFormat::DETAILED_UNCACHED,
    TestSummaryFormat::SHORT,
    TestSummaryFormat::SHORT_UNCACHED,
];
// Java: com/google/devtools/build/lib/runtime/TerminalTestResultNotifier.java:164 TerminalTestResultNotifier.SHOW_NO_STATUS_TESTS_FORMATS
const SHOW_NO_STATUS_TESTS_FORMATS: &[TestSummaryFormat] = &[
    TestSummaryFormat::DETAILED,
    TestSummaryFormat::DETAILED_UNCACHED,
];
// Java: com/google/devtools/build/lib/runtime/TerminalTestResultNotifier.java:166 TerminalTestResultNotifier.SHOW_ALL_TEST_CASES_FORMATS
const SHOW_ALL_TEST_CASES_FORMATS: &[TestSummaryFormat] = &[
    TestSummaryFormat::DETAILED,
    TestSummaryFormat::DETAILED_UNCACHED,
];
// Java: com/google/devtools/build/lib/runtime/TerminalTestResultNotifier.java:167 TerminalTestResultNotifier.SHOW_CACHED_TESTS_FORMATS
const SHOW_CACHED_TESTS_FORMATS: &[TestSummaryFormat] =
    &[TestSummaryFormat::DETAILED, TestSummaryFormat::SHORT];

/// `StringUtil.joinEnglishList(choices, lastSeparator)`: `["a", "b", "c"]` -> `"a, b, and c"`;
/// an empty list is `"nothing"`.
// Java: com/google/devtools/build/lib/util/StringUtil.java:54 StringUtil.joinEnglishList
fn join_english_list(choices: &[String], last_separator: &str) -> String {
    // joinEnglishList(choices, lastSeparator, "", /* oxfordComma= */ true)
    let mut buf = String::new();
    for (num_choices_seen, choice) in choices.iter().enumerate() {
        let has_next = num_choices_seen + 1 < choices.len();
        if !buf.is_empty() {
            if has_next || num_choices_seen >= 2 {
                buf.push(',');
            }
            if !has_next {
                buf.push(' ');
                buf.push_str(last_separator);
            }
            buf.push(' ');
        }
        buf.push_str(choice);
    }
    if buf.is_empty() {
        "nothing".to_string()
    } else {
        buf
    }
}

impl TerminalTestResultNotifier {
    /// `printer`: The terminal to print to.
    // Java: com/google/devtools/build/lib/runtime/TerminalTestResultNotifier.java:79 TerminalTestResultNotifier.<init>
    pub fn new(
        printer: Arc<dyn TerminalPrinter>,
        test_log_path_formatter: Arc<dyn TestLogPathFormatter>,
        execution_options: Arc<ExecutionOptions>,
        summary_options: Arc<TestSummaryOptions>,
        main_repo_mapping: RepositoryMapping,
    ) -> TerminalTestResultNotifier {
        TerminalTestResultNotifier {
            printer,
            test_log_path_formatter,
            execution_options,
            summary_options,
            main_repo_mapping,
        }
    }

    /// Decide if two tests with the same label are contained in the set of test summaries
    // Java: com/google/devtools/build/lib/runtime/TerminalTestResultNotifier.java:94 TerminalTestResultNotifier.duplicateLabels
    fn duplicate_labels(&self, summaries: &[Arc<TestSummary>]) -> bool {
        let mut labels_seen: HashSet<Label> = HashSet::new();
        for summary in summaries {
            if labels_seen.contains(&summary.get_label()) {
                return true;
            }
            labels_seen.insert(summary.get_label());
        }
        false
    }

    /// Prints test result summary.
    ///
    /// `show_all_tests`: if true, print information about each test regardless of its status.
    /// `show_no_status_tests`: if true, print information about not executed tests (no status
    /// tests). `show_all_test_cases`: if true, print all test cases status and detailed
    /// information.
    // Java: com/google/devtools/build/lib/runtime/TerminalTestResultNotifier.java:113 TerminalTestResultNotifier.printSummary
    fn print_summary(
        &self,
        summaries: &[Arc<TestSummary>],
        show_all_tests: bool,
        show_no_status_tests: bool,
        show_all_test_cases: bool,
        show_cached_tests: bool,
    ) {
        let with_config = self.duplicate_labels(summaries);
        let mut num_failed_to_build_reported = 0;
        for summary in summaries {
            if !show_all_tests
                && (BlazeTestStatus::Passed == summary.get_status()
                    || (!show_no_status_tests && BlazeTestStatus::NoStatus == summary.get_status()))
            {
                continue;
            }
            if BlazeTestStatus::FailedToBuild == summary.get_status() {
                if num_failed_to_build_reported == NUM_FAILED_TO_BUILD {
                    self.printer
                        .print_ln("(Skipping other failed to build tests)");
                }
                num_failed_to_build_reported += 1;
                if num_failed_to_build_reported > NUM_FAILED_TO_BUILD {
                    continue;
                }
            }

            if !show_cached_tests
                && summary.get_status() == BlazeTestStatus::Passed
                && !summary.action_ran()
            {
                continue;
            }

            test_summary_printer::print_with_configuration(
                summary,
                self.printer.as_ref(),
                self.test_log_path_formatter.as_ref(),
                self.summary_options.verbose_summary,
                show_all_test_cases,
                with_config,
                &self.main_repo_mapping,
            );
        }
    }

    /// Returns true iff the --check_tests_up_to_date option is enabled.
    // Java: com/google/devtools/build/lib/runtime/TerminalTestResultNotifier.java:157 TerminalTestResultNotifier.optionCheckTestsUpToDate
    fn option_check_tests_up_to_date(&self) -> bool {
        self.execution_options.test_check_up_to_date
    }

    // Java: com/google/devtools/build/lib/runtime/TerminalTestResultNotifier.java:250 TerminalTestResultNotifier.addFailureToErrorList
    fn add_failure_to_error_list(list: &mut Vec<String>, failure_description: &str, count: i32) {
        Self::add_to_list(
            list,
            Mode::Error,
            "fails",
            "fail",
            failure_description,
            count,
        );
    }

    // Java: com/google/devtools/build/lib/runtime/TerminalTestResultNotifier.java:254 TerminalTestResultNotifier.addToWarningList
    fn add_to_warning_list(
        list: &mut Vec<String>,
        singular_prefix: &str,
        plural_prefix: &str,
        message: &str,
        count: i32,
    ) {
        Self::add_to_list(
            list,
            Mode::Warning,
            singular_prefix,
            plural_prefix,
            message,
            count,
        );
    }

    // Java: com/google/devtools/build/lib/runtime/TerminalTestResultNotifier.java:259 TerminalTestResultNotifier.addToList
    fn add_to_list(
        list: &mut Vec<String>,
        mode: Mode,
        singular_prefix: &str,
        plural_prefix: &str,
        message: &str,
        count: i32,
    ) {
        if count > 0 {
            list.push(format!(
                "{}{} {} {}{}",
                mode,
                count,
                if count == 1 {
                    singular_prefix
                } else {
                    plural_prefix
                },
                message,
                Mode::Default
            ));
        }
    }

    // Java: com/google/devtools/build/lib/runtime/TerminalTestResultNotifier.java:278 TerminalTestResultNotifier.printStats
    fn print_stats(&self, stats: &TestResultStats) {
        let test_summary_format = self.execution_options.test_summary;
        if test_summary_format == TestSummaryFormat::DETAILED
            || test_summary_format == TestSummaryFormat::DETAILED_UNCACHED
            || test_summary_format == TestSummaryFormat::TESTCASE
        {
            let pass_count = stats.total_test_cases
                - stats.total_failed_test_cases
                - stats.total_unknown_test_cases
                - stats.total_skipped_test_cases;
            let mut message = format!(
                "Test cases: finished with {}{} passing{}, {}{} skipped{} and {}{} failing{} out of {} test cases",
                if pass_count > 0 {
                    Mode::Info.to_string()
                } else {
                    String::new()
                },
                pass_count,
                Mode::Default,
                if stats.total_skipped_test_cases > 0 {
                    Mode::Warning.to_string()
                } else {
                    String::new()
                },
                stats.total_skipped_test_cases,
                Mode::Default,
                if stats.total_failed_test_cases > 0 {
                    Mode::Error.to_string()
                } else {
                    String::new()
                },
                stats.total_failed_test_cases,
                Mode::Default,
                stats.total_test_cases
            );
            if stats.total_unknown_test_cases != 0 {
                // It is possible for a target to fail even if all of its test cases pass. To avoid
                // confusion, we append the following disclaimer.
                message += " (some targets did not have test case information)";
            }
            self.printer.print_ln(&message);
        }

        if !self.option_check_tests_up_to_date() {
            let mut results: Vec<String> = Vec::new();
            if stats.pass_count == 1 {
                results.push(format!("{} test passes", stats.pass_count));
            } else if stats.pass_count > 0 {
                results.push(format!("{} tests pass", stats.pass_count));
            }
            Self::add_failure_to_error_list(&mut results, "to build", stats.failed_to_build_count);
            Self::add_failure_to_error_list(&mut results, "locally", stats.failed_locally_count);
            Self::add_failure_to_error_list(&mut results, "remotely", stats.failed_remotely_count);
            Self::add_to_warning_list(
                &mut results,
                "was",
                "were",
                "skipped",
                stats.no_status_count,
            );
            self.printer.print(&format!(
                "\nExecuted {} out of {} {}: {}.\n",
                stats.number_of_executed_targets,
                stats.number_of_targets,
                if stats.number_of_targets == 1 {
                    "test"
                } else {
                    "tests"
                },
                join_english_list(&results, "and")
            ));
        } else {
            let failing_up_to_date_count = stats.failed_count - stats.no_status_count;
            self.printer.print(&format!(
                "\nFinished with {} passing and {}{} failing{} tests up to date, {}{} out of date.{}\n",
                stats.pass_count,
                if failing_up_to_date_count > 0 {
                    Mode::Error.to_string()
                } else {
                    String::new()
                },
                failing_up_to_date_count,
                Mode::Default,
                if stats.no_status_count > 0 {
                    Mode::Error.to_string()
                } else {
                    String::new()
                },
                stats.no_status_count,
                Mode::Default
            ));
        }

        if stats.was_unreported_wrong_size {
            self.printer.print(
                "There were tests whose specified size is too big. Use the \
                 --test_verbose_timeout_warnings command line option to see which \
                 ones these are.\n",
            );
        }
    }
}

impl TestResultNotifier for TerminalTestResultNotifier {
    /// Prints a test summary information for all tests to the terminal.
    // Java: com/google/devtools/build/lib/runtime/TerminalTestResultNotifier.java:177 TerminalTestResultNotifier.notify
    fn notify(&self, summaries: &[Arc<TestSummary>], number_of_executed_targets: i32) {
        let mut stats = TestResultStats {
            number_of_targets: summaries.len() as i32,
            number_of_executed_targets,
            ..Default::default()
        };

        let execution_options = &self.execution_options;
        let test_output = execution_options.test_output;

        for summary in summaries {
            if summary.is_local_action_cached()
                && test_log_helper::should_output_test_log(
                    test_output,
                    TestResult::is_blaze_test_status_passed(summary.get_status()),
                )
            {
                test_summary_printer::print_cached_output(
                    summary,
                    test_output,
                    self.printer.as_ref(),
                    self.test_log_path_formatter.as_ref(),
                    execution_options.max_test_output_bytes,
                );
            }
        }

        for summary in summaries {
            if TestResult::is_blaze_test_status_passed(summary.get_status()) {
                stats.pass_count += 1;
            } else if summary.get_status() == BlazeTestStatus::NoStatus
                || summary.get_status() == BlazeTestStatus::BlazeHaltedBeforeTesting
            {
                stats.no_status_count += 1;
            } else if summary.get_status() == BlazeTestStatus::FailedToBuild {
                stats.failed_to_build_count += 1;
            } else if summary.ran_remotely() {
                stats.failed_remotely_count += 1;
            } else {
                stats.failed_locally_count += 1;
            }

            if summary.was_unreported_wrong_size() {
                stats.was_unreported_wrong_size = true;
            }

            stats.total_test_cases += summary.get_total_test_cases();
            stats.total_unknown_test_cases += summary.get_unknown_test_cases();
            stats.total_failed_test_cases += summary.get_failed_test_cases().len() as i32;
            stats.total_skipped_test_cases += summary.get_skipped_test_cases().len() as i32;
        }

        stats.failed_count = summaries.len() as i32 - stats.pass_count;

        let test_summary_format = execution_options.test_summary;
        match test_summary_format {
            TestSummaryFormat::DETAILED
            | TestSummaryFormat::DETAILED_UNCACHED
            | TestSummaryFormat::SHORT
            | TestSummaryFormat::SHORT_UNCACHED
            | TestSummaryFormat::TERSE => {
                let show_all_tests = SHOW_ALL_TESTS_FORMATS.contains(&test_summary_format);
                let show_no_status_tests =
                    SHOW_NO_STATUS_TESTS_FORMATS.contains(&test_summary_format);
                let show_all_test_cases =
                    SHOW_ALL_TEST_CASES_FORMATS.contains(&test_summary_format);
                let show_cached_tests = SHOW_CACHED_TESTS_FORMATS.contains(&test_summary_format);
                self.print_summary(
                    summaries,
                    show_all_tests,
                    show_no_status_tests,
                    show_all_test_cases,
                    show_cached_tests,
                );
            }

            TestSummaryFormat::TESTCASE | TestSummaryFormat::NONE => {}
        }

        self.print_stats(&stats);
    }
}
