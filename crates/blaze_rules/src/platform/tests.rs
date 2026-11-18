//! Ports of the `rules/platform` JUnit tests (they analyze BUILD files).

// Java: com/google/devtools/build/lib/rules/platform/ConstraintCollectionApiTest.java testConstraintSettings
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_constraint_settings() {}

// Java: com/google/devtools/build/lib/rules/platform/ConstraintCollectionApiTest.java testConstraintValue
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_constraint_value() {}

// Java: com/google/devtools/build/lib/rules/platform/ConstraintCollectionApiTest.java testContraintValue_parent
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_contraint_value_parent() {}

// Java: com/google/devtools/build/lib/rules/platform/ConstraintCollectionApiTest.java testConstraintValue_starlark
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_constraint_value_starlark() {}

// Java: com/google/devtools/build/lib/rules/platform/ConstraintCollectionApiTest.java testGet_defaultConstraintValues
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_get_default_constraint_values() {}

// Java: com/google/devtools/build/lib/rules/platform/ConstraintTest.java testConstraint
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_constraint() {}

// Java: com/google/devtools/build/lib/rules/platform/ConstraintTest.java testConstraint_defaultValue
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_constraint_default_value() {}

// Java: com/google/devtools/build/lib/rules/platform/ConstraintTest.java testConstraint_defaultValue_differentPackageFails
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_constraint_default_value_different_package_fails() {}

// Java: com/google/devtools/build/lib/rules/platform/ConstraintTest.java testConstraint_defaultValue_nonExistentTargetFails
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_constraint_default_value_non_existent_target_fails() {}

// Java: com/google/devtools/build/lib/rules/platform/ConstraintTest.java testConstraint_defaultValue_starlark
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_constraint_default_value_starlark() {}

// Java: com/google/devtools/build/lib/rules/platform/ConstraintTest.java testConstraint_refinesConstraintValue
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_constraint_refines_constraint_value() {}

// Java: com/google/devtools/build/lib/rules/platform/ConstraintTest.java testConstraint_refinesConstraintValue_alias
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_constraint_refines_constraint_value_alias() {}

// Java: com/google/devtools/build/lib/rules/platform/ConstraintTest.java testConstraint_refinesConstraintValue_chain
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_constraint_refines_constraint_value_chain() {}

// Java: com/google/devtools/build/lib/rules/platform/ConstraintTest.java testConstraint_refinesConstraintValue_withDefault
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_constraint_refines_constraint_value_with_default() {}

// Java: com/google/devtools/build/lib/rules/platform/ConstraintTest.java testConstraint_defaultValue_notSet_starlark
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_constraint_default_value_not_set_starlark() {}

// Java: com/google/devtools/build/lib/rules/platform/PlatformConfigurationApiTest.java testHostPlatform
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_host_platform() {}

// Java: com/google/devtools/build/lib/rules/platform/PlatformConfigurationApiTest.java testTargetPlatform_single
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_target_platform_single() {}

// Java: com/google/devtools/build/lib/rules/platform/PlatformInfoApiTest.java constructor
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn constructor() {}

// Java: com/google/devtools/build/lib/rules/platform/PlatformInfoApiTest.java tooManyParentsError
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn too_many_parents_error() {}

// Java: com/google/devtools/build/lib/rules/platform/PlatformInfoApiTest.java constraints_overlappingError
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn constraints_overlapping_error() {}

// Java: com/google/devtools/build/lib/rules/platform/PlatformInfoApiTest.java constraints_repeatedViaAlias
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn constraints_repeated_via_alias() {}

// Java: com/google/devtools/build/lib/rules/platform/PlatformInfoApiTest.java constraints_refinesConstraintValue_satisfied
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn constraints_refines_constraint_value_satisfied() {}

// Java: com/google/devtools/build/lib/rules/platform/PlatformInfoApiTest.java constraints_refinesConstraintValue_alias_satisfied
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn constraints_refines_constraint_value_alias_satisfied() {}

