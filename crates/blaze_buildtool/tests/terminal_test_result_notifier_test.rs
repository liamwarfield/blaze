//! Port of `src/test/java/com/google/devtools/build/lib/runtime/TerminalTestResultNotifierTest.java`.
//!
//! PORT: Java mocks `TestSummary`; these tests build real summaries with the same getter values
//! (`TestSummarySpec.build`), and record the printer calls with [`common::RecordingPrinter`].

mod common;

use std::sync::Arc;

use blaze_buildtool::runtime::terminal_test_result_notifier::{
    NUM_FAILED_TO_BUILD, TerminalTestResultNotifier, TestResultNotifier,
};
use blaze_buildtool::runtime::test_summary::TestSummary;
use blaze_cmdline::Label;
use blaze_cmdline::repository_mapping::EMPTY;
use blaze_events::{AnsiTerminalPrinter, ByteArrayOutputStream, Mode};
use blaze_optdefs::command::execution_options::{ExecutionOptions, TestSummaryFormat};
use blaze_optdefs::command::test_summary_options::TestSummaryOptions;
use blaze_optdefs::common::test_timeout::TestTimeout;
use blaze_protos::blaze::test_case::Status;
use blaze_protos::blaze::{BlazeTestStatus, TestCase};
use blaze_vfs::Path;
use common::{RecordingPrinter, StubTarget, configuration};

const SOME_TARGETS_ARE_MISSING_TEST_CASES_DISCLAIMER: &str =
    "some targets did not have test case information";

fn stub_target(label: &str) -> Arc<StubTarget> {
    Arc::new(StubTarget {
        label: Label::parse_canonical(label).unwrap(),
        checksum: "abcdef".to_string(),
        shards: 1,
        runs: 1,
        runs_detects_flakes: false,
        timeout: TestTimeout::SHORT,
        test_status_artifacts: Vec::new(),
    })
}

/// A helper that creates `TestSummary` values for testing.
struct TestSummarySpec {
    status: BlazeTestStatus,
    failed_test_cases: i32,
    skipped_test_cases: i32,
    unknown_test_cases: i32,
    total_test_cases: i32,
    action_ran: bool,
}

impl TestSummarySpec {
    fn build(&self) -> Arc<TestSummary> {
        let mut builder = TestSummary::new_builder(stub_target("//foo:bar"));
        builder
            .set_status(self.status)
            .set_action_ran(self.action_ran);
        let leaf = |status: Status| TestCase {
            status: Some(status as i32),
            ..Default::default()
        };
        let passed = self.total_test_cases
            - self.unknown_test_cases
            - self.failed_test_cases
            - self.skipped_test_cases;
        let mut children = Vec::new();
        children.extend((0..self.failed_test_cases).map(|_| leaf(Status::Failed)));
        children.extend((0..self.skipped_test_cases).map(|_| leaf(Status::Skipped)));
        children.extend((0..passed).map(|_| leaf(Status::Passed)));
        if !children.is_empty() {
            let root = TestCase {
                child: children,
                ..Default::default()
            };
            builder.collect_test_cases(Some(&root));
        }
        for _ in 0..self.unknown_test_cases {
            builder.collect_test_cases(None);
        }
        let summary = builder.build();
        assert_eq!(summary.get_total_test_cases(), self.total_test_cases);
        assert_eq!(summary.get_unknown_test_cases(), self.unknown_test_cases);
        assert_eq!(
            summary.get_failed_test_cases().len() as i32,
            self.failed_test_cases
        );
        assert_eq!(
            summary.get_skipped_test_cases().len() as i32,
            self.skipped_test_cases
        );
        summary
    }
}

fn spec(
    status: BlazeTestStatus,
    failed: i32,
    skipped: i32,
    unknown: i32,
    total: i32,
    action_ran: bool,
) -> TestSummarySpec {
    TestSummarySpec {
        status,
        failed_test_cases: failed,
        skipped_test_cases: skipped,
        unknown_test_cases: unknown,
        total_test_cases: total,
        action_ran,
    }
}

