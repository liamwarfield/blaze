//! Java's `GraphlessQueryTest` (src/test/java/com/google/devtools/build/lib/query2/engine/GraphlessQueryTest.java):
//! `AbstractQueryTest` on a `GraphlessBlazeQueryEnvironment`, `--nokeep_going`.

mod common;

use common::{AbstractQueryTest, Suite};

#[test]
fn bad_rule_in_deps() {
    common::abstract_query_tests::bad_rule_in_deps(&mut AbstractQueryTest::new(Suite::Graphless));
}

#[test]
fn bounded_deps_with_error() {
    common::abstract_query_tests::bounded_deps_with_error(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn buildfiles_bazel() {
    common::abstract_query_tests::buildfiles_bazel(&mut AbstractQueryTest::new(Suite::Graphless));
}

#[test]
fn bzl_package_bad_due_to_broken_load() {
    common::abstract_query_tests::bzl_package_bad_due_to_broken_load(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn bzl_package_bad_due_to_broken_syntax() {
    common::abstract_query_tests::bzl_package_bad_due_to_broken_syntax(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_algebraic_set_operations() {
    common::abstract_query_tests::test_algebraic_set_operations(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_algebraic_set_operations_many_operands() {
    common::abstract_query_tests::test_algebraic_set_operations_many_operands(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_all_paths_operator() {
    common::abstract_query_tests::test_all_paths_operator(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_ambiguous_all_resolves_to_test_suite_named_all() {
    common::abstract_query_tests::test_ambiguous_all_resolves_to_test_suite_named_all(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_aspect_on_rule_without_declared_providers() {
    common::abstract_query_tests::test_aspect_on_rule_without_declared_providers(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_attr_on_package_default_visibility() {
    common::abstract_query_tests::test_attr_on_package_default_visibility(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_attr_operator() {
    common::abstract_query_tests::test_attr_operator(&mut AbstractQueryTest::new(Suite::Graphless));
}

#[test]
fn test_attr_operator_on_booleans() {
    common::abstract_query_tests::test_attr_operator_on_booleans(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_attr_operator_on_name() {
    common::abstract_query_tests::test_attr_operator_on_name(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_bad_target_literals() {
    common::abstract_query_tests::test_bad_target_literals(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_big_expression() {
    common::abstract_query_tests::test_big_expression(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_bounded_deps_streaming() {
    common::abstract_query_tests::test_bounded_deps_streaming(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_bounded_rdeps_streaming() {
    common::abstract_query_tests::test_bounded_rdeps_streaming(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_build_files() {
    common::abstract_query_tests::test_build_files(&mut AbstractQueryTest::new(Suite::Graphless));
}

#[test]
fn test_build_files_does_not_return_visibility_of_build() {
    common::abstract_query_tests::test_build_files_does_not_return_visibility_of_build(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_build_files_does_not_return_visibility_of_rule() {
    common::abstract_query_tests::test_build_files_does_not_return_visibility_of_rule(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_buildfiles_containing_scl() {
    common::abstract_query_tests::test_buildfiles_containing_scl(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_buildfiles_of_buildfiles() {
    common::abstract_query_tests::test_buildfiles_of_buildfiles(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_buildfiles_with_duplicates() {
    common::abstract_query_tests::test_buildfiles_with_duplicates(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_cycle_in_starlark() {
    common::abstract_query_tests::test_cycle_in_starlark(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_cycle_in_starlark_parent_dir() {
    common::abstract_query_tests::test_cycle_in_starlark_parent_dir(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_cycle_in_subpackage() {
    common::abstract_query_tests::test_cycle_in_subpackage(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_deep_nested_let() {
    common::run_with_large_stack(|| {
        common::abstract_query_tests::test_deep_nested_let(&mut AbstractQueryTest::new(
            Suite::Graphless,
        ))
    });
}

#[test]
fn test_default_visibility_returned_in_deps() {
    common::abstract_query_tests::test_default_visibility_returned_in_deps(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_default_visibility_returned_in_deps_for_input_files() {
    common::abstract_query_tests::test_default_visibility_returned_in_deps_for_input_files(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_default_visibility_returned_in_deps_non_empty_dependency_filter() {
    common::abstract_query_tests::test_default_visibility_returned_in_deps_non_empty_dependency_filter(&mut AbstractQueryTest::new(Suite::Graphless));
}

#[test]
fn test_deps() {
    common::abstract_query_tests::test_deps(&mut AbstractQueryTest::new(Suite::Graphless));
}

#[test]
fn test_deps_does_not_include_build_files() {
    common::abstract_query_tests::test_deps_does_not_include_build_files(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_dot_dot_dot_with_cycle() {
    common::abstract_query_tests::test_dot_dot_dot_with_cycle(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_dot_dot_dot_with_unrelated_cycle() {
    common::abstract_query_tests::test_dot_dot_dot_with_unrelated_cycle(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_equality_of_ordered_thread_safe_immutable_set() {
    common::abstract_query_tests::test_equality_of_ordered_thread_safe_immutable_set(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_executables_query() {
    common::abstract_query_tests::test_executables_query(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_external_repo_all_targets_below() {
    common::abstract_query_tests::test_external_repo_all_targets_below(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_external_repo_all_targets_in_package() {
    common::abstract_query_tests::test_external_repo_all_targets_in_package(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_filter_operator() {
    common::abstract_query_tests::test_filter_operator(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_genquery_scope() {
    common::abstract_query_tests::test_genquery_scope(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
#[ignore = "needs the native mock rules of TestAspects (BASE_RULE, ASPECT_REQUIRING_RULE, EXTRA_ATTRIBUTE_ASPECT_REQUIRING_PROVIDER_RULE, HONEST_RULE, SIMPLE_RULE) through SkyframeQueryHelper.useRuleClassProvider; Blaze rule class providers have no native test rules"]
fn test_have_deps_on_aspects_attributes() {
    common::abstract_query_tests::test_have_deps_on_aspects_attributes(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_includes_returned_in_deps() {
    common::abstract_query_tests::test_includes_returned_in_deps(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_kind_operator() {
    common::abstract_query_tests::test_kind_operator(&mut AbstractQueryTest::new(Suite::Graphless));
}

#[test]
fn test_label_flag_default_appears_in_deps_query() {
    common::abstract_query_tests::test_label_flag_default_appears_in_deps_query(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_label_setting_default_appears_in_deps_query() {
    common::abstract_query_tests::test_label_setting_default_appears_in_deps_query(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_labels_operator() {
    common::abstract_query_tests::test_labels_operator(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_let() {
    common::abstract_query_tests::test_let(&mut AbstractQueryTest::new(Suite::Graphless));
}

#[test]
fn test_native_rule_toolchain_deps() {
    common::abstract_query_tests::test_native_rule_toolchain_deps(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_nested_let_expressions() {
    common::abstract_query_tests::test_nested_let_expressions(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
#[ignore = "needs the native mock rules of TestAspects (BASE_RULE, ASPECT_REQUIRING_RULE, EXTRA_ATTRIBUTE_ASPECT_REQUIRING_PROVIDER_RULE, HONEST_RULE, SIMPLE_RULE) through SkyframeQueryHelper.useRuleClassProvider; Blaze rule class providers have no native test rules"]
fn test_no_deps_on_aspect_attribute_when_aspect_missing() {
    common::abstract_query_tests::test_no_deps_on_aspect_attribute_when_aspect_missing(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
#[ignore = "needs the native mock rules of TestAspects (BASE_RULE, ASPECT_REQUIRING_RULE, EXTRA_ATTRIBUTE_ASPECT_REQUIRING_PROVIDER_RULE, HONEST_RULE, SIMPLE_RULE) through SkyframeQueryHelper.useRuleClassProvider; Blaze rule class providers have no native test rules"]
fn test_no_deps_on_aspect_attribute_with_no_impicit_deps() {
    common::abstract_query_tests::test_no_deps_on_aspect_attribute_with_no_impicit_deps(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_no_implicit_deps() {
    common::abstract_query_tests::test_no_implicit_deps(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_nodep_deps_default_is_true() {
    common::abstract_query_tests::test_nodep_deps_default_is_true(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_nodep_deps_false() {
    common::abstract_query_tests::test_nodep_deps_false(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_null_attr_operator() {
    common::abstract_query_tests::test_null_attr_operator(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
#[ignore = "BuildViewTestCase creates the target configuration with its executor, which needs @platforms//host; the empty workspace of useReducedSetOfRules has none (Java SkyframeQueryHelper creates no configuration); needs a loading-only mode of blaze_sky_top::testing::BuildViewTestCase"]
fn test_package_group_all_beneath() {
    common::abstract_query_tests::test_package_group_all_beneath(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_path_operators_with_output_file() {
    common::abstract_query_tests::test_path_operators_with_output_file(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_query_starlark_aspect_with_parameters() {
    common::abstract_query_tests::test_query_starlark_aspect_with_parameters(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_query_starlark_aspects() {
    common::abstract_query_tests::test_query_starlark_aspects(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_query_starlark_aspects_no_implicit_deps() {
    common::abstract_query_tests::test_query_starlark_aspects_no_implicit_deps(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_query_time_loading_of_target_pattern_happy_path() {
    common::abstract_query_tests::test_query_time_loading_of_target_pattern_happy_path(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_query_time_loading_of_targets_below_package_happy_path() {
    common::abstract_query_tests::test_query_time_loading_of_targets_below_package_happy_path(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_query_time_loading_targets_below_missing_package() {
    common::abstract_query_tests::test_query_time_loading_targets_below_missing_package(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_query_time_loading_targets_below_non_package_directory() {
    common::abstract_query_tests::test_query_time_loading_targets_below_non_package_directory(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_rdeps() {
    common::abstract_query_tests::test_rdeps(&mut AbstractQueryTest::new(Suite::Graphless));
}

#[test]
fn test_regression1267510() {
    common::abstract_query_tests::test_regression1267510(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_regression1309697() {
    common::abstract_query_tests::test_regression1309697(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_regression1352570() {
    common::abstract_query_tests::test_regression1352570(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_rule_output_ambiguity_isnt_fatal() {
    common::abstract_query_tests::test_rule_output_ambiguity_isnt_fatal(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_same_package_rdeps_criss_cross() {
    common::abstract_query_tests::test_same_package_rdeps_criss_cross(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_same_package_rdeps_duplicate() {
    common::abstract_query_tests::test_same_package_rdeps_duplicate(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_same_package_rdeps_simple() {
    common::abstract_query_tests::test_same_package_rdeps_simple(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_same_package_rdeps_two() {
    common::abstract_query_tests::test_same_package_rdeps_two(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_same_package_rdeps_two_packages() {
    common::abstract_query_tests::test_same_package_rdeps_two_packages(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_scope_of_let_expressions() {
    common::abstract_query_tests::test_scope_of_let_expressions(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_set() {
    common::abstract_query_tests::test_set(&mut AbstractQueryTest::new(Suite::Graphless));
}

#[test]
fn test_siblings_duplicate_packages() {
    common::abstract_query_tests::test_siblings_duplicate_packages(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_siblings_matches_target_named_all() {
    common::abstract_query_tests::test_siblings_matches_target_named_all(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_siblings_same_package_rdeps() {
    common::abstract_query_tests::test_siblings_same_package_rdeps(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_siblings_simple() {
    common::abstract_query_tests::test_siblings_simple(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_siblings_with_buildfiles() {
    common::abstract_query_tests::test_siblings_with_buildfiles(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
#[ignore = "BuildViewTestCase creates the target configuration with its executor, which needs @platforms//host; the empty workspace of useReducedSetOfRules has none (Java SkyframeQueryHelper creates no configuration); needs a loading-only mode of blaze_sky_top::testing::BuildViewTestCase"]
fn test_slash_slash_dot_dot_dot() {
    common::abstract_query_tests::test_slash_slash_dot_dot_dot(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_some_operator_count_parameter_not_equal_actual_count() {
    common::abstract_query_tests::test_some_operator_count_parameter_not_equal_actual_count(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_some_operator_nested_some_test() {
    common::abstract_query_tests::test_some_operator_nested_some_test(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_some_operator_no_count_parameter() {
    common::abstract_query_tests::test_some_operator_no_count_parameter(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_some_path_operator() {
    common::abstract_query_tests::test_some_path_operator(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_some_path_operator_ordering() {
    common::abstract_query_tests::test_some_path_operator_ordering(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_starlark_diamond_equality() {
    common::abstract_query_tests::test_starlark_diamond_equality(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_starlark_rule_toolchain_deps() {
    common::abstract_query_tests::test_starlark_rule_toolchain_deps(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_strict_test_suite_with_file() {
    common::abstract_query_tests::test_strict_test_suite_with_file(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_subdir_symlink_cycle() {
    common::abstract_query_tests::test_subdir_symlink_cycle(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_target_literal_with_missing_targets() {
    common::abstract_query_tests::test_target_literal_with_missing_targets(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_target_literals() {
    common::abstract_query_tests::test_target_literals(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_targets_from_buildfiles_and_real_targets() {
    common::abstract_query_tests::test_targets_from_buildfiles_and_real_targets(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_test_suite_in_tests_attribute_and_vice_versa() {
    common::abstract_query_tests::test_test_suite_in_tests_attribute_and_vice_versa(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_test_suite_with_file() {
    common::abstract_query_tests::test_test_suite_with_file(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_tests_operator_crosses_packages() {
    common::abstract_query_tests::test_tests_operator_crosses_packages(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_tests_operator_expands_tests_and_excludes_non_tests() {
    common::abstract_query_tests::test_tests_operator_expands_tests_and_excludes_non_tests(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_tests_operator_filters_by_negative_tag() {
    common::abstract_query_tests::test_tests_operator_filters_by_negative_tag(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_tests_operator_filters_by_tag_size_and_env() {
    common::abstract_query_tests::test_tests_operator_filters_by_tag_size_and_env(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_tests_operator_handles_cycles_gracefully() {
    common::abstract_query_tests::test_tests_operator_handles_cycles_gracefully(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_tests_operator_reports_missing_targets() {
    common::abstract_query_tests::test_tests_operator_reports_missing_targets(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_unconditional_query_exception() {
    common::abstract_query_tests::test_unconditional_query_exception(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_unsuccessful_inner_future_in_nested_let_transform_async_fast_path() {
    common::abstract_query_tests::test_unsuccessful_inner_future_in_nested_let_transform_async_fast_path(&mut AbstractQueryTest::new(Suite::Graphless));
}

#[test]
#[ignore = "BuildViewTestCase creates the target configuration with its executor, which needs @platforms//host; the empty workspace of useReducedSetOfRules has none (Java SkyframeQueryHelper creates no configuration); needs a loading-only mode of blaze_sky_top::testing::BuildViewTestCase"]
fn test_visible_default_private() {
    common::abstract_query_tests::test_visible_default_private(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
#[ignore = "BuildViewTestCase creates the target configuration with its executor, which needs @platforms//host; the empty workspace of useReducedSetOfRules has none (Java SkyframeQueryHelper creates no configuration); needs a loading-only mode of blaze_sky_top::testing::BuildViewTestCase"]
fn test_visible_default_public() {
    common::abstract_query_tests::test_visible_default_public(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
#[ignore = "BuildViewTestCase creates the target configuration with its executor, which needs @platforms//host; the empty workspace of useReducedSetOfRules has none (Java SkyframeQueryHelper creates no configuration); needs a loading-only mode of blaze_sky_top::testing::BuildViewTestCase"]
fn test_visible_java_javatests() {
    common::abstract_query_tests::test_visible_java_javatests(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
#[ignore = "BuildViewTestCase creates the target configuration with its executor, which needs @platforms//host; the empty workspace of useReducedSetOfRules has none (Java SkyframeQueryHelper creates no configuration); needs a loading-only mode of blaze_sky_top::testing::BuildViewTestCase"]
fn test_visible_java_javatests_different_package() {
    common::abstract_query_tests::test_visible_java_javatests_different_package(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
#[ignore = "BuildViewTestCase creates the target configuration with its executor, which needs @platforms//host; the empty workspace of useReducedSetOfRules has none (Java SkyframeQueryHelper creates no configuration); needs a loading-only mode of blaze_sky_top::testing::BuildViewTestCase"]
fn test_visible_javatests_java() {
    common::abstract_query_tests::test_visible_javatests_java(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
#[ignore = "BuildViewTestCase creates the target configuration with its executor, which needs @platforms//host; the empty workspace of useReducedSetOfRules has none (Java SkyframeQueryHelper creates no configuration); needs a loading-only mode of blaze_sky_top::testing::BuildViewTestCase"]
fn test_visible_package_group() {
    common::abstract_query_tests::test_visible_package_group(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
#[ignore = "BuildViewTestCase creates the target configuration with its executor, which needs @platforms//host; the empty workspace of useReducedSetOfRules has none (Java SkyframeQueryHelper creates no configuration); needs a loading-only mode of blaze_sky_top::testing::BuildViewTestCase"]
fn test_visible_package_group_include() {
    common::abstract_query_tests::test_visible_package_group_include(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
#[ignore = "BuildViewTestCase creates the target configuration with its executor, which needs @platforms//host; the empty workspace of useReducedSetOfRules has none (Java SkyframeQueryHelper creates no configuration); needs a loading-only mode of blaze_sky_top::testing::BuildViewTestCase"]
fn test_visible_package_group_invisible() {
    common::abstract_query_tests::test_visible_package_group_invisible(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
#[ignore = "BuildViewTestCase creates the target configuration with its executor, which needs @platforms//host; the empty workspace of useReducedSetOfRules has none (Java SkyframeQueryHelper creates no configuration); needs a loading-only mode of blaze_sky_top::testing::BuildViewTestCase"]
fn test_visible_private_same_package() {
    common::abstract_query_tests::test_visible_private_same_package(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
#[ignore = "BuildViewTestCase creates the target configuration with its executor, which needs @platforms//host; the empty workspace of useReducedSetOfRules has none (Java SkyframeQueryHelper creates no configuration); needs a loading-only mode of blaze_sky_top::testing::BuildViewTestCase"]
fn test_visible_simple_different_subpackages() {
    common::abstract_query_tests::test_visible_simple_different_subpackages(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
#[ignore = "BuildViewTestCase creates the target configuration with its executor, which needs @platforms//host; the empty workspace of useReducedSetOfRules has none (Java SkyframeQueryHelper creates no configuration); needs a loading-only mode of blaze_sky_top::testing::BuildViewTestCase"]
fn test_visible_simple_package() {
    common::abstract_query_tests::test_visible_simple_package(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
#[ignore = "BuildViewTestCase creates the target configuration with its executor, which needs @platforms//host; the empty workspace of useReducedSetOfRules has none (Java SkyframeQueryHelper creates no configuration); needs a loading-only mode of blaze_sky_top::testing::BuildViewTestCase"]
fn test_visible_simple_private() {
    common::abstract_query_tests::test_visible_simple_private(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
#[ignore = "BuildViewTestCase creates the target configuration with its executor, which needs @platforms//host; the empty workspace of useReducedSetOfRules has none (Java SkyframeQueryHelper creates no configuration); needs a loading-only mode of blaze_sky_top::testing::BuildViewTestCase"]
fn test_visible_simple_public() {
    common::abstract_query_tests::test_visible_simple_public(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
#[ignore = "BuildViewTestCase creates the target configuration with its executor, which needs @platforms//host; the empty workspace of useReducedSetOfRules has none (Java SkyframeQueryHelper creates no configuration); needs a loading-only mode of blaze_sky_top::testing::BuildViewTestCase"]
fn test_visible_simple_subpackages() {
    common::abstract_query_tests::test_visible_simple_subpackages(&mut AbstractQueryTest::new(
        Suite::Graphless,
    ));
}

#[test]
fn test_visible_with_non_package_group_visibility() {
    common::abstract_query_tests::test_visible_with_non_package_group_visibility(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_visible_with_package_group_with_non_package_group_includes() {
    common::abstract_query_tests::test_visible_with_package_group_with_non_package_group_includes(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

#[test]
fn test_wildcards_dont_load_unnecessary_packages() {
    common::abstract_query_tests::test_wildcards_dont_load_unnecessary_packages(
        &mut AbstractQueryTest::new(Suite::Graphless),
    );
}

// Java: testutil/AbstractQueryTest.java testNoImplicitDeps_computedDefault
#[test]
#[ignore = "needs a native MockRule with an Attribute.ComputedDefault (MockRule.define) through SkyframeQueryHelper.useRuleClassProvider; Blaze rule class providers have no native test rules"]
fn test_no_implicit_deps_computed_default() {}

// Java: src/test/java/com/google/devtools/build/lib/query2/engine/GraphlessQueryTest.java:41 GraphlessQueryTest.boundedRdepsWithError
#[test]
fn bounded_rdeps_with_error() {
    let mut t = AbstractQueryTest::new(Suite::Graphless);
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
    let message = t
        .eval_throws("rdeps(//foo:foo, //foo:dep, 1)", false)
        .message;
    assert!(
        message.contains("preloading transitive closure failed"),
        "{message}"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/engine/GraphlessQueryTest.java:62 GraphlessQueryTest.testGraphOrderOfWildcards
/// Test assumes that the result is of type DigraphQueryEvalResult, which is not true for the
/// GraphlessBlazeQueryEnvironment (Java overrides it with an empty body).
#[test]
fn test_graph_order_of_wildcards() {}
