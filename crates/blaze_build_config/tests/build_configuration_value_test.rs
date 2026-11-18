//! Port of
//! `src/test/java/com/google/devtools/build/lib/analysis/config/BuildConfigurationValueTest.java`.
//!
//! The Java class extends `ConfigurationTestCase`: almost every test creates its configuration
//! through `create(...)`/`createExec(...)`, which run the configuration SkyFunctions of a mock
//! analysis environment (`AnalysisMock`). Those tests are kept as ignored tests.

use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_options::Options;
use blaze_options::definition::OptionsBase;

#[test]
fn test_basics() {
    // `if (analysisMock.isThisBazel()) return;`: Bazel's AnalysisMock is "this Bazel", so the Java
    // test returns before any assertion.
}

#[test]
fn test_platform_suffix() {
    // `if (analysisMock.isThisBazel()) return;`: Bazel's AnalysisMock is "this Bazel", so the Java
    // test returns before any assertion.
}

#[test]
fn test_environment() {
    // `if (analysisMock.isThisBazel()) return;`: Bazel's AnalysisMock is "this Bazel", so the Java
    // test returns before any assertion.
}

#[test]
fn test_caching() {
    let a = Options::<CoreOptions>::get_defaults();
    let b = Options::<CoreOptions>::get_defaults();
    let a: &dyn OptionsBase = &a;
    let b: &dyn OptionsBase = &b;
    // The String representations of the CoreOptions must be equal even if these are
    // different objects, if they were created with the same options (no options in this case).
    assert_eq!(b.java_to_string(), a.java_to_string());
    assert_eq!(b.cache_key(), a.cache_key());
}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock)"]
fn test_target_environment() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock)"]
fn test_global_make_variable_override() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock) and TestConstants.PLATFORM_LABEL"]
fn test_target_cpu_from_cpu_flag() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock) and TestConstants.PLATFORM_LABEL"]
fn test_target_cpu_from_platform() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock) and TestConstants.PLATFORM_LABEL"]
fn test_target_cpu_from_platform_with_cpu_mapping() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock) and TestConstants.PLATFORM_LABEL"]
fn test_target_cpu_from_platform_multiple_cpu_mappings_last_one_wins() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock) and TestConstants.PIII_PLATFORM_LABEL"]
fn test_exec_config_target_cpu_from_platform() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock) and TestConstants.PIII_PLATFORM_LABEL"]
fn test_exec_config_target_cpu_from_platform_with_cpu_mapping() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock) and BuildConfigurationValue.getBuildOptionDetails"]
fn test_get_build_option_details() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock) and a caching FragmentFactory (the Rust FragmentFactory does not cache fragments)"]
fn test_config_fragments_are_shareable_across_configurations() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock)"]
fn test_command_line_variables() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock)"]
fn test_command_line_variables_override() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock)"]
fn test_normalization_defines_with_different_names() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock)"]
fn test_normalization_defines_with_same_name() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock)"]
fn test_command_line_variables_with_funny_characters() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock)"]
fn test_exec_define_is_allowed_by_default() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock)"]
fn test_exec_define_is_ignored_if_excluded_and_not_allowed() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock)"]
fn test_exec_define_is_propagated_if_allowed_by_flag() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock), scratch BUILD/bzl files and Starlark build settings"]
fn test_exec_starlark_flag_is_allowed_by_default() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock), scratch BUILD/bzl files and Starlark build settings"]
fn test_exec_starlark_flag_is_ignored_if_excluded_and_not_allowed() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock), scratch BUILD/bzl files and Starlark build settings"]
fn test_exec_starlark_flag_is_propagated_if_allowed_by_flag() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock), scratch BUILD/bzl files and Starlark build settings"]
fn test_exec_starlark_flag_is_propagated_by_target_pattern() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock), scratch BUILD/bzl files, Starlark build settings and AnalysisTestUtil.execOptions"]
fn starlark_flag_exec_scopes() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock), scratch BUILD/bzl files and Starlark build settings"]
fn starlark_flag_exec_scopes_take_precedence_over_custom_overrides() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock), scratch BUILD files and label_flag build settings"]
fn label_flag_exec_scopes() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock)"]
fn test_host_compilation_mode_default() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock)"]
fn test_host_compilation_mode_non_default() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock)"]
fn test_incompatible_merge_genfiles_directory() {}

#[test]
#[ignore = "needs serialization (SerializationTester) and needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock)"]
fn test_codec() {}

#[test]
#[ignore = "needs serialization (SerializationTester) and needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock)"]
fn test_key_codec() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock)"]
fn test_platform_in_output_dir_legacy_default_platform() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock) and scratch BUILD files"]
fn test_platform_in_output_dir_legacy_with_platform() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock)"]
fn test_platform_in_output_dir_default_platform() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock) and scratch BUILD files"]
fn test_platform_in_output_dir_with_platform() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock) and scratch BUILD files"]
fn test_platform_in_output_dir_with_platform_and_matching_override() {}

#[test]
#[ignore = "needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock) and scratch BUILD files"]
fn test_platform_in_output_dir_with_platform_and_non_matching_override() {}

#[test]
#[ignore = "needs ConfigurationTestCase.createRaw/parseBuildOptions, EqualsTester and BuildConfigurationValue equality"]
fn test_configuration_equality() {}
