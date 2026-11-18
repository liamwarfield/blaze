//! Transcription of `com/google/devtools/build/lib/bazel/rules/BazelRulesModule.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/bazel/rules/BazelRulesModule.java:45 BazelRulesModule.BuildGraveyardOptions
options! {
    /// This is where deprecated options used by both Bazel and Blaze but only needed for the build
    /// command go to die.
    ///
    /// <p>To deprecate Bazel-only build command options, use {@link BazelBuildGraveyardOptions}.
    ///
    /// <p>To deprecate Bazel+Blaze options common to all commands, use {@link
    /// AllCommandGraveyardOptions}.
    pub struct BazelRulesModuleBuildGraveyardOptions {
        #[option(
            name = "incompatible_use_python_toolchains",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "Deprecated. No-op.",
        )]
        pub incompatible_use_python_toolchains: bool,

        #[option(
            name = "experimental_starlark_cc_import",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "Deprecated. No-op.",
        )]
        pub experimental_starlark_cc_import: bool,

        #[option(
            name = "experimental_platform_cc_test",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "Deprecated. No-op.",
        )]
        pub experimental_platform_cc_test: bool,

        #[option(
            name = "j2objc_dead_code_removal",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "Deprecated. No-op.",
        )]
        pub remove_dead_code: bool,

        #[option(
            name = "experimental_proto_extra_actions",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [EXPERIMENTAL],
            help = "Deprecated. No-op.",
        )]
        pub experimental_proto_extra_actions: bool,

        #[option(
            name = "enable_fdo_profile_absolute_path",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "Deprecated. No-op.",
        )]
        pub enable_fdo_profile_absolute_path: bool,

        #[option(
            name = "experimental_genquery_use_graphless_query",
            default_value = "auto",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "Deprecated. No-op.",
        )]
        pub use_graphless_query: TriState,

        #[option(
            name = "use_top_level_targets_for_symlinks",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "Deprecated. No-op.",
        )]
        pub use_top_level_targets_for_symlinks: bool,

        #[option(
            name = "experimental_skyframe_prepare_analysis",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            deprecation_warning = "This flag is a no-op and will be deleted in a future release.",
            help = "Deprecated. No-op.",
        )]
        pub skyframe_prepare_analysis: bool,

        #[option(
            name = "incompatible_disable_legacy_flags_cc_toolchain_api",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [INCOMPATIBLE_CHANGE, DEPRECATED],
            help = concat!(
                "Flag for disabling the legacy cc_toolchain Starlark API for accessing legacy ",
                "CROSSTOOL fields.",
            ),
        )]
        pub disable_legacy_flags_cc_toolchain_api: bool,

        #[option(
            name = "incompatible_enable_profile_by_default",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "No-op.",
        )]
        pub enable_profile_by_default: bool,

        #[option(
            name = "incompatible_override_toolchain_transition",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            deprecation_warning = "This is now always set, please remove this flag.",
            help = "Deprecated, this is no longer in use and should be removed.",
        )]
        pub override_toolchain_transition: bool,

        #[option(
            name = "experimental_parse_headers_skipped_if_corresponding_srcs_found",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op.",
        )]
        pub parse_headers_skipped_if_corresponding_srcs_found: bool,

        #[option(
            name = "experimental_worker_as_resource",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op, will be removed soon.",
        )]
        pub worker_as_resource: bool,

        #[option(
            name = "high_priority_workers",
            default_value = "null",
            allow_multiple = true,
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op, will be removed soon.",
        )]
        pub high_priority_workers: Vec<String>,

        #[option(
            name = "target_platform_fallback",
            default_value = "",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "This option is deprecated and has no effect.",
        )]
        pub target_platform_fallback: String,

        #[option(
            name = "incompatible_auto_configure_host_platform",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "This option is deprecated and has no effect.",
        )]
        pub auto_configure_host_platform: bool,

        #[option(
            name = "experimental_require_availability_info",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            deprecated = true,
            help = "Deprecated no-op.",
        )]
        pub require_availability_info: bool,

        #[option(
            name = "experimental_collect_local_action_metrics",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "Deprecated no-op.",
        )]
        pub collect_local_execution_statistics: bool,

        #[option(
            name = "experimental_collect_local_sandbox_action_metrics",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "Deprecated no-op.",
        )]
        pub collect_local_sandbox_execution_statistics: bool,

        #[option(
            name = "experimental_enable_starlark_doc_extract",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [EXPERIMENTAL],
            help = "Deprecated no-op.",
        )]
        pub enable_bzl_doc_dump: bool,

        #[option(
            name = "experimental_parallel_aquery_output",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op.",
        )]
        pub parallel_aquery_output: bool,

        #[option(
            name = "experimental_show_artifacts",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "Deprecated no-op.",
        )]
        pub show_artifacts: bool,

        #[option(
            name = "announce",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            deprecation_warning = "This option is now deprecated and is a no-op",
            help = "Deprecated. No-op.",
        )]
        pub announce: bool,

        #[option(
            name = "action_cache_store_output_metadata",
            old_name = "experimental_action_cache_store_output_metadata",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "no-op",
        )]
        pub action_cache_store_output_metadata: bool,

        #[option(
            name = "discard_actions_after_execution",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "This option is deprecated and has no effect.",
        )]
        pub discard_actions_after_execution: bool,

        #[option(
            name = "defer_param_files",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "This option is deprecated and has no effect and will be removed in the future.",
        )]
        pub defer_param_files: bool,

        #[option(
            name = "check_fileset_dependencies_recursively",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            deprecation_warning = concat!(
                "This flag is a no-op and fileset dependencies are always checked ",
                "to ensure correctness of builds.",
            ),
        )]
        pub check_fileset_dependencies_recursively: bool,

        #[option(
            name = "experimental_skyframe_native_filesets",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            deprecation_warning = "This flag is a no-op and skyframe-native-filesets is always true.",
        )]
        pub skyframe_native_fileset: bool,

        #[option(
            name = "collapse_duplicate_defines",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "no-op",
        )]
        pub collapse_duplicate_defines: bool,

        #[option(
            name = "incompatible_require_javaplugininfo_in_javacommon",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "When enabled java_common.compile only accepts JavaPluginInfo for plugins.",
        )]
        pub require_java_plugin_info: bool,

        #[option(
            name = "experimental_build_transitive_python_runfiles",
            old_name = "incompatible_build_transitive_python_runfiles",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op",
        )]
        pub build_transitive_runfiles_trees: bool,

        #[option(
            name = "experimental_use_new_worker_pool",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op",
        )]
        pub use_new_worker_pool: bool,

        #[option(
            name = "host_crosstool_top",
            default_value = "null",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op.",
        )]
        pub host_crosstool_top: Option<String>,

        #[option(
            name = "experimental_use_semaphore_for_jobs",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op.",
        )]
        pub use_semaphore_for_jobs: bool,

        #[option(
            name = "experimental_skip_ttvs_for_genquery",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [DEPRECATED],
            help = "No-op. Will be removed soon.",
        )]
        pub skip_ttvs: bool,
    }
}

