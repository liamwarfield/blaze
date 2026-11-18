//! Ports of the BUILD-loading JUnit tests of `packages/` whose subject is owned here
//! (`PackageFactoryTest`, `ExportsFilesTest`, `NativeGlobTest`, `SelectTest`,
//! `NativeExistingRulesTest`, `NativeSubpackagesTest`). They load BUILD files through the
//! package loading pipeline (Java `PackageLoadingTestCase` / `BuildViewTestCase`), which agent
//! D's PACKAGE function provides; until it can run them they are ignored, never deleted.

// Java: src/test/java/com/google/devtools/build/lib/packages/PackageFactoryTest.java
mod package_factory_test {
    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_create_package() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_bad_rule_name() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_no_rule_name() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_bad_package_name() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_colon_in_exports_files_target_name() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_exports_files_visibility_must_be_sequence() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_exports_files_licenses_must_be_sequence() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_package_name_with_protected_is_ok() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_duplicated_dependencies() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_prefix_within_same_rule1() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_prefix_within_same_rule2() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_prefix_between_rules1() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_prefix_between_rules2() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_package_name_function() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_duplicate_rule_name() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_build_file_target_exists() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_creation_of_input_files() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_duplicate_rule_is_not_added_to_package() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_conflicting_rule_does_not_update_package() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_all_rules_in_errant_package_are_in_error() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_helpful_error_for_missing_exports_files() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_test_suites_implicitly_depend_on_all_rules_in_package() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_package_validation_failure_registered_after_loading() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_glob_directory_exclusion() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_recursive_glob() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_too_many_arguments_glob_errors() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_glob_enforces_list_argument() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_glob_enforces_list_of_strings_arguments() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_glob_negative_test() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_glob_exclude_directories() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_glob_does_not_exclude_directories() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_glob_with_empty_excluded_list() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_glob_with_question_mark_produces_error() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_glob_without_question_mark_doesnt_produce_error() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_glob_with_non_matching_excluded_list() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_glob_with_two_matching_glob_expressions_and_nonmatching_exclusion() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_glob_with_subdir_match_and_exclusion() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_bad_character_in_glob() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_bad_exclude_pattern() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_glob_escapes_at() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_glob_with_io_errors() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_native_module_is_disabled() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_package_group_spec_minimal() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_package_group_spec_simple() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_package_group_spec_empty() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_package_group_spec_includes() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_package_group_spec_bad() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_package_groups_with_same_name() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_package_group_named_arguments() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_package_spec_minimal() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_package_spec_simple() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_package_spec_bad() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_double_package_specification() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_empty_package_specification() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_default_testonly() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_default_deprecation() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_exports_build_file() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_default_deprecation_propagation() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_default_testonly_propagation() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_default_deprecation_overriding() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_package_features() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_transient_errors_in_globbing() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_export_twice_public_ok() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_export_twice_public_ok2() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_export_twice_fail() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_export_twice_fail2() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_export_license_twice() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_export_genrule_conflict() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_genrule_export_conflict() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_valid_environment_group() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_incomplete_environment_group() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_environment_group_missing_target() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_environment_group_wrong_target_type() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_environment_group_wrong_package() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_environment_group_invalid_default() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_environment_group_duplicate_environments() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_environment_group_duplicate_defaults() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_multiple_environment_groups_valid_membership() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_multiple_environment_groups_conflicting_membership() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_fulfills_references_wrong_target_type() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_fulfills_not_in_environment_group() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_package_default_environments() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_package_default_compatibility_duplicates() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_package_default_restriction_duplicates() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_symbolic_macro_duplicate_macro_names_disallowed() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_symbolic_macro_macro_and_rule_clash_macro_declared_first() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_symbolic_macro_macro_and_rule_clash_rule_declared_first() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_symbolic_macro_macro_and_output_clash_macro_declared_first() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_symbolic_macro_macro_and_output_clash_output_declared_first() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_symbolic_macro_macro_may_collide_with_prefix_of_output() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_symbolic_macro_macro_and_environment_group_clash_macro_declared_first() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_symbolic_macro_macro_and_environment_group_clash_environment_group_declared_first() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_symbolic_macro_macro_and_package_group_clash_macro_declared_first() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_symbolic_macro_macro_and_package_group_clash_package_group_declared_first() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_symbolic_macro_macro_and_input_clash_macro_declared_first() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_symbolic_macro_macro_and_input_clash_input_declared_first() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_symbolic_macro_implicitly_created_input_is_created_even_inside_macro_namespace() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_symbolic_macro_implicitly_created_input_is_not_created_if_macro_declares_target() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_symbolic_macro_implicitly_created_input_is_not_created_if_macro_name_matches_exactly() {
    }

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_symbolic_macro_implicitly_created_input_is_created_by_usage_in_macro_attr() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_symbolic_macro_implicitly_created_input_is_not_created_by_usage_in_macro_body() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_symbolic_macro_deferred_evaluation_expands_transitively() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_symbolic_macro_recursion_prohibited_with_eager_evaluation() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_symbolic_macro_recursion_prohibited_with_deferred_evaluation() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_symbolic_macro_indirect_recursion_also_prohibited() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_declaration_visibility_unioning_occurs_both_inside_and_outside_macros() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_declaration_visibility_unioning_uses_innermost_macro_location() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_declaration_visibility_unioning_does_not_apply_package_default_visibility() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_implicit_visibility_works_with_package_default_visibility() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_package_default_visibility_plays_well_with_private_visibility() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_package_default_visibility_succeeds_if_no_default_visibility_set() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_declaration_visibility_unioning_works_with_public_private_and_duplicate_visibilities() {
    }

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_declaration_visibility_unioning_applies_to_exports_files() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_declaration_visibility_unioning_has_no_effect_on_package_groups() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_declaration_visibility_unioning_fails_gracefully_on_invalid_visibility() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_glob_pattern_extractor() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_def_in_build() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_lambda_in_build() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_for_statement_forbidden_in_build() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_if_statement_forbidden_in_build() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_kwargs_forbidden_in_build() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_args_forbidden_in_build() {}
}

