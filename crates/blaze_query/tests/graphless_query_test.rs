//! Port (pending) of `src/test/java/com/google/devtools/build/lib/query2/engine/GraphlessQueryTest.java`,
//! which runs `testutil/AbstractQueryTest` and `testutil/QueryTest` against
//! `GraphlessBlazeQueryEnvironment`. The tests need the `AbstractQueryHelper` /
//! `SkyframeQueryHelper` harness (a scratch workspace loaded through Skyframe), which waits for
//! blaze_sky_loading's target pattern evaluation; each is ignored until then.

// From testutil/AbstractQueryTest.java (AbstractQueryTest).
// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testTargetLiteralWithMissingTargets
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_target_literal_with_missing_targets() {
    blaze_util::unported!("AbstractQueryTest.testTargetLiteralWithMissingTargets");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testTargetLiterals
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_target_literals() {
    blaze_util::unported!("AbstractQueryTest.testTargetLiterals");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testBadTargetLiterals
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_bad_target_literals() {
    blaze_util::unported!("AbstractQueryTest.testBadTargetLiterals");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testAlgebraicSetOperations
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_algebraic_set_operations() {
    blaze_util::unported!("AbstractQueryTest.testAlgebraicSetOperations");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testAlgebraicSetOperations_manyOperands
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_algebraic_set_operations_many_operands() {
    blaze_util::unported!("AbstractQueryTest.testAlgebraicSetOperations_manyOperands");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testKindOperator
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_kind_operator() {
    blaze_util::unported!("AbstractQueryTest.testKindOperator");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testFilterOperator
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_filter_operator() {
    blaze_util::unported!("AbstractQueryTest.testFilterOperator");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testAttrOperatorOnName
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_attr_operator_on_name() {
    blaze_util::unported!("AbstractQueryTest.testAttrOperatorOnName");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testAttrOperator
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_attr_operator() {
    blaze_util::unported!("AbstractQueryTest.testAttrOperator");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testNullAttrOperator
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_null_attr_operator() {
    blaze_util::unported!("AbstractQueryTest.testNullAttrOperator");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testAttrOperatorOnBooleans
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_attr_operator_on_booleans() {
    blaze_util::unported!("AbstractQueryTest.testAttrOperatorOnBooleans");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testGenqueryScope
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_genquery_scope() {
    blaze_util::unported!("AbstractQueryTest.testGenqueryScope");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testAttrOnPackageDefaultVisibility
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_attr_on_package_default_visibility() {
    blaze_util::unported!("AbstractQueryTest.testAttrOnPackageDefaultVisibility");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testSomeOperator_noCountParameter
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_some_operator_no_count_parameter() {
    blaze_util::unported!("AbstractQueryTest.testSomeOperator_noCountParameter");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testSomeOperator_countParameterNotEqualActualCount
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_some_operator_count_parameter_not_equal_actual_count() {
    blaze_util::unported!("AbstractQueryTest.testSomeOperator_countParameterNotEqualActualCount");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testSomeOperator_nestedSomeTest
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_some_operator_nested_some_test() {
    blaze_util::unported!("AbstractQueryTest.testSomeOperator_nestedSomeTest");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testSomePathOperator
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_some_path_operator() {
    blaze_util::unported!("AbstractQueryTest.testSomePathOperator");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testSomePathOperatorOrdering
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_some_path_operator_ordering() {
    blaze_util::unported!("AbstractQueryTest.testSomePathOperatorOrdering");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testAllPathsOperator
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_all_paths_operator() {
    blaze_util::unported!("AbstractQueryTest.testAllPathsOperator");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testPathOperatorsWithOutputFile
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_path_operators_with_output_file() {
    blaze_util::unported!("AbstractQueryTest.testPathOperatorsWithOutputFile");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testDeps
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_deps() {
    blaze_util::unported!("AbstractQueryTest.testDeps");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testDepsDoesNotIncludeBuildFiles
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_deps_does_not_include_build_files() {
    blaze_util::unported!("AbstractQueryTest.testDepsDoesNotIncludeBuildFiles");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testAspectOnRuleWithoutDeclaredProviders
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_aspect_on_rule_without_declared_providers() {
    blaze_util::unported!("AbstractQueryTest.testAspectOnRuleWithoutDeclaredProviders");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testQueryStarlarkAspects
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_query_starlark_aspects() {
    blaze_util::unported!("AbstractQueryTest.testQueryStarlarkAspects");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testQueryStarlarkAspectWithParameters
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_query_starlark_aspect_with_parameters() {
    blaze_util::unported!("AbstractQueryTest.testQueryStarlarkAspectWithParameters");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testQueryStarlarkAspectsNoImplicitDeps
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_query_starlark_aspects_no_implicit_deps() {
    blaze_util::unported!("AbstractQueryTest.testQueryStarlarkAspectsNoImplicitDeps");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testStarlarkDiamondEquality
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_starlark_diamond_equality() {
    blaze_util::unported!("AbstractQueryTest.testStarlarkDiamondEquality");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testRdeps
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_rdeps() {
    blaze_util::unported!("AbstractQueryTest.testRdeps");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testLet
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_let() {
    blaze_util::unported!("AbstractQueryTest.testLet");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testScopeOfLetExpressions
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_scope_of_let_expressions() {
    blaze_util::unported!("AbstractQueryTest.testScopeOfLetExpressions");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testSubdirSymlinkCycle
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_subdir_symlink_cycle() {
    blaze_util::unported!("AbstractQueryTest.testSubdirSymlinkCycle");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testCycleInSubpackage
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_cycle_in_subpackage() {
    blaze_util::unported!("AbstractQueryTest.testCycleInSubpackage");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testCycleInStarlarkParentDir
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_cycle_in_starlark_parent_dir() {
    blaze_util::unported!("AbstractQueryTest.testCycleInStarlarkParentDir");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testNestedLetExpressions
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_nested_let_expressions() {
    blaze_util::unported!("AbstractQueryTest.testNestedLetExpressions");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testBuildFiles
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_build_files() {
    blaze_util::unported!("AbstractQueryTest.testBuildFiles");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testBuildFilesDoesNotReturnVisibilityOfRule
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_build_files_does_not_return_visibility_of_rule() {
    blaze_util::unported!("AbstractQueryTest.testBuildFilesDoesNotReturnVisibilityOfRule");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testBuildFilesDoesNotReturnVisibilityOfBUILD
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_build_files_does_not_return_visibility_of_build() {
    blaze_util::unported!("AbstractQueryTest.testBuildFilesDoesNotReturnVisibilityOfBUILD");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testNoImplicitDeps
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_no_implicit_deps() {
    blaze_util::unported!("AbstractQueryTest.testNoImplicitDeps");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testNodepDeps_defaultIsTrue
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_nodep_deps_default_is_true() {
    blaze_util::unported!("AbstractQueryTest.testNodepDeps_defaultIsTrue");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testNodepDeps_false
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_nodep_deps_false() {
    blaze_util::unported!("AbstractQueryTest.testNodepDeps_false");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testCycleInStarlark
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_cycle_in_starlark() {
    blaze_util::unported!("AbstractQueryTest.testCycleInStarlark");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testLabelsOperator
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_labels_operator() {
    blaze_util::unported!("AbstractQueryTest.testLabelsOperator");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testTestsOperatorExpandsTestsAndExcludesNonTests
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_tests_operator_expands_tests_and_excludes_non_tests() {
    blaze_util::unported!("AbstractQueryTest.testTestsOperatorExpandsTestsAndExcludesNonTests");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testTestsOperatorFiltersByTagSizeAndEnv
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_tests_operator_filters_by_tag_size_and_env() {
    blaze_util::unported!("AbstractQueryTest.testTestsOperatorFiltersByTagSizeAndEnv");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testTestsOperatorFiltersByNegativeTag
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_tests_operator_filters_by_negative_tag() {
    blaze_util::unported!("AbstractQueryTest.testTestsOperatorFiltersByNegativeTag");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testTestsOperatorCrossesPackages
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_tests_operator_crosses_packages() {
    blaze_util::unported!("AbstractQueryTest.testTestsOperatorCrossesPackages");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testTestsOperatorHandlesCyclesGracefully
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_tests_operator_handles_cycles_gracefully() {
    blaze_util::unported!("AbstractQueryTest.testTestsOperatorHandlesCyclesGracefully");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testTestSuiteInTestsAttributeAndViceVersa
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_test_suite_in_tests_attribute_and_vice_versa() {
    blaze_util::unported!("AbstractQueryTest.testTestSuiteInTestsAttributeAndViceVersa");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testTestsOperatorReportsMissingTargets
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_tests_operator_reports_missing_targets() {
    blaze_util::unported!("AbstractQueryTest.testTestsOperatorReportsMissingTargets");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testDotDotDotWithUnrelatedCycle
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_dot_dot_dot_with_unrelated_cycle() {
    blaze_util::unported!("AbstractQueryTest.testDotDotDotWithUnrelatedCycle");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testDotDotDotWithCycle
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_dot_dot_dot_with_cycle() {
    blaze_util::unported!("AbstractQueryTest.testDotDotDotWithCycle");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testExecutablesQuery
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_executables_query() {
    blaze_util::unported!("AbstractQueryTest.testExecutablesQuery");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testSet
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_set() {
    blaze_util::unported!("AbstractQueryTest.testSet");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testRuleOutputAmbiguityIsntFatal
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_rule_output_ambiguity_isnt_fatal() {
    blaze_util::unported!("AbstractQueryTest.testRuleOutputAmbiguityIsntFatal");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testDefaultVisibilityReturnedInDeps
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_default_visibility_returned_in_deps() {
    blaze_util::unported!("AbstractQueryTest.testDefaultVisibilityReturnedInDeps");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testDefaultVisibilityReturnedInDeps_nonEmptyDependencyFilter
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_default_visibility_returned_in_deps_non_empty_dependency_filter() {
    blaze_util::unported!(
        "AbstractQueryTest.testDefaultVisibilityReturnedInDeps_nonEmptyDependencyFilter"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testDefaultVisibilityReturnedInDepsForInputFiles
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_default_visibility_returned_in_deps_for_input_files() {
    blaze_util::unported!("AbstractQueryTest.testDefaultVisibilityReturnedInDepsForInputFiles");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testIncludesReturnedInDeps
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_includes_returned_in_deps() {
    blaze_util::unported!("AbstractQueryTest.testIncludesReturnedInDeps");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testRegression1267510
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_regression1267510() {
    blaze_util::unported!("AbstractQueryTest.testRegression1267510");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testRegression1309697
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_regression1309697() {
    blaze_util::unported!("AbstractQueryTest.testRegression1309697");
}

// AbstractQueryTest.testGraphOrderOfWildcards is overridden by GraphlessQueryTest (below).

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testWildcardsDontLoadUnnecessaryPackages
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_wildcards_dont_load_unnecessary_packages() {
    blaze_util::unported!("AbstractQueryTest.testWildcardsDontLoadUnnecessaryPackages");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testRegression1352570
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_regression1352570() {
    blaze_util::unported!("AbstractQueryTest.testRegression1352570");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testTestSuiteWithFile
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_test_suite_with_file() {
    blaze_util::unported!("AbstractQueryTest.testTestSuiteWithFile");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testStrictTestSuiteWithFile
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_strict_test_suite_with_file() {
    blaze_util::unported!("AbstractQueryTest.testStrictTestSuiteWithFile");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testAmbiguousAllResolvesToTestSuiteNamedAll
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_ambiguous_all_resolves_to_test_suite_named_all() {
    blaze_util::unported!("AbstractQueryTest.testAmbiguousAllResolvesToTestSuiteNamedAll");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testBigExpression
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_big_expression() {
    blaze_util::unported!("AbstractQueryTest.testBigExpression");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testSlashSlashDotDotDot
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_slash_slash_dot_dot_dot() {
    blaze_util::unported!("AbstractQueryTest.testSlashSlashDotDotDot");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testQueryTimeLoadingOfTargetPatternHappyPath
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_query_time_loading_of_target_pattern_happy_path() {
    blaze_util::unported!("AbstractQueryTest.testQueryTimeLoadingOfTargetPatternHappyPath");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testQueryTimeLoadingOfTargetsBelowPackageHappyPath
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_query_time_loading_of_targets_below_package_happy_path() {
    blaze_util::unported!("AbstractQueryTest.testQueryTimeLoadingOfTargetsBelowPackageHappyPath");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testQueryTimeLoadingTargetsBelowMissingPackage
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_query_time_loading_targets_below_missing_package() {
    blaze_util::unported!("AbstractQueryTest.testQueryTimeLoadingTargetsBelowMissingPackage");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testQueryTimeLoadingTargetsBelowNonPackageDirectory
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_query_time_loading_targets_below_non_package_directory() {
    blaze_util::unported!("AbstractQueryTest.testQueryTimeLoadingTargetsBelowNonPackageDirectory");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testHaveDepsOnAspectsAttributes
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_have_deps_on_aspects_attributes() {
    blaze_util::unported!("AbstractQueryTest.testHaveDepsOnAspectsAttributes");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testNoDepsOnAspectAttributeWhenAspectMissing
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_no_deps_on_aspect_attribute_when_aspect_missing() {
    blaze_util::unported!("AbstractQueryTest.testNoDepsOnAspectAttributeWhenAspectMissing");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testNoDepsOnAspectAttributeWithNoImpicitDeps
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_no_deps_on_aspect_attribute_with_no_impicit_deps() {
    blaze_util::unported!("AbstractQueryTest.testNoDepsOnAspectAttributeWithNoImpicitDeps");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testVisible_simple_public
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_visible_simple_public() {
    blaze_util::unported!("AbstractQueryTest.testVisible_simple_public");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testVisible_simple_private
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_visible_simple_private() {
    blaze_util::unported!("AbstractQueryTest.testVisible_simple_private");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testVisible_simple_package
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_visible_simple_package() {
    blaze_util::unported!("AbstractQueryTest.testVisible_simple_package");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testVisible_simple_subpackages
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_visible_simple_subpackages() {
    blaze_util::unported!("AbstractQueryTest.testVisible_simple_subpackages");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testVisible_simple_different_subpackages
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_visible_simple_different_subpackages() {
    blaze_util::unported!("AbstractQueryTest.testVisible_simple_different_subpackages");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testVisible_private_same_package
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_visible_private_same_package() {
    blaze_util::unported!("AbstractQueryTest.testVisible_private_same_package");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testVisible_package_group
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_visible_package_group() {
    blaze_util::unported!("AbstractQueryTest.testVisible_package_group");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testVisible_package_group_invisible
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_visible_package_group_invisible() {
    blaze_util::unported!("AbstractQueryTest.testVisible_package_group_invisible");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testVisible_package_group_include
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_visible_package_group_include() {
    blaze_util::unported!("AbstractQueryTest.testVisible_package_group_include");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testVisible_java_javatests
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_visible_java_javatests() {
    blaze_util::unported!("AbstractQueryTest.testVisible_java_javatests");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testVisible_java_javatests_different_package
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_visible_java_javatests_different_package() {
    blaze_util::unported!("AbstractQueryTest.testVisible_java_javatests_different_package");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testVisible_javatests_java
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_visible_javatests_java() {
    blaze_util::unported!("AbstractQueryTest.testVisible_javatests_java");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testVisible_default_private
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_visible_default_private() {
    blaze_util::unported!("AbstractQueryTest.testVisible_default_private");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testVisible_default_public
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_visible_default_public() {
    blaze_util::unported!("AbstractQueryTest.testVisible_default_public");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testPackageGroupAllBeneath
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_package_group_all_beneath() {
    blaze_util::unported!("AbstractQueryTest.testPackageGroupAllBeneath");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testBuildfilesWithDuplicates
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_buildfiles_with_duplicates() {
    blaze_util::unported!("AbstractQueryTest.testBuildfilesWithDuplicates");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.bzlPackageBadDueToBrokenLoad
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn bzl_package_bad_due_to_broken_load() {
    blaze_util::unported!("AbstractQueryTest.bzlPackageBadDueToBrokenLoad");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.bzlPackageBadDueToBrokenSyntax
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn bzl_package_bad_due_to_broken_syntax() {
    blaze_util::unported!("AbstractQueryTest.bzlPackageBadDueToBrokenSyntax");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testBuildfilesContainingScl
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_buildfiles_containing_scl() {
    blaze_util::unported!("AbstractQueryTest.testBuildfilesContainingScl");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.badRuleInDeps
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn bad_rule_in_deps() {
    blaze_util::unported!("AbstractQueryTest.badRuleInDeps");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.buildfilesBazel
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn buildfiles_bazel() {
    blaze_util::unported!("AbstractQueryTest.buildfilesBazel");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testTargetsFromBuildfilesAndRealTargets
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_targets_from_buildfiles_and_real_targets() {
    blaze_util::unported!("AbstractQueryTest.testTargetsFromBuildfilesAndRealTargets");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testBuildfilesOfBuildfiles
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_buildfiles_of_buildfiles() {
    blaze_util::unported!("AbstractQueryTest.testBuildfilesOfBuildfiles");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testBoundedDepsStreaming
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_bounded_deps_streaming() {
    blaze_util::unported!("AbstractQueryTest.testBoundedDepsStreaming");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testBoundedRdepsStreaming
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_bounded_rdeps_streaming() {
    blaze_util::unported!("AbstractQueryTest.testBoundedRdepsStreaming");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.boundedDepsWithError
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn bounded_deps_with_error() {
    blaze_util::unported!("AbstractQueryTest.boundedDepsWithError");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.boundedRdepsWithError
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn bounded_rdeps_with_error() {
    blaze_util::unported!("AbstractQueryTest.boundedRdepsWithError");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testEqualityOfOrderedThreadSafeImmutableSet
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_equality_of_ordered_thread_safe_immutable_set() {
    blaze_util::unported!("AbstractQueryTest.testEqualityOfOrderedThreadSafeImmutableSet");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testSiblings_simple
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_siblings_simple() {
    blaze_util::unported!("AbstractQueryTest.testSiblings_simple");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testSiblings_duplicatePackages
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_siblings_duplicate_packages() {
    blaze_util::unported!("AbstractQueryTest.testSiblings_duplicatePackages");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testSiblings_samePackageRdeps
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_siblings_same_package_rdeps() {
    blaze_util::unported!("AbstractQueryTest.testSiblings_samePackageRdeps");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testSiblings_matchesTargetNamedAll
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_siblings_matches_target_named_all() {
    blaze_util::unported!("AbstractQueryTest.testSiblings_matchesTargetNamedAll");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testSiblings_withBuildfiles
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_siblings_with_buildfiles() {
    blaze_util::unported!("AbstractQueryTest.testSiblings_withBuildfiles");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testSamePackageRdeps_simple
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_same_package_rdeps_simple() {
    blaze_util::unported!("AbstractQueryTest.testSamePackageRdeps_simple");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testSamePackageRdeps_duplicate
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_same_package_rdeps_duplicate() {
    blaze_util::unported!("AbstractQueryTest.testSamePackageRdeps_duplicate");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testSamePackageRdeps_two
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_same_package_rdeps_two() {
    blaze_util::unported!("AbstractQueryTest.testSamePackageRdeps_two");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testSamePackageRdeps_twoPackages
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_same_package_rdeps_two_packages() {
    blaze_util::unported!("AbstractQueryTest.testSamePackageRdeps_twoPackages");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testSamePackageRdeps_crissCross
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_same_package_rdeps_criss_cross() {
    blaze_util::unported!("AbstractQueryTest.testSamePackageRdeps_crissCross");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testVisibleWithNonPackageGroupVisibility
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_visible_with_non_package_group_visibility() {
    blaze_util::unported!("AbstractQueryTest.testVisibleWithNonPackageGroupVisibility");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testVisibleWithPackageGroupWithNonPackageGroupIncludes
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_visible_with_package_group_with_non_package_group_includes() {
    blaze_util::unported!(
        "AbstractQueryTest.testVisibleWithPackageGroupWithNonPackageGroupIncludes"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testDeepNestedLet
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_deep_nested_let() {
    blaze_util::unported!("AbstractQueryTest.testDeepNestedLet");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testUnsuccessfulInnerFutureInNestedLetTransformAsyncFastPath
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_unsuccessful_inner_future_in_nested_let_transform_async_fast_path() {
    blaze_util::unported!(
        "AbstractQueryTest.testUnsuccessfulInnerFutureInNestedLetTransformAsyncFastPath"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testUnconditionalQueryException
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_unconditional_query_exception() {
    blaze_util::unported!("AbstractQueryTest.testUnconditionalQueryException");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testNoImplicitDeps_computedDefault
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_no_implicit_deps_computed_default() {
    blaze_util::unported!("AbstractQueryTest.testNoImplicitDeps_computedDefault");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testExternalRepo_allTargetsInPackage
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_external_repo_all_targets_in_package() {
    blaze_util::unported!("AbstractQueryTest.testExternalRepo_allTargetsInPackage");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testExternalRepo_allTargetsBelow
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_external_repo_all_targets_below() {
    blaze_util::unported!("AbstractQueryTest.testExternalRepo_allTargetsBelow");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testLabelFlagDefaultAppearsInDepsQuery
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_label_flag_default_appears_in_deps_query() {
    blaze_util::unported!("AbstractQueryTest.testLabelFlagDefaultAppearsInDepsQuery");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testLabelSettingDefaultAppearsInDepsQuery
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_label_setting_default_appears_in_deps_query() {
    blaze_util::unported!("AbstractQueryTest.testLabelSettingDefaultAppearsInDepsQuery");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testStarlarkRuleToolchainDeps
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_starlark_rule_toolchain_deps() {
    blaze_util::unported!("AbstractQueryTest.testStarlarkRuleToolchainDeps");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java AbstractQueryTest.testNativeRuleToolchainDeps
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_native_rule_toolchain_deps() {
    blaze_util::unported!("AbstractQueryTest.testNativeRuleToolchainDeps");
}

// From testutil/QueryTest.java (QueryTest).
// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java QueryTest.testFindsAllTargets_nativeRuleMacro
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_finds_all_targets_native_rule_macro() {
    blaze_util::unported!("QueryTest.testFindsAllTargets_nativeRuleMacro");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java QueryTest.testFindsAllTargets_starlarkRuleMacro
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_finds_all_targets_starlark_rule_macro() {
    blaze_util::unported!("QueryTest.testFindsAllTargets_starlarkRuleMacro");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java QueryTest.testFindsAllTargets_symbolicMacro
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_finds_all_targets_symbolic_macro() {
    blaze_util::unported!("QueryTest.testFindsAllTargets_symbolicMacro");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java QueryTest.testFindsAllTargets_symbolicMacro_withLazyMacroExpansion
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_finds_all_targets_symbolic_macro_with_lazy_macro_expansion() {
    blaze_util::unported!("QueryTest.testFindsAllTargets_symbolicMacro_withLazyMacroExpansion");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java QueryTest.testBuildfiles_starlarkDep
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_buildfiles_starlark_dep() {
    blaze_util::unported!("QueryTest.testBuildfiles_starlarkDep");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java QueryTest.testBuildfiles_starlarkDep_withLazyMacroExpansion
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_buildfiles_starlark_dep_with_lazy_macro_expansion() {
    blaze_util::unported!("QueryTest.testBuildfiles_starlarkDep_withLazyMacroExpansion");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java QueryTest.testLoadfiles_starlarkDep
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_loadfiles_starlark_dep() {
    blaze_util::unported!("QueryTest.testLoadfiles_starlarkDep");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java QueryTest.testLoadfiles_sclDep
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_loadfiles_scl_dep() {
    blaze_util::unported!("QueryTest.testLoadfiles_sclDep");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java QueryTest.testLoadfiles_sclDep_withLazyMacroExpansion
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_loadfiles_scl_dep_with_lazy_macro_expansion() {
    blaze_util::unported!("QueryTest.testLoadfiles_sclDep_withLazyMacroExpansion");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java QueryTest.testDeps_labelKeyedStringDictDeps
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_deps_label_keyed_string_dict_deps() {
    blaze_util::unported!("QueryTest.testDeps_labelKeyedStringDictDeps");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java QueryTest.testBuildfiles_transitiveStarlarkDeps
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_buildfiles_transitive_starlark_deps() {
    blaze_util::unported!("QueryTest.testBuildfiles_transitiveStarlarkDeps");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java QueryTest.testBuildfiles_diamondStarlarkDeps
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_buildfiles_diamond_starlark_deps() {
    blaze_util::unported!("QueryTest.testBuildfiles_diamondStarlarkDeps");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java QueryTest.testBuildfiles_starlarkDepPackageBuildfileIncluded
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_buildfiles_starlark_dep_package_buildfile_included() {
    blaze_util::unported!("QueryTest.testBuildfiles_starlarkDepPackageBuildfileIncluded");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java QueryTest.testQueryTimeLoadingWhenPackageDoesNotExist
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_query_time_loading_when_package_does_not_exist() {
    blaze_util::unported!("QueryTest.testQueryTimeLoadingWhenPackageDoesNotExist");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java QueryTest.testQueryTimeLoadingWhenPackageIsMalformed
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_query_time_loading_when_package_is_malformed() {
    blaze_util::unported!("QueryTest.testQueryTimeLoadingWhenPackageIsMalformed");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java QueryTest.testQueryTimeLoadingOfSymlinkCyclePackage
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_query_time_loading_of_symlink_cycle_package() {
    blaze_util::unported!("QueryTest.testQueryTimeLoadingOfSymlinkCyclePackage");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java QueryTest.boundedDepsQueryWithError
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn bounded_deps_query_with_error() {
    blaze_util::unported!("QueryTest.boundedDepsQueryWithError");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java QueryTest.testIgnoredSubdirectories
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_ignored_subdirectories() {
    blaze_util::unported!("QueryTest.testIgnoredSubdirectories");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java QueryTest.testQueryStarlarkComputedDefault
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_query_starlark_computed_default() {
    blaze_util::unported!("QueryTest.testQueryStarlarkComputedDefault");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java QueryTest.testQueryStarlarkComputedDefaultWithConfigurableDeps
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_query_starlark_computed_default_with_configurable_deps() {
    blaze_util::unported!("QueryTest.testQueryStarlarkComputedDefaultWithConfigurableDeps");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java QueryTest.testQueryStarlarkComputedDefaultWithConfigurableDepsUsedTwice
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_query_starlark_computed_default_with_configurable_deps_used_twice() {
    blaze_util::unported!(
        "QueryTest.testQueryStarlarkComputedDefaultWithConfigurableDepsUsedTwice"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java QueryTest.testFileTargetLiteralInSubdirectory
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_file_target_literal_in_subdirectory() {
    blaze_util::unported!("QueryTest.testFileTargetLiteralInSubdirectory");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java QueryTest.testShorthandTargetLiteralUnion
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_shorthand_target_literal_union() {
    blaze_util::unported!("QueryTest.testShorthandTargetLiteralUnion");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java QueryTest.testShorthandAbsoluteTargetLiteralUnion
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_shorthand_absolute_target_literal_union() {
    blaze_util::unported!("QueryTest.testShorthandAbsoluteTargetLiteralUnion");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java QueryTest.testLoadfilesWithDuplicates
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_loadfiles_with_duplicates() {
    blaze_util::unported!("QueryTest.testLoadfilesWithDuplicates");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java QueryTest.testRdepsUnboundedWithNonDefaultDependencyFilter
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_rdeps_unbounded_with_non_default_dependency_filter() {
    blaze_util::unported!("QueryTest.testRdepsUnboundedWithNonDefaultDependencyFilter");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java QueryTest.testRdepsBoundedWithNonDefaultDependencyFilter
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_rdeps_bounded_with_non_default_dependency_filter() {
    blaze_util::unported!("QueryTest.testRdepsBoundedWithNonDefaultDependencyFilter");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java QueryTest.testDefaultVisibilityOfOutputTarget_noImplicitDeps
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_default_visibility_of_output_target_no_implicit_deps() {
    blaze_util::unported!("QueryTest.testDefaultVisibilityOfOutputTarget_noImplicitDeps");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java QueryTest.testDormantDepsAreReturned
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_dormant_deps_are_returned() {
    blaze_util::unported!("QueryTest.testDormantDepsAreReturned");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java QueryTest.testMaterializerRuleQuery
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_materializer_rule_query() {
    blaze_util::unported!("QueryTest.testMaterializerRuleQuery");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java QueryTest.testMaterializerRuleRealDepsQuery
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_materializer_rule_real_deps_query() {
    blaze_util::unported!("QueryTest.testMaterializerRuleRealDepsQuery");
}

// From engine/GraphlessQueryTest.java (GraphlessQueryTest).
// Java: src/test/java/com/google/devtools/build/lib/query2/engine/GraphlessQueryTest.java GraphlessQueryTest.testGraphOrderOfWildcards
#[test]
#[ignore = "needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryHelper port)"]
fn test_graph_order_of_wildcards() {
    blaze_util::unported!("GraphlessQueryTest.testGraphOrderOfWildcards");
}
