//! A fingerprint of `StarlarkSemantics` that can be compared across Bazel versions.
//!
//! Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java
//! (`stableFingerprint`, `computeFingerprint`, `setFlags` and the key constants).
//!
//! PORT: move to blaze_starlark (or the crate that ports `BuildLanguageOptions`). Java caches the
//! result per semantics object (`FINGERPRINT_CACHE`); Blaze recomputes it.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use blaze_starlark::StarlarkSemantics;
use blaze_starlark::semantics::Utf8EnforcementMode;
use blaze_util::fingerprint::Fingerprint;

/// Java `toString()` of a `List<String>` value: `[a, b]`.
fn java_list_to_string(list: &[String]) -> String {
    format!("[{}]", list.join(", "))
}

/// Java `Utf8EnforcementMode.toString()` (the enum constant's name).
fn utf8_enforcement_mode_to_string(mode: Utf8EnforcementMode) -> String {
    match mode {
        Utf8EnforcementMode::Off => "OFF",
        Utf8EnforcementMode::Warning => "WARNING",
        Utf8EnforcementMode::Error => "ERROR",
    }
    .to_owned()
}

/// The receiver of [`set_flags`] (Java `BuildLanguageOptions.FlagConsumer`).
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:884 BuildLanguageOptions.FlagConsumer
trait FlagConsumer {
    /// `set(StarlarkSemantics.Key<T> key, T value)`, with the key's name and the value's Java
    /// `toString()`.
    fn set(&mut self, key: &str, value: String) -> &mut Self;

    /// `setBool(String key, boolean enabled)`; `key` has the `+`/`-` default prefix.
    fn set_bool(&mut self, key: &str, enabled: bool) -> &mut Self;
}