fn options(
    test_summary_format: TestSummaryFormat,
) -> (Arc<ExecutionOptions>, Arc<TestSummaryOptions>) {
    let execution_options = ExecutionOptions {
        test_summary: test_summary_format,
        ..Default::default()
    };
    let test_summary_options = TestSummaryOptions {
        verbose_summary: true,
        ..Default::default()
    };
    (Arc::new(execution_options), Arc::new(test_summary_options))
}

fn path_string(p: &Path) -> String {
    p.get_path_string().to_string()
}

fn print_failed_to_build_summaries(
    printer: &Arc<RecordingPrinter>,
    test_summary_format: TestSummaryFormat,
) {
    let (execution_options, test_summary_options) = options(test_summary_format);

    // Java orders the summaries by label name (ImmutableSortedSet); "baz0".."baz5" already are.
    let mut summaries = Vec::new();
    for i in 0..NUM_FAILED_TO_BUILD + 1 {
        let summary = TestSummary::new_builder(stub_target(&format!("//foo/bar:baz{}", i)))
            .set_status(BlazeTestStatus::FailedToBuild)
            .set_action_ran(false)
            .build();
        summaries.push(summary);
    }

    let terminal_test_result_notifier = TerminalTestResultNotifier::new(
        printer.clone(),
        Arc::new(path_string),
        execution_options,
        test_summary_options,
        EMPTY.clone(),
    );
    terminal_test_result_notifier.notify(&summaries, 0);
}

fn print_test_case_summary(
    printer: &Arc<RecordingPrinter>,
    test_summary_spec: TestSummarySpec,
    test_summary_format: TestSummaryFormat,
) {
    let (execution_options, test_summary_options) = options(test_summary_format);
    let terminal_test_result_notifier = TerminalTestResultNotifier::new(
        printer.clone(),
        Arc::new(path_string),
        execution_options,
        test_summary_options,
        EMPTY.clone(),
    );
    terminal_test_result_notifier.notify(&[test_summary_spec.build()], 1);
}

/// `verify(ansiTerminalPrinter).printLn(captor.capture())`: exactly one `printLn`.
fn get_printed_message(printer: &RecordingPrinter) -> String {
    let lns = printer.print_lns();
    assert_eq!(lns.len(), 1, "{:?}", lns);
    lns[0].clone()
}

fn verify_no_summary_printed(printer: &RecordingPrinter) {
    assert!(printer.print_lns().is_empty(), "{:?}", printer.print_lns());
}

fn info(message: &str) -> String {
    format!("{}{}{}", Mode::Info, message, Mode::Default)
}

fn warn(message: &str) -> String {
    format!("{}{}{}", Mode::Warning, message, Mode::Default)
}

fn error(message: &str) -> String {
    format!("{}{}{}", Mode::Error, message, Mode::Default)
}

#[test]
fn test_case_option_all_pass() {
    let p = RecordingPrinter::new();
    print_test_case_summary(
        &p,
        spec(BlazeTestStatus::Passed, 0, 0, 0, 10, true),
        TestSummaryFormat::TESTCASE,
    );

    let printed = get_printed_message(&p);
    assert!(printed.contains(&info("10 passing")));
    assert!(printed.contains("0 failing"));
    assert!(printed.contains("out of 10 test cases"));
    assert!(!printed.contains(SOME_TARGETS_ARE_MISSING_TEST_CASES_DISCLAIMER));
    assert!(!printed.contains(&Mode::Error.to_string()));
}

#[test]
fn test_case_option_all_pass_but_target_fails() {
    let p = RecordingPrinter::new();
    print_test_case_summary(
        &p,
        spec(BlazeTestStatus::Failed, 0, 0, 10, 10, true),
        TestSummaryFormat::TESTCASE,
    );

    let printed = get_printed_message(&p);
    assert!(printed.contains("0 passing"));
    assert!(printed.contains("0 failing"));
    assert!(printed.contains("out of 10 test cases"));
    assert!(printed.contains(SOME_TARGETS_ARE_MISSING_TEST_CASES_DISCLAIMER));
    assert!(!printed.contains(&Mode::Error.to_string()));
}

