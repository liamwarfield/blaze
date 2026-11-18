//! `#[ignore]` stubs for the tests of
//! `src/test/java/com/google/devtools/build/lib/starlark/StarlarkRuleClassFunctionsTest.java`
//! that are not ported, each with the reason.
//!
//! Where the ported tests of that Java class are:
//! - `crates/blaze_sky_loading/src/tests/starlark_rule_class_functions_test.rs`: the tests that
//!   only evaluate .bzl code or only load packages, on the package-loading harness.
//! - `crates/blaze_sky_loading/src/tests/review_s2_natives_test.rs`:
//!   `initializer_failsSettingPrivateAttribute_outsideBuiltins`,
//!   `initializer_settingPrivateAttribute_insideBuiltins`, `initializer_failsCreatingAnotherRule`,
//!   `initializer_failsWithExistingRules`, `initializer_nativeModule`.
//! - `tests.rs` next to this file: the .bzl-only tests ported with the module-level harness
//!   (`testFunctionAsAttrDefault` among them).

// Java: StarlarkRuleClassFunctionsTest.builtInAttributesAreNotStarlarkDefined
#[test]
#[ignore = "inspects the RuleClass behind ev.lookup (getRuleClass); portable with the .bzl harness of starlark_defs/tests.rs (get_rule_class), not ported yet"]
fn built_in_attributes_are_not_starlark_defined() {}

// Java: StarlarkRuleClassFunctionsTest.testImplicitArgsAttribute
#[test]
#[ignore = "inspects the RuleClass behind ev.lookup (getRuleClass); portable with the .bzl harness of starlark_defs/tests.rs (get_rule_class), not ported yet"]
fn test_implicit_args_attribute() {}

// Java: StarlarkRuleClassFunctionsTest.testSymbolicMacro_failsWithoutFlag
#[test]
#[ignore = "symbolic macros are not ported (MacroClass.instantiateAndAddMacro)"]
fn test_symbolic_macro_fails_without_flag() {}

// Java: StarlarkRuleClassFunctionsTest.testSymbolicMacro_instantiationRegistersOnPackage
#[test]
#[ignore = "symbolic macros are not ported (MacroClass.instantiateAndAddMacro)"]
fn test_symbolic_macro_instantiation_registers_on_package() {}

// Java: StarlarkRuleClassFunctionsTest.testSymbolicMacro_instantiationRequiresExport
#[test]
#[ignore = "symbolic macros are not ported (MacroClass.instantiateAndAddMacro)"]
fn test_symbolic_macro_instantiation_requires_export() {}

// Java: StarlarkRuleClassFunctionsTest.testSymbolicMacro_cannotInstantiateInBzlThread
#[test]
#[ignore = "symbolic macros are not ported (MacroClass.instantiateAndAddMacro)"]
fn test_symbolic_macro_cannot_instantiate_in_bzl_thread() {}

// Java: StarlarkRuleClassFunctionsTest.testSymbolicMacro_requiresNameAttribute
#[test]
#[ignore = "symbolic macros are not ported (MacroClass.instantiateAndAddMacro)"]
fn test_symbolic_macro_requires_name_attribute() {}

// Java: StarlarkRuleClassFunctionsTest.testSymbolicMacro_prohibitsPositionalArgs
#[test]
#[ignore = "symbolic macros are not ported (MacroClass.instantiateAndAddMacro)"]
fn test_symbolic_macro_prohibits_positional_args() {}

// Java: StarlarkRuleClassFunctionsTest.testSymbolicMacroCanAcceptAttributes
#[test]
#[ignore = "symbolic macros are not ported (MacroClass.instantiateAndAddMacro)"]
fn test_symbolic_macro_can_accept_attributes() {}

// Java: StarlarkRuleClassFunctionsTest.testSymbolicMacro_rejectsUnknownAttribute
#[test]
#[ignore = "symbolic macros are not ported (MacroClass.instantiateAndAddMacro)"]
fn test_symbolic_macro_rejects_unknown_attribute() {}

// Java: StarlarkRuleClassFunctionsTest.testSymbolicMacro_rejectsReservedAttributeName
#[test]
#[ignore = "symbolic macros are not ported (MacroClass.instantiateAndAddMacro)"]
fn test_symbolic_macro_rejects_reserved_attribute_name() {}

// Java: StarlarkRuleClassFunctionsTest.testSymbolicMacro_requiresMandatoryAttribute
#[test]
#[ignore = "symbolic macros are not ported (MacroClass.instantiateAndAddMacro)"]
fn test_symbolic_macro_requires_mandatory_attribute() {}

// Java: StarlarkRuleClassFunctionsTest.testSymbolicMacro_cannotOverrideImplicitAttribute
#[test]
#[ignore = "symbolic macros are not ported (MacroClass.instantiateAndAddMacro)"]
fn test_symbolic_macro_cannot_override_implicit_attribute() {}

// Java: StarlarkRuleClassFunctionsTest.testSymbolicMacro_doesNotSupportComputedDefaults
#[test]
#[ignore = "symbolic macros are not ported (MacroClass.instantiateAndAddMacro)"]
fn test_symbolic_macro_does_not_support_computed_defaults() {}

// Java: StarlarkRuleClassFunctionsTest.testSymbolicMacro_doesNotSupportLateBoundDefaults
#[test]
#[ignore = "symbolic macros are not ported (MacroClass.instantiateAndAddMacro)"]
fn test_symbolic_macro_does_not_support_late_bound_defaults() {}

// Java: StarlarkRuleClassFunctionsTest.testSymbolicMacro_macroFunctionApi
#[test]
#[ignore = "symbolic macros are not ported (MacroClass.instantiateAndAddMacro)"]
fn test_symbolic_macro_macro_function_api() {}

