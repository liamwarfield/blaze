//! Top-level configuration creation from command options: `BlazeRuntime.createBuildOptions`
//! (BuildRequest -> BuildOptions) and the `InfoCommand`/`BuildView` sequence
//! (`setBaselineConfiguration`, then `createConfiguration`/`getConfiguration`).

use std::sync::Arc;

use blaze_events::ExtendedEventHandler;
use blaze_optdefs::command::build_language_options::{BuildLanguageOptions, Utf8EnforcementMode};
use blaze_options::OptionsParser;
use blaze_starlark::StarlarkSemantics;
use blaze_vfs::Path;

use crate::bazel_rule_class_provider::BazelRuleClassProvider;
use crate::build_configuration_value::BuildConfigurationValue;
use crate::build_options::BuildOptions;
use crate::fragment::InvalidConfigurationException;
use crate::output_directories::ConfigDirectories;
use crate::platform::SharedPlatformValueProvider;
use crate::skyframe::{ConfigurationRequest, ConfigurationSkyframe};

// Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:902 BlazeRuntime.createBuildOptions
pub fn create_build_options(
    rule_class_provider: &BazelRuleClassProvider,
    options_provider: &OptionsParser,
) -> BuildOptions {
    BuildOptions::of(rule_class_provider.get_options_classes(), options_provider)
}

/// The effective Starlark semantics of a command (Java `PrecomputedValue.STARLARK_SEMANTICS`,
/// set by `SkyframeExecutor.sync` from the command's `BuildLanguageOptions`).
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1722 SkyframeExecutor.getEffectiveStarlarkSemantics
pub fn starlark_semantics_from(options: &OptionsParser) -> StarlarkSemantics {
    let semantics = match options.get_options::<BuildLanguageOptions>() {
        Some(build_language_options) => to_starlark_semantics(&build_language_options),
        // PORT: Java only syncs package loading for commands that have BuildLanguageOptions; a
        // parser without the class gets the semantics of the option defaults.
        None => {
            to_starlark_semantics(&blaze_options::Options::<BuildLanguageOptions>::get_defaults())
        }
    };
    check_unported_semantics_keys(&semantics);
    semantics
}

/// Semantics keys whose Java readers are not ported anywhere in Blaze, so a non-default value
/// would be accepted and silently ignored (analysis review R2-2). Each entry names the Java
/// readers; setting the key to a non-default value fails loudly instead. Keys with no Java reader
/// at the pin (`EXPERIMENTAL_ENABLE_ANDROID_MIGRATION_APIS`,
/// `EXPERIMENTAL_DISABLE_EXTERNAL_PACKAGE`, `INCOMPATIBLE_DO_NOT_SPLIT_LINKING_CMDLINE`, and the
/// options `--experimental_cc_skylark_api_enabled_packages` / `--experimental_starlark_types`,
/// which reach no key) are no-ops in Bazel too and are not listed.
pub fn check_unported_semantics_keys(semantics: &StarlarkSemantics) {
    let defaults = StarlarkSemantics::default();
    // Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalFunction.java (the
    // only reader; the function is not ported).
    if semantics.incompatible_check_external_repo_source_dir_package_boundary
        != defaults.incompatible_check_external_repo_source_dir_package_boundary
    {
        blaze_util::unported!(
            "RecursiveFilesystemTraversalFunction",
            "--incompatible_check_external_repo_source_dir_package_boundary={}",
            semantics.incompatible_check_external_repo_source_dir_package_boundary
        );
    }
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryModule.java:197
    // StarlarkRepositoryModule.failWithIncompatibleUseCcConfigureFromRulesCc (not ported).
    if semantics.incompatible_use_cc_configure_from_rules_cc
        != defaults.incompatible_use_cc_configure_from_rules_cc
    {
        blaze_util::unported!(
            "StarlarkRepositoryModule.failWithIncompatibleUseCcConfigureFromRulesCc",
            "--incompatible_use_cc_configure_from_rules_cc={}",
            semantics.incompatible_use_cc_configure_from_rules_cc
        );
    }
    // Java: net/starlark/java/lib/json/Json.java, net/starlark/java/eval/StringModule.java,
    // com/google/devtools/build/lib/bazel/bzlmod/FactsAdapter.java read
    // StarlarkSemantics.INTERNAL_BAZEL_ONLY_UTF_8_BYTE_STRINGS; Blaze keeps code-point strings
    // (docs/decisions.md 2026-11-17, byte-string migration pending), which is the default's
    // behaviour for ASCII only.
    if semantics.internal_starlark_utf_8_byte_strings
        != defaults.internal_starlark_utf_8_byte_strings
    {
        blaze_util::unported!(
            "StarlarkSemantics.INTERNAL_BAZEL_ONLY_UTF_8_BYTE_STRINGS",
            "--internal_starlark_utf_8_byte_strings={}",
            semantics.internal_starlark_utf_8_byte_strings
        );
    }
}

// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:997 BuildLanguageOptions.toStarlarkSemantics
/// Constructs a `StarlarkSemantics` object corresponding to this set of option values.
///
/// This function connects command-line flags to their corresponding StarlarkSemantics keys: one
/// assignment per `setFlags` entry (Java
/// `packages/semantics/BuildLanguageOptions.java:890 BuildLanguageOptions.setFlags`), in Java
/// order; each comment names the Java key and its string.
///
/// PORT: `blaze_starlark::StarlarkSemantics` is a struct with one field per key instead of
/// Java's key/value map. The options `--experimental_cc_skylark_api_enabled_packages` and
/// `--experimental_starlark_types` have no semantics key in Java (`setFlags` does not read them),
/// so the struct fields of those names keep their defaults, as does `allow_recursion`
/// (`StarlarkSemantics.ALLOW_RECURSION`, set by tests only).
// One assignment per `setFlags` entry, in Java order.
#[allow(clippy::field_reassign_with_default)]
pub fn to_starlark_semantics(options: &BuildLanguageOptions) -> StarlarkSemantics {
    let mut semantics = StarlarkSemantics::default();
    // INCOMPATIBLE_STOP_EXPORTING_LANGUAGE_MODULES ("-incompatible_stop_exporting_language_modules")
    semantics.incompatible_stop_exporting_language_modules =
        options.incompatible_stop_exporting_language_modules;
    // INCOMPATIBLE_ALLOW_TAGS_PROPAGATION ("+incompatible_allow_tags_propagation")
    semantics.incompatible_allow_tags_propagation = options.experimental_allow_tags_propagation;
    // EXPERIMENTAL_BUILTINS_BZL_PATH.name ("experimental_builtins_bzl_path")
    semantics.experimental_builtins_bzl_path = options.experimental_builtins_bzl_path.clone();
    // INCOMPATIBLE_AUTOLOAD_EXTERNALLY.name ("incompatible_autoload_externally")
    semantics.incompatible_autoload_externally = options.incompatible_autoload_externally.clone();
    // REPOSITORIES_WITHOUT_AUTOLOAD.name ("repositories_without_autoloads")
    semantics.repositories_without_autoloads = options.repositories_without_autoloads.clone();
    // INCOMPATIBLE_DISABLE_AUTOLOADS_IN_MAIN_REPO ("+incompatible_disable_autoloads_in_main_repo")
    semantics.incompatible_disable_autoloads_in_main_repo =
        options.incompatible_disable_autoloads_in_main_repo;
    // EXPERIMENTAL_BUILTINS_DUMMY ("-experimental_builtins_dummy")
    semantics.experimental_builtins_dummy = options.experimental_builtins_dummy;
    // EXPERIMENTAL_BUILTINS_INJECTION_OVERRIDE.name ("experimental_builtins_injection_override")
    // PORT: the semantics field is `None` for the key's default (the empty list).
    semantics.experimental_builtins_injection_override =
        if options.experimental_builtins_injection_override.is_empty() {
            None
        } else {
            Some(options.experimental_builtins_injection_override.clone())
        };
    // EXPERIMENTAL_BZL_VISIBILITY ("+experimental_bzl_visibility")
    semantics.experimental_bzl_visibility = options.experimental_bzl_visibility;
    // ALLOW_EXPERIMENTAL_LOADS ("-allow_experimental_loads")
    semantics.allow_experimental_loads = options.allow_experimental_loads;
    // CHECK_BZL_VISIBILITY ("+check_bzl_visibility")
    semantics.check_bzl_visibility = options.check_bzl_visibility;
    // EXPERIMENTAL_ENABLE_ANDROID_MIGRATION_APIS ("-experimental_enable_android_migration_apis")
    semantics.experimental_enable_android_migration_apis =
        options.experimental_enable_android_migration_apis;
    // EXPERIMENTAL_SINGLE_PACKAGE_TOOLCHAIN_BINDING ("-experimental_single_package_toolchain_binding")
    semantics.experimental_single_package_toolchain_binding =
        options.experimental_single_package_toolchain_binding;
    // EXPERIMENTAL_ENABLE_FIRST_CLASS_MACROS ("+experimental_enable_first_class_macros")
    semantics.experimental_enable_first_class_macros =
        options.experimental_enable_first_class_macros;
    // EXPERIMENTAL_ENABLE_SCL_DIALECT ("+experimental_enable_scl_dialect")
    semantics.experimental_enable_scl_dialect = options.experimental_enable_scl_dialect;
    // EXPERIMENTAL_ISOLATED_EXTENSION_USAGES ("-experimental_isolated_extension_usages")
    semantics.experimental_isolated_extension_usages =
        options.experimental_isolated_extension_usages;
    // INCOMPATIBLE_NO_IMPLICIT_WATCH_LABEL ("+incompatible_no_implicit_watch_label")
    semantics.incompatible_no_implicit_watch_label = options.incompatible_no_implicit_watch_label;
    // EXPERIMENTAL_GOOGLE_LEGACY_API ("-experimental_google_legacy_api")
    semantics.experimental_google_legacy_api = options.experimental_google_legacy_api;
    // EXPERIMENTAL_PLATFORMS_API ("-experimental_platforms_api")
    semantics.experimental_platforms_api = options.experimental_platforms_api;
    // EXPERIMENTAL_CC_SHARED_LIBRARY ("-experimental_cc_shared_library")
    semantics.experimental_cc_shared_library = options.experimental_cc_shared_library;
    // EXPERIMENTAL_REPO_REMOTE_EXEC ("-experimental_repo_remote_exec")
    semantics.experimental_repo_remote_exec = options.experimental_repo_remote_exec;
    // EXPERIMENTAL_DISABLE_EXTERNAL_PACKAGE ("-experimental_disable_external_package")
    semantics.experimental_disable_external_package = options.experimental_disable_external_package;
    // EXPERIMENTAL_SIBLING_REPOSITORY_LAYOUT ("-experimental_sibling_repository_layout")
    semantics.experimental_sibling_repository_layout =
        options.experimental_sibling_repository_layout;
    // INCOMPATIBLE_ALWAYS_CHECK_DEPSET_ELEMENTS ("+incompatible_always_check_depset_elements")
    semantics.incompatible_always_check_depset_elements =
        options.incompatible_always_check_depset_elements;
    // INCOMPATIBLE_CHECK_EXTERNAL_REPO_SOURCE_DIR_PACKAGE_BOUNDARY ("-incompatible_check_external_repo_source_dir_package_boundary")
    semantics.incompatible_check_external_repo_source_dir_package_boundary =
        options.incompatible_check_external_repo_source_dir_package_boundary;
    // INCOMPATIBLE_DISALLOW_EMPTY_GLOB ("+incompatible_disallow_empty_glob")
    semantics.incompatible_disallow_empty_glob = options.incompatible_disallow_empty_glob;
    // INCOMPATIBLE_PACKAGE_GROUP_HAS_PUBLIC_SYNTAX ("+incompatible_package_group_has_public_syntax")
    semantics.incompatible_package_group_has_public_syntax =
        options.incompatible_package_group_has_public_syntax;
    // INCOMPATIBLE_FIX_PACKAGE_GROUP_REPOROOT_SYNTAX ("+incompatible_fix_package_group_reporoot_syntax")
    semantics.incompatible_fix_package_group_reporoot_syntax =
        options.incompatible_fix_package_group_reporoot_syntax;
    // INCOMPATIBLE_JAVA_INFO_MERGE_RUNTIME_MODULE_FLAGS ("-incompatible_java_info_merge_runtime_module_flags")
    semantics.incompatible_java_info_merge_runtime_module_flags =
        options.incompatible_java_info_merge_runtime_module_flags;
    // INCOMPATIBLE_NO_ATTR_LICENSE ("+incompatible_no_attr_license")
    semantics.incompatible_no_attr_license = options.incompatible_no_attr_license;
    // INCOMPATIBLE_NO_IMPLICIT_FILE_EXPORT ("-incompatible_no_implicit_file_export")
    semantics.incompatible_no_implicit_file_export = options.incompatible_no_implicit_file_export;
    // INCOMPATIBLE_NO_RULE_OUTPUTS_PARAM ("-incompatible_no_rule_outputs_param")
    semantics.incompatible_no_rule_outputs_param = options.incompatible_no_rule_outputs_param;
    // INCOMPATIBLE_RUN_SHELL_COMMAND_STRING ("+incompatible_run_shell_command_string")
    semantics.incompatible_run_shell_command_string = options.incompatible_run_shell_command_string;
    // STARLARK_SEMANTICS_PRINT_TEST_MARKER ("-print_test_marker")
    semantics.internal_starlark_flag_test_canary = options.internal_starlark_flag_test_canary;
    // INCOMPATIBLE_DO_NOT_SPLIT_LINKING_CMDLINE ("+incompatible_do_not_split_linking_cmdline")
    semantics.incompatible_do_not_split_linking_cmdline =
        options.incompatible_do_not_split_linking_cmdline;
    // INCOMPATIBLE_ENFORCE_STARLARK_UTF8.name ("incompatible_enforce_starlark_utf8")
    semantics.incompatible_enforce_starlark_utf8 = match options.incompatible_enforce_starlark_utf8
    {
        Utf8EnforcementMode::OFF => blaze_starlark::semantics::Utf8EnforcementMode::Off,
        Utf8EnforcementMode::WARNING => blaze_starlark::semantics::Utf8EnforcementMode::Warning,
        Utf8EnforcementMode::ERROR => blaze_starlark::semantics::Utf8EnforcementMode::Error,
    };
    // INCOMPATIBLE_USE_CC_CONFIGURE_FROM_RULES_CC ("-incompatible_use_cc_configure_from_rules")
    semantics.incompatible_use_cc_configure_from_rules_cc =
        options.incompatible_use_cc_configure_from_rules_cc;
    // INCOMPATIBLE_UNAMBIGUOUS_LABEL_STRINGIFICATION ("+incompatible_unambiguous_label_stringification")
    semantics.incompatible_unambiguous_label_stringification =
        options.incompatible_unambiguous_label_stringification;
    // MAX_COMPUTATION_STEPS.name ("max_computation_steps")
    semantics.max_computation_steps = options.max_computation_steps;
    // NESTED_SET_DEPTH_LIMIT.name ("nested_set_depth_limit")
    semantics.nested_set_depth_limit = options.nested_set_depth_limit;
    // INCOMPATIBLE_DISABLE_STARLARK_HOST_TRANSITIONS ("-incompatible_disable_starlark_host_transitions")
    semantics.incompatible_disable_starlark_host_transitions =
        options.incompatible_disable_starlark_host_transitions;
    // INCOMPATIBLE_DISABLE_OBJC_LIBRARY_TRANSITION ("+incompatible_disable_objc_library_transition")
    semantics.incompatible_disable_objc_library_transition =
        options.incompatible_disable_objc_library_transition;
    // INCOMPATIBLE_DISABLE_TRANSITIONS_OPTIONS.name ("incompatible_disable_transitions_on")
    semantics.incompatible_disable_transitions_on =
        options.incompatible_disable_transitions_on.clone();
    // ADD_GO_EXEC_GROUPS_TO_BINARY_RULES ("-add_go_exec_groups_to_binary_rules")
    semantics.add_go_exec_groups_to_binary_rules = options.add_go_exec_groups_to_binary_rules;
    // INCOMPATIBLE_FAIL_ON_UNKNOWN_ATTRIBUTES ("+incompatible_fail_on_unknown_attributes")
    semantics.incompatible_fail_on_unknown_attributes =
        options.incompatible_fail_on_unknown_attributes;
    // INCOMPATIBLE_ENABLE_PROTO_TOOLCHAIN_RESOLUTION ("+incompatible_enable_proto_toolchain_resolution")
    semantics.incompatible_enable_proto_toolchain_resolution =
        options.incompatible_enable_proto_toolchain_resolution;
    // INCOMPATIBLE_DISABLE_NON_EXECUTABLE_JAVA_BINARY ("-incompatible_disable_non_executable_java_binary")
    semantics.incompatible_disable_non_executable_java_binary =
        options.incompatible_disable_non_executable_java_binary;
    // INCOMPATIBLE_DISABLE_TARGET_DEFAULT_PROVIDER_FIELDS ("-incompatible_disable_target_default_provider_fields")
    semantics.incompatible_disable_target_default_provider_fields =
        options.incompatible_disable_target_default_provider_fields;
    // EXPERIMENTAL_RULE_EXTENSION_API ("+experimental_rule_extension_api")
    semantics.experimental_rule_extension_api = options.experimental_rule_extension_api;
    // EXPERIMENTAL_DORMANT_DEPS ("-experimental_dormant_deps")
    semantics.experimental_dormant_deps = options.experimental_dormant_deps;
    // EXPERIMENTAL_STARLARK_TYPE_SYNTAX ("+experimental_starlark_type_syntax")
    semantics.experimental_starlark_type_syntax = options.experimental_starlark_type_syntax;
    // STARLARK_SEMANTICS_EXPERIMENTAL_STARLARK_TYPE_CHECKING ("-experimental_starlark_type_checking")
    semantics.experimental_starlark_type_checking = options.experimental_starlark_type_checking;
    // EXPERIMENTAL_STARLARK_TYPES_ALLOWED_PATHS.name ("experimental_starlark_types")
    semantics.experimental_starlark_types_allowed_paths =
        options.experimental_starlark_types_allowed_paths.clone();
    // INCOMPATIBLE_ENABLE_DEPRECATED_LABEL_APIS ("+incompatible_enable_deprecated_label_apis")
    semantics.incompatible_enable_deprecated_label_apis = options.enable_deprecated_label_apis;
    // INCOMPATIBLE_STOP_EXPORTING_BUILD_FILE_PATH ("-incompatible_stop_exporting_build_file_path")
    semantics.incompatible_stop_exporting_build_file_path =
        options.incompatible_stop_exporting_build_file_path;
    // INCOMPATIBLE_DISALLOW_CTX_RESOLVE_TOOLS ("+incompatible_disallow_ctx_resolve_tools")
    semantics.incompatible_disallow_ctx_resolve_tools =
        options.incompatible_disallow_ctx_resolve_tools;
    // INCOMPATIBLE_SIMPLIFY_UNCONDITIONAL_SELECTS_IN_RULE_ATTRS ("+incompatible_simplify_unconditional_selects_in_rule_attrs")
    semantics.incompatible_simplify_unconditional_selects_in_rule_attrs =
        options.incompatible_simplify_unconditional_selects_in_rule_attrs;
    // INCOMPATIBLE_LOCATIONS_PREFERS_EXECUTABLE ("+incompatible_locations_prefers_executable")
    semantics.incompatible_locations_prefers_executable =
        options.incompatible_locations_prefers_executable;
    // INCOMPATIBLE_RESOLVE_SELECT_KEYS_EAGERLY ("-incompatible_resolve_select_keys_eagerly")
    semantics.incompatible_resolve_select_keys_eagerly =
        options.incompatible_resolve_select_keys_eagerly;
    // STARLARK_SEMANTICS_EXPERIMENTAL_ENABLE_STARLARK_SET ("+experimental_enable_starlark_set")
    semantics.experimental_enable_starlark_set = options.experimental_enable_starlark_set;
    // STARLARK_SEMANTICS_INTERNAL_BAZEL_ONLY_UTF_8_BYTE_STRINGS ("-internal_bazel_only_utf_8_byte_strings")
    semantics.internal_starlark_utf_8_byte_strings = options.internal_starlark_utf8_byte_strings;
    // EXPERIMENTAL_REPOSITORY_CTX_EXECUTE_WASM ("-experimental_repository_ctx_execute_wasm")
    semantics.experimental_repository_ctx_execute_wasm = options.repository_ctx_execute_wasm;
    // STARLARK_SEMANTICS_FORCE_STARLARK_STACK_TRACE ("-force_starlark_stack_trace")
    semantics.force_starlark_stack_trace = options.force_starlark_stack_trace;
    semantics
}

