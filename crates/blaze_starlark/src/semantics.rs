//! Starlark semantics: the typed form of Bazel's `BuildLanguageOptions` flags.
//!
//! Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java (flags,
//! defaults) and net/starlark/java/eval/StarlarkSemantics.java (the immutable key/value set).
//! Generated from the Java at the pin (one field per `@Option`, named after the flag). The option
//! classes themselves are transcribed in `blaze_optdefs`; until that is wired, callers build
//! semantics with [`StarlarkSemantics::from_flags`] (flag name -> raw string, as on the command
//! line).

use std::collections::BTreeMap;

/// Java: BuildLanguageOptions.java:813 Utf8EnforcementMode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Utf8EnforcementMode {
    Off,
    Warning,
    Error,
}

impl Utf8EnforcementMode {
    // Java: BuildLanguageOptions.java:819 Utf8EnforcementMode.Converter (BoolOrEnumConverter:
    // true -> ERROR, false -> OFF).
    fn parse(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "off" | "false" | "no" | "0" | "f" | "n" => Ok(Utf8EnforcementMode::Off),
            "warning" => Ok(Utf8EnforcementMode::Warning),
            "error" | "true" | "yes" | "1" | "t" | "y" => Ok(Utf8EnforcementMode::Error),
            _ => Err(format!("Not a valid value: '{s}'")),
        }
    }
}

/// Error from [`StarlarkSemantics::from_flags`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct SemanticsError(pub String);