// Java: StarlarkRuleClassFunctionsTest.testAttrEquality
#[test]
#[ignore = "needs Guava EqualsTester over attr descriptors (StarlarkAttrModule.Descriptor equality)"]
fn test_attr_equality() {}

// Java: StarlarkRuleClassFunctionsTest.testAttrWithList
#[test]
#[ignore = "inspects the built Attribute (buildAttribute); portable with the .bzl harness of starlark_defs/tests.rs (build_attribute), not ported yet"]
fn test_attr_with_list() {}

// Java: StarlarkRuleClassFunctionsTest.testAttrSingleFileWithList
#[test]
#[ignore = "inspects the built Attribute (buildAttribute); portable with the .bzl harness of starlark_defs/tests.rs (build_attribute), not ported yet"]
fn test_attr_single_file_with_list() {}

// Java: StarlarkRuleClassFunctionsTest.testAttrWithProviders
#[test]
#[ignore = "inspects the built Attribute (buildAttribute); portable with the .bzl harness of starlark_defs/tests.rs (build_attribute), not ported yet"]
fn test_attr_with_providers() {}

// Java: StarlarkRuleClassFunctionsTest.testAttrWithProvidersOneEmpty
#[test]
#[ignore = "inspects the built Attribute (buildAttribute); portable with the .bzl harness of starlark_defs/tests.rs (build_attribute), not ported yet"]
fn test_attr_with_providers_one_empty() {}

// Java: StarlarkRuleClassFunctionsTest.testAttrWithProvidersList
#[test]
#[ignore = "inspects the built Attribute (buildAttribute); portable with the .bzl harness of starlark_defs/tests.rs (build_attribute), not ported yet"]
fn test_attr_with_providers_list() {}

// Java: StarlarkRuleClassFunctionsTest.testLabelListWithAspects
#[test]
#[ignore = "inspects the StarlarkDefinedAspect or the attribute aspects behind ev.lookup; portable with the .bzl harness of starlark_defs/tests.rs, not ported yet"]
fn test_label_list_with_aspects() {}

// Java: StarlarkRuleClassFunctionsTest.testLabelWithAspects
#[test]
#[ignore = "inspects the StarlarkDefinedAspect or the attribute aspects behind ev.lookup; portable with the .bzl harness of starlark_defs/tests.rs, not ported yet"]
fn test_label_with_aspects() {}

// Java: StarlarkRuleClassFunctionsTest.testAttrWithAspectRequiringAspects_stackOfRequiredAspects
#[test]
#[ignore = "inspects the StarlarkDefinedAspect or the attribute aspects behind ev.lookup; portable with the .bzl harness of starlark_defs/tests.rs, not ported yet"]
fn test_attr_with_aspect_requiring_aspects_stack_of_required_aspects() {}

// Java: StarlarkRuleClassFunctionsTest.testAttrWithAspectRequiringAspects_aspectRequiredByMultipleAspects
#[test]
#[ignore = "inspects the StarlarkDefinedAspect or the attribute aspects behind ev.lookup; portable with the .bzl harness of starlark_defs/tests.rs, not ported yet"]
fn test_attr_with_aspect_requiring_aspects_aspect_required_by_multiple_aspects() {}

// Java: StarlarkRuleClassFunctionsTest.testAttrWithAspectRequiringAspects_aspectRequiredByMultipleAspects2
#[test]
#[ignore = "inspects the StarlarkDefinedAspect or the attribute aspects behind ev.lookup; portable with the .bzl harness of starlark_defs/tests.rs, not ported yet"]
fn test_attr_with_aspect_requiring_aspects_aspect_required_by_multiple_aspects2() {}

// Java: StarlarkRuleClassFunctionsTest.testAttrWithAspectRequiringAspects_requireExistingAspect_passed
#[test]
#[ignore = "inspects the StarlarkDefinedAspect or the attribute aspects behind ev.lookup; portable with the .bzl harness of starlark_defs/tests.rs, not ported yet"]
fn test_attr_with_aspect_requiring_aspects_require_existing_aspect_passed() {}

// Java: StarlarkRuleClassFunctionsTest.testAspectExtraDeps
#[test]
#[ignore = "inspects the StarlarkDefinedAspect or the attribute aspects behind ev.lookup; portable with the .bzl harness of starlark_defs/tests.rs, not ported yet"]
fn test_aspect_extra_deps() {}

// Java: StarlarkRuleClassFunctionsTest.testAspectParameter
#[test]
#[ignore = "inspects the StarlarkDefinedAspect or the attribute aspects behind ev.lookup; portable with the .bzl harness of starlark_defs/tests.rs, not ported yet"]
fn test_aspect_parameter() {}

// Java: StarlarkRuleClassFunctionsTest.testAspectParameterWithDefaultValue
#[test]
#[ignore = "inspects the StarlarkDefinedAspect or the attribute aspects behind ev.lookup; portable with the .bzl harness of starlark_defs/tests.rs, not ported yet"]
fn test_aspect_parameter_with_default_value() {}

// Java: StarlarkRuleClassFunctionsTest.testAspectParameterNotRequireValues
#[test]
#[ignore = "inspects the StarlarkDefinedAspect or the attribute aspects behind ev.lookup; portable with the .bzl harness of starlark_defs/tests.rs, not ported yet"]
fn test_aspect_parameter_not_require_values() {}

// Java: StarlarkRuleClassFunctionsTest.testAspectParameterAndExtraDeps
#[test]
#[ignore = "inspects the StarlarkDefinedAspect or the attribute aspects behind ev.lookup; portable with the .bzl harness of starlark_defs/tests.rs, not ported yet"]
fn test_aspect_parameter_and_extra_deps() {}

// Java: StarlarkRuleClassFunctionsTest.testAspectAddToolchain
#[test]
#[ignore = "inspects the StarlarkDefinedAspect or the attribute aspects behind ev.lookup; portable with the .bzl harness of starlark_defs/tests.rs, not ported yet"]
fn test_aspect_add_toolchain() {}