// Java: com/google/devtools/build/lib/bazel/rules/BazelRulesModule.java:358 BazelRulesModule.BazelBuildGraveyardOptions
options! {
    /// This is where deprecated Bazel-specific options only used by the build command go to die.
    pub struct BazelRulesModuleBazelBuildGraveyardOptions {
        // PORT: inherits BuildGraveyardOptions (its @Option fields are copied here).
        #[option(
            name = "incompatible_use_python_toolchains",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "Deprecated. No-op.",
        )]
        pub incompatible_use_python_toolchains: bool,

        #[option(
            name = "experimental_starlark_cc_import",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "Deprecated. No-op.",
        )]
        pub experimental_starlark_cc_import: bool,

        #[option(
            name = "experimental_platform_cc_test",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "Deprecated. No-op.",
        )]
        pub experimental_platform_cc_test: bool,

        #[option(
            name = "j2objc_dead_code_removal",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "Deprecated. No-op.",
        )]
        pub remove_dead_code: bool,

        #[option(
            name = "experimental_proto_extra_actions",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [EXPERIMENTAL],
            help = "Deprecated. No-op.",
        )]
        pub experimental_proto_extra_actions: bool,

        #[option(
            name = "enable_fdo_profile_absolute_path",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "Deprecated. No-op.",
        )]
        pub enable_fdo_profile_absolute_path: bool,

        #[option(
            name = "experimental_genquery_use_graphless_query",
            default_value = "auto",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "Deprecated. No-op.",
        )]
        pub use_graphless_query: TriState,

        #[option(
            name = "use_top_level_targets_for_symlinks",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "Deprecated. No-op.",
        )]
        pub use_top_level_targets_for_symlinks: bool,

        #[option(
            name = "experimental_skyframe_prepare_analysis",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            deprecation_warning = "This flag is a no-op and will be deleted in a future release.",
            help = "Deprecated. No-op.",
        )]
        pub skyframe_prepare_analysis: bool,

        #[option(
            name = "incompatible_disable_legacy_flags_cc_toolchain_api",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [INCOMPATIBLE_CHANGE, DEPRECATED],
            help = concat!(
                "Flag for disabling the legacy cc_toolchain Starlark API for accessing legacy ",
                "CROSSTOOL fields.",
            ),
        )]
        pub disable_legacy_flags_cc_toolchain_api: bool,

        #[option(
            name = "incompatible_enable_profile_by_default",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "No-op.",
        )]
        pub enable_profile_by_default: bool,

        #[option(
            name = "incompatible_override_toolchain_transition",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            deprecation_warning = "This is now always set, please remove this flag.",
            help = "Deprecated, this is no longer in use and should be removed.",
        )]
        pub override_toolchain_transition: bool,

        #[option(
            name = "experimental_parse_headers_skipped_if_corresponding_srcs_found",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op.",
        )]
        pub parse_headers_skipped_if_corresponding_srcs_found: bool,

        #[option(
            name = "experimental_worker_as_resource",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op, will be removed soon.",
        )]
        pub worker_as_resource: bool,

        #[option(
            name = "high_priority_workers",
            default_value = "null",
            allow_multiple = true,
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op, will be removed soon.",
        )]
        pub high_priority_workers: Vec<String>,

        #[option(
            name = "target_platform_fallback",
            default_value = "",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "This option is deprecated and has no effect.",
        )]
        pub target_platform_fallback: String,

        #[option(
            name = "incompatible_auto_configure_host_platform",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "This option is deprecated and has no effect.",
        )]
        pub auto_configure_host_platform: bool,

        #[option(
            name = "experimental_require_availability_info",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            deprecated = true,
            help = "Deprecated no-op.",
        )]
        pub require_availability_info: bool,

        #[option(
            name = "experimental_collect_local_action_metrics",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "Deprecated no-op.",
        )]
        pub collect_local_execution_statistics: bool,

        #[option(
            name = "experimental_collect_local_sandbox_action_metrics",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "Deprecated no-op.",
        )]
        pub collect_local_sandbox_execution_statistics: bool,

        #[option(
            name = "experimental_enable_starlark_doc_extract",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [EXPERIMENTAL],
            help = "Deprecated no-op.",
        )]
        pub enable_bzl_doc_dump: bool,

        #[option(
            name = "experimental_parallel_aquery_output",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op.",
        )]
        pub parallel_aquery_output: bool,

        #[option(
            name = "experimental_show_artifacts",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "Deprecated no-op.",
        )]
        pub show_artifacts: bool,

        #[option(
            name = "announce",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            deprecation_warning = "This option is now deprecated and is a no-op",
            help = "Deprecated. No-op.",
        )]
        pub announce: bool,

        #[option(
            name = "action_cache_store_output_metadata",
            old_name = "experimental_action_cache_store_output_metadata",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "no-op",
        )]
        pub action_cache_store_output_metadata: bool,

        #[option(
            name = "discard_actions_after_execution",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "This option is deprecated and has no effect.",
        )]
        pub discard_actions_after_execution: bool,

        #[option(
            name = "defer_param_files",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "This option is deprecated and has no effect and will be removed in the future.",
        )]
        pub defer_param_files: bool,

        #[option(
            name = "check_fileset_dependencies_recursively",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            deprecation_warning = concat!(
                "This flag is a no-op and fileset dependencies are always checked ",
                "to ensure correctness of builds.",
            ),
        )]
        pub check_fileset_dependencies_recursively: bool,

        #[option(
            name = "experimental_skyframe_native_filesets",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            deprecation_warning = "This flag is a no-op and skyframe-native-filesets is always true.",
        )]
        pub skyframe_native_fileset: bool,

        #[option(
            name = "collapse_duplicate_defines",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "no-op",
        )]
        pub collapse_duplicate_defines: bool,

        #[option(
            name = "incompatible_require_javaplugininfo_in_javacommon",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "When enabled java_common.compile only accepts JavaPluginInfo for plugins.",
        )]
        pub require_java_plugin_info: bool,

        #[option(
            name = "experimental_build_transitive_python_runfiles",
            old_name = "incompatible_build_transitive_python_runfiles",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op",
        )]
        pub build_transitive_runfiles_trees: bool,

        #[option(
            name = "experimental_use_new_worker_pool",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op",
        )]
        pub use_new_worker_pool: bool,

        #[option(
            name = "host_crosstool_top",
            default_value = "null",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op.",
        )]
        pub host_crosstool_top: Option<String>,

        #[option(
            name = "experimental_use_semaphore_for_jobs",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op.",
        )]
        pub use_semaphore_for_jobs: bool,

        #[option(
            name = "experimental_skip_ttvs_for_genquery",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [DEPRECATED],
            help = "No-op. Will be removed soon.",
        )]
        pub skip_ttvs: bool,

        // Fields declared by BazelBuildGraveyardOptions itself.
        #[option(
            name = "python_top",
            default_value = "",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "Deprecated. No-op.",
        )]
        pub python_top: String,

        #[option(
            name = "incompatible_load_python_rules_from_bzl",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "Deprecated no-op.",
        )]
        pub load_python_rules_from_bzl: bool,

        #[option(
            name = "incompatible_load_proto_rules_from_bzl",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "Deprecated no-op.",
        )]
        pub load_proto_rules_from_bzl: bool,

        #[option(
            name = "incompatible_load_java_rules_from_bzl",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "Deprecated no-op.",
        )]
        pub load_java_rules_from_bzl: bool,

        #[option(
            name = "make_variables_source",
            default_value = "configuration",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [HIDDEN, DEPRECATED],
        )]
        pub make_variable_source: String,

        #[option(
            name = "force_ignore_dash_static",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "noop",
        )]
        pub force_ignore_dash_static: bool,

        #[option(
            name = "experimental_profile_action_counts",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op.",
        )]
        pub enable_action_count_profile: bool,

        #[option(
            name = "incompatible_remove_binary_profile",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "No-op.",
        )]
        pub remove_binary_profile: bool,

        #[option(
            name = "experimental_post_profile_started_event",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op.",
        )]
        pub post_profile_started_event: bool,

        #[option(
            name = "incompatible_dont_use_javasourceinfoprovider",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "No-op",
        )]
        pub dont_use_java_source_info_provider: bool,

        #[option(
            name = "android_sdk",
            default_value = "",
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            deprecation_warning = "Legacy Android flags have been deprecated. See https://blog.bazel.build/2023/11/15/android-platforms.html for details and migration directions",
            help = "No-op",
        )]
        pub sdk: String,

        #[option(
            name = "android_cpu",
            default_value = "",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            deprecation_warning = "Legacy Android flags have been deprecated. See https://blog.bazel.build/2023/11/15/android-platforms.html for details and migration directions",
            help = "No-op",
        )]
        pub cpu: String,

        #[option(
            name = "android_crosstool_top",
            default_value = "null",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            deprecation_warning = "Legacy Android flags have been deprecated. See https://blog.bazel.build/2023/11/15/android-platforms.html for details and migration directions",
            help = "No-op",
        )]
        pub android_crosstool_top: Option<String>,

        #[option(
            name = "android_grte_top",
            default_value = "null",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            deprecation_warning = "Legacy Android flags have been deprecated. See https://blog.bazel.build/2023/11/15/android-platforms.html for details and migration directions",
            help = "No-op",
        )]
        pub android_libc_top_label: Option<String>,

        #[option(
            name = "incompatible_enable_android_toolchain_resolution",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            deprecation_warning = "Legacy Android flags have been deprecated. See https://blog.bazel.build/2023/11/15/android-platforms.html for details and migration directions",
            help = "No-op",
        )]
        pub incompatible_use_toolchain_resolution: bool,

        #[option(
            name = "fat_apk_cpu",
            default_value = "armeabi-v7a",
            converter = CommaSeparatedOptionSetConverter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            deprecation_warning = "Legacy Android flags have been deprecated. See https://blog.bazel.build/2023/11/15/android-platforms.html for details and migration directions",
            help = "No-op",
        )]
        pub fat_apk_cpus: Vec<String>,

        #[option(
            name = "incompatible_enable_cgo_toolchain_resolution",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "No-op",
        )]
        pub incompatible_enable_go_toolchain_resolution: bool,
    }
}

