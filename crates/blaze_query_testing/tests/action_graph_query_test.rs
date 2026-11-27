//! Java's `ActionGraphQueryTest` (src/test/java/com/google/devtools/build/lib/query2/aquery/ActionGraphQueryTest.java):
//! `AbstractQueryTest` and `PostAnalysisQueryTest` on an `ActionGraphQueryEnvironment`,
//! `--nokeep_going`.

mod common;

use common::{AbstractQueryTest, Suite};

#[test]
fn test_algebraic_set_operations() {
    common::abstract_query_tests::test_algebraic_set_operations(&mut AbstractQueryTest::new(
        Suite::Aquery,
    ));
}

#[test]
fn test_algebraic_set_operations_many_operands() {
    common::abstract_query_tests::test_algebraic_set_operations_many_operands(
        &mut AbstractQueryTest::new(Suite::Aquery),
    );
}

#[test]
fn test_all_paths_operator() {
    common::abstract_query_tests::test_all_paths_operator(&mut AbstractQueryTest::new(
        Suite::Aquery,
    ));
}

#[test]
fn test_aspect_on_rule_without_declared_providers() {
    common::abstract_query_tests::test_aspect_on_rule_without_declared_providers(
        &mut AbstractQueryTest::new(Suite::Aquery),
    );
}

#[test]
fn test_attr_on_package_default_visibility() {
    common::abstract_query_tests::test_attr_on_package_default_visibility(
        &mut AbstractQueryTest::new(Suite::Aquery),
    );
}

#[test]
fn test_attr_operator() {
    common::abstract_query_tests::test_attr_operator(&mut AbstractQueryTest::new(Suite::Aquery));
}

#[test]
fn test_attr_operator_on_booleans() {
    common::abstract_query_tests::test_attr_operator_on_booleans(&mut AbstractQueryTest::new(
        Suite::Aquery,
    ));
}

#[test]
fn test_attr_operator_on_name() {
    common::abstract_query_tests::test_attr_operator_on_name(&mut AbstractQueryTest::new(
        Suite::Aquery,
    ));
}

#[test]
fn test_big_expression() {
    common::abstract_query_tests::test_big_expression(&mut AbstractQueryTest::new(Suite::Aquery));
}

#[test]
fn test_bounded_deps_streaming() {
    common::abstract_query_tests::test_bounded_deps_streaming(&mut AbstractQueryTest::new(
        Suite::Aquery,
    ));
}

#[test]
fn test_bounded_rdeps_streaming() {
    common::abstract_query_tests::test_bounded_rdeps_streaming(&mut AbstractQueryTest::new(
        Suite::Aquery,
    ));
}

#[test]
fn test_deep_nested_let() {
    common::run_with_large_stack(|| {
        common::abstract_query_tests::test_deep_nested_let(&mut AbstractQueryTest::new(
            Suite::Aquery,
        ))
    });
}

#[test]
fn test_default_visibility_returned_in_deps() {
    common::abstract_query_tests::test_default_visibility_returned_in_deps(
        &mut AbstractQueryTest::new(Suite::Aquery),
    );
}

#[test]
fn test_default_visibility_returned_in_deps_for_input_files() {
    common::abstract_query_tests::test_default_visibility_returned_in_deps_for_input_files(
        &mut AbstractQueryTest::new(Suite::Aquery),
    );
}

#[test]
fn test_deps() {
    common::abstract_query_tests::test_deps(&mut AbstractQueryTest::new(Suite::Aquery));
}

#[test]
fn test_deps_does_not_include_build_files() {
    common::abstract_query_tests::test_deps_does_not_include_build_files(
        &mut AbstractQueryTest::new(Suite::Aquery),
    );
}

#[test]
fn test_executables_query() {
    common::abstract_query_tests::test_executables_query(&mut AbstractQueryTest::new(
        Suite::Aquery,
    ));
}

#[test]
fn test_external_repo_all_targets_below() {
    common::abstract_query_tests::test_external_repo_all_targets_below(
        &mut AbstractQueryTest::new(Suite::Aquery),
    );
}

#[test]
fn test_external_repo_all_targets_in_package() {
    common::abstract_query_tests::test_external_repo_all_targets_in_package(
        &mut AbstractQueryTest::new(Suite::Aquery),
    );
}

#[test]
fn test_filter_operator() {
    common::abstract_query_tests::test_filter_operator(&mut AbstractQueryTest::new(Suite::Aquery));
}

#[test]
fn test_includes_returned_in_deps() {
    common::abstract_query_tests::test_includes_returned_in_deps(&mut AbstractQueryTest::new(
        Suite::Aquery,
    ));
}

#[test]
fn test_kind_operator() {
    common::abstract_query_tests::test_kind_operator(&mut AbstractQueryTest::new(Suite::Aquery));
}