// Java: StarlarkRuleClassFunctionsTest.testAttrAllowedRuleClassesSpecificRuleClasses
#[test]
#[ignore = "inspects the built Attribute (buildAttribute); portable with the .bzl harness of starlark_defs/tests.rs (build_attribute), not ported yet"]
fn test_attr_allowed_rule_classes_specific_rule_classes() {}

// Java: StarlarkRuleClassFunctionsTest.testAttrDefaultValue
#[test]
#[ignore = "inspects the built Attribute (buildAttribute); portable with the .bzl harness of starlark_defs/tests.rs (build_attribute), not ported yet"]
fn test_attr_default_value() {}

// Java: StarlarkRuleClassFunctionsTest.testLabelAttrDefaultValueAsString
#[test]
#[ignore = "inspects the built Attribute (buildAttribute); portable with the .bzl harness of starlark_defs/tests.rs (build_attribute), not ported yet"]
fn test_label_attr_default_value_as_string() {}

// Java: StarlarkRuleClassFunctionsTest.testAttrCfgTarget_string
#[test]
#[ignore = "inspects the built Attribute (buildAttribute); portable with the .bzl harness of starlark_defs/tests.rs (build_attribute), not ported yet"]
fn test_attr_cfg_target_string() {}

// Java: StarlarkRuleClassFunctionsTest.testAttrCfgTarget_object
#[test]
#[ignore = "inspects the built Attribute (buildAttribute); portable with the .bzl harness of starlark_defs/tests.rs (build_attribute), not ported yet"]
fn test_attr_cfg_target_object() {}

// Java: StarlarkRuleClassFunctionsTest.testAttrCfgNone
#[test]
#[ignore = "inspects the built Attribute (buildAttribute); portable with the .bzl harness of starlark_defs/tests.rs (build_attribute), not ported yet"]
fn test_attr_cfg_none() {}

// Java: StarlarkRuleClassFunctionsTest.testRuleCfg_starlark
#[test]
#[ignore = "needs analysis (getConfiguredTarget and the providers or actions of the configured target)"]
fn test_rule_cfg_starlark() {}

// Java: StarlarkRuleClassFunctionsTest.testAttrDoc
#[test]
#[ignore = "inspects the built Attribute (buildAttribute); portable with the .bzl harness of starlark_defs/tests.rs (build_attribute), not ported yet"]
fn test_attr_doc() {}

// Java: StarlarkRuleClassFunctionsTest.testRuleImplementation
#[test]
#[ignore = "needs analysis (getConfiguredTarget and the providers or actions of the configured target)"]
fn test_rule_implementation() {}

// Java: StarlarkRuleClassFunctionsTest.testRuleDoc
#[test]
#[ignore = "inspects the RuleClass behind ev.lookup (getRuleClass); portable with the .bzl harness of starlark_defs/tests.rs (get_rule_class), not ported yet"]
fn test_rule_doc() {}

// Java: StarlarkRuleClassFunctionsTest.testStarlarkLabelAndName
#[test]
#[ignore = "needs analysis (createRuleContext: the StarlarkRuleContext of a configured target)"]
fn test_starlark_label_and_name() {}

// Java: StarlarkRuleClassFunctionsTest.testRuleAddAttribute
#[test]
#[ignore = "inspects the RuleClass behind ev.lookup (getRuleClass); portable with the .bzl harness of starlark_defs/tests.rs (get_rule_class), not ported yet"]
fn test_rule_add_attribute() {}

// Java: StarlarkRuleClassFunctionsTest.testOutputToGenfiles
#[test]
#[ignore = "inspects the RuleClass behind ev.lookup (getRuleClass); portable with the .bzl harness of starlark_defs/tests.rs (get_rule_class), not ported yet"]
fn test_output_to_genfiles() {}

// Java: StarlarkRuleClassFunctionsTest.testRuleAddMultipleAttributes
#[test]
#[ignore = "inspects the RuleClass behind ev.lookup (getRuleClass); portable with the .bzl harness of starlark_defs/tests.rs (get_rule_class), not ported yet"]
fn test_rule_add_multiple_attributes() {}

// Java: StarlarkRuleClassFunctionsTest.testRuleAttributeFlag
#[test]
#[ignore = "inspects the RuleClass behind ev.lookup (getRuleClass); portable with the .bzl harness of starlark_defs/tests.rs (get_rule_class), not ported yet"]
fn test_rule_attribute_flag() {}

// Java: StarlarkRuleClassFunctionsTest.testRuleOutputs
#[test]
#[ignore = "inspects the RuleClass behind ev.lookup (getRuleClass); portable with the .bzl harness of starlark_defs/tests.rs (get_rule_class), not ported yet"]
fn test_rule_outputs() {}

// Java: StarlarkRuleClassFunctionsTest.testLabel
#[test]
#[ignore = "asserts Java Label.toString() or identity of ev.eval results, which Starlark str(Label) does not show"]
fn test_label() {}

// Java: StarlarkRuleClassFunctionsTest.testLabelIdempotence
#[test]
#[ignore = "asserts Java Label.toString() or identity of ev.eval results, which Starlark str(Label) does not show"]
fn test_label_idempotence() {}

// Java: StarlarkRuleClassFunctionsTest.testLabelSameInstance
#[test]
#[ignore = "asserts Java Label.toString() or identity of ev.eval results, which Starlark str(Label) does not show"]
fn test_label_same_instance() {}

// Java: StarlarkRuleClassFunctionsTest.testRuleLabelDefaultValue
#[test]
#[ignore = "inspects the RuleClass behind ev.lookup (getRuleClass); portable with the .bzl harness of starlark_defs/tests.rs (get_rule_class), not ported yet"]
fn test_rule_label_default_value() {}