/// Typed `StarlarkSemantics`. Field names are the flag names.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StarlarkSemantics {
    /// `--incompatible_stop_exporting_language_modules` (Java: BuildLanguageOptions.java:72 incompatibleStopExportingLanguageModules)
    pub incompatible_stop_exporting_language_modules: bool,
    /// `--experimental_builtins_bzl_path` (Java: BuildLanguageOptions.java:84 experimentalBuiltinsBzlPath)
    pub experimental_builtins_bzl_path: String,
    /// `--incompatible_autoload_externally` (Java: BuildLanguageOptions.java:104 incompatibleAutoloadExternally)
    pub incompatible_autoload_externally: Vec<String>,
    /// `--repositories_without_autoloads` (Java: BuildLanguageOptions.java:144 repositoriesWithoutAutoloads)
    pub repositories_without_autoloads: Vec<String>,
    /// `--incompatible_disable_autoloads_in_main_repo` (Java: BuildLanguageOptions.java:158 incompatibleDisableAutoloadsInMainRepo)
    pub incompatible_disable_autoloads_in_main_repo: bool,
    /// `--experimental_builtins_dummy` (Java: BuildLanguageOptions.java:170 experimentalBuiltinsDummy)
    pub experimental_builtins_dummy: bool,
    /// `--experimental_builtins_injection_override` (Java: BuildLanguageOptions.java:179 experimentalBuiltinsInjectionOverride)
    pub experimental_builtins_injection_override: Option<Vec<String>>,
    /// `--experimental_bzl_visibility` (Java: BuildLanguageOptions.java:201 experimentalBzlVisibility)
    pub experimental_bzl_visibility: bool,
    /// `--experimental_single_package_toolchain_binding` (Java: BuildLanguageOptions.java:212 experimentalSinglePackageToolchainBinding)
    pub experimental_single_package_toolchain_binding: bool,
    /// `--allow_experimental_loads` (Java: BuildLanguageOptions.java:223 allowExperimentalLoads)
    pub allow_experimental_loads: bool,
    /// `--check_bzl_visibility` (Java: BuildLanguageOptions.java:232 checkBzlVisibility)
    pub check_bzl_visibility: bool,
    /// `--experimental_cc_skylark_api_enabled_packages` (Java: BuildLanguageOptions.java:240 experimentalCcStarlarkApiEnabledPackages)
    pub experimental_cc_skylark_api_enabled_packages: Vec<String>,
    /// `--experimental_enable_android_migration_apis` (Java: BuildLanguageOptions.java:252 experimentalEnableAndroidMigrationApis)
    pub experimental_enable_android_migration_apis: bool,
    /// `--experimental_enable_first_class_macros` (Java: BuildLanguageOptions.java:260 experimentalEnableFirstClassMacros)
    pub experimental_enable_first_class_macros: bool,
    /// `--experimental_enable_scl_dialect` (Java: BuildLanguageOptions.java:268 experimentalEnableSclDialect)
    pub experimental_enable_scl_dialect: bool,
    /// `--experimental_isolated_extension_usages` (Java: BuildLanguageOptions.java:277 experimentalIsolatedExtensionUsages)
    pub experimental_isolated_extension_usages: bool,
    /// `--incompatible_no_implicit_watch_label` (Java: BuildLanguageOptions.java:288 incompatibleNoImplicitWatchLabel)
    pub incompatible_no_implicit_watch_label: bool,
    /// `--incompatible_stop_exporting_build_file_path` (Java: BuildLanguageOptions.java:302 incompatibleStopExportingBuildFilePath)
    pub incompatible_stop_exporting_build_file_path: bool,
    /// `--experimental_google_legacy_api` (Java: BuildLanguageOptions.java:313 experimentalGoogleLegacyApi)
    pub experimental_google_legacy_api: bool,
    /// `--experimental_platforms_api` (Java: BuildLanguageOptions.java:324 experimentalPlatformsApi)
    pub experimental_platforms_api: bool,
    /// `--experimental_cc_shared_library` (Java: BuildLanguageOptions.java:335 experimentalCcSharedLibrary)
    pub experimental_cc_shared_library: bool,
    /// `--experimental_repo_remote_exec` (Java: BuildLanguageOptions.java:348 experimentalRepoRemoteExec)
    pub experimental_repo_remote_exec: bool,
    /// `--experimental_disable_external_package` (Java: BuildLanguageOptions.java:359 experimentalDisableExternalPackage)
    pub experimental_disable_external_package: bool,
    /// `--experimental_sibling_repository_layout` (Java: BuildLanguageOptions.java:373 experimentalSiblingRepositoryLayout)
    pub experimental_sibling_repository_layout: bool,
    /// `--incompatible_allow_tags_propagation` (Java: BuildLanguageOptions.java:394 experimentalAllowTagsPropagation)
    pub incompatible_allow_tags_propagation: bool,
    /// `--incompatible_always_check_depset_elements` (Java: BuildLanguageOptions.java:409 incompatibleAlwaysCheckDepsetElements)
    pub incompatible_always_check_depset_elements: bool,
    /// `--incompatible_disable_target_default_provider_fields` (Java: BuildLanguageOptions.java:422 incompatibleDisableTargetDefaultProviderFields)
    pub incompatible_disable_target_default_provider_fields: bool,
    /// `--incompatible_disallow_empty_glob` (Java: BuildLanguageOptions.java:440 incompatibleDisallowEmptyGlob)
    pub incompatible_disallow_empty_glob: bool,
    /// `--incompatible_package_group_has_public_syntax` (Java: BuildLanguageOptions.java:450 incompatiblePackageGroupHasPublicSyntax)
    pub incompatible_package_group_has_public_syntax: bool,
    /// `--incompatible_fix_package_group_reporoot_syntax` (Java: BuildLanguageOptions.java:461 incompatibleFixPackageGroupReporootSyntax)
    pub incompatible_fix_package_group_reporoot_syntax: bool,
    /// `--incompatible_no_attr_license` (Java: BuildLanguageOptions.java:475 incompatibleNoAttrLicense)
    pub incompatible_no_attr_license: bool,
    /// `--incompatible_no_implicit_file_export` (Java: BuildLanguageOptions.java:484 incompatibleNoImplicitFileExport)
    pub incompatible_no_implicit_file_export: bool,
    /// `--incompatible_no_rule_outputs_param` (Java: BuildLanguageOptions.java:496 incompatibleNoRuleOutputsParam)
    pub incompatible_no_rule_outputs_param: bool,
    /// `--incompatible_run_shell_command_string` (Java: BuildLanguageOptions.java:505 incompatibleRunShellCommandString)
    pub incompatible_run_shell_command_string: bool,
    /// `--internal_starlark_flag_test_canary` (Java: BuildLanguageOptions.java:515 internalStarlarkFlagTestCanary)
    pub internal_starlark_flag_test_canary: bool,
    /// `--incompatible_do_not_split_linking_cmdline` (Java: BuildLanguageOptions.java:522 incompatibleDoNotSplitLinkingCmdline)
    pub incompatible_do_not_split_linking_cmdline: bool,
    /// `--incompatible_use_cc_configure_from_rules_cc` (Java: BuildLanguageOptions.java:534 incompatibleUseCcConfigureFromRulesCc)
    pub incompatible_use_cc_configure_from_rules_cc: bool,
    /// `--incompatible_unambiguous_label_stringification` (Java: BuildLanguageOptions.java:546 incompatibleUnambiguousLabelStringification)
    pub incompatible_unambiguous_label_stringification: bool,
    /// `--incompatible_java_info_merge_runtime_module_flags` (Java: BuildLanguageOptions.java:559 incompatibleJavaInfoMergeRuntimeModuleFlags)
    pub incompatible_java_info_merge_runtime_module_flags: bool,
    /// `--max_computation_steps` (Java: BuildLanguageOptions.java:570 maxComputationSteps)
    pub max_computation_steps: i64,
    /// `--nested_set_depth_limit` (Java: BuildLanguageOptions.java:580 nestedSetDepthLimit)
    pub nested_set_depth_limit: i32,
    /// `--incompatible_disable_starlark_host_transitions` (Java: BuildLanguageOptions.java:590 incompatibleDisableStarlarkHostTransitions)
    pub incompatible_disable_starlark_host_transitions: bool,
    /// `--incompatible_disable_objc_library_transition` (Java: BuildLanguageOptions.java:601 incompatibleDisableObjcLibraryTransition)
    pub incompatible_disable_objc_library_transition: bool,
    /// `--incompatible_disable_transitions_on` (Java: BuildLanguageOptions.java:612 incompatibleDisableTransitionsOn)
    pub incompatible_disable_transitions_on: Vec<String>,
    /// `--add_go_exec_groups_to_binary_rules` (Java: BuildLanguageOptions.java:623 addGoExecGroupsToBinaryRules)
    pub add_go_exec_groups_to_binary_rules: bool,
    /// `--incompatible_fail_on_unknown_attributes` (Java: BuildLanguageOptions.java:634 incompatibleFailOnUnknownAttributes)
    pub incompatible_fail_on_unknown_attributes: bool,
    /// `--incompatible_enable_proto_toolchain_resolution` (Java: BuildLanguageOptions.java:644 incompatibleEnableProtoToolchainResolution)
    pub incompatible_enable_proto_toolchain_resolution: bool,
    /// `--incompatible_disable_non_executable_java_binary` (Java: BuildLanguageOptions.java:654 incompatibleDisableNonExecutableJavaBinary)
    pub incompatible_disable_non_executable_java_binary: bool,
    /// `--experimental_rule_extension_api` (Java: BuildLanguageOptions.java:663 experimentalRuleExtensionApi)
    pub experimental_rule_extension_api: bool,
    /// `--experimental_dormant_deps` (Java: BuildLanguageOptions.java:672 experimentalDormantDeps)
    pub experimental_dormant_deps: bool,
    /// `--experimental_starlark_type_syntax` (Java: BuildLanguageOptions.java:684 experimentalStarlarkTypeSyntax)
    pub experimental_starlark_type_syntax: bool,
    /// `--experimental_starlark_type_checking` (Java: BuildLanguageOptions.java:698 experimentalStarlarkTypeChecking)
    pub experimental_starlark_type_checking: bool,
    /// `--experimental_starlark_types` (Java: BuildLanguageOptions.java:711 experimentalStarlarkTypes)
    pub experimental_starlark_types: bool,
    /// `--experimental_starlark_types_allowed_paths` (Java: BuildLanguageOptions.java:722 experimentalStarlarkTypesAllowedPaths)
    pub experimental_starlark_types_allowed_paths: Vec<String>,
    /// `--incompatible_enable_deprecated_label_apis` (Java: BuildLanguageOptions.java:732 enableDeprecatedLabelApis)
    pub incompatible_enable_deprecated_label_apis: bool,
    /// `--incompatible_disallow_ctx_resolve_tools` (Java: BuildLanguageOptions.java:742 incompatibleDisallowCtxResolveTools)
    pub incompatible_disallow_ctx_resolve_tools: bool,
    /// `--incompatible_simplify_unconditional_selects_in_rule_attrs` (Java: BuildLanguageOptions.java:754 incompatibleSimplifyUnconditionalSelectsInRuleAttrs)
    pub incompatible_simplify_unconditional_selects_in_rule_attrs: bool,
    /// `--incompatible_check_external_repo_source_dir_package_boundary` (Java: BuildLanguageOptions.java:767 incompatibleCheckExternalRepoSourceDirPackageBoundary)
    pub incompatible_check_external_repo_source_dir_package_boundary: bool,
    /// `--experimental_enable_starlark_set` (Java: BuildLanguageOptions.java:780 experimentalEnableStarlarkSet)
    pub experimental_enable_starlark_set: bool,
    /// `--incompatible_locations_prefers_executable` (Java: BuildLanguageOptions.java:789 incompatibleLocationsPrefersExecutable)
    pub incompatible_locations_prefers_executable: bool,
    /// `--internal_starlark_utf_8_byte_strings` (Java: BuildLanguageOptions.java:801 internalStarlarkUtf8ByteStrings)
    pub internal_starlark_utf_8_byte_strings: bool,
    /// `--incompatible_enforce_starlark_utf8` (Java: BuildLanguageOptions.java:830 incompatibleEnforceStarlarkUtf8)
    pub incompatible_enforce_starlark_utf8: Utf8EnforcementMode,
    /// `--experimental_repository_ctx_execute_wasm` (Java: BuildLanguageOptions.java:844 repositoryCtxExecuteWasm)
    pub experimental_repository_ctx_execute_wasm: bool,
    /// `--incompatible_resolve_select_keys_eagerly` (Java: BuildLanguageOptions.java:853 incompatibleResolveSelectKeysEagerly)
    pub incompatible_resolve_select_keys_eagerly: bool,
    /// `--force_starlark_stack_trace` (Java: BuildLanguageOptions.java:865 forceStarlarkStackTrace)
    pub force_starlark_stack_trace: bool,
    /// Not a flag: Bazel unconditionally forbids recursion (StarlarkSemantics.ALLOW_RECURSION).
    pub allow_recursion: bool,
}