#[test]
fn test_label_flag_default_appears_in_deps_query() {
    common::abstract_query_tests::test_label_flag_default_appears_in_deps_query(
        &mut AbstractQueryTest::new(Suite::Aquery),
    );
}

#[test]
fn test_label_setting_default_appears_in_deps_query() {
    common::abstract_query_tests::test_label_setting_default_appears_in_deps_query(
        &mut AbstractQueryTest::new(Suite::Aquery),
    );
}

#[test]
fn test_native_rule_toolchain_deps() {
    common::abstract_query_tests::test_native_rule_toolchain_deps(&mut AbstractQueryTest::new(
        Suite::Aquery,
    ));
}

#[test]
fn test_nested_let_expressions() {
    common::abstract_query_tests::test_nested_let_expressions(&mut AbstractQueryTest::new(
        Suite::Aquery,
    ));
}

#[test]
fn test_nodep_deps_default_is_true() {
    common::abstract_query_tests::test_nodep_deps_default_is_true(&mut AbstractQueryTest::new(
        Suite::Aquery,
    ));
}

#[test]
fn test_null_attr_operator() {
    common::abstract_query_tests::test_null_attr_operator(&mut AbstractQueryTest::new(
        Suite::Aquery,
    ));
}

#[test]
fn test_path_operators_with_output_file() {
    common::abstract_query_tests::test_path_operators_with_output_file(
        &mut AbstractQueryTest::new(Suite::Aquery),
    );
}

#[test]
fn test_query_starlark_aspect_with_parameters() {
    common::abstract_query_tests::test_query_starlark_aspect_with_parameters(
        &mut AbstractQueryTest::new(Suite::Aquery),
    );
}

#[test]
fn test_query_starlark_aspects() {
    common::abstract_query_tests::test_query_starlark_aspects(&mut AbstractQueryTest::new(
        Suite::Aquery,
    ));
}

#[test]
fn test_query_starlark_aspects_no_implicit_deps() {
    common::abstract_query_tests::test_query_starlark_aspects_no_implicit_deps(
        &mut AbstractQueryTest::new(Suite::Aquery),
    );
}

#[test]
fn test_query_time_loading_of_target_pattern_happy_path() {
    common::abstract_query_tests::test_query_time_loading_of_target_pattern_happy_path(
        &mut AbstractQueryTest::new(Suite::Aquery),
    );
}

#[test]
fn test_rdeps() {
    common::abstract_query_tests::test_rdeps(&mut AbstractQueryTest::new(Suite::Aquery));
}

#[test]
fn test_regression1267510() {
    common::abstract_query_tests::test_regression1267510(&mut AbstractQueryTest::new(
        Suite::Aquery,
    ));
}

#[test]
fn test_regression1352570() {
    common::abstract_query_tests::test_regression1352570(&mut AbstractQueryTest::new(
        Suite::Aquery,
    ));
}

#[test]
fn test_rule_output_ambiguity_isnt_fatal() {
    common::abstract_query_tests::test_rule_output_ambiguity_isnt_fatal(
        &mut AbstractQueryTest::new(Suite::Aquery),
    );
}

#[test]
fn test_scope_of_let_expressions() {
    common::abstract_query_tests::test_scope_of_let_expressions(&mut AbstractQueryTest::new(
        Suite::Aquery,
    ));
}

#[test]
fn test_some_operator_count_parameter_not_equal_actual_count() {
    common::abstract_query_tests::test_some_operator_count_parameter_not_equal_actual_count(
        &mut AbstractQueryTest::new(Suite::Aquery),
    );
}

#[test]
fn test_some_operator_nested_some_test() {
    common::abstract_query_tests::test_some_operator_nested_some_test(&mut AbstractQueryTest::new(
        Suite::Aquery,
    ));
}

#[test]
fn test_some_operator_no_count_parameter() {
    common::abstract_query_tests::test_some_operator_no_count_parameter(
        &mut AbstractQueryTest::new(Suite::Aquery),
    );
}

#[test]
fn test_some_path_operator() {
    common::abstract_query_tests::test_some_path_operator(&mut AbstractQueryTest::new(
        Suite::Aquery,
    ));
}

#[test]
fn test_some_path_operator_ordering() {
    common::abstract_query_tests::test_some_path_operator_ordering(&mut AbstractQueryTest::new(
        Suite::Aquery,
    ));
}

#[test]
fn test_starlark_diamond_equality() {
    common::abstract_query_tests::test_starlark_diamond_equality(&mut AbstractQueryTest::new(
        Suite::Aquery,
    ));
}

#[test]
fn test_starlark_rule_toolchain_deps() {
    common::abstract_query_tests::test_starlark_rule_toolchain_deps(&mut AbstractQueryTest::new(
        Suite::Aquery,
    ));
}

