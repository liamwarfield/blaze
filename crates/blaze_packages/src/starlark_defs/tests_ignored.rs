//! `#[ignore]` stubs for the tests of `src/test/java/.../starlark/StarlarkRuleClassFunctionsTest`
//! that need BUILD loading, analysis, configured targets or other unported infrastructure.
//! The ported ones are in `tests.rs`.

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_cannot_override_built_in_attribute() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_cannot_override_built_in_attribute_name() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn built_in_attributes_are_not_starlark_defined() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_implicit_args_attribute() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_symbolic_macro_fails_without_flag() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_symbolic_macro_instantiation_registers_on_package() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_symbolic_macro_instantiation_requires_export() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_symbolic_macro_cannot_instantiate_in_bzl_thread() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_symbolic_macro_requires_name_attribute() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_symbolic_macro_prohibits_positional_args() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_symbolic_macro_can_accept_attributes() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_symbolic_macro_rejects_unknown_attribute() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_symbolic_macro_rejects_reserved_attribute_name() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_symbolic_macro_requires_mandatory_attribute() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_symbolic_macro_cannot_override_implicit_attribute() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_symbolic_macro_does_not_support_computed_defaults() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_symbolic_macro_does_not_support_late_bound_defaults() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_symbolic_macro_macro_function_api() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_attr_equality() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_rule_class_too_many_attributes() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_rule_class_too_long_attribute_name() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_attr_with_list() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_attr_single_file_with_list() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_rule_cannot_set_configurable_on_attr() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_aspect_cannot_set_configurable_on_attr() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_attr_with_providers() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_attr_with_providers_one_empty() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_attr_with_providers_list() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_attr_with_wrong_providers_list() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_label_list_with_aspects() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_label_with_aspects() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_label_list_with_aspects_error() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_attr_with_aspect_requiring_aspects_stack_of_required_aspects() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_attr_with_aspect_requiring_aspects_aspect_required_by_multiple_aspects() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_attr_with_aspect_requiring_aspects_aspect_required_by_multiple_aspects2() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_attr_with_aspect_requiring_aspects_require_existing_aspect_passed() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_attr_with_aspect_requiring_aspects_require_existing_aspect_failed() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_aspect_extra_deps() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_aspect_parameter() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_aspect_parameter_with_default_value() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_aspect_parameter_bad_default_value() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_aspect_parameter_not_require_values() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_aspect_parameter_bad_type() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_aspect_parameter_and_extra_deps() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_aspect_no_default_value_attribute() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_aspect_add_toolchain() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_non_label_attr_with_providers() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_attr_allowed_rule_classes_specific_rule_classes() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_attr_default_value() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_label_attr_default_value_as_string() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_label_attr_default_value_as_string_bad_value() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_attr_bad_keyword_arguments() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_attr_cfg_target_string() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_attr_cfg_target_object() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_attr_cfg_none() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_rule_cfg_exec_string_fails() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_rule_cfg_exec_obj_fails() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_rule_cfg_starlark() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn incompatible_data_transition() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_attr_doc() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_no_attr_license() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_attr_doc_value_bad_type() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_rule_implementation() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_rule_doc() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_starlark_label_and_name() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_function_as_attr_default() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_rule_add_attribute() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_output_to_genfiles() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_rule_add_multiple_attributes() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_rule_attribute_flag() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn unknown_rule_attribute_flags_forbidden() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_rule_outputs() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_rule_unknown_keyword() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_rule_implementation_missing() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_rule_bad_type_for_add() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_rule_bad_type_in_add() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_label() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_label_idempotence() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_label_same_instance() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_label_name_and_package() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_rule_label_default_value() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_int_default_value() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_int_default_value_must_be_int32() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_int_attribute_value_must_be_int32() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_integer_concat_truncates() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_rule_inherits_base_rule_attributes() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_simple_text_messages() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_none_struct_value() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_proto_fields_order() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_text_message_escapes() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_text_message_invalid_structure() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_starlark_json_module() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_json_boolean_fields() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_json_dict_fields() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_json_encoding() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_json_escapes() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_json_nested_list_structure() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_json_invalid_structure() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_json_file_encoding() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_label_attr_wrong_default() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_label_get_relative() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_label_get_relative_syntax_error() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_struct_creation() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_struct_fields() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_struct_equality() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_struct_incomparability() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_struct_accessing_fields_from_starlark() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_struct_accessing_unknown_field() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_struct_accessing_unknown_field_with_args() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_struct_accessing_non_function_field_with_args() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_struct_accessing_function_field_with_args() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_struct_pos_args() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_struct_concatenation_field_names() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_struct_concatenation_field_values() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_struct_concatenation_common_fields() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_conditional_struct_concatenation() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_getattr_no_attr() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_getattr() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_hasattr() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_struct_str() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_structs_in_sets() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_structs_in_dicts() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_struct_dict_members_are_mutable() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_depset_good_composite_item() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_struct_mutability_shallow() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_struct_mutability_deep() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn declared_providers() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn declared_provider_documentation() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn declared_providers_with_init() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn declared_providers_with_failing_init_raw_constructor_succeeds() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn declared_providers_with_failing_init_normal_constructor_fails() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn declared_providers_with_init_returning_invalid_type_normal_constructor_fails() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn declared_providers_with_init_returning_invalid_dict_normal_constructor_fails() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn declared_providers_with_init_returning_unexpected_fields_normal_constructor_fails() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn declared_providers_concat_success() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn declared_providers_with_init_concat_success() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn declared_providers_concat_error() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn declared_providers_with_fields_concat_success() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn declared_providers_with_fields_concat_error() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn declared_providers_with_overlapping_fields_concat_error() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn structs_as_declared_providers_test() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn declared_providers_doc() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn declared_providers_bad_type_for_doc() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn aspect_all_attrs() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn aspect_empty_attrs() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn aspect_default_attrs() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn aspect_required_aspect_providers_single() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn aspect_required_aspect_providers_alternatives() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn aspect_required_aspect_providers_empty() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn aspect_required_aspect_providers_default() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn aspect_required_providers_not_allowed_with_apply_to_generating_rules() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn aspect_required_providers_single() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn aspect_required_providers_alternatives() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn aspect_required_providers_empty() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn aspect_required_providers_default() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn aspect_provides() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn aspect_doc() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn aspect_bad_type_for_doc() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn fancy_exports() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn multiple_top_levels() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn provider_with_fields() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn provider_with_fields_dict() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn provider_with_fields_optional() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn provider_with_fields_optional_error() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn provider_with_extra_fields_error() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn provider_with_empty_fields_error() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn provider_with_duplicate_fields_error() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn invalid_toolchains_aspects_type() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_mandatory_config_parameter_for_executable_labels() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_rule_add_toolchain_duplicate() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_rule_add_exec_group() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_rule_ordered_requirements() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_rule_function_returns_none() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_type_of_struct() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn rule_definition_environment_digest_unaffected_by_target_attr_value_change() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn rule_definition_environment_digest_accounts_for_function_when_creating_rule_with_a_macro() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn rule_definition_environment_digest_accounts_for_attrs_when_creating_rule_with_macro() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn rule_class_defined_in_build_file_fails() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_attr_with_aspect_requiring_aspects_required_native_aspect_gets_params_from_base_rules() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn initializer_allowlist() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn initializer_basic() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn initializer_name_unchanged() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn initializer_name_changed() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn initializer_string_list_dict() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn string_keyed_label_dict_with_split_configuration() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn string_keyed_label_dict() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn initializer_label_keyed_string_dict() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn label_list_dict() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn label_list_dict_with_split_configuration() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn initializer_label_list_dict() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn initializer_legacy_any_type() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn initializer_wrong_type() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn initializer_with_select() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn initializer_pass_through() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn initializer_overrides_attribute_default() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn initializer_returning_none_sets_default() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn initializer_omitted_value_is_not_passed() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn initializer_none_value_is_not_passed() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn initializer_incorrect_return_type() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn initializer_incorrect_return_dicts() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn initializer_fails_setting_base_attribute() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn initializer_fails_setting_private_attribute_outside_builtins() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn initializer_setting_private_attribute_inside_builtins() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn initializer_fails_setting_unknown_attr() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn initializer_fails_creating_another_rule() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn initializer_fails_with_existing_rules() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn initializer_with_fails() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn initializer_native_module() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_allowlist() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_cc_binary() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_basic_use() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_with_initializers() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_super_not_called() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_super_called_twice() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_no_parent_super_called() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_extend_rule_twice() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_extend_rule_twice_super_not_called() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn ctx_super_called_from_aspect() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_attribute_additional_aspects() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_aspect_with_invalid_toolchains_aspects_fails() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_aspect_with_valid_toolchains_aspects_parses_correctly() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_aspect_with_wildcard_toolchains_aspects_parses_correctly() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_aspect_with_wildcard_toolchains_aspects_mixed_with_labels_fails() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn toolchains_aspects_default_empty_list() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_override_private_attribute_fails() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_parent_private_attr_default_visibility_checked_against_parent() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_attribute_override_default() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_attribute_collision() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_executable_matches() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_test_matches() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_controlled_parameters_fail() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_fragments_merged() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_not_extendable() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_native_rule_not_extendable() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_extendable_allowed() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_extendable_disallowed() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_extendable_allowlisted() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_extendable_allowlist_denied() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_extendable_default() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_toolchains_merged() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_advertised_providers_merged() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_exec_compatible_with_merged() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_exec_groups_merged() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_exec_groups_overwritten() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_cfg_from_parent() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_cfg_on_child() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn extend_rule_cfg_on_child_and_from_parent() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_analysis_test() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_analysis_test_attrs() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_analysis_test_duplicate_name_same_package() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_analysis_test_duplicate_name_different_attrs_different_package() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_analysis_test_bad_name() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_analysis_test_bad_args() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_analysis_test_error_on_export() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_analysis_test_error_overriding_name() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_label_with_strict_visibility() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_package_metadata_attr_on_all_rules() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn test_label_list_computed_default_checks_element_types() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn starlark_rule_function_codec() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn aspect_propagation_predicate_not_function_fails() {}

#[test]
#[ignore = "needs BUILD loading, analysis or configured targets (StarlarkRuleClassFunctionsTest)"]
fn aspect_apply_to_generating_rules_has_propagation_predicate_fails() {}