impl Default for StarlarkSemantics {
    fn default() -> Self {
        StarlarkSemantics {
            incompatible_stop_exporting_language_modules: false,
            experimental_builtins_bzl_path: String::from("%bundled%"),
            incompatible_autoload_externally: Vec::new(),
            repositories_without_autoloads: Vec::new(),
            incompatible_disable_autoloads_in_main_repo: true,
            experimental_builtins_dummy: false,
            experimental_builtins_injection_override: None,
            experimental_bzl_visibility: true,
            experimental_single_package_toolchain_binding: false,
            allow_experimental_loads: false,
            check_bzl_visibility: true,
            experimental_cc_skylark_api_enabled_packages: Vec::new(),
            experimental_enable_android_migration_apis: false,
            experimental_enable_first_class_macros: true,
            experimental_enable_scl_dialect: true,
            experimental_isolated_extension_usages: false,
            incompatible_no_implicit_watch_label: true,
            incompatible_stop_exporting_build_file_path: false,
            experimental_google_legacy_api: false,
            experimental_platforms_api: false,
            experimental_cc_shared_library: false,
            experimental_repo_remote_exec: false,
            experimental_disable_external_package: false,
            experimental_sibling_repository_layout: false,
            incompatible_allow_tags_propagation: true,
            incompatible_always_check_depset_elements: true,
            incompatible_disable_target_default_provider_fields: false,
            incompatible_disallow_empty_glob: true,
            incompatible_package_group_has_public_syntax: true,
            incompatible_fix_package_group_reporoot_syntax: true,
            incompatible_no_attr_license: true,
            incompatible_no_implicit_file_export: false,
            incompatible_no_rule_outputs_param: false,
            incompatible_run_shell_command_string: true,
            internal_starlark_flag_test_canary: false,
            incompatible_do_not_split_linking_cmdline: true,
            incompatible_use_cc_configure_from_rules_cc: false,
            incompatible_unambiguous_label_stringification: true,
            incompatible_java_info_merge_runtime_module_flags: false,
            max_computation_steps: 0,
            nested_set_depth_limit: 3500,
            incompatible_disable_starlark_host_transitions: false,
            incompatible_disable_objc_library_transition: true,
            incompatible_disable_transitions_on: Vec::new(),
            add_go_exec_groups_to_binary_rules: false,
            incompatible_fail_on_unknown_attributes: true,
            incompatible_enable_proto_toolchain_resolution: true,
            incompatible_disable_non_executable_java_binary: false,
            experimental_rule_extension_api: true,
            experimental_dormant_deps: false,
            experimental_starlark_type_syntax: true,
            experimental_starlark_type_checking: false,
            experimental_starlark_types: false,
            experimental_starlark_types_allowed_paths: Vec::new(),
            incompatible_enable_deprecated_label_apis: true,
            incompatible_disallow_ctx_resolve_tools: true,
            incompatible_simplify_unconditional_selects_in_rule_attrs: true,
            incompatible_check_external_repo_source_dir_package_boundary: false,
            experimental_enable_starlark_set: true,
            incompatible_locations_prefers_executable: true,
            internal_starlark_utf_8_byte_strings: true,
            incompatible_enforce_starlark_utf8: Utf8EnforcementMode::Warning,
            experimental_repository_ctx_execute_wasm: false,
            incompatible_resolve_select_keys_eagerly: false,
            force_starlark_stack_trace: false,
            allow_recursion: false,
        }
    }
}