#[test]
fn test_case_option_some_fail() {
    let p = RecordingPrinter::new();
    print_test_case_summary(
        &p,
        spec(BlazeTestStatus::Failed, 2, 0, 0, 10, true),
        TestSummaryFormat::TESTCASE,
    );

    let printed = get_printed_message(&p);
    assert!(printed.contains(&info("8 passing")));
    assert!(printed.contains(&error("2 failing")));
    assert!(printed.contains("out of 10 test cases"));
    assert!(!printed.contains(SOME_TARGETS_ARE_MISSING_TEST_CASES_DISCLAIMER));
}

fn check_some_fail_to_build(format: TestSummaryFormat) {
    let num_failed_to_build_test_cases = NUM_FAILED_TO_BUILD + 1;
    let p = RecordingPrinter::new();
    print_failed_to_build_summaries(&p, format);

    let skipped_message = get_printed_message(&p);
    assert_eq!(skipped_message, "(Skipping other failed to build tests)");

    let values = p.prints();
    assert_eq!(values.len() as i32, num_failed_to_build_test_cases);

    for message in values
        .iter()
        .take(num_failed_to_build_test_cases as usize - 1)
    {
        assert!(message.contains("//foo/bar:baz"));
        assert!(message.contains(&"FAILED_TO_BUILD".replace('_', " ")));
    }

    let last = &values[num_failed_to_build_test_cases as usize - 1];
    assert!(last.contains("Executed 0 out of 6 tests"));
    assert!(last.contains(&format!("{} fail to build", num_failed_to_build_test_cases)));
}

#[test]
fn short_option_some_fail_to_build() {
    check_some_fail_to_build(TestSummaryFormat::SHORT);
}

#[test]
fn short_uncached_option_some_fail_to_build() {
    check_some_fail_to_build(TestSummaryFormat::SHORT_UNCACHED);
}

#[test]
fn test_case_option_all_fail() {
    let p = RecordingPrinter::new();
    print_test_case_summary(
        &p,
        spec(BlazeTestStatus::Failed, 10, 0, 0, 10, true),
        TestSummaryFormat::TESTCASE,
    );

    let printed = get_printed_message(&p);
    assert!(printed.contains("0 passing"));
    assert!(printed.contains(&error("10 failing")));
    assert!(printed.contains("out of 10 test cases"));
    assert!(!printed.contains(SOME_TARGETS_ARE_MISSING_TEST_CASES_DISCLAIMER));
    assert!(!printed.contains(&Mode::Info.to_string()));
}

fn check_all_pass(format: TestSummaryFormat, action_ran: bool) {
    let p = RecordingPrinter::new();
    print_test_case_summary(
        &p,
        spec(BlazeTestStatus::Passed, 0, 0, 0, 10, action_ran),
        format,
    );

    let printed = get_printed_message(&p);
    assert!(printed.contains(&info("10 passing")));
    assert!(printed.contains("0 failing"));
    assert!(printed.contains("0 skipped"));
    assert!(printed.contains("out of 10 test cases"));
    assert!(!printed.contains(SOME_TARGETS_ARE_MISSING_TEST_CASES_DISCLAIMER));
    assert!(!printed.contains(&Mode::Error.to_string()));
}

#[test]
fn detailed_option_all_pass() {
    check_all_pass(TestSummaryFormat::DETAILED, true);
}

#[test]
fn detailed_uncached_option_all_pass_uncached() {
    check_all_pass(TestSummaryFormat::DETAILED_UNCACHED, true);
}

#[test]
fn detailed_uncached_option_all_pass_cached() {
    check_all_pass(TestSummaryFormat::DETAILED_UNCACHED, false);
}

fn check_all_pass_but_some_skipped(format: TestSummaryFormat, action_ran: bool) {
    let p = RecordingPrinter::new();
    print_test_case_summary(
        &p,
        spec(BlazeTestStatus::Passed, 0, 2, 0, 10, action_ran),
        format,
    );

    let printed = get_printed_message(&p);
    assert!(printed.contains(&info("8 passing")));
    assert!(printed.contains("0 failing"));
    assert!(printed.contains(&warn("2 skipped")));
    assert!(printed.contains("out of 10 test cases"));
    assert!(!printed.contains(SOME_TARGETS_ARE_MISSING_TEST_CASES_DISCLAIMER));
    assert!(!printed.contains(&Mode::Error.to_string()));
}

