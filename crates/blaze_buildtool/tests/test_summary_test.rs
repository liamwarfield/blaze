//! Port of `src/test/java/com/google/devtools/build/lib/runtime/TestSummaryTest.java`.
//!
//! PORT: Java mocks `AnsiTerminalPrinter`; [`common::RecordingPrinter`] records the raw strings.
//! Several Java tests call `terminalPrinter.print(find(...))` on the mock without `verify(...)`,
//! which checks nothing; those calls are kept as comments and not asserted.

mod common;

use std::io::Write;
use std::sync::Arc;

use blaze_buildtool::runtime::test_summary::{TestSummary, TestSummaryBuilder};
use blaze_buildtool::runtime::test_summary_printer;
use blaze_protos::blaze::test_case::Status;
use blaze_protos::blaze::{BlazeTestStatus, FailedTestCasesStatus, TestCase};
use blaze_sky_exec::analysis_seam::ConfiguredTargetView;
use blaze_vfs::{DigestHashFunction, Path, UnixFileSystem};
use common::{RecordingPrinter, configuration, contains, find, target};

const PATH: &str = "package";
const TARGET_NAME: &str = "name";
const SMALL_TIMING: &[i64] = &[1, 2, 3, 4];

const CACHED: i32 = SMALL_TIMING.len() as i32;
const NOT_CACHED: i32 = 0;

fn stub_target() -> Arc<dyn ConfiguredTargetView> {
    target(PATH, TARGET_NAME)
}

fn get_template_builder() -> TestSummaryBuilder {
    let mut b = TestSummary::new_builder(stub_target());
    b.set_configuration(configuration())
        .set_status(BlazeTestStatus::Passed)
        .set_num_cached(NOT_CACHED)
        .set_action_ran(true)
        .set_ran_remotely(false)
        .set_was_unreported_wrong_size(false);
    b
}

/// `fs.getPath(name)` on the Java in-memory file system: the tests only print these paths.
fn get_path_list(names: &[&str]) -> Vec<Path> {
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    names
        .iter()
        .map(|n| Path::create_from_str(n, &fs))
        .collect()
}

fn path_string(p: &Path) -> String {
    p.get_path_string().to_string()
}

fn create_test_summary_for(
    target: Arc<dyn ConfiguredTargetView>,
    status: BlazeTestStatus,
    num_cached: i32,
) -> Arc<TestSummary> {
    TestSummary::new_builder(target)
        .set_status(status)
        .set_num_cached(num_cached)
        .set_action_ran(true)
        .set_ran_remotely(false)
        .set_was_unreported_wrong_size(false)
        .add_failed_test_cases(&[], FailedTestCasesStatus::Full)
        .add_test_times(SMALL_TIMING)
        .build()
}

fn create_test_summary(status: BlazeTestStatus, num_cached: i32) -> Arc<TestSummary> {
    get_template_builder()
        .set_status(status)
        .set_num_cached(num_cached)
        .add_test_times(SMALL_TIMING)
        .build()
}

fn create_passed_test_summary(status: BlazeTestStatus, details: &[TestCase]) -> Arc<TestSummary> {
    get_template_builder()
        .set_status(status)
        .add_passed_test_cases(details)
        .build()
}

fn create_test_summary_with_details(
    status: BlazeTestStatus,
    details: &[TestCase],
) -> Arc<TestSummary> {
    get_template_builder()
        .set_status(status)
        .add_failed_test_cases(details, FailedTestCasesStatus::Full)
        .build()
}

fn create_test_summary_with_details_status(
    status: BlazeTestStatus,
    test_case_list: &[TestCase],
    details_status: FailedTestCasesStatus,
) -> Arc<TestSummary> {
    get_template_builder()
        .set_status(status)
        .add_failed_test_cases(test_case_list, details_status)
        .build()
}

fn new_detail(name: &str, status: Status, duration: i64) -> TestCase {
    TestCase {
        name: Some(name.to_string()),
        status: Some(status as i32),
        run_duration_millis: Some(duration),
        ..Default::default()
    }
}