// Java: com/google/devtools/common/options/Converters.java BooleanConverter
fn parse_bool(name: &str, v: &str) -> Result<bool, SemanticsError> {
    match v.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "t" | "y" => Ok(true),
        "false" | "0" | "no" | "f" | "n" => Ok(false),
        _ => Err(SemanticsError(format!(
            "While parsing option --{name}={v}: '{v}' is not a boolean"
        ))),
    }
}

// Java: Converters.SeparatedOptionListConverter (comma, empty values allowed).
fn parse_list(v: &str) -> Vec<String> {
    if v.is_empty() {
        Vec::new()
    } else {
        v.split(',').map(str::to_owned).collect()
    }
}

// Java: Converters.SeparatedOptionSetConverter (distinct, sorted).
fn parse_set(v: &str) -> Vec<String> {
    let mut l = parse_list(v);
    l.sort();
    l.dedup();
    l
}

fn parse_int<T: std::str::FromStr>(name: &str, v: &str) -> Result<T, SemanticsError> {
    v.parse().map_err(|_| {
        SemanticsError(format!(
            "While parsing option --{name}={v}: '{v}' is not an int"
        ))
    })
}

impl StarlarkSemantics {
    /// Builds semantics from `flag name -> raw value` (no leading `--`). For booleans, `--noflag`
    /// style is the caller's job: pass `"false"`. A repeatable list flag takes its values joined
    /// by `,`. Unknown names are an error.
    pub fn from_flags(flags: &BTreeMap<String, String>) -> Result<Self, SemanticsError> {
        let mut s = StarlarkSemantics::default();
        for (name, v) in flags {
            s.set_flag(name, v)?;
        }
        Ok(s)
    }