// Java: StarlarkRuleClassFunctionsTest.testIntDefaultValue
#[test]
#[ignore = "inspects the RuleClass behind ev.lookup (getRuleClass); portable with the .bzl harness of starlark_defs/tests.rs (get_rule_class), not ported yet"]
fn test_int_default_value() {}

// Java: StarlarkRuleClassFunctionsTest.testIntAttributeValueMustBeInt32
#[test]
#[ignore = "needs analysis (createRuleContext: the StarlarkRuleContext of a configured target)"]
fn test_int_attribute_value_must_be_int32() {}

// Java: StarlarkRuleClassFunctionsTest.testIntegerConcatTruncates
#[test]
#[ignore = "needs analysis (createRuleContext: the StarlarkRuleContext of a configured target)"]
fn test_integer_concat_truncates() {}

// Java: StarlarkRuleClassFunctionsTest.testRuleInheritsBaseRuleAttributes
#[test]
#[ignore = "inspects the RuleClass behind ev.lookup (getRuleClass); portable with the .bzl harness of starlark_defs/tests.rs (get_rule_class), not ported yet"]
fn test_rule_inherits_base_rule_attributes() {}

// Java: StarlarkRuleClassFunctionsTest.testJsonFileEncoding
#[test]
#[ignore = "needs analysis (createRuleContext: the StarlarkRuleContext of a configured target)"]
fn test_json_file_encoding() {}

// Java: StarlarkRuleClassFunctionsTest.testLabelGetRelative
#[test]
#[ignore = "asserts Java Label.toString() or identity of ev.eval results, which Starlark str(Label) does not show"]
fn test_label_get_relative() {}

// Java: StarlarkRuleClassFunctionsTest.testStructCreation
#[test]
#[ignore = "inspects Java values behind ev.lookup (StarlarkProvider key and schema, StructImpl provider and field names, Depset); not ported yet"]
fn test_struct_creation() {}

// Java: StarlarkRuleClassFunctionsTest.testStructConcatenationFieldNames
#[test]
#[ignore = "inspects Java values behind ev.lookup (StarlarkProvider key and schema, StructImpl provider and field names, Depset); not ported yet"]
fn test_struct_concatenation_field_names() {}

// Java: StarlarkRuleClassFunctionsTest.testDepsetGoodCompositeItem
#[test]
#[ignore = "inspects Java values behind ev.lookup (StarlarkProvider key and schema, StructImpl provider and field names, Depset); not ported yet"]
fn test_depset_good_composite_item() {}

// Java: StarlarkRuleClassFunctionsTest.testStructMutabilityShallow
#[test]
#[ignore = "tests Java Starlark.isImmutable on hand-built StructImpl values (no .bzl code)"]
fn test_struct_mutability_shallow() {}

// Java: StarlarkRuleClassFunctionsTest.testStructMutabilityDeep
#[test]
#[ignore = "tests Java Starlark.isImmutable on hand-built StructImpl values (no .bzl code)"]
fn test_struct_mutability_deep() {}

// Java: StarlarkRuleClassFunctionsTest.declaredProviders
#[test]
#[ignore = "inspects Java values behind ev.lookup (StarlarkProvider key and schema, StructImpl provider and field names, Depset); not ported yet"]
fn declared_providers() {}

// Java: StarlarkRuleClassFunctionsTest.declaredProviderDocumentation
#[test]
#[ignore = "inspects Java values behind ev.lookup (StarlarkProvider key and schema, StructImpl provider and field names, Depset); not ported yet"]
fn declared_provider_documentation() {}

// Java: StarlarkRuleClassFunctionsTest.declaredProvidersWithInit
#[test]
#[ignore = "inspects Java values behind ev.lookup (StarlarkProvider key and schema, StructImpl provider and field names, Depset); not ported yet"]
fn declared_providers_with_init() {}

// Java: StarlarkRuleClassFunctionsTest.declaredProvidersWithFailingInit_rawConstructorSucceeds
#[test]
#[ignore = "inspects Java values behind ev.lookup (StarlarkProvider key and schema, StructImpl provider and field names, Depset); not ported yet"]
fn declared_providers_with_failing_init_raw_constructor_succeeds() {}

// Java: StarlarkRuleClassFunctionsTest.declaredProvidersConcatSuccess
#[test]
#[ignore = "inspects Java values behind ev.lookup (StarlarkProvider key and schema, StructImpl provider and field names, Depset); not ported yet"]
fn declared_providers_concat_success() {}

// Java: StarlarkRuleClassFunctionsTest.declaredProvidersWithInitConcatSuccess
#[test]
#[ignore = "inspects Java values behind ev.lookup (StarlarkProvider key and schema, StructImpl provider and field names, Depset); not ported yet"]
fn declared_providers_with_init_concat_success() {}

// Java: StarlarkRuleClassFunctionsTest.structsAsDeclaredProvidersTest
#[test]
#[ignore = "inspects Java values behind ev.lookup (StarlarkProvider key and schema, StructImpl provider and field names, Depset); not ported yet"]
fn structs_as_declared_providers_test() {}

// Java: StarlarkRuleClassFunctionsTest.aspectAllAttrs
#[test]
#[ignore = "inspects the StarlarkDefinedAspect or the attribute aspects behind ev.lookup; portable with the .bzl harness of starlark_defs/tests.rs, not ported yet"]
fn aspect_all_attrs() {}

// Java: StarlarkRuleClassFunctionsTest.aspectEmptyAttrs
#[test]
#[ignore = "inspects the StarlarkDefinedAspect or the attribute aspects behind ev.lookup; portable with the .bzl harness of starlark_defs/tests.rs, not ported yet"]
fn aspect_empty_attrs() {}

// Java: StarlarkRuleClassFunctionsTest.aspectDefaultAttrs
#[test]
#[ignore = "inspects the StarlarkDefinedAspect or the attribute aspects behind ev.lookup; portable with the .bzl harness of starlark_defs/tests.rs, not ported yet"]
fn aspect_default_attrs() {}