#[test]
fn detailed_option_all_pass_but_some_skipped() {
    check_all_pass_but_some_skipped(TestSummaryFormat::DETAILED, true);
}

#[test]
fn detailed_uncached_option_all_pass_uncached_but_some_skipped() {
    check_all_pass_but_some_skipped(TestSummaryFormat::DETAILED_UNCACHED, true);
}

#[test]
fn detailed_uncached_option_all_pass_cached_but_some_skipped() {
    check_all_pass_but_some_skipped(TestSummaryFormat::DETAILED_UNCACHED, false);
}

fn check_all_pass_but_target_fails(format: TestSummaryFormat) {
    let p = RecordingPrinter::new();
    print_test_case_summary(
        &p,
        spec(BlazeTestStatus::Failed, 0, 0, 10, 10, true),
        format,
    );

    let printed = get_printed_message(&p);
    assert!(printed.contains("0 passing"));
    assert!(printed.contains("0 failing"));
    assert!(printed.contains("out of 10 test cases"));
    assert!(printed.contains(SOME_TARGETS_ARE_MISSING_TEST_CASES_DISCLAIMER));
    assert!(!printed.contains(&Mode::Error.to_string()));
}

#[test]
fn detailed_option_all_pass_but_target_fails() {
    check_all_pass_but_target_fails(TestSummaryFormat::DETAILED);
}

#[test]
fn detailed_uncached_option_all_pass_but_target_fails() {
    check_all_pass_but_target_fails(TestSummaryFormat::DETAILED_UNCACHED);
}

fn check_some_fail(format: TestSummaryFormat) {
    let p = RecordingPrinter::new();
    print_test_case_summary(&p, spec(BlazeTestStatus::Failed, 2, 0, 0, 10, true), format);

    let printed = get_printed_message(&p);
    assert!(printed.contains(&info("8 passing")));
    assert!(printed.contains(&error("2 failing")));
    assert!(printed.contains("out of 10 test cases"));
    assert!(!printed.contains(SOME_TARGETS_ARE_MISSING_TEST_CASES_DISCLAIMER));
}

#[test]
fn detailed_option_some_fail() {
    check_some_fail(TestSummaryFormat::DETAILED);
}

#[test]
fn detailed_uncached_option_some_fail() {
    check_some_fail(TestSummaryFormat::DETAILED_UNCACHED);
}

fn check_all_fail(format: TestSummaryFormat) {
    let p = RecordingPrinter::new();
    print_test_case_summary(
        &p,
        spec(BlazeTestStatus::Failed, 10, 0, 0, 10, true),
        format,
    );

    let printed = get_printed_message(&p);
    assert!(printed.contains("0 passing"));
    assert!(printed.contains(&error("10 failing")));
    assert!(printed.contains("out of 10 test cases"));
    assert!(!printed.contains(SOME_TARGETS_ARE_MISSING_TEST_CASES_DISCLAIMER));
    assert!(!printed.contains(&Mode::Info.to_string()));
}

#[test]
fn detailed_option_all_fail() {
    check_all_fail(TestSummaryFormat::DETAILED);
}

#[test]
fn detailed_uncached_option_all_fail() {
    check_all_fail(TestSummaryFormat::DETAILED_UNCACHED);
}

fn check_no_summary_printed(format: TestSummaryFormat) {
    let p = RecordingPrinter::new();
    print_test_case_summary(&p, spec(BlazeTestStatus::Failed, 2, 0, 0, 10, true), format);

    verify_no_summary_printed(&p);
}

#[test]
fn short_option_no_summary_printed() {
    check_no_summary_printed(TestSummaryFormat::SHORT);
}

#[test]
fn short_uncached_option_no_summary_printed() {
    check_no_summary_printed(TestSummaryFormat::SHORT_UNCACHED);
}