    /// Sets one flag from its raw command-line value.
    pub fn set_flag(&mut self, name: &str, v: &str) -> Result<(), SemanticsError> {
        match name {
            "incompatible_stop_exporting_language_modules" => {
                self.incompatible_stop_exporting_language_modules = parse_bool(name, v)?
            }
            "experimental_builtins_bzl_path" => self.experimental_builtins_bzl_path = v.to_owned(),
            "incompatible_autoload_externally" => {
                self.incompatible_autoload_externally = parse_set(v)
            }
            "repositories_without_autoloads" => self.repositories_without_autoloads = parse_set(v),
            "incompatible_disable_autoloads_in_main_repo" => {
                self.incompatible_disable_autoloads_in_main_repo = parse_bool(name, v)?
            }
            "experimental_builtins_dummy" => {
                self.experimental_builtins_dummy = parse_bool(name, v)?
            }
            "experimental_builtins_injection_override" => {
                self.experimental_builtins_injection_override = Some(parse_list(v))
            }
            "experimental_bzl_visibility" => {
                self.experimental_bzl_visibility = parse_bool(name, v)?
            }
            "experimental_single_package_toolchain_binding" => {
                self.experimental_single_package_toolchain_binding = parse_bool(name, v)?
            }
            "allow_experimental_loads" => self.allow_experimental_loads = parse_bool(name, v)?,
            "check_bzl_visibility" => self.check_bzl_visibility = parse_bool(name, v)?,
            "experimental_cc_skylark_api_enabled_packages" => {
                self.experimental_cc_skylark_api_enabled_packages = parse_list(v)
            }
            "experimental_enable_android_migration_apis" => {
                self.experimental_enable_android_migration_apis = parse_bool(name, v)?
            }
            "experimental_enable_first_class_macros" => {
                self.experimental_enable_first_class_macros = parse_bool(name, v)?
            }
            "experimental_enable_scl_dialect" => {
                self.experimental_enable_scl_dialect = parse_bool(name, v)?
            }
            "experimental_isolated_extension_usages" => {
                self.experimental_isolated_extension_usages = parse_bool(name, v)?
            }
            "incompatible_no_implicit_watch_label" => {
                self.incompatible_no_implicit_watch_label = parse_bool(name, v)?
            }
            "incompatible_stop_exporting_build_file_path" => {
                self.incompatible_stop_exporting_build_file_path = parse_bool(name, v)?
            }
            "experimental_google_legacy_api" => {
                self.experimental_google_legacy_api = parse_bool(name, v)?
            }
            "experimental_platforms_api" => self.experimental_platforms_api = parse_bool(name, v)?,
            "experimental_cc_shared_library" => {
                self.experimental_cc_shared_library = parse_bool(name, v)?
            }
            "experimental_repo_remote_exec" => {
                self.experimental_repo_remote_exec = parse_bool(name, v)?
            }
            "experimental_disable_external_package" => {
                self.experimental_disable_external_package = parse_bool(name, v)?
            }
            "experimental_sibling_repository_layout" => {
                self.experimental_sibling_repository_layout = parse_bool(name, v)?
            }
            "incompatible_allow_tags_propagation" => {
                self.incompatible_allow_tags_propagation = parse_bool(name, v)?
            }
            "incompatible_always_check_depset_elements" => {
                self.incompatible_always_check_depset_elements = parse_bool(name, v)?
            }
            "incompatible_disable_target_default_provider_fields" => {
                self.incompatible_disable_target_default_provider_fields = parse_bool(name, v)?
            }
            "incompatible_disallow_empty_glob" => {
                self.incompatible_disallow_empty_glob = parse_bool(name, v)?
            }
            "incompatible_package_group_has_public_syntax" => {
                self.incompatible_package_group_has_public_syntax = parse_bool(name, v)?
            }
            "incompatible_fix_package_group_reporoot_syntax" => {
                self.incompatible_fix_package_group_reporoot_syntax = parse_bool(name, v)?
            }
            "incompatible_no_attr_license" => {
                self.incompatible_no_attr_license = parse_bool(name, v)?
            }
            "incompatible_no_implicit_file_export" => {
                self.incompatible_no_implicit_file_export = parse_bool(name, v)?
            }
            "incompatible_no_rule_outputs_param" => {
                self.incompatible_no_rule_outputs_param = parse_bool(name, v)?
            }
            "incompatible_run_shell_command_string" => {
                self.incompatible_run_shell_command_string = parse_bool(name, v)?
            }
            "internal_starlark_flag_test_canary" => {
                self.internal_starlark_flag_test_canary = parse_bool(name, v)?
            }
            "incompatible_do_not_split_linking_cmdline" => {
                self.incompatible_do_not_split_linking_cmdline = parse_bool(name, v)?
            }
            "incompatible_use_cc_configure_from_rules_cc" => {
                self.incompatible_use_cc_configure_from_rules_cc = parse_bool(name, v)?
            }
            "incompatible_unambiguous_label_stringification" => {
                self.incompatible_unambiguous_label_stringification = parse_bool(name, v)?
            }
            "incompatible_java_info_merge_runtime_module_flags" => {
                self.incompatible_java_info_merge_runtime_module_flags = parse_bool(name, v)?
            }
            "max_computation_steps" => self.max_computation_steps = parse_int(name, v)?,
            "nested_set_depth_limit" => self.nested_set_depth_limit = parse_int(name, v)?,
            "incompatible_disable_starlark_host_transitions" => {
                self.incompatible_disable_starlark_host_transitions = parse_bool(name, v)?
            }
            "incompatible_disable_objc_library_transition" => {
                self.incompatible_disable_objc_library_transition = parse_bool(name, v)?
            }
            "incompatible_disable_transitions_on" => {
                self.incompatible_disable_transitions_on = parse_set(v)
            }
            "add_go_exec_groups_to_binary_rules" => {
                self.add_go_exec_groups_to_binary_rules = parse_bool(name, v)?
            }
            "incompatible_fail_on_unknown_attributes" => {
                self.incompatible_fail_on_unknown_attributes = parse_bool(name, v)?
            }
            "incompatible_enable_proto_toolchain_resolution" => {
                self.incompatible_enable_proto_toolchain_resolution = parse_bool(name, v)?
            }
            "incompatible_disable_non_executable_java_binary" => {
                self.incompatible_disable_non_executable_java_binary = parse_bool(name, v)?
            }
            "experimental_rule_extension_api" => {
                self.experimental_rule_extension_api = parse_bool(name, v)?
            }
            "experimental_dormant_deps" => self.experimental_dormant_deps = parse_bool(name, v)?,
            "experimental_starlark_type_syntax" => {
                self.experimental_starlark_type_syntax = parse_bool(name, v)?
            }
            "experimental_starlark_type_checking" => {
                self.experimental_starlark_type_checking = parse_bool(name, v)?
            }
            "experimental_starlark_types" => {
                self.experimental_starlark_types = parse_bool(name, v)?
            }
            "experimental_starlark_types_allowed_paths" => {
                self.experimental_starlark_types_allowed_paths = parse_list(v)
            }
            "incompatible_enable_deprecated_label_apis" => {
                self.incompatible_enable_deprecated_label_apis = parse_bool(name, v)?
            }
            "incompatible_disallow_ctx_resolve_tools" => {
                self.incompatible_disallow_ctx_resolve_tools = parse_bool(name, v)?
            }
            "incompatible_simplify_unconditional_selects_in_rule_attrs" => {
                self.incompatible_simplify_unconditional_selects_in_rule_attrs =
                    parse_bool(name, v)?
            }
            "incompatible_check_external_repo_source_dir_package_boundary" => {
                self.incompatible_check_external_repo_source_dir_package_boundary =
                    parse_bool(name, v)?
            }
            "experimental_enable_starlark_set" => {
                self.experimental_enable_starlark_set = parse_bool(name, v)?
            }
            "incompatible_locations_prefers_executable" => {
                self.incompatible_locations_prefers_executable = parse_bool(name, v)?
            }
            "internal_starlark_utf_8_byte_strings" => {
                self.internal_starlark_utf_8_byte_strings = parse_bool(name, v)?
            }
            "incompatible_enforce_starlark_utf8" => {
                self.incompatible_enforce_starlark_utf8 =
                    Utf8EnforcementMode::parse(v).map_err(SemanticsError)?
            }
            "experimental_repository_ctx_execute_wasm" => {
                self.experimental_repository_ctx_execute_wasm = parse_bool(name, v)?
            }
            "incompatible_resolve_select_keys_eagerly" => {
                self.incompatible_resolve_select_keys_eagerly = parse_bool(name, v)?
            }
            "force_starlark_stack_trace" => self.force_starlark_stack_trace = parse_bool(name, v)?,
            _ => return Err(SemanticsError(format!("Unrecognized option: --{name}"))),
        }
        Ok(())
    }