// Java: src/test/java/com/google/devtools/build/lib/packages/ExportsFilesTest.java
mod exports_files_test {
    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_exports_files_registers_files_with_package() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_file_thats_not_registered_yields_unknown_target_exception() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_registered_files_are_retrievable() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn test_exports_files_and_rule_name_conflict() {}
}

// Java: src/test/java/com/google/devtools/build/lib/packages/NativeGlobTest.java
mod native_glob_test {
    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn glob_simple() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn glob_not_empty() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn glob_simple_subdirs() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn glob_incremental() {}
}

// Java: src/test/java/com/google/devtools/build/lib/packages/SelectTest.java
mod select_test {
    #[test]
    #[ignore = "needs SelectorList.of / binaryOp (types/select.rs, A1) and a BUILD eval harness"]
    fn test_select() {}

    #[test]
    #[ignore = "needs SelectorList.of / binaryOp (types/select.rs, A1) and a BUILD eval harness"]
    fn test_plus() {}

    #[test]
    #[ignore = "needs SelectorList.of / binaryOp (types/select.rs, A1) and a BUILD eval harness"]
    fn test_plus_incompatible_type() {}

    #[test]
    #[ignore = "needs SelectorList.of / binaryOp (types/select.rs, A1) and a BUILD eval harness"]
    fn test_union_incompatible_type() {}

    #[test]
    #[ignore = "needs SelectorList.of / binaryOp (types/select.rs, A1) and a BUILD eval harness"]
    fn test_repr() {}

    #[test]
    #[ignore = "needs SelectorList.of / binaryOp (types/select.rs, A1) and a BUILD eval harness"]
    fn test_key_resolution() {}
}

// Java: src/test/java/com/google/devtools/build/lib/packages/NativeExistingRulesTest.java
mod native_existing_rules_test {
    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn existing_rule_handles_select() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn existing_rule_handles_select_with_none_values_for_label_valued_attributes() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn existing_rule_returns_none() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn existing_rule_round_trips_select() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn existing_rule_label_stringification() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn existing_rules_finds_rules_and_attributes() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn existing_rule_ignores_hidden_attributes() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn existing_rule_returns_immutable_object() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn existing_rule_returns_dict_like_object() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn existing_rule_as_dict_argument() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn existing_rule_as_dict_update_argument() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn existing_rule_unionable_with_dict() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn existing_rule_as_kwargs() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn existing_rule_encodes_to_json() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn existing_rules_returns_immutable_object() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn existing_rules_returns_deeply_immutable_view() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn existing_rules_returns_dict_like_object() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn existing_rules_returns_snapshot_of_only_rules_instantiated_up_to_that_point() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn existing_rules_encode_to_json() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn existing_rule_round_trip_through_rule() {}
}

// Java: src/test/java/com/google/devtools/build/lib/packages/NativeSubpackagesTest.java
mod native_subpackages_test {
    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn subpackages_simple_sub_dir() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn subpackages_simple_include() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn subpackages_simple_exclude() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn subpackages_simple_empty_allow() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn subpackages_simple_empty_disallow() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn subpackages_deeply_nested_with_subdirs() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn subpackages_incremental_add_sub_pkg() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn subpackages_incremental_del_sub_pkg() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn subpackages_incremental_convert_sub_dir_to_pkg() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn invalid_positional_params() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn invalid_missing_include() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn valid_no_wild_card_include() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn include_valid_match_subdir() {}

    #[test]
    #[ignore = "needs PACKAGE (BUILD-file loading test harness)"]
    fn include_valid_sub_match_subdir() {}
}