// Java: StarlarkRuleClassFunctionsTest.aspectRequiredAspectProvidersSingle
#[test]
#[ignore = "inspects the StarlarkDefinedAspect or the attribute aspects behind ev.lookup; portable with the .bzl harness of starlark_defs/tests.rs, not ported yet"]
fn aspect_required_aspect_providers_single() {}

// Java: StarlarkRuleClassFunctionsTest.aspectRequiredAspectProvidersAlternatives
#[test]
#[ignore = "inspects the StarlarkDefinedAspect or the attribute aspects behind ev.lookup; portable with the .bzl harness of starlark_defs/tests.rs, not ported yet"]
fn aspect_required_aspect_providers_alternatives() {}

// Java: StarlarkRuleClassFunctionsTest.aspectRequiredAspectProvidersEmpty
#[test]
#[ignore = "inspects the StarlarkDefinedAspect or the attribute aspects behind ev.lookup; portable with the .bzl harness of starlark_defs/tests.rs, not ported yet"]
fn aspect_required_aspect_providers_empty() {}

// Java: StarlarkRuleClassFunctionsTest.aspectRequiredAspectProvidersDefault
#[test]
#[ignore = "inspects the StarlarkDefinedAspect or the attribute aspects behind ev.lookup; portable with the .bzl harness of starlark_defs/tests.rs, not ported yet"]
fn aspect_required_aspect_providers_default() {}

// Java: StarlarkRuleClassFunctionsTest.aspectRequiredProvidersSingle
#[test]
#[ignore = "inspects the StarlarkDefinedAspect or the attribute aspects behind ev.lookup; portable with the .bzl harness of starlark_defs/tests.rs, not ported yet"]
fn aspect_required_providers_single() {}

// Java: StarlarkRuleClassFunctionsTest.aspectRequiredProvidersAlternatives
#[test]
#[ignore = "inspects the StarlarkDefinedAspect or the attribute aspects behind ev.lookup; portable with the .bzl harness of starlark_defs/tests.rs, not ported yet"]
fn aspect_required_providers_alternatives() {}

// Java: StarlarkRuleClassFunctionsTest.aspectRequiredProvidersEmpty
#[test]
#[ignore = "inspects the StarlarkDefinedAspect or the attribute aspects behind ev.lookup; portable with the .bzl harness of starlark_defs/tests.rs, not ported yet"]
fn aspect_required_providers_empty() {}

// Java: StarlarkRuleClassFunctionsTest.aspectRequiredProvidersDefault
#[test]
#[ignore = "inspects the StarlarkDefinedAspect or the attribute aspects behind ev.lookup; portable with the .bzl harness of starlark_defs/tests.rs, not ported yet"]
fn aspect_required_providers_default() {}

// Java: StarlarkRuleClassFunctionsTest.aspectProvides
#[test]
#[ignore = "inspects the StarlarkDefinedAspect or the attribute aspects behind ev.lookup; portable with the .bzl harness of starlark_defs/tests.rs, not ported yet"]
fn aspect_provides() {}

// Java: StarlarkRuleClassFunctionsTest.aspectDoc
#[test]
#[ignore = "inspects the StarlarkDefinedAspect or the attribute aspects behind ev.lookup; portable with the .bzl harness of starlark_defs/tests.rs, not ported yet"]
fn aspect_doc() {}

// Java: StarlarkRuleClassFunctionsTest.fancyExports
#[test]
#[ignore = "inspects the StarlarkDefinedAspect or the attribute aspects behind ev.lookup; portable with the .bzl harness of starlark_defs/tests.rs, not ported yet"]
fn fancy_exports() {}

// Java: StarlarkRuleClassFunctionsTest.multipleTopLevels
#[test]
#[ignore = "inspects Java values behind ev.lookup (StarlarkProvider key and schema, StructImpl provider and field names, Depset); not ported yet"]
fn multiple_top_levels() {}

// Java: StarlarkRuleClassFunctionsTest.providerWithFields
#[test]
#[ignore = "inspects Java values behind ev.lookup (StarlarkProvider key and schema, StructImpl provider and field names, Depset); not ported yet"]
fn provider_with_fields() {}

// Java: StarlarkRuleClassFunctionsTest.providerWithFieldsDict
#[test]
#[ignore = "inspects Java values behind ev.lookup (StarlarkProvider key and schema, StructImpl provider and field names, Depset); not ported yet"]
fn provider_with_fields_dict() {}

// Java: StarlarkRuleClassFunctionsTest.providerWithFieldsOptional
#[test]
#[ignore = "inspects Java values behind ev.lookup (StarlarkProvider key and schema, StructImpl provider and field names, Depset); not ported yet"]
fn provider_with_fields_optional() {}

// Java: StarlarkRuleClassFunctionsTest.testMandatoryConfigParameterForExecutableLabels
#[test]
#[ignore = "needs analysis (createRuleContext: the StarlarkRuleContext of a configured target)"]
fn test_mandatory_config_parameter_for_executable_labels() {}

// Java: StarlarkRuleClassFunctionsTest.testRuleAddToolchain_duplicate
#[test]
#[ignore = "inspects the RuleClass behind ev.lookup (getRuleClass); portable with the .bzl harness of starlark_defs/tests.rs (get_rule_class), not ported yet"]
fn test_rule_add_toolchain_duplicate() {}

// Java: StarlarkRuleClassFunctionsTest.testRuleAddExecGroup
#[test]
#[ignore = "inspects the RuleClass behind ev.lookup (getRuleClass); portable with the .bzl harness of starlark_defs/tests.rs (get_rule_class), not ported yet"]
fn test_rule_add_exec_group() {}