fn print(summary: &TestSummary, printer: &RecordingPrinter, verbose: bool, show_all: bool) {
    test_summary_printer::print(summary, printer, &path_string, verbose, show_all);
}

#[test]
fn test_should_properly_test_labels() {
    let target = target("somepath", "MyTarget");
    let printer = RecordingPrinter::new();
    let summary_status = create_test_summary_for(target, BlazeTestStatus::Passed, CACHED);
    print(&summary_status, &printer, true, false);
    // Java: terminalPrinter.print(find(ANY_STRING + "//somepath:MyTarget" + ANY_STRING)), unverified.
}

#[test]
fn test_should_print_passed_status() {
    let printer = RecordingPrinter::new();
    let summary = create_test_summary_for(stub_target(), BlazeTestStatus::Passed, NOT_CACHED);
    print(&summary, &printer, true, false);

    printer.verify_print(find(".*?INFO.*?PASSED.*?"));
}

#[test]
fn test_should_print_failed_status() {
    let printer = RecordingPrinter::new();
    let summary = create_test_summary_for(stub_target(), BlazeTestStatus::Failed, NOT_CACHED);
    print(&summary, &printer, true, false);
    // Java: terminalPrinter.print(find(".*?ERROR.*?FAILED.*?")), unverified.
}

fn assert_should_not_print(status: BlazeTestStatus, verbose_summary: bool) {
    let printer = RecordingPrinter::new();
    print(
        &create_test_summary_for(stub_target(), status, NOT_CACHED),
        &printer,
        verbose_summary,
        false,
    );
    printer.verify_never_print();
}

#[test]
fn test_should_print_failed_to_build_status() {
    let printer = RecordingPrinter::new();
    let summary = create_test_summary(BlazeTestStatus::FailedToBuild, NOT_CACHED);
    print(&summary, &printer, true, false);
    // Java: terminalPrinter.print(find(".*?INFO.*?FAILED_TO_BUILD")), unverified.
}

#[test]
fn test_should_not_print_failed_to_build_status() {
    assert_should_not_print(BlazeTestStatus::FailedToBuild, false);
}

#[test]
fn test_should_not_print_halted_status() {
    assert_should_not_print(BlazeTestStatus::BlazeHaltedBeforeTesting, true);
}

#[test]
fn test_should_print_cached_status() {
    let printer = RecordingPrinter::new();
    let summary = create_test_summary_for(stub_target(), BlazeTestStatus::Passed, CACHED);
    print(&summary, &printer, true, false);
    // Java: terminalPrinter.print(find(".*?\\(cached.*?")), unverified.
}

#[test]
fn test_partial_cached_status() {
    let printer = RecordingPrinter::new();
    let summary = create_test_summary_for(stub_target(), BlazeTestStatus::Passed, CACHED - 1);
    print(&summary, &printer, true, false);
    // Java: terminalPrinter.print(find(".*?\\(3/4 cached.*?")), unverified.
}

#[test]
fn test_incomplete_cached() {
    let printer = RecordingPrinter::new();
    let summary = create_test_summary_for(stub_target(), BlazeTestStatus::Incomplete, CACHED - 1);
    print(&summary, &printer, true, false);
    printer.verify_print(|s| !s.contains("cached"));
}

#[test]
fn test_should_print_uncached_status() {
    let printer = RecordingPrinter::new();
    let summary = create_test_summary_for(stub_target(), BlazeTestStatus::Passed, NOT_CACHED);
    print(&summary, &printer, true, false);
    printer.verify_print(|s| !s.contains("cached"));
}

#[test]
fn test_no_timing() {
    let printer = RecordingPrinter::new();
    let summary = create_test_summary_for(stub_target(), BlazeTestStatus::Passed, NOT_CACHED);
    print(&summary, &printer, true, false);
    // Java: terminalPrinter.print(find(".*?INFO.*?PASSED")), unverified.
}