    /// Bazel's `StarlarkSemantics.INTERNAL_BAZEL_ONLY_UTF_8_BYTE_STRINGS`.
    pub fn utf8_byte_strings(&self) -> bool {
        self.internal_starlark_utf_8_byte_strings
    }

    /// Names of all flags, in Java declaration order.
    pub const FLAG_NAMES: &'static [&'static str] = &[
        "incompatible_stop_exporting_language_modules",
        "experimental_builtins_bzl_path",
        "incompatible_autoload_externally",
        "repositories_without_autoloads",
        "incompatible_disable_autoloads_in_main_repo",
        "experimental_builtins_dummy",
        "experimental_builtins_injection_override",
        "experimental_bzl_visibility",
        "experimental_single_package_toolchain_binding",
        "allow_experimental_loads",
        "check_bzl_visibility",
        "experimental_cc_skylark_api_enabled_packages",
        "experimental_enable_android_migration_apis",
        "experimental_enable_first_class_macros",
        "experimental_enable_scl_dialect",
        "experimental_isolated_extension_usages",
        "incompatible_no_implicit_watch_label",
        "incompatible_stop_exporting_build_file_path",
        "experimental_google_legacy_api",
        "experimental_platforms_api",
        "experimental_cc_shared_library",
        "experimental_repo_remote_exec",
        "experimental_disable_external_package",
        "experimental_sibling_repository_layout",
        "incompatible_allow_tags_propagation",
        "incompatible_always_check_depset_elements",
        "incompatible_disable_target_default_provider_fields",
        "incompatible_disallow_empty_glob",
        "incompatible_package_group_has_public_syntax",
        "incompatible_fix_package_group_reporoot_syntax",
        "incompatible_no_attr_license",
        "incompatible_no_implicit_file_export",
        "incompatible_no_rule_outputs_param",
        "incompatible_run_shell_command_string",
        "internal_starlark_flag_test_canary",
        "incompatible_do_not_split_linking_cmdline",
        "incompatible_use_cc_configure_from_rules_cc",
        "incompatible_unambiguous_label_stringification",
        "incompatible_java_info_merge_runtime_module_flags",
        "max_computation_steps",
        "nested_set_depth_limit",
        "incompatible_disable_starlark_host_transitions",
        "incompatible_disable_objc_library_transition",
        "incompatible_disable_transitions_on",
        "add_go_exec_groups_to_binary_rules",
        "incompatible_fail_on_unknown_attributes",
        "incompatible_enable_proto_toolchain_resolution",
        "incompatible_disable_non_executable_java_binary",
        "experimental_rule_extension_api",
        "experimental_dormant_deps",
        "experimental_starlark_type_syntax",
        "experimental_starlark_type_checking",
        "experimental_starlark_types",
        "experimental_starlark_types_allowed_paths",
        "incompatible_enable_deprecated_label_apis",
        "incompatible_disallow_ctx_resolve_tools",
        "incompatible_simplify_unconditional_selects_in_rule_attrs",
        "incompatible_check_external_repo_source_dir_package_boundary",
        "experimental_enable_starlark_set",
        "incompatible_locations_prefers_executable",
        "internal_starlark_utf_8_byte_strings",
        "incompatible_enforce_starlark_utf8",
        "experimental_repository_ctx_execute_wasm",
        "incompatible_resolve_select_keys_eagerly",
        "force_starlark_stack_trace",
    ];
}