// Java: StarlarkRuleClassFunctionsTest.testRuleOrderedRequirements
#[test]
#[ignore = "inspects the RuleClass behind ev.lookup (getRuleClass); portable with the .bzl harness of starlark_defs/tests.rs (get_rule_class), not ported yet"]
fn test_rule_ordered_requirements() {}

// Java: StarlarkRuleClassFunctionsTest.testRuleFunctionReturnsNone
#[test]
#[ignore = "needs analysis (createRuleContext: the StarlarkRuleContext of a configured target)"]
fn test_rule_function_returns_none() {}

// Java: StarlarkRuleClassFunctionsTest.ruleDefinitionEnvironmentDigest_unaffectedByTargetAttrValueChange
#[test]
#[ignore = "needs analysis (createRuleContext: the StarlarkRuleContext of a configured target)"]
fn rule_definition_environment_digest_unaffected_by_target_attr_value_change() {}

// Java: StarlarkRuleClassFunctionsTest.ruleDefinitionEnvironmentDigest_accountsForFunctionWhenCreatingRuleWithAMacro
#[test]
#[ignore = "symbolic macros are not ported (MacroClass.instantiateAndAddMacro)"]
fn rule_definition_environment_digest_accounts_for_function_when_creating_rule_with_a_macro() {}

// Java: StarlarkRuleClassFunctionsTest.ruleDefinitionEnvironmentDigest_accountsForAttrsWhenCreatingRuleWithMacro
#[test]
#[ignore = "symbolic macros are not ported (MacroClass.instantiateAndAddMacro)"]
fn rule_definition_environment_digest_accounts_for_attrs_when_creating_rule_with_macro() {}

// Java: StarlarkRuleClassFunctionsTest.testAttrWithAspectRequiringAspects_requiredNativeAspect_getsParamsFromBaseRules
#[test]
#[ignore = "needs analysis (createRuleContext: the StarlarkRuleContext of a configured target)"]
fn test_attr_with_aspect_requiring_aspects_required_native_aspect_gets_params_from_base_rules() {}

// Java: StarlarkRuleClassFunctionsTest.initializer_basic
#[test]
#[ignore = "needs analysis (getConfiguredTarget and the providers or actions of the configured target)"]
fn initializer_basic() {}

// Java: StarlarkRuleClassFunctionsTest.initializer_stringListDict
#[test]
#[ignore = "needs analysis (getConfiguredTarget and the providers or actions of the configured target)"]
fn initializer_string_list_dict() {}

// Java: StarlarkRuleClassFunctionsTest.stringKeyedLabelDictWithSplitConfiguration
#[test]
#[ignore = "needs analysis (getConfiguredTarget and the providers or actions of the configured target)"]
fn string_keyed_label_dict_with_split_configuration() {}

// Java: StarlarkRuleClassFunctionsTest.stringKeyedLabelDict
#[test]
#[ignore = "needs analysis (getConfiguredTarget and the providers or actions of the configured target)"]
fn string_keyed_label_dict() {}

// Java: StarlarkRuleClassFunctionsTest.initializer_labelKeyedStringDict
#[test]
#[ignore = "needs analysis (getConfiguredTarget and the providers or actions of the configured target)"]
fn initializer_label_keyed_string_dict() {}

// Java: StarlarkRuleClassFunctionsTest.labelListDict
#[test]
#[ignore = "needs analysis (getConfiguredTarget and the providers or actions of the configured target)"]
fn label_list_dict() {}

// Java: StarlarkRuleClassFunctionsTest.labelListDictWithSplitConfiguration
#[test]
#[ignore = "needs analysis (getConfiguredTarget and the providers or actions of the configured target)"]
fn label_list_dict_with_split_configuration() {}

// Java: StarlarkRuleClassFunctionsTest.initializer_labelListDict
#[test]
#[ignore = "needs analysis (getConfiguredTarget and the providers or actions of the configured target)"]
fn initializer_label_list_dict() {}

// Java: StarlarkRuleClassFunctionsTest.initializer_legacyAnyType
#[test]
#[ignore = "needs analysis (getConfiguredTarget and the providers or actions of the configured target)"]
fn initializer_legacy_any_type() {}

// Java: StarlarkRuleClassFunctionsTest.initializer_withSelect
#[test]
#[ignore = "needs analysis (getConfiguredTarget and the providers or actions of the configured target)"]
fn initializer_with_select() {}

// Java: StarlarkRuleClassFunctionsTest.initializer_overridesAttributeDefault
#[test]
#[ignore = "needs analysis (getConfiguredTarget and the providers or actions of the configured target)"]
fn initializer_overrides_attribute_default() {}

// Java: StarlarkRuleClassFunctionsTest.initializer_returningNoneSetsDefault
#[test]
#[ignore = "needs analysis (getConfiguredTarget and the providers or actions of the configured target)"]
fn initializer_returning_none_sets_default() {}

// Java: StarlarkRuleClassFunctionsTest.extendRule_allowlist
#[test]
#[ignore = "loading-time rule(parent = ...) error asserted through getConfiguredTarget; needs the scratchParentRule fixture, not ported yet"]
fn extend_rule_allowlist() {}

// Java: StarlarkRuleClassFunctionsTest.extendRule_ccBinary
#[test]
#[ignore = "needs analysis (getConfiguredTarget and the providers or actions of the configured target)"]
fn extend_rule_cc_binary() {}

// Java: StarlarkRuleClassFunctionsTest.extendRule_basicUse
#[test]
#[ignore = "needs analysis (createRuleContext: the StarlarkRuleContext of a configured target)"]
fn extend_rule_basic_use() {}

// Java: StarlarkRuleClassFunctionsTest.extendRule_withInitializers
#[test]
#[ignore = "needs analysis (createRuleContext: the StarlarkRuleContext of a configured target)"]
fn extend_rule_with_initializers() {}