#[test]
fn test_builder() {
    let mut basic_builder = get_template_builder();
    // No need to copy if built twice in a row; no direct setters on the object.
    let summary = basic_builder.build();
    let same_summary = basic_builder.build();
    assert!(Arc::ptr_eq(&same_summary, &summary));

    basic_builder.add_test_times(&[40]);

    let summary_copy = basic_builder.build();
    assert!(Arc::ptr_eq(summary_copy.get_target(), summary.get_target()));
    assert_eq!(summary_copy.get_status(), summary.get_status());
    assert_eq!(summary_copy.num_cached(), summary.num_cached());
    assert!(!Arc::ptr_eq(&summary_copy, &summary));
    assert_eq!(summary.total_runs(), 0);
    assert_eq!(summary_copy.total_runs(), 1);

    // Check that the builder can add a new warning to the copy,
    // despite the immutability of the original.
    basic_builder.add_test_times(&[60]);

    let fifty_cached = basic_builder.set_num_cached(50).build();
    assert_eq!(fifty_cached.get_status(), summary.get_status());
    assert_eq!(fifty_cached.num_cached(), 50);
    assert_eq!(fifty_cached.total_runs(), 2);

    let sixty_cached = basic_builder.set_num_cached(60).build();
    assert_eq!(sixty_cached.num_cached(), 60);
    assert_eq!(fifty_cached.num_cached(), 50);
}

#[test]
#[ignore = "needs the BEP port (TestSummary.asStreamProto, BuildEventContext, PathConverter)"]
fn test_as_stream_proto() {}

#[test]
fn test_single_time() {
    let printer = RecordingPrinter::new();
    let summary = get_template_builder().add_test_times(&[3412]).build();
    print(&summary, &printer, true, false);
    // Java: terminalPrinter.print(find(".*?INFO.*?PASSED.*?in 3.4s")), unverified.
}

#[test]
fn test_no_time() {
    let printer = RecordingPrinter::new();
    let summary = get_template_builder().add_test_times(&[3412]).build();
    print(&summary, &printer, false, false);
    // Java: terminalPrinter.print(find(".*?INFO.*?PASSED(?!in)*")), unverified.
}

#[test]
fn test_multiple_times() {
    let printer = RecordingPrinter::new();
    let summary = get_template_builder()
        .add_test_times(&[1000, 2000, 3000])
        .build();
    print(&summary, &printer, true, false);
    // Java: terminalPrinter.print(find(... "\n  Stats over 3 runs: max = 3.0s, min = 1.0s, avg =
    // 2.0s, dev = 0.8s")), unverified.
}

#[test]
fn test_coverage_data_references() {
    // PORT: Java's in-memory file system is rooted at '/'; here the files live in a temporary
    // directory and the path formatter prints them relative to it, as '/cov1.dat' etc.
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let base_str = base.to_str().unwrap().to_string();
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let paths: Vec<Path> = ["/cov1.dat", "/cov2.dat", "/cov3.dat", "/cov4.dat"]
        .iter()
        .map(|n| Path::create_from_str(&format!("{}{}", base_str, n), &fs))
        .collect();
    let write = |p: &Path, content: &str| {
        let mut out = p.get_output_stream().unwrap();
        out.write_all(content.as_bytes()).unwrap();
        out.flush().unwrap();
    };
    write(&paths[1], "something");
    write(&paths[3], "");
    write(&paths[3], "something else");
    let summary = get_template_builder().add_coverage_files(&paths).build();

    let printer = RecordingPrinter::new();
    let formatter = move |p: &Path| -> String { p.get_path_string()[base_str.len()..].to_string() };
    test_summary_printer::print(&summary, printer.as_ref(), &formatter, true, false);
    printer.verify_print(find(".*?INFO.*?PASSED"));
    printer.verify_print(find("  /cov2.dat"));
    printer.verify_print(find("  /cov4.dat"));
}