/// Feeds every flag bound to a `StarlarkSemantics` key to `consumer`, in Java's order, with the
/// Java key names (which sometimes differ from the flag names).
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:890 BuildLanguageOptions.setFlags
fn set_flags(s: &StarlarkSemantics, consumer: &mut impl FlagConsumer) {
    consumer
        .set_bool(
            "-incompatible_stop_exporting_language_modules",
            s.incompatible_stop_exporting_language_modules,
        )
        .set_bool(
            "+incompatible_allow_tags_propagation",
            s.incompatible_allow_tags_propagation,
        )
        .set(
            "experimental_builtins_bzl_path",
            s.experimental_builtins_bzl_path.clone(),
        )
        .set(
            "incompatible_autoload_externally",
            java_list_to_string(&s.incompatible_autoload_externally),
        )
        .set(
            "repositories_without_autoloads",
            java_list_to_string(&s.repositories_without_autoloads),
        )
        .set_bool(
            "+incompatible_disable_autoloads_in_main_repo",
            s.incompatible_disable_autoloads_in_main_repo,
        )
        .set_bool(
            "-experimental_builtins_dummy",
            s.experimental_builtins_dummy,
        )
        // PORT: an unset `allowMultiple` option is an empty list in Java (Blaze: `None`).
        .set(
            "experimental_builtins_injection_override",
            java_list_to_string(
                s.experimental_builtins_injection_override
                    .as_deref()
                    .unwrap_or(&[]),
            ),
        )
        .set_bool(
            "+experimental_bzl_visibility",
            s.experimental_bzl_visibility,
        )
        .set_bool("-allow_experimental_loads", s.allow_experimental_loads)
        .set_bool("+check_bzl_visibility", s.check_bzl_visibility)
        .set_bool(
            "-experimental_enable_android_migration_apis",
            s.experimental_enable_android_migration_apis,
        )
        .set_bool(
            "-experimental_single_package_toolchain_binding",
            s.experimental_single_package_toolchain_binding,
        )
        .set_bool(
            "+experimental_enable_first_class_macros",
            s.experimental_enable_first_class_macros,
        )
        .set_bool(
            "+experimental_enable_scl_dialect",
            s.experimental_enable_scl_dialect,
        )
        .set_bool(
            "-experimental_isolated_extension_usages",
            s.experimental_isolated_extension_usages,
        )
        .set_bool(
            "+incompatible_no_implicit_watch_label",
            s.incompatible_no_implicit_watch_label,
        )
        .set_bool(
            "-experimental_google_legacy_api",
            s.experimental_google_legacy_api,
        )
        .set_bool("-experimental_platforms_api", s.experimental_platforms_api)
        .set_bool(
            "-experimental_cc_shared_library",
            s.experimental_cc_shared_library,
        )
        .set_bool(
            "-experimental_repo_remote_exec",
            s.experimental_repo_remote_exec,
        )
        .set_bool(
            "-experimental_disable_external_package",
            s.experimental_disable_external_package,
        )
        .set_bool(
            "-experimental_sibling_repository_layout",
            s.experimental_sibling_repository_layout,
        )
        .set_bool(
            "+incompatible_always_check_depset_elements",
            s.incompatible_always_check_depset_elements,
        )
        .set_bool(
            "-incompatible_check_external_repo_source_dir_package_boundary",
            s.incompatible_check_external_repo_source_dir_package_boundary,
        )
        .set_bool(
            "+incompatible_disallow_empty_glob",
            s.incompatible_disallow_empty_glob,
        )
        .set_bool(
            "+incompatible_package_group_has_public_syntax",
            s.incompatible_package_group_has_public_syntax,
        )
        .set_bool(
            "+incompatible_fix_package_group_reporoot_syntax",
            s.incompatible_fix_package_group_reporoot_syntax,
        )
        .set_bool(
            "-incompatible_java_info_merge_runtime_module_flags",
            s.incompatible_java_info_merge_runtime_module_flags,
        )
        .set_bool(
            "+incompatible_no_attr_license",
            s.incompatible_no_attr_license,
        )
        .set_bool(
            "-incompatible_no_implicit_file_export",
            s.incompatible_no_implicit_file_export,
        )
        .set_bool(
            "-incompatible_no_rule_outputs_param",
            s.incompatible_no_rule_outputs_param,
        )
        .set_bool(
            "+incompatible_run_shell_command_string",
            s.incompatible_run_shell_command_string,
        )
        // Java: StarlarkSemantics.PRINT_TEST_MARKER
        .set_bool("-print_test_marker", s.internal_starlark_flag_test_canary)
        .set_bool(
            "+incompatible_do_not_split_linking_cmdline",
            s.incompatible_do_not_split_linking_cmdline,
        )
        .set(
            "incompatible_enforce_starlark_utf8",
            utf8_enforcement_mode_to_string(s.incompatible_enforce_starlark_utf8),
        )
        // Java's key name lacks the "_cc" suffix of the flag name.
        .set_bool(
            "-incompatible_use_cc_configure_from_rules",
            s.incompatible_use_cc_configure_from_rules_cc,
        )
        .set_bool(
            "+incompatible_unambiguous_label_stringification",
            s.incompatible_unambiguous_label_stringification,
        )
        .set("max_computation_steps", s.max_computation_steps.to_string())
        .set(
            "nested_set_depth_limit",
            s.nested_set_depth_limit.to_string(),
        )
        .set_bool(
            "-incompatible_disable_starlark_host_transitions",
            s.incompatible_disable_starlark_host_transitions,
        )
        .set_bool(
            "+incompatible_disable_objc_library_transition",
            s.incompatible_disable_objc_library_transition,
        )
        .set(
            "incompatible_disable_transitions_on",
            java_list_to_string(&s.incompatible_disable_transitions_on),
        )
        .set_bool(
            "-add_go_exec_groups_to_binary_rules",
            s.add_go_exec_groups_to_binary_rules,
        )
        .set_bool(
            "+incompatible_fail_on_unknown_attributes",
            s.incompatible_fail_on_unknown_attributes,
        )
        .set_bool(
            "+incompatible_enable_proto_toolchain_resolution",
            s.incompatible_enable_proto_toolchain_resolution,
        )
        .set_bool(
            "-incompatible_disable_non_executable_java_binary",
            s.incompatible_disable_non_executable_java_binary,
        )
        .set_bool(
            "-incompatible_disable_target_default_provider_fields",
            s.incompatible_disable_target_default_provider_fields,
        )
        .set_bool(
            "+experimental_rule_extension_api",
            s.experimental_rule_extension_api,
        )
        .set_bool("-experimental_dormant_deps", s.experimental_dormant_deps)
        .set_bool(
            "+experimental_starlark_type_syntax",
            s.experimental_starlark_type_syntax,
        )
        // Java: StarlarkSemantics.EXPERIMENTAL_STARLARK_TYPE_CHECKING
        .set_bool(
            "-experimental_starlark_type_checking",
            s.experimental_starlark_type_checking,
        )
        // Java's key EXPERIMENTAL_STARLARK_TYPES_ALLOWED_PATHS is named "experimental_starlark_types".
        .set(
            "experimental_starlark_types",
            java_list_to_string(&s.experimental_starlark_types_allowed_paths),
        )
        .set_bool(
            "+incompatible_enable_deprecated_label_apis",
            s.incompatible_enable_deprecated_label_apis,
        )
        .set_bool(
            "-incompatible_stop_exporting_build_file_path",
            s.incompatible_stop_exporting_build_file_path,
        )
        .set_bool(
            "+incompatible_disallow_ctx_resolve_tools",
            s.incompatible_disallow_ctx_resolve_tools,
        )
        .set_bool(
            "+incompatible_simplify_unconditional_selects_in_rule_attrs",
            s.incompatible_simplify_unconditional_selects_in_rule_attrs,
        )
        .set_bool(
            "+incompatible_locations_prefers_executable",
            s.incompatible_locations_prefers_executable,
        )
        .set_bool(
            "-incompatible_resolve_select_keys_eagerly",
            s.incompatible_resolve_select_keys_eagerly,
        )
        // Java: StarlarkSemantics.EXPERIMENTAL_ENABLE_STARLARK_SET
        .set_bool(
            "+experimental_enable_starlark_set",
            s.experimental_enable_starlark_set,
        )
        // Java: StarlarkSemantics.INTERNAL_BAZEL_ONLY_UTF_8_BYTE_STRINGS
        .set_bool(
            "-internal_bazel_only_utf_8_byte_strings",
            s.internal_starlark_utf_8_byte_strings,
        )
        .set_bool(
            "-experimental_repository_ctx_execute_wasm",
            s.experimental_repository_ctx_execute_wasm,
        )
        // Java: StarlarkSemantics.FORCE_STARLARK_STACK_TRACE
        .set_bool("-force_starlark_stack_trace", s.force_starlark_stack_trace);
}