#[test]
fn terse_option_no_summary_printed() {
    check_no_summary_printed(TestSummaryFormat::TERSE);
}

#[test]
fn none_option_no_summary_printed() {
    check_no_summary_printed(TestSummaryFormat::NONE);
}

/// re2's `bazel test //...` on the oracle: 21 passing tests, one of them too big for its size.
/// Test times are the oracle's BEP `testAttemptDurationMillis`; the expected text is the
/// oracle's stdout (`oracle-records/oracle-9.3.2/re2/raw/build.stdout`), printed through a real
/// `AnsiTerminalPrinter` without color.
#[test]
fn oracle_re2_test_summary() {
    let tests: &[(&str, i64)] = &[
        ("//:charclass_test", 53),
        ("//:compile_test", 56),
        ("//:dfa_test", 334923),
        ("//:exhaustive1_test", 90591),
        ("//:exhaustive2_test", 10323),
        ("//:exhaustive3_test", 101788),
        ("//:exhaustive_test", 39172),
        ("//:filtered_re2_test", 51),
        ("//:mimics_pcre_test", 42),
        ("//:parse_test", 56),
        ("//:possible_match_test", 1544),
        ("//:random_test", 298291),
        ("//:re2_arg_test", 77),
        ("//:re2_test", 6687),
        ("//:regexp_test", 271),
        ("//:required_prefix_test", 53),
        ("//:search_test", 3372),
        ("//:set_test", 79),
        ("//:simplify_test", 64),
        ("//:string_generator_test", 53),
        ("//python:re2_test", 821),
    ];
    let mut summaries: Vec<Arc<TestSummary>> = tests
        .iter()
        .map(|(label, millis)| {
            TestSummary::new_builder(stub_target(label))
                .set_configuration(configuration())
                .set_status(BlazeTestStatus::Passed)
                .set_action_ran(true)
                .add_test_times(&[*millis])
                .set_was_unreported_wrong_size(*label == "//:charclass_test")
                .build()
        })
        .collect();
    // The natural ordering of the summaries defines their output order.
    summaries.sort_by(|a, b| a.compare_to(b));

    let stream = Arc::new(ByteArrayOutputStream::new());
    let printer = Arc::new(AnsiTerminalPrinter::new(stream.clone(), false));
    let (execution_options, test_summary_options) = options(TestSummaryFormat::SHORT);
    let notifier = TerminalTestResultNotifier::new(
        printer,
        Arc::new(path_string),
        execution_options,
        test_summary_options,
        EMPTY.clone(),
    );
    notifier.notify(&summaries, 21);

    let expected = "\
//:charclass_test                                                        PASSED in 0.1s
//:compile_test                                                          PASSED in 0.1s
//:dfa_test                                                              PASSED in 334.9s
//:exhaustive1_test                                                      PASSED in 90.6s
//:exhaustive2_test                                                      PASSED in 10.3s
//:exhaustive3_test                                                      PASSED in 101.8s
//:exhaustive_test                                                       PASSED in 39.2s
//:filtered_re2_test                                                     PASSED in 0.1s
//:mimics_pcre_test                                                      PASSED in 0.0s
//:parse_test                                                            PASSED in 0.1s
//:possible_match_test                                                   PASSED in 1.5s
//:random_test                                                           PASSED in 298.3s
//:re2_arg_test                                                          PASSED in 0.1s
//:re2_test                                                              PASSED in 6.7s
//:regexp_test                                                           PASSED in 0.3s
//:required_prefix_test                                                  PASSED in 0.1s
//:search_test                                                           PASSED in 3.4s
//:set_test                                                              PASSED in 0.1s
//:simplify_test                                                         PASSED in 0.1s
//:string_generator_test                                                 PASSED in 0.1s
//python:re2_test                                                        PASSED in 0.8s

Executed 21 out of 21 tests: 21 tests pass.
There were tests whose specified size is too big. Use the --test_verbose_timeout_warnings command line option to see which ones these are.
";
    assert_eq!(String::from_utf8(stream.to_byte_array()).unwrap(), expected);
}