#[test]
fn test_flaky_attempts() {
    let printer = RecordingPrinter::new();
    let summary = get_template_builder()
        .set_status(BlazeTestStatus::Flaky)
        .add_passed_logs(&get_path_list(&["/a"]))
        .add_failed_logs(&get_path_list(&["/b", "/c"]))
        .build();
    print(&summary, &printer, true, false);
    // Java: terminalPrinter.print(find(".*?WARNING.*?FLAKY.*?, failed in 2 out of 3")),
    // unverified.
}

#[test]
fn test_number_of_failed_runs() {
    let printer = RecordingPrinter::new();
    let summary = get_template_builder()
        .set_status(BlazeTestStatus::Failed)
        .add_passed_logs(&get_path_list(&["/a"]))
        .add_failed_logs(&get_path_list(&["/b", "/c"]))
        .build();
    print(&summary, &printer, true, false);
    // Java: terminalPrinter.print(find(".*?ERROR.*?FAILED.*?in 2 out of 3")), unverified.
}

#[test]
fn test_file_names_not_shown() {
    let summary = get_template_builder()
        .set_status(BlazeTestStatus::Failed)
        .add_passed_logs(&get_path_list(&["/apple"]))
        .add_failed_logs(&get_path_list(&["/pear"]))
        .add_coverage_files(&get_path_list(&["/maracuja"]))
        .add_failed_test_cases(&[], FailedTestCasesStatus::Full)
        .build();

    // Check that only //package:name is printed.
    let printer = RecordingPrinter::new();
    print(&summary, &printer, true, true);
    printer.verify_print(contains("//package:name"));
}

#[test]
fn test_message_shown_when_test_cases_missing() {
    let summary = create_test_summary_with_details_status(
        BlazeTestStatus::Failed,
        &[],
        FailedTestCasesStatus::NotAvailable,
    );

    let printer = RecordingPrinter::new();
    print(&summary, &printer, true, true);
    printer.verify_print(contains("//package:name"));
    printer.verify_print(contains("not available"));
}

#[test]
fn test_message_shown_for_partial_results() {
    let test_cases = vec![new_detail("orange", Status::Failed, 1500)];
    let summary = create_test_summary_with_details_status(
        BlazeTestStatus::Failed,
        &test_cases,
        FailedTestCasesStatus::Partial,
    );

    let printer = RecordingPrinter::new();
    print(&summary, &printer, true, true);
    printer.verify_print(contains("//package:name"));
    printer.verify_print(find("FAILED.*orange"));
    printer.verify_print(contains("incomplete"));
}

#[test]
fn test_test_case_names_shown_when_needed() {
    let detail_passed = new_detail("strawberry", Status::Passed, 1000);
    let detail_failed = new_detail("orange", Status::Failed, 1500);

    let summary_passed = create_test_summary_with_details(
        BlazeTestStatus::Passed,
        std::slice::from_ref(&detail_passed),
    );

    let summary_failed =
        create_test_summary_with_details(BlazeTestStatus::Failed, &[detail_passed, detail_failed]);
    assert_eq!(summary_failed.get_status(), BlazeTestStatus::Failed);

    let printer_passed = RecordingPrinter::new();
    print(&summary_passed, &printer_passed, true, true);
    printer_passed.verify_print(contains("//package:name"));

    let printer_failed = RecordingPrinter::new();
    print(&summary_failed, &printer_failed, true, true);
    printer_failed.verify_print(contains("//package:name"));
    printer_failed.verify_print(find("FAILED.*orange *\\(1\\.5"));
}

#[test]
fn test_show_test_case_names() {
    let detail_passed = new_detail("strawberry", Status::Passed, 1000);
    let detail_failed = new_detail("orange", Status::Failed, 1500);

    let summary_passed = create_passed_test_summary(
        BlazeTestStatus::Passed,
        std::slice::from_ref(&detail_passed),
    );

    let summary_failed =
        create_test_summary_with_details(BlazeTestStatus::Failed, &[detail_passed, detail_failed]);
    assert_eq!(summary_failed.get_status(), BlazeTestStatus::Failed);

    let printer_passed = RecordingPrinter::new();
    print(&summary_passed, &printer_passed, true, true);
    printer_passed.verify_print(contains("//package:name"));
    printer_passed.verify_print(find("PASSED.*strawberry *\\(1\\.0"));

    let printer_failed = RecordingPrinter::new();
    print(&summary_failed, &printer_failed, true, true);
    printer_failed.verify_print(contains("//package:name"));
    printer_failed.verify_print(find("FAILED.*orange *\\(1\\.5"));
}

