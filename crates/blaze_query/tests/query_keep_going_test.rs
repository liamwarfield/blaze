//! Port (pending) of `src/test/java/com/google/devtools/build/lib/query2/engine/QueryKeepGoingTest.java`
//! (`testutil/AbstractQueryKeepGoingTest`). Ignored until the query test harness can load
//! packages (see `graphless_query_test.rs`).

// From testutil/AbstractQueryKeepGoingTest.java (AbstractQueryKeepGoingTest).
// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java AbstractQueryKeepGoingTest.testErrorWhenResultContainsLabelsCrossingSubpackage
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_error_when_result_contains_labels_crossing_subpackage() {
    blaze_util::unported!(
        "AbstractQueryKeepGoingTest.testErrorWhenResultContainsLabelsCrossingSubpackage"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java AbstractQueryKeepGoingTest.testErrorWhenWildcardResultContainsLabelsCrossingSubpackage
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_error_when_wildcard_result_contains_labels_crossing_subpackage() {
    blaze_util::unported!(
        "AbstractQueryKeepGoingTest.testErrorWhenWildcardResultContainsLabelsCrossingSubpackage"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java AbstractQueryKeepGoingTest.testNoFailFastOnLabelsExpression
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_no_fail_fast_on_labels_expression() {
    blaze_util::unported!("AbstractQueryKeepGoingTest.testNoFailFastOnLabelsExpression");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java AbstractQueryKeepGoingTest.testBadBuildFileKeepGoing
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_bad_build_file_keep_going() {
    blaze_util::unported!("AbstractQueryKeepGoingTest.testBadBuildFileKeepGoing");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java AbstractQueryKeepGoingTest.testStrictTestSuiteWithFileAndKeepGoing
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_strict_test_suite_with_file_and_keep_going() {
    blaze_util::unported!("AbstractQueryKeepGoingTest.testStrictTestSuiteWithFileAndKeepGoing");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java AbstractQueryKeepGoingTest.testQueryAllForBrokenPackage
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_query_all_for_broken_package() {
    blaze_util::unported!("AbstractQueryKeepGoingTest.testQueryAllForBrokenPackage");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java AbstractQueryKeepGoingTest.testQueryDotDotDotForBrokenPackage
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_query_dot_dot_dot_for_broken_package() {
    blaze_util::unported!("AbstractQueryKeepGoingTest.testQueryDotDotDotForBrokenPackage");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java AbstractQueryKeepGoingTest.testNonExistentDotDotDot
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_non_existent_dot_dot_dot() {
    blaze_util::unported!("AbstractQueryKeepGoingTest.testNonExistentDotDotDot");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToMissingPkgExistingFile_TBD
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_error_reported_when_starlark_load_refers_to_missing_pkg_existing_file_tbd() {
    blaze_util::unported!(
        "AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToMissingPkgExistingFile_TBD"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToMissingPkgExistingFile_TIP
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_error_reported_when_starlark_load_refers_to_missing_pkg_existing_file_tip() {
    blaze_util::unported!(
        "AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToMissingPkgExistingFile_TIP"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToMissingPkgExistingFile_ST
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_error_reported_when_starlark_load_refers_to_missing_pkg_existing_file_st() {
    blaze_util::unported!(
        "AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToMissingPkgExistingFile_ST"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToMissingPkgExistingFile_IPAT
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_error_reported_when_starlark_load_refers_to_missing_pkg_existing_file_ipat() {
    blaze_util::unported!(
        "AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToMissingPkgExistingFile_IPAT"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java AbstractQueryKeepGoingTest.testPluralErrorsReportedWhenStarlarkLoadRefersToMissingPkgExistingFile
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_plural_errors_reported_when_starlark_load_refers_to_missing_pkg_existing_file() {
    blaze_util::unported!(
        "AbstractQueryKeepGoingTest.testPluralErrorsReportedWhenStarlarkLoadRefersToMissingPkgExistingFile"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToExistingPkgMissingFile_TBD
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_error_reported_when_starlark_load_refers_to_existing_pkg_missing_file_tbd() {
    blaze_util::unported!(
        "AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToExistingPkgMissingFile_TBD"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToExistingPkgMissingFile_TIP
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_error_reported_when_starlark_load_refers_to_existing_pkg_missing_file_tip() {
    blaze_util::unported!(
        "AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToExistingPkgMissingFile_TIP"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToExistingPkgMissingFile_ST
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_error_reported_when_starlark_load_refers_to_existing_pkg_missing_file_st() {
    blaze_util::unported!(
        "AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToExistingPkgMissingFile_ST"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToExistingPkgMissingFile_IPAT
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_error_reported_when_starlark_load_refers_to_existing_pkg_missing_file_ipat() {
    blaze_util::unported!(
        "AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToExistingPkgMissingFile_IPAT"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToFileInSymlinkCycle_TBD
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_error_reported_when_starlark_load_refers_to_file_in_symlink_cycle_tbd() {
    blaze_util::unported!(
        "AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToFileInSymlinkCycle_TBD"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToFileInSymlinkCycle_TIP
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_error_reported_when_starlark_load_refers_to_file_in_symlink_cycle_tip() {
    blaze_util::unported!(
        "AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToFileInSymlinkCycle_TIP"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToFileInSymlinkCycle_ST
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_error_reported_when_starlark_load_refers_to_file_in_symlink_cycle_st() {
    blaze_util::unported!(
        "AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToFileInSymlinkCycle_ST"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToFileInSymlinkCycle_IPAT
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_error_reported_when_starlark_load_refers_to_file_in_symlink_cycle_ipat() {
    blaze_util::unported!(
        "AbstractQueryKeepGoingTest.testErrorReportedWhenStarlarkLoadRefersToFileInSymlinkCycle_IPAT"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java AbstractQueryKeepGoingTest.testNoErrorReportedWhenUniverseIncludesBrokenPkgButQueryDoesNot
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_no_error_reported_when_universe_includes_broken_pkg_but_query_does_not() {
    blaze_util::unported!(
        "AbstractQueryKeepGoingTest.testNoErrorReportedWhenUniverseIncludesBrokenPkgButQueryDoesNot"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java AbstractQueryKeepGoingTest.boundedRdepsWithError
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn bounded_rdeps_with_error() {
    blaze_util::unported!("AbstractQueryKeepGoingTest.boundedRdepsWithError");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java AbstractQueryKeepGoingTest.testIgnoredSubdirectoryIsTBDQuery
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_ignored_subdirectory_is_tbd_query() {
    blaze_util::unported!("AbstractQueryKeepGoingTest.testIgnoredSubdirectoryIsTBDQuery");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java AbstractQueryKeepGoingTest.bogusVisibility
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn bogus_visibility() {
    blaze_util::unported!("AbstractQueryKeepGoingTest.bogusVisibility");
}