/// Collects the fingerprinted parts of the semantics (the anonymous `FlagConsumer` of
/// `computeFingerprint`).
#[derive(Default)]
struct FingerprintCollector {
    enabled_bool_flags: BTreeSet<String>,
    other_flags: BTreeMap<String, String>,
}

impl FlagConsumer for FingerprintCollector {
    // Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1045 BuildLanguageOptions.computeFingerprint
    fn set(&mut self, key: &str, value: String) -> &mut Self {
        // This assumes that all non-boolean values have a stable and unique string
        // representation, which seems likely to remain true over time.
        self.other_flags.insert(key.to_owned(), value);
        self
    }

    // Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1053 BuildLanguageOptions.computeFingerprint
    fn set_bool(&mut self, key: &str, enabled: bool) -> &mut Self {
        // Only fingerprint enabled options so that the fingerprint is stable across Bazel
        // versions that only add new options (e.g., minor and patch versions). This relies on
        // the assumption that disabled new options have no observable effect.
        if enabled {
            // Trim the leading '+' or '-' from the flag names - the default value doesn't matter
            // for the current value, which is what we need to fingerprint.
            self.enabled_bool_flags.insert(key[1..].to_owned());
        }
        self
    }
}

/// Returns a fingerprint of the given `StarlarkSemantics` that can be compared across Bazel
/// versions.
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1021 BuildLanguageOptions.stableFingerprint
pub fn stable_fingerprint(semantics: &StarlarkSemantics) -> Vec<u8> {
    compute_fingerprint(semantics)
}

// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1037 BuildLanguageOptions.computeFingerprint
fn compute_fingerprint(semantics: &StarlarkSemantics) -> Vec<u8> {
    let mut collector = FingerprintCollector::default();
    set_flags(semantics, &mut collector);
    Fingerprint::new()
        .add_strings(collector.enabled_bool_flags.iter())
        .add_string_map(
            collector
                .other_flags
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str())),
        )
        .digest_and_reset()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_semantics_collect_the_java_key_names() {
        let mut collector = FingerprintCollector::default();
        set_flags(&StarlarkSemantics::default(), &mut collector);
        // Non-boolean keys, by Java key name.
        let other: Vec<&str> = collector.other_flags.keys().map(|k| k.as_str()).collect();
        assert_eq!(
            other,
            vec![
                "experimental_builtins_bzl_path",
                "experimental_builtins_injection_override",
                "experimental_starlark_types",
                "incompatible_autoload_externally",
                "incompatible_disable_transitions_on",
                "incompatible_enforce_starlark_utf8",
                "max_computation_steps",
                "nested_set_depth_limit",
                "repositories_without_autoloads",
            ]
        );
        assert_eq!(collector.other_flags["nested_set_depth_limit"], "3500");
        assert_eq!(
            collector.other_flags["incompatible_enforce_starlark_utf8"],
            "WARNING"
        );
        assert_eq!(
            collector.other_flags["incompatible_autoload_externally"],
            "[]"
        );
        assert!(
            collector
                .enabled_bool_flags
                .contains("incompatible_disallow_empty_glob")
        );
        assert!(!collector.enabled_bool_flags.contains("print_test_marker"));
    }
}

#[cfg(test)]
mod oracle_tests {
    use super::*;

    /// Expected value from the pinned Bazel: `scripts/w5r/java-probe/run semantics-fingerprint`
    /// (`BuildLanguageOptions.stableFingerprint` of the default semantics, from A-server.jar).
    #[test]
    fn default_semantics_fingerprint_matches_bazel() {
        assert_eq!(
            hex::encode(stable_fingerprint(&StarlarkSemantics::default())),
            "afebaa1da57f77d89d8e6c4d3123507e31c2c5057c23e0844ee8eae757a9d677"
        );
    }
}