#[test]
fn test_test_case_names_ordered() {
    let details = [
        new_detail("apple", Status::Failed, 1000),
        new_detail("banana", Status::Failed, 1000),
        new_detail("cranberry", Status::Failed, 1000),
    ];

    // The exceedingly dumb approach: writing all the permutations down manually
    // is simply easier than any way of generating them.
    let permutations = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];

    for permutation in permutations {
        let permutated_details: Vec<TestCase> =
            permutation.iter().map(|&e| details[e].clone()).collect();

        let summary =
            create_test_summary_with_details(BlazeTestStatus::Failed, &permutated_details);

        // A mock that checks the ordering of method calls
        let printer = RecordingPrinter::new();
        print(&summary, &printer, true, true);
        let prints = printer.prints();
        let pos = |m: &dyn Fn(&str) -> bool| prints.iter().position(|s| m(s)).unwrap();
        let p0 = pos(&contains("//package:name"));
        let p1 = pos(&find("FAILED.*apple"));
        let p2 = pos(&find("FAILED.*banana"));
        let p3 = pos(&find("FAILED.*cranberry"));
        assert!(p0 < p1 && p1 < p2 && p2 < p3, "{:?}", prints);
    }
}

#[test]
fn test_cached_results_first_in_sort() {
    let summary_failed_cached = create_test_summary(BlazeTestStatus::Failed, CACHED);
    let summary_failed_not_cached = create_test_summary(BlazeTestStatus::Failed, NOT_CACHED);
    let summary_passed_cached = create_test_summary(BlazeTestStatus::Passed, CACHED);
    let summary_passed_not_cached = create_test_summary(BlazeTestStatus::Passed, NOT_CACHED);

    // This way we can make the test independent from the sort order of FAILEd
    // and PASSED.

    assert!(
        summary_failed_cached
            .compare_to(&summary_passed_not_cached)
            .is_lt()
    );
    assert!(
        summary_passed_cached
            .compare_to(&summary_failed_not_cached)
            .is_lt()
    );
}

fn root_case(name: &str, children: Vec<TestCase>) -> TestCase {
    TestCase {
        name: Some(name.to_string()),
        run_duration_millis: Some(5000),
        child: children,
        ..Default::default()
    }
}

#[test]
fn test_collecting_failed_details() {
    let root_case = root_case(
        "tests",
        vec![
            new_detail("apple", Status::Failed, 1000),
            new_detail("cherry", Status::Error, 1000),
        ],
    );

    let summary = get_template_builder()
        .collect_test_cases(Some(&root_case))
        .set_status(BlazeTestStatus::Failed)
        .build();

    let printer = RecordingPrinter::new();
    print(&summary, &printer, true, true);
    printer.verify_print(contains("//package:name"));
    printer.verify_print(find("FAILED.*apple"));
    printer.verify_print(find("ERROR.*cherry"));
}

#[test]
fn test_collecting_all_details() {
    let root_case = root_case(
        "tests",
        vec![
            new_detail("apple", Status::Failed, 1000),
            new_detail("banana", Status::Passed, 1000),
            new_detail("cherry", Status::Error, 1000),
            new_detail("sugarcane", Status::Skipped, 0),
        ],
    );

    let summary = get_template_builder()
        .collect_test_cases(Some(&root_case))
        .set_status(BlazeTestStatus::Failed)
        .build();

    let printer = RecordingPrinter::new();
    print(&summary, &printer, true, true);
    printer.verify_print(contains("//package:name"));
    printer.verify_print(find("FAILED.*apple"));
    printer.verify_print(find("PASSED.*banana"));
    printer.verify_print(find("ERROR.*cherry"));
    printer.verify_print(find("SKIPPED.*sugarcane"));
}