// Java: com/google/devtools/build/lib/bazel/rules/BazelRulesModule.java:518 BazelRulesModule.AllCommandGraveyardOptions
options! {
    /// This is where deprecated options which need to be available for all commands go to die. If you
    /// want to graveyard an all-command option specific to Blaze or Bazel, create a subclass.
    pub struct BazelRulesModuleAllCommandGraveyardOptions {
        #[option(
            name = "incompatible_python_disable_py2",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op.",
        )]
        pub disable_py2: bool,

        #[option(
            name = "force_python",
            default_value = "null",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op.",
        )]
        pub force_python: Option<String>,

        #[option(
            name = "host_force_python",
            default_value = "null",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op.",
        )]
        pub host_force_python: Option<String>,

        #[option(
            name = "incompatible_py3_is_default",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op.",
        )]
        pub incompatible_py3_is_default: bool,

        #[option(
            name = "incompatible_py2_outputs_are_suffixed",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op.",
        )]
        pub incompatible_py2_outputs_are_suffixed: bool,

        #[option(
            name = "python_version",
            default_value = "null",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op.",
        )]
        pub python_version: Option<String>,

        #[option(
            name = "incompatible_remove_old_python_version_api",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op.",
        )]
        pub incompatible_remove_old_python_version_api: bool,

        #[option(
            name = "incompatible_allow_python_version_transitions",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op.",
        )]
        pub incompatible_allow_python_version_transitions: bool,

        #[option(
            name = "incompatible_disallow_legacy_py_provider",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op.",
        )]
        pub incompatible_disallow_legacy_py_provider: bool,

        #[option(
            name = "experimental_disallow_legacy_java_toolchain_flags",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op.",
        )]
        pub experimental_disallow_legacy_java_toolchain_flags: bool,

        #[option(
            name = "javabase",
            default_value = "null",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op.",
        )]
        pub javabase: Option<String>,

        #[option(
            name = "java_toolchain",
            default_value = "null",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op.",
        )]
        pub java_toolchain: Option<String>,

        #[option(
            name = "host_java_toolchain",
            default_value = "null",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op.",
        )]
        pub host_java_toolchain: Option<String>,

        #[option(
            name = "host_javabase",
            default_value = "null",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op.",
        )]
        pub host_javabase: Option<String>,

        #[option(
            name = "apple_crosstool_top",
            default_value = "null",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op",
        )]
        pub apple_crosstool_top: Option<String>,

        #[option(
            name = "legacy_bazel_java_test",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op",
        )]
        pub legacy_bazel_java_test: bool,

        #[option(
            name = "strict_deps_java_protos",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op",
        )]
        pub strict_deps_java_protos: bool,

        #[option(
            name = "disallow_strict_deps_for_jpl",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op",
        )]
        pub is_disallow_strict_deps_for_jpl: bool,

        #[option(
            name = "experimental_import_deps_checking",
            default_value = "null",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op",
        )]
        pub import_deps_checking_level: Option<String>,

        #[option(
            name = "experimental_allow_runtime_deps_on_neverlink",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op",
        )]
        pub allow_runtime_deps_on_never_link: bool,

        #[option(
            name = "experimental_limit_android_lint_to_android_constrained_java",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op",
        )]
        pub limit_android_lint_to_android_compatible: bool,

        #[option(
            name = "experimental_java_header_input_pruning",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op",
        )]
        pub experimental_java_header_input_pruning: bool,

        #[option(
            name = "incompatible_dont_collect_native_libraries_in_data",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            deprecated = true,
            help = "No-op.",
        )]
        pub dont_collect_data_libraries: bool,

        #[option(
            name = "experimental_java_header_compilation_direct_deps",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            deprecated = true,
            help = "No-op.",
        )]
        pub java_header_compilation_direct_deps: bool,

        #[option(
            name = "jplPropagateCcLinkParamsStore",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op",
        )]
        pub jpl_propagate_cc_link_params_store: bool,

        #[option(
            name = "incompatible_require_linker_input_cc_api",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op",
        )]
        pub incompatible_require_linker_input_cc_api: bool,

        #[option(
            name = "incompatible_depset_for_libraries_to_link_getter",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op",
        )]
        pub incompatible_depset_for_libraries_to_link_getter: bool,

        #[option(
            name = "legacy_external_runfiles",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op",
        )]
        pub legacy_external_runfiles: bool,

        #[option(
            name = "incompatible_disable_target_provider_fields",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "No-op",
        )]
        pub incompatible_disable_target_provider_fields: bool,

        #[option(
            name = "incompatible_disallow_struct_provider_syntax",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "No-op",
        )]
        pub incompatible_disallow_struct_provider_syntax: bool,

        #[option(
            name = "auto_cpu_environment_group",
            default_value = "",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [EXPERIMENTAL],
            help = "No-op",
        )]
        pub auto_cpu_environment_group: String,

        #[option(
            name = "incompatible_top_level_aspects_require_providers",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "No-op",
        )]
        pub incompatible_top_level_aspects_require_providers: bool,

        #[option(
            name = "separate_aspect_deps",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op",
        )]
        pub separate_aspect_deps: bool,

        #[option(
            name = "incompatible_visibility_private_attributes_at_definition",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "No-op",
        )]
        pub incompatible_visibility_private_attributes_at_definition: bool,

        #[option(
            name = "incompatible_new_actions_api",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "No-op",
        )]
        pub incompatible_new_actions_api: bool,

        #[option(
            name = "experimental_enable_aspect_hints",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
        )]
        pub enable_aspect_hints: bool,

        #[option(
            name = "bes_best_effort",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            deprecation_warning = concat!(
                "BES best effort upload has been removed. The flag has no more ",
                "functionality attached to it and will be removed in a future release.",
            ),
            help = "No-op",
        )]
        pub bes_best_effort: bool,

        #[option(
            name = "experimental_use_event_based_build_completion_status",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS, EXECUTION],
            metadata_tags = [EXPERIMENTAL],
            help = "No-op",
        )]
        pub use_event_based_build_completion_status: bool,

        #[option(
            name = "experimental_java_proto_add_allowed_public_imports",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [EXPERIMENTAL],
            help = "This flag is a noop and scheduled for removal.",
        )]
        pub experimental_java_proto_add_allowed_public_imports: bool,

        #[option(
            name = "java_optimization_mode",
            default_value = "",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "Do not use.",
        )]
        pub java_optimization_mode: String,

        #[option(
            name = "incompatible_depset_for_java_output_source_jars",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "No-op.",
        )]
        pub incompatible_depset_for_java_output_source_jars: bool,

        #[option(
            name = "incompatible_use_plus_in_repo_names",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS],
            help = "No-op.",
        )]
        pub incompatible_use_plus_in_repo_names: bool,

        #[option(
            name = "enable_bzlmod",
            old_name = "experimental_enable_bzlmod",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS],
            help = "No-op.",
        )]
        pub enable_bzlmod: bool,

        #[option(
            name = "enable_workspace",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS],
            help = "No-op.",
        )]
        pub enable_workspace: bool,

        #[option(
            name = "experimental_announce_profile_path",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op.",
        )]
        pub announce_profile_path: bool,

        #[option(
            name = "incompatible_existing_rules_immutable_view",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op.",
        )]
        pub incompatible_existing_rules_immutable_view: bool,

        #[option(
            name = "incompatible_disable_native_repo_rules",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op.",
        )]
        pub incompatible_disable_native_repo_rules: bool,

        #[option(
            name = "incompatible_no_package_distribs",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op.",
        )]
        pub incompatible_no_package_distribs: bool,

        #[option(
            name = "experimental_action_resource_set",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op.",
        )]
        pub experimental_action_resource_set: bool,

        #[option(
            name = "incompatible_macos_set_install_name",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "No-op.",
        )]
        pub macos_set_install_name: bool,

        #[option(
            name = "verbose_explanations",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [AFFECTS_OUTPUTS],
            help = "No-op.",
        )]
        pub verbose_explanations: bool,

        #[option(
            name = "experimental_cc_static_library",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op.",
        )]
        pub experimental_cc_static_library: bool,

        #[option(
            name = "incompatible_legacy_local_fallback",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op.",
        )]
        pub legacy_local_fallback: bool,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const BAZEL_RULES_MODULE_BUILD_GRAVEYARD_OPTIONS_OPTION_NAMES: &[&str] = &[
        "incompatible_use_python_toolchains",
        "experimental_starlark_cc_import",
        "experimental_platform_cc_test",
        "j2objc_dead_code_removal",
        "experimental_proto_extra_actions",
        "enable_fdo_profile_absolute_path",
        "experimental_genquery_use_graphless_query",
        "use_top_level_targets_for_symlinks",
        "experimental_skyframe_prepare_analysis",
        "incompatible_disable_legacy_flags_cc_toolchain_api",
        "incompatible_enable_profile_by_default",
        "incompatible_override_toolchain_transition",
        "experimental_parse_headers_skipped_if_corresponding_srcs_found",
        "experimental_worker_as_resource",
        "high_priority_workers",
        "target_platform_fallback",
        "incompatible_auto_configure_host_platform",
        "experimental_require_availability_info",
        "experimental_collect_local_action_metrics",
        "experimental_collect_local_sandbox_action_metrics",
        "experimental_enable_starlark_doc_extract",
        "experimental_parallel_aquery_output",
        "experimental_show_artifacts",
        "announce",
        "action_cache_store_output_metadata",
        "discard_actions_after_execution",
        "defer_param_files",
        "check_fileset_dependencies_recursively",
        "experimental_skyframe_native_filesets",
        "collapse_duplicate_defines",
        "incompatible_require_javaplugininfo_in_javacommon",
        "experimental_build_transitive_python_runfiles",
        "experimental_use_new_worker_pool",
        "host_crosstool_top",
        "experimental_use_semaphore_for_jobs",
        "experimental_skip_ttvs_for_genquery",
    ];

    #[test]
    fn bazel_rules_module_build_graveyard_options_option_names() {
        super::super::test_util::check_option_names(
            BazelRulesModuleBuildGraveyardOptions::class(),
            BAZEL_RULES_MODULE_BUILD_GRAVEYARD_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn bazel_rules_module_build_graveyard_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<BazelRulesModuleBuildGraveyardOptions>(
            SKIP_ROUND_TRIP,
        );
    }

    /// Option names in Java declaration order (extracted from the Java source).
    const BAZEL_RULES_MODULE_BAZEL_BUILD_GRAVEYARD_OPTIONS_OPTION_NAMES: &[&str] = &[
        "incompatible_use_python_toolchains",
        "experimental_starlark_cc_import",
        "experimental_platform_cc_test",
        "j2objc_dead_code_removal",
        "experimental_proto_extra_actions",
        "enable_fdo_profile_absolute_path",
        "experimental_genquery_use_graphless_query",
        "use_top_level_targets_for_symlinks",
        "experimental_skyframe_prepare_analysis",
        "incompatible_disable_legacy_flags_cc_toolchain_api",
        "incompatible_enable_profile_by_default",
        "incompatible_override_toolchain_transition",
        "experimental_parse_headers_skipped_if_corresponding_srcs_found",
        "experimental_worker_as_resource",
        "high_priority_workers",
        "target_platform_fallback",
        "incompatible_auto_configure_host_platform",
        "experimental_require_availability_info",
        "experimental_collect_local_action_metrics",
        "experimental_collect_local_sandbox_action_metrics",
        "experimental_enable_starlark_doc_extract",
        "experimental_parallel_aquery_output",
        "experimental_show_artifacts",
        "announce",
        "action_cache_store_output_metadata",
        "discard_actions_after_execution",
        "defer_param_files",
        "check_fileset_dependencies_recursively",
        "experimental_skyframe_native_filesets",
        "collapse_duplicate_defines",
        "incompatible_require_javaplugininfo_in_javacommon",
        "experimental_build_transitive_python_runfiles",
        "experimental_use_new_worker_pool",
        "host_crosstool_top",
        "experimental_use_semaphore_for_jobs",
        "experimental_skip_ttvs_for_genquery",
        "python_top",
        "incompatible_load_python_rules_from_bzl",
        "incompatible_load_proto_rules_from_bzl",
        "incompatible_load_java_rules_from_bzl",
        "make_variables_source",
        "force_ignore_dash_static",
        "experimental_profile_action_counts",
        "incompatible_remove_binary_profile",
        "experimental_post_profile_started_event",
        "incompatible_dont_use_javasourceinfoprovider",
        "android_sdk",
        "android_cpu",
        "android_crosstool_top",
        "android_grte_top",
        "incompatible_enable_android_toolchain_resolution",
        "fat_apk_cpu",
        "incompatible_enable_cgo_toolchain_resolution",
    ];

    #[test]
    fn bazel_rules_module_bazel_build_graveyard_options_option_names() {
        super::super::test_util::check_option_names(
            BazelRulesModuleBazelBuildGraveyardOptions::class(),
            BAZEL_RULES_MODULE_BAZEL_BUILD_GRAVEYARD_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn bazel_rules_module_bazel_build_graveyard_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<
            BazelRulesModuleBazelBuildGraveyardOptions,
        >(SKIP_ROUND_TRIP);
    }

    /// Option names in Java declaration order (extracted from the Java source).
    const BAZEL_RULES_MODULE_ALL_COMMAND_GRAVEYARD_OPTIONS_OPTION_NAMES: &[&str] = &[
        "incompatible_python_disable_py2",
        "force_python",
        "host_force_python",
        "incompatible_py3_is_default",
        "incompatible_py2_outputs_are_suffixed",
        "python_version",
        "incompatible_remove_old_python_version_api",
        "incompatible_allow_python_version_transitions",
        "incompatible_disallow_legacy_py_provider",
        "experimental_disallow_legacy_java_toolchain_flags",
        "javabase",
        "java_toolchain",
        "host_java_toolchain",
        "host_javabase",
        "apple_crosstool_top",
        "legacy_bazel_java_test",
        "strict_deps_java_protos",
        "disallow_strict_deps_for_jpl",
        "experimental_import_deps_checking",
        "experimental_allow_runtime_deps_on_neverlink",
        "experimental_limit_android_lint_to_android_constrained_java",
        "experimental_java_header_input_pruning",
        "incompatible_dont_collect_native_libraries_in_data",
        "experimental_java_header_compilation_direct_deps",
        "jplPropagateCcLinkParamsStore",
        "incompatible_require_linker_input_cc_api",
        "incompatible_depset_for_libraries_to_link_getter",
        "legacy_external_runfiles",
        "incompatible_disable_target_provider_fields",
        "incompatible_disallow_struct_provider_syntax",
        "auto_cpu_environment_group",
        "incompatible_top_level_aspects_require_providers",
        "separate_aspect_deps",
        "incompatible_visibility_private_attributes_at_definition",
        "incompatible_new_actions_api",
        "experimental_enable_aspect_hints",
        "bes_best_effort",
        "experimental_use_event_based_build_completion_status",
        "experimental_java_proto_add_allowed_public_imports",
        "java_optimization_mode",
        "incompatible_depset_for_java_output_source_jars",
        "incompatible_use_plus_in_repo_names",
        "enable_bzlmod",
        "enable_workspace",
        "experimental_announce_profile_path",
        "incompatible_existing_rules_immutable_view",
        "incompatible_disable_native_repo_rules",
        "incompatible_no_package_distribs",
        "experimental_action_resource_set",
        "incompatible_macos_set_install_name",
        "verbose_explanations",
        "experimental_cc_static_library",
        "incompatible_legacy_local_fallback",
    ];

    #[test]
    fn bazel_rules_module_all_command_graveyard_options_option_names() {
        super::super::test_util::check_option_names(
            BazelRulesModuleAllCommandGraveyardOptions::class(),
            BAZEL_RULES_MODULE_ALL_COMMAND_GRAVEYARD_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn bazel_rules_module_all_command_graveyard_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<
            BazelRulesModuleAllCommandGraveyardOptions,
        >(SKIP_ROUND_TRIP);
    }
}

// ---- hand-written below this line (preserved by the generator) ----

/// Options whose default cannot be re-parsed in a test (none).
#[cfg(test)]
const SKIP_ROUND_TRIP: &[&str] = &[];

#[cfg(test)]
mod tests {
    use super::super::test_util::parse;
    use super::*;

    #[test]
    fn graveyard_options_parse() {
        let o = parse::<BazelRulesModuleAllCommandGraveyardOptions>(&[
            "--enable_bzlmod",
            "--python_version=PY3",
            "--noincompatible_py3_is_default",
        ]);
        assert!(o.enable_bzlmod);
        assert_eq!(o.python_version.as_deref(), Some("PY3"));
        let o = parse::<BazelRulesModuleBazelBuildGraveyardOptions>(&[
            "--experimental_require_availability_info",
        ]);
        assert!(o.require_availability_info);
    }
}