/// Inputs to create the top-level configuration of a command.
pub struct TopLevelConfigurationInputs {
    pub directories: ConfigDirectories,
    pub rule_class_provider: Arc<BazelRuleClassProvider>,
    pub starlark_semantics: Arc<StarlarkSemantics>,
    pub package_path_roots: Vec<Path>,
    pub platforms: SharedPlatformValueProvider,
    pub event_handler: Arc<dyn ExtendedEventHandler>,
}

/// The top-level configuration and the configuration evaluator it was created in.
pub struct TopLevelConfiguration {
    pub skyframe: ConfigurationSkyframe,
    pub build_options: Arc<BuildOptions>,
    pub configuration: Arc<BuildConfigurationValue>,
}

// Java: com/google/devtools/build/lib/runtime/commands/InfoCommand.java:169 InfoCommand.exec
/// The body of InfoCommand's configuration supplier (and of `BuildView`'s configuration
/// creation): `setBaselineConfiguration(buildOptions)` then `getConfiguration(buildOptions,
/// keepGoing)`.
pub fn create_top_level_configuration(
    inputs: TopLevelConfigurationInputs,
    build_options: BuildOptions,
    keep_going: bool,
) -> Result<TopLevelConfiguration, InvalidConfigurationException> {
    let build_options = Arc::new(build_options);
    let skyframe = ConfigurationSkyframe::set_baseline_configuration(
        ConfigurationRequest {
            directories: inputs.directories,
            rule_class_provider: inputs.rule_class_provider,
            starlark_semantics: inputs.starlark_semantics,
            package_path_roots: inputs.package_path_roots,
            platforms: inputs.platforms,
        },
        build_options.clone(),
        inputs.event_handler,
    )?;
    let configuration = skyframe.get_configuration(build_options.clone(), keep_going)?;
    Ok(TopLevelConfiguration {
        skyframe,
        build_options,
        configuration,
    })
}