#[test]
fn test_collecting_passed_details() {
    let root_case = root_case(
        "tests",
        vec![
            new_detail("apple", Status::Passed, 1000),
            new_detail("banana", Status::Skipped, 0),
        ],
    );

    let summary = get_template_builder()
        .collect_test_cases(Some(&root_case))
        .set_status(BlazeTestStatus::Passed)
        .build();

    let printer = RecordingPrinter::new();
    print(&summary, &printer, true, true);
    printer.verify_print(contains("//package:name"));
    printer.verify_print(find("PASSED.*apple"));
    printer.verify_print(find("SKIPPED.*banana"));
}

#[test]
fn count_total_test_cases() {
    let root_case = root_case(
        "tests",
        vec![
            new_detail("apple", Status::Failed, 1000),
            new_detail("banana", Status::Passed, 1000),
            new_detail("cherry", Status::Error, 1000),
            new_detail("sugarcane", Status::Skipped, 0),
        ],
    );

    let summary = get_template_builder()
        .collect_test_cases(Some(&root_case))
        .set_status(BlazeTestStatus::Failed)
        .build();

    assert_eq!(summary.get_total_test_cases(), 4);
}

#[test]
fn count_unknown_test_cases() {
    let summary = get_template_builder()
        .collect_test_cases(None)
        .set_status(BlazeTestStatus::Failed)
        .build();

    assert_eq!(summary.get_total_test_cases(), 1);
    assert_eq!(summary.get_unknown_test_cases(), 1);
}

#[test]
fn count_not_run_test_cases() {
    let leaf = |name: &str, run: bool| TestCase {
        name: Some(name.to_string()),
        status: Some(Status::Passed as i32),
        run: Some(run),
        ..Default::default()
    };
    let a = TestCase {
        child: vec![leaf("A", true), leaf("B", true), leaf("C", false)],
        ..Default::default()
    };
    let summary = get_template_builder()
        .collect_test_cases(Some(&a))
        .set_status(BlazeTestStatus::Failed)
        .build();

    assert_eq!(summary.get_total_test_cases(), 2);
    assert_eq!(summary.get_unknown_test_cases(), 0);
    assert!(summary.get_failed_test_cases().is_empty());
}

#[test]
fn count_total_test_cases_in_nested_tree() {
    let a_case = root_case(
        "tests-1",
        vec![
            new_detail("apple", Status::Failed, 1000),
            new_detail("banana", Status::Passed, 1000),
            new_detail("cherry", Status::Error, 1000),
        ],
    );
    let another_case = root_case(
        "tests-2",
        vec![
            new_detail("apple", Status::Failed, 1000),
            new_detail("banana", Status::Passed, 1000),
            new_detail("cherry", Status::Error, 1000),
        ],
    );

    let root_case = TestCase {
        name: Some("tests".to_string()),
        child: vec![a_case, another_case],
        ..Default::default()
    };

    let summary = get_template_builder()
        .collect_test_cases(Some(&root_case))
        .set_status(BlazeTestStatus::Failed)
        .build();

    assert_eq!(summary.get_total_test_cases(), 6);
}

/// Exact output of a passing, uncached summary line, compared with re2's oracle stdout
/// (`//:charclass_test` ... `PASSED in 0.1s`, padded to column 79).
#[test]
fn oracle_re2_passed_line() {
    let summary = TestSummary::new_builder(target("", "charclass_test"))
        .set_configuration(configuration())
        .set_status(BlazeTestStatus::Passed)
        .set_action_ran(true)
        .add_test_times(&[100])
        .build();
    let printer = RecordingPrinter::new();
    print(&summary, &printer, true, false);
    let printed = printer.prints().join("");
    let printed = printed.replace("{#INFO#}", "").replace("{#DEFAULT#}", "");
    assert_eq!(
        printed,
        "//:charclass_test                                                        PASSED in 0.1s\n"
    );
}