// Java: StarlarkRuleClassFunctionsTest.extendRule_superNotCalled
#[test]
#[ignore = "needs analysis (getConfiguredTarget); the asserted error is reported by analysis or was not established to be a loading error"]
fn extend_rule_super_not_called() {}

// Java: StarlarkRuleClassFunctionsTest.extendRule_superCalledTwice
#[test]
#[ignore = "needs analysis (getConfiguredTarget); the asserted error is reported by analysis or was not established to be a loading error"]
fn extend_rule_super_called_twice() {}

// Java: StarlarkRuleClassFunctionsTest.extendRule_noParent_superCalled
#[test]
#[ignore = "needs analysis (getConfiguredTarget); the asserted error is reported by analysis or was not established to be a loading error"]
fn extend_rule_no_parent_super_called() {}

// Java: StarlarkRuleClassFunctionsTest.extendRule_extendRuleTwice
#[test]
#[ignore = "needs analysis (getConfiguredTarget and the providers or actions of the configured target)"]
fn extend_rule_extend_rule_twice() {}

// Java: StarlarkRuleClassFunctionsTest.extendRule_extendRuleTwice_superNotCalled
#[test]
#[ignore = "needs analysis (getConfiguredTarget); the asserted error is reported by analysis or was not established to be a loading error"]
fn extend_rule_extend_rule_twice_super_not_called() {}

// Java: StarlarkRuleClassFunctionsTest.ctxSuper_calledFromAspect
#[test]
#[ignore = "needs analysis (getConfiguredTarget); the asserted error is reported by analysis or was not established to be a loading error"]
fn ctx_super_called_from_aspect() {}

// Java: StarlarkRuleClassFunctionsTest.extendRule_attributeAdditionalAspects
#[test]
#[ignore = "needs analysis (createRuleContext: the StarlarkRuleContext of a configured target)"]
fn extend_rule_attribute_additional_aspects() {}

// Java: StarlarkRuleClassFunctionsTest.testAspectWithInvalidToolchainsAspects_fails
#[test]
#[ignore = "needs analysis (getConfiguredTarget); the asserted error is reported by analysis or was not established to be a loading error"]
fn test_aspect_with_invalid_toolchains_aspects_fails() {}

// Java: StarlarkRuleClassFunctionsTest.testAspectWithValidToolchainsAspects_parsesCorrectly
#[test]
#[ignore = "inspects the StarlarkDefinedAspect or the attribute aspects behind ev.lookup; portable with the .bzl harness of starlark_defs/tests.rs, not ported yet"]
fn test_aspect_with_valid_toolchains_aspects_parses_correctly() {}

// Java: StarlarkRuleClassFunctionsTest.testAspectWithWildcardToolchainsAspects_parsesCorrectly
#[test]
#[ignore = "inspects the StarlarkDefinedAspect or the attribute aspects behind ev.lookup; portable with the .bzl harness of starlark_defs/tests.rs, not ported yet"]
fn test_aspect_with_wildcard_toolchains_aspects_parses_correctly() {}

// Java: StarlarkRuleClassFunctionsTest.toolchainsAspectsDefault_emptyList
#[test]
#[ignore = "inspects the StarlarkDefinedAspect or the attribute aspects behind ev.lookup; portable with the .bzl harness of starlark_defs/tests.rs, not ported yet"]
fn toolchains_aspects_default_empty_list() {}

// Java: StarlarkRuleClassFunctionsTest.extendRule_overridePrivateAttribute_fails
#[test]
#[ignore = "needs analysis (getConfiguredTarget); the asserted error is reported by analysis or was not established to be a loading error"]
fn extend_rule_override_private_attribute_fails() {}

// Java: StarlarkRuleClassFunctionsTest.extendRule_parentPrivateAttrDefault_visibilityCheckedAgainstParent
#[test]
#[ignore = "needs analysis (getConfiguredTarget and the providers or actions of the configured target)"]
fn extend_rule_parent_private_attr_default_visibility_checked_against_parent() {}

// Java: StarlarkRuleClassFunctionsTest.extendRule_attributeOverrideDefault
#[test]
#[ignore = "needs analysis (getConfiguredTarget and the providers or actions of the configured target)"]
fn extend_rule_attribute_override_default() {}

// Java: StarlarkRuleClassFunctionsTest.extendRule_attributeCollision
#[test]
#[ignore = "loading-time rule(parent = ...) error asserted through getConfiguredTarget; needs the scratchParentRule fixture, not ported yet"]
fn extend_rule_attribute_collision() {}

// Java: StarlarkRuleClassFunctionsTest.extendRule_executableMatches
#[test]
#[ignore = "needs analysis (createRuleContext: the StarlarkRuleContext of a configured target)"]
fn extend_rule_executable_matches() {}

// Java: StarlarkRuleClassFunctionsTest.extendRule_testMatches
#[test]
#[ignore = "needs analysis (createRuleContext: the StarlarkRuleContext of a configured target)"]
fn extend_rule_test_matches() {}

// Java: StarlarkRuleClassFunctionsTest.extendRule_controlledParameters_fail
#[test]
#[ignore = "needs analysis (BuildViewTestCase.update with a Java-registered implementation)"]
fn extend_rule_controlled_parameters_fail() {}

// Java: StarlarkRuleClassFunctionsTest.extendRule_fragments_merged
#[test]
#[ignore = "needs analysis (createRuleContext: the StarlarkRuleContext of a configured target)"]
fn extend_rule_fragments_merged() {}

// Java: StarlarkRuleClassFunctionsTest.extendRule_notExtendable
#[test]
#[ignore = "needs analysis (BuildViewTestCase.update with a Java-registered implementation)"]
fn extend_rule_not_extendable() {}

// Java: StarlarkRuleClassFunctionsTest.extendRule_extendableAllowed
#[test]
#[ignore = "needs analysis (getConfiguredTarget and the providers or actions of the configured target)"]
fn extend_rule_extendable_allowed() {}