#[test]
fn test_subdir_symlink_cycle() {
    common::abstract_query_tests::test_subdir_symlink_cycle(&mut AbstractQueryTest::new(
        Suite::Aquery,
    ));
}

#[test]
fn test_target_literals() {
    common::abstract_query_tests::test_target_literals(&mut AbstractQueryTest::new(Suite::Aquery));
}

#[test]
fn test_unconditional_query_exception() {
    common::abstract_query_tests::test_unconditional_query_exception(&mut AbstractQueryTest::new(
        Suite::Aquery,
    ));
}

#[test]
fn test_unsuccessful_inner_future_in_nested_let_transform_async_fast_path() {
    common::abstract_query_tests::test_unsuccessful_inner_future_in_nested_let_transform_async_fast_path(&mut AbstractQueryTest::new(Suite::Aquery));
}

#[test]
fn bounded_deps_with_error() {
    common::post_analysis_query_tests::bounded_deps_with_error(&mut AbstractQueryTest::new(
        Suite::Aquery,
    ));
}

#[test]
fn label_points_to_multiple_configured_targets() {
    common::post_analysis_query_tests::label_points_to_multiple_configured_targets(
        &mut AbstractQueryTest::new(Suite::Aquery),
    );
}

#[test]
fn test_bad_target_literals() {
    common::post_analysis_query_tests::test_bad_target_literals(&mut AbstractQueryTest::new(
        Suite::Aquery,
    ));
}

#[test]
fn test_default_visibility_returned_in_deps_non_empty_dependency_filter() {
    common::post_analysis_query_tests::test_default_visibility_returned_in_deps_non_empty_dependency_filter(&mut AbstractQueryTest::new(Suite::Aquery));
}

#[test]
fn test_equality_of_ordered_thread_safe_immutable_set() {
    common::post_analysis_query_tests::test_equality_of_ordered_thread_safe_immutable_set(
        &mut AbstractQueryTest::new(Suite::Aquery),
    );
}

#[test]
fn test_genquery_scope() {
    common::post_analysis_query_tests::test_genquery_scope(&mut AbstractQueryTest::new(
        Suite::Aquery,
    ));
}

#[test]
fn test_graph_order_of_wildcards() {
    common::post_analysis_query_tests::test_graph_order_of_wildcards(&mut AbstractQueryTest::new(
        Suite::Aquery,
    ));
}

#[test]
fn test_implicit_deps_on_output_file() {
    common::post_analysis_query_tests::test_implicit_deps_on_output_file(
        &mut AbstractQueryTest::new(Suite::Aquery),
    );
}

#[test]
fn test_let() {
    common::post_analysis_query_tests::test_let(&mut AbstractQueryTest::new(Suite::Aquery));
}

#[test]
fn test_no_implicit_deps_cc_toolchains() {
    common::post_analysis_query_tests::test_no_implicit_deps_cc_toolchains(
        &mut AbstractQueryTest::new(Suite::Aquery),
    );
}

#[test]
fn test_no_implicit_deps_from_auto_exec_groups_auto_exec_groups_enabled() {
    common::post_analysis_query_tests::test_no_implicit_deps_from_auto_exec_groups_auto_exec_groups_enabled(&mut AbstractQueryTest::new(Suite::Aquery));
}

#[test]
fn test_no_implicit_deps_from_custom_exec_groups_auto_exec_groups_enabled() {
    common::post_analysis_query_tests::test_no_implicit_deps_from_custom_exec_groups_auto_exec_groups_enabled(&mut AbstractQueryTest::new(Suite::Aquery));
}

#[test]
fn test_no_implicit_deps_on_output_file() {
    common::post_analysis_query_tests::test_no_implicit_deps_on_output_file(
        &mut AbstractQueryTest::new(Suite::Aquery),
    );
}

#[test]
fn test_no_implicit_deps_starlark_toolchains() {
    common::post_analysis_query_tests::test_no_implicit_deps_starlark_toolchains(
        &mut AbstractQueryTest::new(Suite::Aquery),
    );
}

#[test]
fn test_nodep_deps_false() {
    common::post_analysis_query_tests::test_nodep_deps_false(&mut AbstractQueryTest::new(
        Suite::Aquery,
    ));
}

#[test]
fn test_non_idempotent_rule_transition() {
    common::post_analysis_query_tests::test_non_idempotent_rule_transition(
        &mut AbstractQueryTest::new(Suite::Aquery),
    );
}

#[test]
fn test_non_idempotent_rule_transition_transitioned_config_is_also_toplevel() {
    common::post_analysis_query_tests::test_non_idempotent_rule_transition_transitioned_config_is_also_toplevel(&mut AbstractQueryTest::new(Suite::Aquery));
}

#[test]
fn test_set() {
    common::post_analysis_query_tests::test_set(&mut AbstractQueryTest::new(Suite::Aquery));
}

