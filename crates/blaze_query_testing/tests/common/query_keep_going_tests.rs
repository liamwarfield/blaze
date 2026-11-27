//! The tests of Java's `AbstractQueryKeepGoingTest`.
//!
//! Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java

use super::*;

#[allow(unused_imports)]
use blaze_query_engine::Setting;

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:99 AbstractQueryKeepGoingTest.testErrorWhenResultContainsLabelsCrossingSubpackage
pub fn test_error_when_result_contains_labels_crossing_subpackage(t: &mut AbstractQueryTest) {
    t.write_file(
        "pear/BUILD",
        &[r#"load('//test_defs:foo_library.bzl', 'foo_library')
foo_library(
    name = "plum/peach",
    srcs = ["peach.sh"],
)

foo_library(
    name = "apple",
    srcs = ["apple.sh"],
)
"#],
    );
    t.write_file("pear/plum/BUILD", &[]);
    let result_and_targets = t.eval_fail("//pear:apple");
    t.assert_contains_event("is invalid because 'pear/plum' is a subpackage");
    if t.helper.reports_universe_evaluation_errors() {
        assert_eq!(
            package_loading_code(
                &result_and_targets
                    .query_eval_result
                    .get_detailed_exit_code()
                    .get_failure_detail()
                    .cloned()
                    .expect("a failure detail")
            ),
            Some(
                blaze_protos::failure_details::package_loading::Code::LabelCrossesPackageBoundary
                    as i32
            )
        );
    } else {
        assert_eq!(
            query_code(
                &result_and_targets
                    .query_eval_result
                    .get_detailed_exit_code()
                    .get_failure_detail()
                    .cloned()
                    .expect("a failure detail")
            ),
            Some(blaze_protos::failure_details::query::Code::BuildFileError as i32)
        );
    }
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:128 AbstractQueryKeepGoingTest.testErrorWhenWildcardResultContainsLabelsCrossingSubpackage
pub fn test_error_when_wildcard_result_contains_labels_crossing_subpackage(
    t: &mut AbstractQueryTest,
) {
    t.write_file(
        "pear/BUILD",
        &[r#"load('//test_defs:foo_library.bzl', 'foo_library')
foo_library(
    name = "plum/peach",
    srcs = ["peach.sh"],
)

foo_library(
    name = "apple",
    srcs = ["apple.sh"],
)
"#],
    );
    t.write_file("pear/plum/BUILD", &[]);
    let result_and_targets = t.eval_fail("//pear:all");
    t.assert_contains_event("is invalid because 'pear/plum' is a subpackage");
    if t.helper.reports_universe_evaluation_errors() {
        assert_eq!(
            package_loading_code(
                &result_and_targets
                    .query_eval_result
                    .get_detailed_exit_code()
                    .get_failure_detail()
                    .cloned()
                    .expect("a failure detail")
            ),
            Some(
                blaze_protos::failure_details::package_loading::Code::LabelCrossesPackageBoundary
                    as i32
            )
        );
    } else {
        assert_eq!(
            query_code(
                &result_and_targets
                    .query_eval_result
                    .get_detailed_exit_code()
                    .get_failure_detail()
                    .cloned()
                    .expect("a failure detail")
            ),
            Some(blaze_protos::failure_details::query::Code::BuildFileError as i32)
        );
    }
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:221 AbstractQueryKeepGoingTest.testNoFailFastOnLabelsExpression
pub fn test_no_fail_fast_on_labels_expression(t: &mut AbstractQueryTest) {
    t.write_file(
        "bad/BUILD",
        &["genrule(name='bad', srcs=['x', '//missing', 'y'], outs=['out'], cmd=':')"],
    );
    let result = labels_of(&t.eval_fail("labels(srcs, //bad)"));
    t.assert_contains_event(
        (format!(
            "{}{}",
            "no such package 'missing': ", "BUILD file not found"
        ))
        .as_str(),
    );
    t.assert_contains_event("--keep_going specified, ignoring errors. Results may be inaccurate");
    assert_eq!(result, t.eval("//bad:x + //bad:y"));
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:234 AbstractQueryKeepGoingTest.testBadBuildFileKeepGoing
pub fn test_bad_build_file_keep_going(t: &mut AbstractQueryTest) {
    t.write_file("bad/BUILD", &["blah blah blah"]);
    let result = t.eval_fail("bad:*");
    if t.helper.reports_universe_evaluation_errors() {
        assert_eq!(
            package_loading_code(
                &result
                    .query_eval_result
                    .get_detailed_exit_code()
                    .get_failure_detail()
                    .cloned()
                    .expect("a failure detail")
            ),
            Some(blaze_protos::failure_details::package_loading::Code::SyntaxError as i32)
        );
    } else {
        assert_eq!(
            query_code(
                &result
                    .query_eval_result
                    .get_detailed_exit_code()
                    .get_failure_detail()
                    .cloned()
                    .expect("a failure detail")
            ),
            Some(blaze_protos::failure_details::query::Code::BuildFileError as i32)
        );
    }
    t.assert_contains_event("syntax error at 'blah'");
    t.assert_contains_event("--keep_going specified, ignoring errors. Results may be inaccurate");
    assert_eq!(labels_of(&result), t.eval_maybe("//bad:BUILD"));
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:251 AbstractQueryKeepGoingTest.testStrictTestSuiteWithFileAndKeepGoing
pub fn test_strict_test_suite_with_file_and_keep_going(t: &mut AbstractQueryTest) {
    t.helper
        .set_query_settings(&[Setting::TestsExpressionStrict]);
    t.write_file("x/BUILD", &["test_suite(name='a', tests=['a.txt'])"]);
    assert!(
        labels_of(&t.eval_fail("tests(//x:a)")).is_empty(),
        "{:?}",
        labels_of(&t.eval_fail("tests(//x:a)"))
    );
    t.assert_contains_event(
        (format!(
            "{}{}",
            "The label '//x:a.txt' in the test_suite '//x:a' does not refer to a test ",
            "or test_suite rule!"
        ))
        .as_str(),
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:295 AbstractQueryKeepGoingTest.testNonExistentDotDotDot
pub fn test_non_existent_dot_dot_dot(t: &mut AbstractQueryTest) {
    assert!(
        labels_of(&t.eval_fail("//does_not_exist/...")).is_empty(),
        "{:?}",
        labels_of(&t.eval_fail("//does_not_exist/..."))
    );
    t.assert_contains_event("no targets found beneath 'does_not_exist'");
    t.assert_contains_event("Results may be inaccurate");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:302 AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToMissingPkgExistingFile_TBD
pub fn test_error_reported_when_starlark_load_refers_to_missing_pkg_existing_file_tbd(
    t: &mut AbstractQueryTest,
) {
    t.run_test_error_reported_when_starlark_load_refers_to_missing_pkg_existing_file(
        "//foo/...",
        1,
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:308 AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToMissingPkgExistingFile_TIP
pub fn test_error_reported_when_starlark_load_refers_to_missing_pkg_existing_file_tip(
    t: &mut AbstractQueryTest,
) {
    t.run_test_error_reported_when_starlark_load_refers_to_missing_pkg_existing_file(
        "//foo/foo:all",
        0,
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:314 AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToMissingPkgExistingFile_ST
pub fn test_error_reported_when_starlark_load_refers_to_missing_pkg_existing_file_st(
    t: &mut AbstractQueryTest,
) {
    t.run_test_error_reported_when_starlark_load_refers_to_missing_pkg_existing_file(
        "//foo/foo:banana",
        0,
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:320 AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToMissingPkgExistingFile_IPAT
pub fn test_error_reported_when_starlark_load_refers_to_missing_pkg_existing_file_ipat(
    t: &mut AbstractQueryTest,
) {
    t.run_test_error_reported_when_starlark_load_refers_to_missing_pkg_existing_file(
        "foo/foo/banana",
        0,
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:369 AbstractQueryKeepGoingTest.testPluralErrorsReportedWhenStarlarkLoadRefersToMissingPkgExistingFile
pub fn test_plural_errors_reported_when_starlark_load_refers_to_missing_pkg_existing_file(
    t: &mut AbstractQueryTest,
) {
    t.write_file(
        "foo/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name='apple', srcs=['apple.sh'])",
        ],
    );
    t.write_file(
        "foo/foo/BUILD",
        &[r#"load("//bar:lib.bzl", "myfunc")
load('//test_defs:foo_library.bzl', 'foo_library')
foo_library(
    name = "banana",
    srcs = ["banana.sh"],
)
"#],
    );
    t.write_file(
        "foo/foo2/BUILD",
        &[r#"load("//bar:lib.bzl", "myfunc")
load('//test_defs:foo_library.bzl', 'foo_library')
foo_library(
    name = "banana",
    srcs = ["banana.sh"],
)
"#],
    );
    t.write_file("bar/lib.bzl", &["custom_rule(name = 'myfunc')"]);
    assert!(
        labels_of(&t.eval_fail("//foo/foo:*")).is_empty(),
        "{:?}",
        labels_of(&t.eval_fail("//foo/foo:*"))
    );
    let expected_error = format!(
        "{}{}",
        "error loading package 'foo/foo': Every .bzl file must have a corresponding package, ",
        "but '//bar:lib.bzl' does not have one"
    );
    t.assert_contains_event(&expected_error);
    t.helper.clear_events();
    assert!(
        labels_of(&t.eval_fail("//foo/foo2:*")).is_empty(),
        "{:?}",
        labels_of(&t.eval_fail("//foo/foo2:*"))
    );
    let expected_error2 = format!(
        "{}{}",
        "error loading package 'foo/foo2': Every .bzl file must have a corresponding package, ",
        "but '//bar:lib.bzl' does not have one"
    );
    t.assert_contains_event((expected_error2).as_str());
    t.helper.clear_events();
    assert!(
        labels_of(&t.eval_fail("//foo/foo:* + //foo/foo2:*")).is_empty(),
        "{:?}",
        labels_of(&t.eval_fail("//foo/foo:* + //foo/foo2:*"))
    );
    t.assert_contains_event(&expected_error);
    t.assert_contains_event((expected_error2).as_str());
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:430 AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToExistingPkgMissingFile_TBD
pub fn test_error_reported_when_starlark_load_refers_to_existing_pkg_missing_file_tbd(
    t: &mut AbstractQueryTest,
) {
    t.run_test_error_reported_when_starlark_load_refers_to_existing_pkg_missing_file(
        "//foo/...",
        1,
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:436 AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToExistingPkgMissingFile_TIP
pub fn test_error_reported_when_starlark_load_refers_to_existing_pkg_missing_file_tip(
    t: &mut AbstractQueryTest,
) {
    t.run_test_error_reported_when_starlark_load_refers_to_existing_pkg_missing_file(
        "//foo/foo:all",
        0,
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:442 AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToExistingPkgMissingFile_ST
pub fn test_error_reported_when_starlark_load_refers_to_existing_pkg_missing_file_st(
    t: &mut AbstractQueryTest,
) {
    t.run_test_error_reported_when_starlark_load_refers_to_existing_pkg_missing_file(
        "//foo/foo:banana",
        0,
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:448 AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToExistingPkgMissingFile_IPAT
pub fn test_error_reported_when_starlark_load_refers_to_existing_pkg_missing_file_ipat(
    t: &mut AbstractQueryTest,
) {
    t.run_test_error_reported_when_starlark_load_refers_to_existing_pkg_missing_file(
        "foo/foo/banana",
        0,
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:496 AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToFileInSymlinkCycle_TBD
pub fn test_error_reported_when_starlark_load_refers_to_file_in_symlink_cycle_tbd(
    t: &mut AbstractQueryTest,
) {
    t.run_test_error_reported_when_starlark_load_refers_to_file_in_symlink_cycle("//foo/...", 1);
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:501 AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToFileInSymlinkCycle_TIP
pub fn test_error_reported_when_starlark_load_refers_to_file_in_symlink_cycle_tip(
    t: &mut AbstractQueryTest,
) {
    t.run_test_error_reported_when_starlark_load_refers_to_file_in_symlink_cycle(
        "//foo/foo:all",
        0,
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:506 AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToFileInSymlinkCycle_ST
pub fn test_error_reported_when_starlark_load_refers_to_file_in_symlink_cycle_st(
    t: &mut AbstractQueryTest,
) {
    t.run_test_error_reported_when_starlark_load_refers_to_file_in_symlink_cycle(
        "//foo/foo:banana",
        0,
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:511 AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToFileInSymlinkCycle_IPAT
pub fn test_error_reported_when_starlark_load_refers_to_file_in_symlink_cycle_ipat(
    t: &mut AbstractQueryTest,
) {
    t.run_test_error_reported_when_starlark_load_refers_to_file_in_symlink_cycle(
        "foo/foo/banana",
        0,
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:643 AbstractQueryKeepGoingTest.bogusVisibility
pub fn bogus_visibility(t: &mut AbstractQueryTest) {
    t.write_file(
        "foo/BUILD",
        &[r#"load('//test_defs:foo_library.bzl', 'foo_library')
package(default_visibility = ["//visibility:public"])

foo_library(
    name = "a",
    visibility = [
        "//bad:visibility",
        "//bar:__pkg__",
    ],
)

foo_library(name = "b")

foo_library(
    name = "c",
    visibility = ["//bad:visibility"],
)
"#],
    );
    t.write_file("bar/BUILD", &[]);
    let result_and_targets = t.evaluate_query("visible(//bar:BUILD, //foo:all)");
    assert!(!result_and_targets.query_eval_result.get_success());
    assert_eq!(
        labels_of(&result_and_targets),
        labels(&["//foo:a", "//foo:b"])
    );
    t.assert_contains_event("Invalid visibility label '//bad:visibility': no such package 'bad'");
    t.assert_contains_event("--keep_going specified, ignoring errors. Results may be inaccurate");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:261 AbstractQueryKeepGoingTest.testQueryAllForBrokenPackage
pub fn test_query_all_for_broken_package(t: &mut AbstractQueryTest) {
    t.write_file(
        "x/BUILD",
        &["filegroup(name = \"a\")

x = 1 // 0

filegroup(name = \"c\")
"], // not executed
    );
    assert_eq!(t.eval_fail("//x:all").get_result_set().len(), 1);
    t.assert_contains_event("division by zero");
    t.assert_contains_event("Results may be inaccurate");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:278 AbstractQueryKeepGoingTest.testQueryDotDotDotForBrokenPackage
pub fn test_query_dot_dot_dot_for_broken_package(t: &mut AbstractQueryTest) {
    t.write_file(
        "x/BUILD",
        &["filegroup(name = \"a\")

x = 1 // 0

filegroup(name = \"c\")
"], // not executed
    );
    assert_eq!(t.eval_fail("//x/...").get_result_set().len(), 1);
    t.assert_contains_event("division by zero");
    t.assert_contains_event("Results may be inaccurate");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:561 AbstractQueryKeepGoingTest.testNoErrorReportedWhenUniverseIncludesBrokenPkgButQueryDoesNot
pub fn test_no_error_reported_when_universe_includes_broken_pkg_but_query_does_not(
    t: &mut AbstractQueryTest,
) {
    if t.helper.reports_universe_evaluation_errors() {
        // This test case is interesting only for query environments that don't report universe
        // evaluation errors. This way we can assert that any error message came from query
        // evaluation.
        return;
    }

    // The package "//foo" is healthy.
    t.write_file(
        "foo/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name='apple', srcs=['apple.sh'])",
        ],
    );

    // The package "//baz" is not healthy: it contains a load statement referring to an unpackaged
    // Starlark file.
    t.write_file(
        "baz/BUILD",
        &["load(\"//bar:lib.bzl\", \"myfunc\")
load('//test_defs:foo_library.bzl', 'foo_library')
foo_library(
    name = \"banana\",
    srcs = [\"banana.sh\"],
)
"],
    );
    t.write_file("bar/lib.bzl", &["custom_rule(name = 'myfunc')"]);

    // Nevertheless, a query affecting just the healthy package emits no errors.
    assert_eq!(t.eval("//foo/...").len(), 1);
    t.assert_does_not_contain_event("error loading package 'baz'");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:596 AbstractQueryKeepGoingTest.boundedRdepsWithError
pub fn bounded_rdeps_with_error(t: &mut AbstractQueryTest) {
    t.write_file(
        "foo/BUILD",
        &["load('//test_defs:foo_library.bzl', 'foo_library')
foo_library(
    name = \"foo\",
    deps = [\":dep\"],
)

foo_library(
    name = \"dep\",
    deps = [\"//bar:missing\"],
)
"],
    );
    let target_result_and_targets = t.eval_fail("rdeps(//foo:foo, //foo:dep, 1)");
    assert_eq!(
        labels_of(&target_result_and_targets),
        labels(&["//foo:dep", "//foo:foo"])
    );
    // Ideally we wouldn't print this irrelevant error (since //bar:missing is a dep of //foo:dep,
    // not an rdep), or make it fail the query.
    assert_eq!(
        target_result_and_targets
            .get_query_eval_result()
            .get_detailed_exit_code()
            .get_exit_code(),
        1 // ExitCode.BUILD_FAILURE
    );
    t.assert_contains_event("no such package 'bar':");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:625 AbstractQueryKeepGoingTest.testIgnoredSubdirectoryIsTBDQuery
pub fn test_ignored_subdirectory_is_tbd_query(t: &mut AbstractQueryTest) {
    let ignored = t.helper.get_ignored_subdirectories_file();
    t.overwrite_file(ignored.get_path_string(), &["a/b"]);
    t.write_file("a/BUILD", &["filegroup(name = 'a')"]);
    t.write_file("a/b/BUILD", &["filegroup(name = 'a_b')"]);
    t.write_file("a/b/c/BUILD", &["filegroup(name = 'a_b_c')"]);

    // Ensure that modified files are invalidated in the skyframe. If a file has
    // already been read prior to the test's writes, this forces the query to
    // pick up the modified versions.
    t.helper.maybe_handle_diffs();

    let result_and_targets = t.evaluate_query("//a/b/...");
    t.assert_contains_event("Pattern '//a/b/...' was filtered out by ignored directory 'a/b'");
    assert!(result_and_targets.get_query_eval_result().get_success());
    assert!(labels_of(&result_and_targets).is_empty());
}