// Java: StarlarkRuleClassFunctionsTest.extendRule_extendableAllowlisted
#[test]
#[ignore = "needs analysis (getConfiguredTarget and the providers or actions of the configured target)"]
fn extend_rule_extendable_allowlisted() {}

// Java: StarlarkRuleClassFunctionsTest.extendRule_extendableAllowlistDenied
#[test]
#[ignore = "needs analysis (getConfiguredTarget); the asserted error is reported by analysis or was not established to be a loading error"]
fn extend_rule_extendable_allowlist_denied() {}

// Java: StarlarkRuleClassFunctionsTest.extendRule_extendableDefault
#[test]
#[ignore = "needs analysis (getConfiguredTarget); the asserted error is reported by analysis or was not established to be a loading error"]
fn extend_rule_extendable_default() {}

// Java: StarlarkRuleClassFunctionsTest.extendRule_toolchains_merged
#[test]
#[ignore = "needs analysis (createRuleContext: the StarlarkRuleContext of a configured target)"]
fn extend_rule_toolchains_merged() {}

// Java: StarlarkRuleClassFunctionsTest.extendRule_advertisedProviders_merged
#[test]
#[ignore = "needs analysis (createRuleContext: the StarlarkRuleContext of a configured target)"]
fn extend_rule_advertised_providers_merged() {}

// Java: StarlarkRuleClassFunctionsTest.extendRule_execCompatibleWith_merged
#[test]
#[ignore = "needs analysis (createRuleContext: the StarlarkRuleContext of a configured target)"]
fn extend_rule_exec_compatible_with_merged() {}

// Java: StarlarkRuleClassFunctionsTest.extendRule_execGroups_merged
#[test]
#[ignore = "needs analysis (createRuleContext: the StarlarkRuleContext of a configured target)"]
fn extend_rule_exec_groups_merged() {}

// Java: StarlarkRuleClassFunctionsTest.extendRule_execGroups_overwritten
#[test]
#[ignore = "inspects the RuleClass behind ev.lookup (getRuleClass); portable with the .bzl harness of starlark_defs/tests.rs (get_rule_class), not ported yet"]
fn extend_rule_exec_groups_overwritten() {}

// Java: StarlarkRuleClassFunctionsTest.extendRule_cfg_fromParent
#[test]
#[ignore = "needs analysis (getConfiguredTarget and the providers or actions of the configured target)"]
fn extend_rule_cfg_from_parent() {}

// Java: StarlarkRuleClassFunctionsTest.extendRule_cfg_onChild
#[test]
#[ignore = "needs analysis (getConfiguredTarget and the providers or actions of the configured target)"]
fn extend_rule_cfg_on_child() {}

// Java: StarlarkRuleClassFunctionsTest.extendRule_cfg_onChildAndFromParent
#[test]
#[ignore = "needs analysis (getConfiguredTarget and the providers or actions of the configured target)"]
fn extend_rule_cfg_on_child_and_from_parent() {}

// Java: StarlarkRuleClassFunctionsTest.testAnalysisTest
#[test]
#[ignore = "symbolic macros are not ported (MacroClass.instantiateAndAddMacro)"]
fn test_analysis_test() {}

// Java: StarlarkRuleClassFunctionsTest.testAnalysisTestAttrs
#[test]
#[ignore = "symbolic macros are not ported (MacroClass.instantiateAndAddMacro)"]
fn test_analysis_test_attrs() {}

// Java: StarlarkRuleClassFunctionsTest.testAnalysisTestDuplicateName_samePackage
#[test]
#[ignore = "testing.analysis_test rule creation is not exercised by the loading harness; not ported yet"]
fn test_analysis_test_duplicate_name_same_package() {}

// Java: StarlarkRuleClassFunctionsTest.testAnalysisTestDuplicateName_differentAttrs_differentPackage
#[test]
#[ignore = "needs analysis (getConfiguredTarget and the providers or actions of the configured target)"]
fn test_analysis_test_duplicate_name_different_attrs_different_package() {}

// Java: StarlarkRuleClassFunctionsTest.testAnalysisTestBadName
#[test]
#[ignore = "symbolic macros are not ported (MacroClass.instantiateAndAddMacro)"]
fn test_analysis_test_bad_name() {}

// Java: StarlarkRuleClassFunctionsTest.testAnalysisTestBadArgs
#[test]
#[ignore = "symbolic macros are not ported (MacroClass.instantiateAndAddMacro)"]
fn test_analysis_test_bad_args() {}

// Java: StarlarkRuleClassFunctionsTest.testAnalysisTestErrorOnExport
#[test]
#[ignore = "symbolic macros are not ported (MacroClass.instantiateAndAddMacro)"]
fn test_analysis_test_error_on_export() {}

// Java: StarlarkRuleClassFunctionsTest.testAnalysisTestErrorOverridingName
#[test]
#[ignore = "symbolic macros are not ported (MacroClass.instantiateAndAddMacro)"]
fn test_analysis_test_error_overriding_name() {}

// Java: StarlarkRuleClassFunctionsTest.testLabelWithStrictVisibility
#[test]
#[ignore = "needs a .bzl module with a hand-made BazelModuleContext and RepositoryMapping (Module.withPredeclaredAndData)"]
fn test_label_with_strict_visibility() {}

// Java: StarlarkRuleClassFunctionsTest.testPackageMetadataAttrOnAllRules
#[test]
#[ignore = "needs analysis (the assertion is on what the rule implementation prints from ctx.attr.package_metadata)"]
fn test_package_metadata_attr_on_all_rules() {}

// Java: StarlarkRuleClassFunctionsTest.starlarkRuleFunctionCodec
#[test]
#[ignore = "needs the Skyframe serialization codecs (RoundTripping)"]
fn starlark_rule_function_codec() {}