// Java: com/google/devtools/build/lib/rules/platform/PlatformInfoApiTest.java constraints_refinesConstraintValue_missingRefined_error
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn constraints_refines_constraint_value_missing_refined_error() {}

// Java: com/google/devtools/build/lib/rules/platform/PlatformInfoApiTest.java constraints_refinesConstraintValue_wrongValue_error
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn constraints_refines_constraint_value_wrong_value_error() {}

// Java: com/google/devtools/build/lib/rules/platform/PlatformInfoApiTest.java constraints_refinesConstraintValue_defaultNotEnforced
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn constraints_refines_constraint_value_default_not_enforced() {}

// Java: com/google/devtools/build/lib/rules/platform/PlatformInfoApiTest.java constraints_refinesConstraintValue_inheritedFromParent
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn constraints_refines_constraint_value_inherited_from_parent() {}

// Java: com/google/devtools/build/lib/rules/platform/PlatformInfoApiTest.java constraints_invalidTarget_error
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn constraints_invalid_target_error() {}

// Java: com/google/devtools/build/lib/rules/platform/PlatformInfoApiTest.java constraints_parent
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn constraints_parent() {}

// Java: com/google/devtools/build/lib/rules/platform/PlatformInfoApiTest.java constraints_parent_override
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn constraints_parent_override() {}

// Java: com/google/devtools/build/lib/rules/platform/PlatformInfoApiTest.java remoteExecutionProperties
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn remote_execution_properties() {}

// Java: com/google/devtools/build/lib/rules/platform/PlatformInfoApiTest.java remoteExecutionProperties_parent_entireOverride
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn remote_execution_properties_parent_entire_override() {}

// Java: com/google/devtools/build/lib/rules/platform/PlatformInfoApiTest.java remoteExecutionProperties_parent_merge
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn remote_execution_properties_parent_merge() {}

// Java: com/google/devtools/build/lib/rules/platform/PlatformInfoApiTest.java remoteExecutionProperties_parentSpecifiesExecProperties_error
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn remote_execution_properties_parent_specifies_exec_properties_error() {}

// Java: com/google/devtools/build/lib/rules/platform/PlatformInfoApiTest.java execProperties
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn exec_properties() {}

// Java: com/google/devtools/build/lib/rules/platform/PlatformInfoApiTest.java execProperties_conflictingProperties_error
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn exec_properties_conflicting_properties_error() {}

// Java: com/google/devtools/build/lib/rules/platform/PlatformInfoApiTest.java execProperties_parent
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn exec_properties_parent() {}

// Java: com/google/devtools/build/lib/rules/platform/PlatformInfoApiTest.java execProperties_parent_merged
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn exec_properties_parent_merged() {}

// Java: com/google/devtools/build/lib/rules/platform/PlatformInfoApiTest.java execProperties_parentSpecifiesRemoteExecutionProperties_error
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn exec_properties_parent_specifies_remote_execution_properties_error() {}

// Java: com/google/devtools/build/lib/rules/platform/PlatformInfoApiTest.java flags
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn flags() {}

// Java: com/google/devtools/build/lib/rules/platform/PlatformInfoApiTest.java flags_parent
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn flags_parent() {}

// Java: com/google/devtools/build/lib/rules/platform/PlatformInfoApiTest.java flags_parent_merged
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn flags_parent_merged() {}

// Java: com/google/devtools/build/lib/rules/platform/PlatformInfoApiTest.java flags_parent_override
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn flags_parent_override() {}

// Java: com/google/devtools/build/lib/rules/platform/ToolchainTest.java testToolchain
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_toolchain() {}

// Java: com/google/devtools/build/lib/rules/platform/ToolchainTest.java testToolchain_targetSetting_matching
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_toolchain_target_setting_matching() {}

// Java: com/google/devtools/build/lib/rules/platform/ToolchainTest.java testToolchain_targetSetting_nonmatching
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_toolchain_target_setting_nonmatching() {}

// Java: com/google/devtools/build/lib/rules/platform/ToolchainTest.java ruleDefinitionIncorrectlyDependsOnToolchainInstance
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn rule_definition_incorrectly_depends_on_toolchain_instance() {}