#[test]
fn test_target_literal_with_missing_targets() {
    common::post_analysis_query_tests::test_target_literal_with_missing_targets(
        &mut AbstractQueryTest::new(Suite::Aquery),
    );
}

#[test]
fn test_visible_with_non_package_group_visibility() {
    common::post_analysis_query_tests::test_visible_with_non_package_group_visibility(
        &mut AbstractQueryTest::new(Suite::Aquery),
    );
}

#[test]
fn test_visible_with_package_group_with_non_package_group_includes() {
    common::post_analysis_query_tests::test_visible_with_package_group_with_non_package_group_includes(&mut AbstractQueryTest::new(Suite::Aquery));
}

#[test]
fn test_wildcards_dont_load_unnecessary_packages() {
    common::post_analysis_query_tests::test_wildcards_dont_load_unnecessary_packages(
        &mut AbstractQueryTest::new(Suite::Aquery),
    );
}

// Java: testutil/PostAnalysisQueryTest.java aliasMinus
#[test]
#[ignore = "needs a native MockRule (MockRule.define) through useRuleClassProvider; Blaze rule class providers have no native test rules"]
fn alias_minus() {}

// Java: testutil/PostAnalysisQueryTest.java inconsistentSkyQueryIncremental
#[test]
#[ignore = "needs SkyframeQueryHelper/AnalysisTestCase.setSyscallCache with TestUtils.makeDisappearingFileCache; the harness has no syscall cache hook"]
fn inconsistent_sky_query_incremental() {}

// Java: testutil/PostAnalysisQueryTest.java testBuildfilesFunctionDoesNotWork
#[test]
#[ignore = "needs a native MockRule (MockRule.define) through useRuleClassProvider; Blaze rule class providers have no native test rules (writeSimpleTarget)"]
fn test_buildfiles_function_does_not_work() {}

// Java: testutil/PostAnalysisQueryTest.java testMultipleTopLevelConfigurations
#[test]
#[ignore = "needs a native MockRule (MockRule.define) through useRuleClassProvider; Blaze rule class providers have no native test rules"]
fn test_multiple_top_level_configurations() {}

// Java: testutil/PostAnalysisQueryTest.java testMultipleTopLevelConfigurations_multipleConfigsPrefersTopLevel
#[test]
#[ignore = "needs a native MockRule (MockRule.define) through useRuleClassProvider; Blaze rule class providers have no native test rules"]
fn test_multiple_top_level_configurations_multiple_configs_prefers_top_level() {}

// Java: testutil/PostAnalysisQueryTest.java testNoImplicitDeps
#[test]
#[ignore = "needs a native MockRule (MockRule.define) through useRuleClassProvider; Blaze rule class providers have no native test rules"]
fn test_no_implicit_deps() {}

// Java: testutil/PostAnalysisQueryTest.java testNoImplicitDeps_computedDefault
#[test]
#[ignore = "needs a native MockRule (MockRule.define) through useRuleClassProvider; Blaze rule class providers have no native test rules"]
fn test_no_implicit_deps_computed_default() {}

// Java: testutil/PostAnalysisQueryTest.java testNoImplicitDeps_platformDeps
#[test]
#[ignore = "needs a native MockRule (MockRule.define) through useRuleClassProvider; Blaze rule class providers have no native test rules"]
fn test_no_implicit_deps_platform_deps() {}

// Java: testutil/PostAnalysisQueryTest.java testNoImplicitDeps_toolchains
#[test]
#[ignore = "needs a native MockRule (MockRule.define) through useRuleClassProvider; Blaze rule class providers have no native test rules"]
fn test_no_implicit_deps_toolchains() {}

// Java: testutil/PostAnalysisQueryTest.java testSiblingsFunctionDoesNotWork
#[test]
#[ignore = "needs a native MockRule (MockRule.define) through useRuleClassProvider; Blaze rule class providers have no native test rules (writeSimpleTarget)"]
fn test_siblings_function_does_not_work() {}

// Java: testutil/PostAnalysisQueryTest.java testVisibleFunctionDoesNotWork
#[test]
#[ignore = "needs a native MockRule (MockRule.define) through useRuleClassProvider; Blaze rule class providers have no native test rules (writeSimpleTarget)"]
fn test_visible_function_does_not_work() {}

#[test]
fn test_implicit_toolchain_binding_contains_toolchain_target() {
    common::action_graph_query_tests::test_implicit_toolchain_binding_contains_toolchain_target(
        &mut AbstractQueryTest::new(Suite::Aquery),
    );
}

#[test]
fn test_multiple_top_level_configurations_null_configs() {
    common::action_graph_query_tests::test_multiple_top_level_configurations_null_configs(
        &mut AbstractQueryTest::new(Suite::Aquery),
    );
}
