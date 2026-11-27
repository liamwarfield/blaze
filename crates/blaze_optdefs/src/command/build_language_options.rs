//! Transcription of `com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:71 BuildLanguageOptions
options! {
    /// Options that affect the semantics of Bazel's build language.
    ///
    /// <p>These are injected into Skyframe (as an instance of {@link StarlarkSemantics}) when a new
    /// build invocation occurs. Changing these options between builds will therefore trigger a
    /// reevaluation of everything that depends on the Starlark interpreter &mdash; in particular,
    /// evaluation of all BUILD and .bzl files.
    ///
    /// <p><em>To add a new option, update the following:</em>
    ///
    /// <ul>
    ///   <li>Add a new {@code @Option}-annotated field to this class. The field name and default value
    ///       should be the same as in {@link StarlarkSemantics}, and the option name in the annotation
    ///       should be that name written in snake_case. Add a line to set the new field in {@link
    ///       #toStarlarkSemantics}. New options should always default to {@code false} and have no
    ///       observable effect when disabled.
    ///   <li>Define a new {@code StarlarkSemantics.Key} or {@code StarlarkSemantics} boolean flag
    ///       identifier.
    ///   <li>Add a line to set the new field in both {@link ConsistencyTest#buildRandomOptions} and
    ///       {@link ConsistencyTest#buildRandomSemantics}.
    ///   <li>Update manual documentation in site/docs/skylark/backward-compatibility.md. Also remember
    ///       to update this when flipping a flag's default value.
    ///   <li>Boolean semantic flags can toggle StarlarkMethod-annotated Java methods (or their
    ///       parameters) on or off, making them selectively invisible to Starlark. To do this, add a new
    ///       entry to {@link BuildLanguageOptions}, then specify the identifier in {@link
    ///       net.starlark.java.annot.StarlarkMethod#enableOnlyWithFlag} or {@link
    ///       net.starlark.java.annot.StarlarkMethod#disableWithFlag}.
    /// </ul>
    pub struct BuildLanguageOptions {
        #[option(
            name = "incompatible_stop_exporting_language_modules",
            default_value = "false",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If enabled, certain language-specific modules (such as `cc_common`) are unavailable in",
                " user .bzl files and may only be called from their respective rules repositories.",
            ),
        )]
        pub incompatible_stop_exporting_language_modules: bool,

        #[option(
            name = "experimental_builtins_bzl_path",
            default_value = "%bundled%",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOSES_INCREMENTAL_STATE, BUILD_FILE_SEMANTICS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "This flag tells Bazel how to find the \"@_builtins\" .bzl files that govern how ",
                "predeclared symbols for BUILD and .bzl files are defined. This flag is only ",
                "intended for Bazel developers, to help when writing @_builtins .bzl code. ",
                "Ordinarily this value is set to \"%bundled%\", which means to use the ",
                "builtins_bzl/ directory packaged in the Bazel binary. However, it can be set to ",
                "the path (relative to the root of the current workspace) of an alternate ",
                "builtins_bzl/ directory, such as one in a Bazel source tree workspace. A literal ",
                "value of \"%workspace%\" is equivalent to the relative package path of ",
                "builtins_bzl/ within a Bazel source tree; this should only be used when running ",
                "Bazel within its own source tree. Finally, a value of the empty string disables ",
                "the builtins injection mechanism entirely.",
            ),
        )]
        pub experimental_builtins_bzl_path: String,

        #[option(
            name = "incompatible_autoload_externally",
            // Java: com/google/devtools/build/lib/packages/semantics/FlagConstants.java:30 FlagConstants.DEFAULT_INCOMPATIBLE_AUTOLOAD_EXTERNALLY
            default_value = "",
            converter = CommaSeparatedOptionSetConverter,
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [LOSES_INCREMENTAL_STATE, BUILD_FILE_SEMANTICS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "A comma-separated list of rules (or other symbols) that were previously part of Bazel",
                " and which are now to be retrieved from their respective external repositories.",
                " This flag is intended to be used to facilitate migration of rules out of Bazel.",
                " See also https://github.com/bazelbuild/bazel/issues/23043.\n",
                "A symbol that is autoloaded within a file behaves as if its built-into-Bazel",
                " definition were replaced by its canonical new definition in an external",
                " repository. For a BUILD file, this essentially means implicitly adding a load()",
                " statement. For a .bzl file, it's either a load() statement or a change to a field",
                " of the `native` object, depending on whether the autoloaded symbol is a rule.\n",
                "Bazel maintains a hardcoded list of all symbols that may be autoloaded; only those",
                " symbols may appear in this flag. For each symbol, Bazel knows the new definition",
                " location in an external repository, as well as a set of special-cased",
                " repositories that must not autoload it to avoid creating cycles.\n",
                "A list item of \"+foo\" in this flag causes symbol foo to be autoloaded, except in",
                " foo's exempt repositories, within which the Bazel-defined version of foo is still",
                " available.\n",
                "A list item of \"foo\" triggers autoloading as above, but the Bazel-defined",
                " version of foo is not made available to the excluded repositories. This ensures",
                " that foo's external repository does not depend on the old Bazel implementation of",
                " foo\n",
                "A list item of \"-foo\" does not trigger any autoloading, but makes the",
                " Bazel-defined version of foo inaccessible throughout the workspace. This is used",
                " to validate that the workspace is ready for foo's definition to be deleted from",
                " Bazel.\n",
                "If a symbol is not named in this flag then it continues to work as normal -- no",
                " autoloading is done, nor is the Bazel-defined version suppressed. For",
                " configuration see",
                " https://github.com/bazelbuild/bazel/blob/master/src/main/java/com/google/devtools/build/lib/packages/AutoloadSymbols.java",
                " As a shortcut also whole repository may be used, for example +@rules_python will",
                " autoload all Python rules.",
            ),
        )]
        pub incompatible_autoload_externally: Vec<String>,

        #[option(
            name = "repositories_without_autoloads",
            default_value = "",
            converter = CommaSeparatedOptionSetConverter,
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [LOSES_INCREMENTAL_STATE, BUILD_FILE_SEMANTICS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "A list of additional repositories (beyond the hardcoded ones Bazel knows about) where ",
                "autoloads are not to be added. This should typically contain repositories that are",
                " transitively depended on by a repository that may be loaded automatically ",
                "(and which can therefore potentially create a cycle).",
            ),
        )]
        pub repositories_without_autoloads: Vec<String>,

        #[option(
            name = "incompatible_disable_autoloads_in_main_repo",
            default_value = "true",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "Controls if the autoloads (set by --incompatible_autoload_externally) are enabled in the",
                "main repository. When enabled the rules (or other symbols) that were previously ",
                "part of Bazel need to have load statements. Use buildifier to add them.",
            ),
        )]
        pub incompatible_disable_autoloads_in_main_repo: bool,

        #[option(
            name = "experimental_builtins_dummy",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BUILD_FILE_SEMANTICS],
            metadata_tags = [EXPERIMENTAL],
            help = "Enables an internal dummy symbol used to test builtins injection.",
        )]
        pub experimental_builtins_dummy: bool,

        #[option(
            name = "experimental_builtins_injection_override",
            default_value = "null",
            converter = CommaSeparatedNonEmptyOptionListConverter,
            allow_multiple = true,
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "A comma-separated list of symbol names prefixed by a + or - character, indicating which",
                " symbols from `@_builtins//:exports.bzl` to inject, overriding their default",
                " injection status. Precisely, this works as follows. Each dict key of",
                " `exported_toplevels` or `exported_rules` has the form `foo`, `+foo`, or `-foo`.",
                " The first two forms mean it gets injected by default, while the last form means",
                " it does not get injected by default. In the first case (unprefixed), the default",
                " is absolute and cannot be overridden. Otherwise, we then consult this options",
                " list, and if we see foo occur here, we take the prefix of its last occurrence and",
                " use that to decide whether or not to inject. It is a no-op to specify an unknown",
                " symbol, or to attempt to not inject a symbol that occurs unprefixed in a dict",
                " key.",
            ),
        )]
        pub experimental_builtins_injection_override: Vec<String>,

        #[option(
            name = "experimental_bzl_visibility",
            default_value = "true",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "If enabled, adds a `visibility()` function that .bzl files may call during top-level",
                " evaluation to set their visibility for the purpose of load() statements.",
            ),
        )]
        pub experimental_bzl_visibility: bool,

        #[option(
            name = "experimental_single_package_toolchain_binding",
            default_value = "false",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If enabled, the register_toolchain function may not include target patterns which may ",
                "refer to more than one package.",
            ),
        )]
        pub experimental_single_package_toolchain_binding: bool,

        #[option(
            name = "allow_experimental_loads",
            default_value = "false",
            documentation_category = INPUT_STRICTNESS,
            effect_tags = [BUILD_FILE_SEMANTICS],
            help = "If enabled, issue only a warning instead of an error for loads of experimental .bzls.",
        )]
        pub allow_experimental_loads: bool,

        #[option(
            name = "check_bzl_visibility",
            default_value = "true",
            documentation_category = INPUT_STRICTNESS,
            effect_tags = [BUILD_FILE_SEMANTICS],
            help = "If disabled, .bzl load visibility errors are demoted to warnings.",
        )]
        pub check_bzl_visibility: bool,

        #[option(
            name = "experimental_cc_skylark_api_enabled_packages",
            default_value = "",
            converter = CommaSeparatedOptionListConverter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "Passes list of packages that can use the C++ Starlark API. Don't enable this flag yet, ",
                "we will be making breaking changes.",
            ),
        )]
        pub experimental_cc_starlark_api_enabled_packages: Vec<String>,

        #[option(
            name = "experimental_enable_android_migration_apis",
            default_value = "false",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [BUILD_FILE_SEMANTICS],
            help = "If set to true, enables the APIs required to support the Android Starlark migration.",
        )]
        pub experimental_enable_android_migration_apis: bool,

        #[option(
            name = "experimental_enable_first_class_macros",
            default_value = "true",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [BUILD_FILE_SEMANTICS],
            help = "If set to true, enables the `macro()` construct for defining symbolic macros.",
        )]
        pub experimental_enable_first_class_macros: bool,

        // TODO(brandjon): point to more extensive user documentation somewhere
        #[option(
            name = "experimental_enable_scl_dialect",
            default_value = "true",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [BUILD_FILE_SEMANTICS],
            help = "If set to true, .scl files may be used in load() statements.",
        )]
        pub experimental_enable_scl_dialect: bool,

        #[option(
            name = "experimental_isolated_extension_usages",
            default_value = "false",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [LOADING_AND_ANALYSIS],
            help = concat!(
                "If true, enables the <code>isolate</code> parameter in the <a",
                " href=\"https://bazel.build/rules/lib/globals/module#use_extension\"><code>use_extension</code></a>",
                " function.",
            ),
        )]
        pub experimental_isolated_extension_usages: bool,

        #[option(
            name = "incompatible_no_implicit_watch_label",
            default_value = "true",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If true, then methods on <code>repository_ctx</code> that are passed a Label will no",
                " longer automatically watch the file under that label for changes even if",
                " <code>watch = \"no\"</code>, and <code>repository_ctx.path</code> no longer",
                " causes the returned path to be watched. Use <code>repository_ctx.watch</code>",
                " instead.",
            ),
        )]
        pub incompatible_no_implicit_watch_label: bool,

        #[option(
            name = "incompatible_stop_exporting_build_file_path",
            default_value = "false",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If set to true, deprecated ctx.build_file_path will not be available. ctx.label.package",
                " + '/BUILD' can be used instead.",
            ),
        )]
        pub incompatible_stop_exporting_build_file_path: bool,

        #[option(
            name = "experimental_google_legacy_api",
            default_value = "false",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "If set to true, exposes a number of experimental pieces of Starlark build API ",
                "pertaining to Google legacy code.",
            ),
        )]
        pub experimental_google_legacy_api: bool,

        #[option(
            name = "experimental_platforms_api",
            default_value = "false",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "If set to true, enables a number of platform-related Starlark APIs useful for ",
                "debugging.",
            ),
        )]
        pub experimental_platforms_api: bool,

        #[option(
            name = "experimental_cc_shared_library",
            default_value = "false",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [BUILD_FILE_SEMANTICS, LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "If set to true, rule attributes and Starlark API methods needed for the rule ",
                "cc_shared_library will be available",
            ),
        )]
        pub experimental_cc_shared_library: bool,

        #[option(
            name = "experimental_repo_remote_exec",
            default_value = "false",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [BUILD_FILE_SEMANTICS, LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
            help = "If set to true, repository_rule gains some remote execution capabilities.",
        )]
        pub experimental_repo_remote_exec: bool,

        #[option(
            name = "experimental_disable_external_package",
            default_value = "false",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [LOADING_AND_ANALYSIS, LOSES_INCREMENTAL_STATE],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "If set to true, the auto-generated //external package will not be available anymore. ",
                "Bazel will still be unable to parse the file 'external/BUILD', but globs reaching ",
                "into external/ from the unnamed package will work.",
            ),
        )]
        pub experimental_disable_external_package: bool,

        #[option(
            name = "experimental_sibling_repository_layout",
            default_value = "false",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [ACTION_COMMAND_LINES, BAZEL_INTERNAL_CONFIGURATION, LOADING_AND_ANALYSIS, LOSES_INCREMENTAL_STATE],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "If set to true, non-main repositories are planted as symlinks to the main repository in",
                " the execution root. That is, all repositories are direct children of the",
                " $output_base/execution_root directory. This has the side effect of freeing up",
                " $output_base/execution_root/__main__/external for the real top-level 'external' ",
                "directory.",
            ),
        )]
        pub experimental_sibling_repository_layout: bool,

        #[option(
            name = "incompatible_allow_tags_propagation",
            old_name = "experimental_allow_tags_propagation",
            default_value = "true",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [BUILD_FILE_SEMANTICS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "If set to true, tags will be propagated from a target to the actions' execution",
                " requirements; otherwise tags are not propagated. See",
                " https://github.com/bazelbuild/bazel/issues/8830 for details.",
            ),
        )]
        pub experimental_allow_tags_propagation: bool,

        #[option(
            name = "incompatible_always_check_depset_elements",
            default_value = "true",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [BUILD_FILE_SEMANTICS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "Check the validity of elements added to depsets, in all constructors. Elements must be",
                " immutable, but historically the depset(direct=...) constructor forgot to check.",
                " Use tuples instead of lists in depset elements.",
                " See https://github.com/bazelbuild/bazel/issues/10313 for details.",
            ),
        )]
        pub incompatible_always_check_depset_elements: bool,

        #[option(
            name = "incompatible_disable_target_default_provider_fields",
            default_value = "false",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [BUILD_FILE_SEMANTICS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If set to true, disable the ability to utilize the default provider via field ",
                "syntax. Use provider-key syntax instead. For example, instead of using ",
                "`ctx.attr.dep.files` to access `files`, utilize `ctx.attr.dep[DefaultInfo].files ",
                "See ",
                "https://github.com/bazelbuild/bazel/issues/9014 for details.",
            ),
        )]
        pub incompatible_disable_target_default_provider_fields: bool,

        #[option(
            name = "incompatible_disallow_empty_glob",
            default_value = "true",
            category = "incompatible changes",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [BUILD_FILE_SEMANTICS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "If set to true, the default value of the `allow_empty` argument of glob() is False.",
        )]
        pub incompatible_disallow_empty_glob: bool,

        #[option(
            name = "incompatible_package_group_has_public_syntax",
            // Java: com/google/devtools/build/lib/packages/semantics/FlagConstants.java:40 FlagConstants.DEFAULT_INCOMPATIBLE_PACKAGE_GROUP_HAS_PUBLIC_SYNTAX
            default_value = "true",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [BUILD_FILE_SEMANTICS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "In package_group's `packages` attribute, allows writing \"public\" or \"private\" to",
                " refer to all packages or no packages respectively.",
            ),
        )]
        pub incompatible_package_group_has_public_syntax: bool,

        #[option(
            name = "incompatible_fix_package_group_reporoot_syntax",
            // Java: com/google/devtools/build/lib/packages/semantics/FlagConstants.java:41 FlagConstants.DEFAULT_INCOMPATIBLE_FIX_PACKAGE_GROUP_REPOROOT_SYNTAX
            default_value = "true",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [BUILD_FILE_SEMANTICS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "In package_group's `packages` attribute, changes the meaning of the value \"//...\" to",
                " refer to all packages in the current repository instead of all packages in any",
                " repository. You can use the special value \"public\" in place of \"//...\" to",
                " obtain the old behavior. This flag requires",
                " that --incompatible_package_group_has_public_syntax also be enabled.",
            ),
        )]
        pub incompatible_fix_package_group_reporoot_syntax: bool,

        #[option(
            name = "incompatible_no_attr_license",
            default_value = "true",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [BUILD_FILE_SEMANTICS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "If set to true, disables the function `attr.license`.",
        )]
        pub incompatible_no_attr_license: bool,

        #[option(
            name = "incompatible_no_implicit_file_export",
            default_value = "false",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [BUILD_FILE_SEMANTICS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If set, (used) source files are package private unless exported explicitly. See ",
                "https://github.com/bazelbuild/proposals/blob/master/designs/",
                "2019-10-24-file-visibility.md",
            ),
        )]
        pub incompatible_no_implicit_file_export: bool,

        #[option(
            name = "incompatible_no_rule_outputs_param",
            default_value = "false",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [BUILD_FILE_SEMANTICS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "If set to true, disables the `outputs` parameter of the `rule()` Starlark function.",
        )]
        pub incompatible_no_rule_outputs_param: bool,

        #[option(
            name = "incompatible_run_shell_command_string",
            default_value = "true",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [BUILD_FILE_SEMANTICS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "If set to true, the command parameter of actions.run_shell will only accept string",
        )]
        pub incompatible_run_shell_command_string: bool,

        /// Used in an integration test to confirm that flags are visible to the interpreter.
        #[option(
            name = "internal_starlark_flag_test_canary",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
        )]
        pub internal_starlark_flag_test_canary: bool,

        #[option(
            name = "incompatible_do_not_split_linking_cmdline",
            default_value = "true",
            documentation_category = BUILD_TIME_OPTIMIZATION,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "When true, Bazel no longer modifies command line flags used for linking, and also ",
                "doesn't selectively decide which flags go to the param file and which don't.  ",
                "See https://github.com/bazelbuild/bazel/issues/7670 for details.",
            ),
        )]
        pub incompatible_do_not_split_linking_cmdline: bool,

        #[option(
            name = "incompatible_use_cc_configure_from_rules_cc",
            default_value = "false",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "When true, Bazel will no longer allow using cc_configure from @bazel_tools. ",
                "Please see https://github.com/bazelbuild/bazel/issues/10134 for details and ",
                "migration instructions.",
            ),
        )]
        pub incompatible_use_cc_configure_from_rules_cc: bool,

        #[option(
            name = "incompatible_unambiguous_label_stringification",
            default_value = "true",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "When true, Bazel will stringify the label @//foo:bar to @//foo:bar, instead of",
                " //foo:bar. This only affects the behavior of str(), the % operator, and so on;",
                " the behavior of repr() is unchanged. See",
                " https://github.com/bazelbuild/bazel/issues/15916 for more information.",
            ),
        )]
        pub incompatible_unambiguous_label_stringification: bool,

        #[option(
            name = "incompatible_java_info_merge_runtime_module_flags",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If set to true, the JavaInfo constructor will merge add_exports and ",
                "add_opens of runtime_deps in addition to deps and exports.",
            ),
        )]
        pub incompatible_java_info_merge_runtime_module_flags: bool,

        #[option(
            name = "max_computation_steps",
            default_value = "0",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [BUILD_FILE_SEMANTICS],
            help = concat!(
                "The maximum number of Starlark computation steps that may be executed by a BUILD file",
                " (zero means no limit).",
            ),
        )]
        pub max_computation_steps: i64,

        #[option(
            name = "nested_set_depth_limit",
            default_value = "3500",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [LOADING_AND_ANALYSIS],
            help = concat!(
                "The maximum depth of the graph internal to a depset (also known as NestedSet), above",
                " which the depset() constructor will fail.",
            ),
        )]
        pub nested_set_depth_limit: i32,

        #[option(
            name = "incompatible_disable_starlark_host_transitions",
            default_value = "false",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If set to true, rule attributes cannot set 'cfg = \"host\"'. Rules should set ",
                "'cfg = \"exec\"' instead.",
            ),
        )]
        pub incompatible_disable_starlark_host_transitions: bool,

        #[option(
            name = "incompatible_disable_objc_library_transition",
            default_value = "true",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [BUILD_FILE_SEMANTICS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "Disable objc_library's custom transition and inherit ",
                "from the top level target instead (No-op in Bazel)",
            ),
        )]
        pub incompatible_disable_objc_library_transition: bool,

        #[option(
            name = "incompatible_disable_transitions_on",
            default_value = "",
            converter = CommaSeparatedOptionSetConverter,
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE, NON_CONFIGURABLE],
            help = "A comma-separated list of flags that cannot be used in transitions inputs or outputs.",
        )]
        pub incompatible_disable_transitions_on: Vec<String>,

        #[option(
            name = "add_go_exec_groups_to_binary_rules",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS],
            help = concat!(
                "When enabled, 'go_build' and 'go_link' execution groups are added to binary rules like",
                " 'cc_binary'.",
            ),
        )]
        pub add_go_exec_groups_to_binary_rules: bool,

        #[option(
            name = "incompatible_fail_on_unknown_attributes",
            default_value = "true",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "If enabled, targets that have unknown attributes set to None fail.",
        )]
        pub incompatible_fail_on_unknown_attributes: bool,

        #[option(
            name = "incompatible_enable_proto_toolchain_resolution",
            // Java: com/google/devtools/build/lib/packages/semantics/FlagConstants.java:48 FlagConstants.DEFAULT_INCOMPATIBLE_ENABLE_PROTO_TOOLCHAIN_RESOLUTION
            default_value = "true",
            documentation_category = TOOLCHAIN,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "If true, proto lang rules define toolchains from protobuf repository.",
        )]
        pub incompatible_enable_proto_toolchain_resolution: bool,

        #[option(
            name = "incompatible_disable_non_executable_java_binary",
            default_value = "false",
            documentation_category = UNCATEGORIZED,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "If true, java_binary is always executable. create_executable attribute is removed.",
        )]
        pub incompatible_disable_non_executable_java_binary: bool,

        #[option(
            name = "experimental_rule_extension_api",
            // Java: com/google/devtools/build/lib/packages/semantics/FlagConstants.java:24 FlagConstants.DEFAULT_EXPERIMENTAL_RULE_EXTENSION_API
            default_value = "true",
            documentation_category = UNCATEGORIZED,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
            help = "Enable experimental rule extension API and subrule APIs",
        )]
        pub experimental_rule_extension_api: bool,

        #[option(
            name = "experimental_dormant_deps",
            default_value = "false",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [BUILD_FILE_SEMANTICS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                " If set to true, attr.label(materializer=), attr(for_dependency_resolution=),",
                " attr.dormant_label(), attr.dormant_label_list() and",
                " rule(for_dependency_resolution=) are allowed.",
            ),
        )]
        pub experimental_dormant_deps: bool,

        #[option(
            name = "experimental_starlark_type_syntax",
            // Java: com/google/devtools/build/lib/packages/semantics/FlagConstants.java:36 FlagConstants.DEFAULT_EXPERIMENTAL_STARLARK_TYPE_SYNTAX
            default_value = "true",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
            help = "Enables type annotations and related syntax in .bzl files. Locations of files where these are allowed is further restricted by `--experimental_starlark_types_allowed_paths`.\nType syntax is never permitted in .scl files regardless of this flag.\n",
        )]
        pub experimental_starlark_type_syntax: bool,

        #[option(
            name = "experimental_starlark_type_checking",
            // Java: com/google/devtools/build/lib/packages/semantics/FlagConstants.java:37 FlagConstants.DEFAULT_EXPERIMENTAL_STARLARK_TYPE_CHECKING
            default_value = "false",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "Enables type checking in files and functions that contain type annotations or related ",
                "syntax. (When this flag is disabled, Bazel is more forgiving of invalid types in ",
                "type annotations.)",
            ),
        )]
        pub experimental_starlark_type_checking: bool,

        #[option(
            name = "experimental_starlark_types",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "No-op. Previously used as --experimental_starlark_type_syntax +",
                " --experimental_starlark_type_checking",
            ),
        )]
        pub experimental_starlark_types: bool,

        #[option(
            name = "experimental_starlark_types_allowed_paths",
            // Java: com/google/devtools/build/lib/packages/semantics/FlagConstants.java:38 FlagConstants.DEFAULT_EXPERIMENTAL_STARLARK_TYPES_ALLOWED_PATHS
            default_value = "",
            converter = CommaSeparatedOptionListConverter,
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
            help = "List of canonical Label prefixes under which Starlark type annotations are allowed.",
        )]
        pub experimental_starlark_types_allowed_paths: Vec<String>,

        #[option(
            name = "incompatible_enable_deprecated_label_apis",
            default_value = "true",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [LOADING_AND_ANALYSIS],
            help = concat!(
                "If enabled, certain deprecated APIs (native.repository_name, Label.workspace_name,",
                " Label.workspace_root, Label.relative) can be used.",
            ),
        )]
        pub enable_deprecated_label_apis: bool,

        #[option(
            name = "incompatible_disallow_ctx_resolve_tools",
            default_value = "true",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If set to true, calling the deprecated ctx.resolve_tools API always fails. Uses of this",
                " API should be replaced by an executable or tools argument to ctx.actions.run or",
                " ctx.actions.run_shell.",
            ),
        )]
        pub incompatible_disallow_ctx_resolve_tools: bool,

        #[option(
            name = "incompatible_simplify_unconditional_selects_in_rule_attrs",
            default_value = "true",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [BUILD_FILE_SEMANTICS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If true, simplify configurable rule attributes which contain only unconditional selects;",
                " for example, if [\"a\"] + select(\"//conditions:default\", [\"b\"]) is assigned",
                " to a rule attribute, it is stored as [\"a\", \"b\"]. This option does not affect",
                " attributes of symbolic macros or attribute default values.",
            ),
        )]
        pub incompatible_simplify_unconditional_selects_in_rule_attrs: bool,

        #[option(
            name = "incompatible_check_external_repo_source_dir_package_boundary",
            default_value = "false",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If true, a source directory in an external repository fails the build if it crosses a",
                " package boundary into a sub-package, matching the behavior that already applies",
                " to source directories in the main repository. If false, such package boundary",
                " crossings inside external repositories are not detected.",
            ),
        )]
        pub incompatible_check_external_repo_source_dir_package_boundary: bool,

        #[option(
            name = "experimental_enable_starlark_set",
            default_value = "true",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [BUILD_FILE_SEMANTICS],
            metadata_tags = [EXPERIMENTAL],
            help = "If true, enable the set data type and set() constructor in Starlark.",
        )]
        pub experimental_enable_starlark_set: bool,

        #[option(
            name = "incompatible_locations_prefers_executable",
            default_value = "true",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "Whether a target that provides an executable expands to the executable rather than the",
                " files in <code>DefaultInfo.files</code> under $(locations ...) expansion if the",
                " number of files is not 1.",
            ),
        )]
        pub incompatible_locations_prefers_executable: bool,

        #[option(
            name = "internal_starlark_utf_8_byte_strings",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BUILD_FILE_SEMANTICS],
            metadata_tags = [HIDDEN],
            help = concat!(
                "Internal use only. Forces the Starlark implementation to operate on strings as raw",
                " UTF-8 byte arrays, matching Bazel's internal string encoding.",
            ),
        )]
        pub internal_starlark_utf8_byte_strings: bool,

        #[option(
            name = "incompatible_enforce_starlark_utf8",
            default_value = "warning",
            converter = Utf8EnforcementModeConverter,
            documentation_category = INPUT_STRICTNESS,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If enabled (or set to 'error'), fail if Starlark files are not UTF-8 encoded. If set to",
                " 'warning', emit a warning instead. If set to 'off', Bazel assumes that Starlark",
                " files are UTF-8 encoded but does not verify this assumption. Note that Starlark",
                " files which are not UTF-8 encoded can cause Bazel to behave inconsistently.",
            ),
        )]
        pub incompatible_enforce_starlark_utf8: Utf8EnforcementMode,

        #[option(
            name = "experimental_repository_ctx_execute_wasm",
            default_value = "false",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
            help = "If true enables the repository_ctx `load_wasm` and `execute_wasm` methods.",
        )]
        pub repository_ctx_execute_wasm: bool,

        #[option(
            name = "incompatible_resolve_select_keys_eagerly",
            default_value = "false",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If enabled, string keys in dicts passed to select() in .bzl files are immediately",
                " resolved to Labels relative to the file instead of being interpreted relative to",
                " the BUILD file they are ultimately loaded from.",
            ),
        )]
        pub incompatible_resolve_select_keys_eagerly: bool,

        #[option(
            name = "force_starlark_stack_trace",
            default_value = "false",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [LOADING_AND_ANALYSIS],
            help = concat!(
                "If --force_starlark_stack_trace=true, Starlark stace traces will always be printed from",
                " calls to fail(), including those normally supressed with fail(..., stack_trace =",
                " False)",
            ),
        )]
        pub force_starlark_stack_trace: bool,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const BUILD_LANGUAGE_OPTIONS_OPTION_NAMES: &[&str] = &[
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

    #[test]
    fn build_language_options_option_names() {
        super::super::test_util::check_option_names(
            BuildLanguageOptions::class(),
            BUILD_LANGUAGE_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn build_language_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<BuildLanguageOptions>(SKIP_ROUND_TRIP);
    }
}

// ---- hand-written below this line (preserved by the generator) ----

// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:813 BuildLanguageOptions.Utf8EnforcementMode
option_enum! {
    /// How strictly Starlark files must be UTF-8 encoded.
    pub enum Utf8EnforcementMode { OFF, WARNING, ERROR }
}

// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:819 BuildLanguageOptions.Utf8EnforcementMode.Converter
bool_or_enum_converter!(
    pub Utf8EnforcementModeConverter,
    Utf8EnforcementMode,
    "UTF-8 enforcement mode",
    Utf8EnforcementMode::ERROR,
    Utf8EnforcementMode::OFF
);

/// A value stored under a `StarlarkSemantics` key.
// PORT: Java's StarlarkSemantics is a typed map (net.starlark.java.eval); Blaze's Starlark layer
// is starlark-rust, so the mapping is kept as data for the wrapper to apply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SemanticsValue {
    Bool(bool),
    String(String),
    StringList(Vec<String>),
    Long(i64),
    Int(i32),
    Utf8EnforcementMode(Utf8EnforcementMode),
}

/// `StarlarkSemantics.Key<T>`: a name and a default value.
#[derive(Clone, Copy, Debug)]
pub struct StarlarkSemanticsKey {
    pub name: &'static str,
    pub default: fn() -> SemanticsValue,
}

// ---- StarlarkSemantics keys (generated from BuildLanguageOptions.java) ----

// Booleans: the +/- prefix indicates the default value (true/false).
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1078 BuildLanguageOptions.INCOMPATIBLE_STOP_EXPORTING_LANGUAGE_MODULES
pub const INCOMPATIBLE_STOP_EXPORTING_LANGUAGE_MODULES: &str =
    "-incompatible_stop_exporting_language_modules";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1082 BuildLanguageOptions.INCOMPATIBLE_ALLOW_TAGS_PROPAGATION
pub const INCOMPATIBLE_ALLOW_TAGS_PROPAGATION: &str = "+incompatible_allow_tags_propagation";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1080 BuildLanguageOptions.INCOMPATIBLE_DISABLE_AUTOLOADS_IN_MAIN_REPO
pub const INCOMPATIBLE_DISABLE_AUTOLOADS_IN_MAIN_REPO: &str =
    "+incompatible_disable_autoloads_in_main_repo";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1084 BuildLanguageOptions.EXPERIMENTAL_BUILTINS_DUMMY
pub const EXPERIMENTAL_BUILTINS_DUMMY: &str = "-experimental_builtins_dummy";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1085 BuildLanguageOptions.EXPERIMENTAL_BZL_VISIBILITY
pub const EXPERIMENTAL_BZL_VISIBILITY: &str = "+experimental_bzl_visibility";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1086 BuildLanguageOptions.ALLOW_EXPERIMENTAL_LOADS
pub const ALLOW_EXPERIMENTAL_LOADS: &str = "-allow_experimental_loads";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1087 BuildLanguageOptions.CHECK_BZL_VISIBILITY
pub const CHECK_BZL_VISIBILITY: &str = "+check_bzl_visibility";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1091 BuildLanguageOptions.EXPERIMENTAL_ENABLE_ANDROID_MIGRATION_APIS
pub const EXPERIMENTAL_ENABLE_ANDROID_MIGRATION_APIS: &str =
    "-experimental_enable_android_migration_apis";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1093 BuildLanguageOptions.EXPERIMENTAL_SINGLE_PACKAGE_TOOLCHAIN_BINDING
pub const EXPERIMENTAL_SINGLE_PACKAGE_TOOLCHAIN_BINDING: &str =
    "-experimental_single_package_toolchain_binding";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1095 BuildLanguageOptions.EXPERIMENTAL_ENABLE_FIRST_CLASS_MACROS
pub const EXPERIMENTAL_ENABLE_FIRST_CLASS_MACROS: &str = "+experimental_enable_first_class_macros";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1097 BuildLanguageOptions.EXPERIMENTAL_ENABLE_SCL_DIALECT
pub const EXPERIMENTAL_ENABLE_SCL_DIALECT: &str = "+experimental_enable_scl_dialect";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1098 BuildLanguageOptions.EXPERIMENTAL_ISOLATED_EXTENSION_USAGES
pub const EXPERIMENTAL_ISOLATED_EXTENSION_USAGES: &str = "-experimental_isolated_extension_usages";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1100 BuildLanguageOptions.INCOMPATIBLE_NO_IMPLICIT_WATCH_LABEL
pub const INCOMPATIBLE_NO_IMPLICIT_WATCH_LABEL: &str = "+incompatible_no_implicit_watch_label";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1102 BuildLanguageOptions.EXPERIMENTAL_GOOGLE_LEGACY_API
pub const EXPERIMENTAL_GOOGLE_LEGACY_API: &str = "-experimental_google_legacy_api";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1103 BuildLanguageOptions.EXPERIMENTAL_PLATFORMS_API
pub const EXPERIMENTAL_PLATFORMS_API: &str = "-experimental_platforms_api";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1088 BuildLanguageOptions.EXPERIMENTAL_CC_SHARED_LIBRARY
pub const EXPERIMENTAL_CC_SHARED_LIBRARY: &str = "-experimental_cc_shared_library";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1104 BuildLanguageOptions.EXPERIMENTAL_REPO_REMOTE_EXEC
pub const EXPERIMENTAL_REPO_REMOTE_EXEC: &str = "-experimental_repo_remote_exec";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1089 BuildLanguageOptions.EXPERIMENTAL_DISABLE_EXTERNAL_PACKAGE
pub const EXPERIMENTAL_DISABLE_EXTERNAL_PACKAGE: &str = "-experimental_disable_external_package";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1105 BuildLanguageOptions.EXPERIMENTAL_SIBLING_REPOSITORY_LAYOUT
pub const EXPERIMENTAL_SIBLING_REPOSITORY_LAYOUT: &str = "-experimental_sibling_repository_layout";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1107 BuildLanguageOptions.INCOMPATIBLE_ALWAYS_CHECK_DEPSET_ELEMENTS
pub const INCOMPATIBLE_ALWAYS_CHECK_DEPSET_ELEMENTS: &str =
    "+incompatible_always_check_depset_elements";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1109 BuildLanguageOptions.INCOMPATIBLE_CHECK_EXTERNAL_REPO_SOURCE_DIR_PACKAGE_BOUNDARY
pub const INCOMPATIBLE_CHECK_EXTERNAL_REPO_SOURCE_DIR_PACKAGE_BOUNDARY: &str =
    "-incompatible_check_external_repo_source_dir_package_boundary";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1113 BuildLanguageOptions.INCOMPATIBLE_DISALLOW_EMPTY_GLOB
pub const INCOMPATIBLE_DISALLOW_EMPTY_GLOB: &str = "+incompatible_disallow_empty_glob";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1114 BuildLanguageOptions.INCOMPATIBLE_PACKAGE_GROUP_HAS_PUBLIC_SYNTAX
// Java: com/google/devtools/build/lib/packages/semantics/FlagConstants.java:43 FlagConstants.INCOMPATIBLE_PACKAGE_GROUP_HAS_PUBLIC_SYNTAX
pub const INCOMPATIBLE_PACKAGE_GROUP_HAS_PUBLIC_SYNTAX: &str =
    "+incompatible_package_group_has_public_syntax";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1116 BuildLanguageOptions.INCOMPATIBLE_FIX_PACKAGE_GROUP_REPOROOT_SYNTAX
// Java: com/google/devtools/build/lib/packages/semantics/FlagConstants.java:45 FlagConstants.INCOMPATIBLE_FIX_PACKAGE_GROUP_REPOROOT_SYNTAX
pub const INCOMPATIBLE_FIX_PACKAGE_GROUP_REPOROOT_SYNTAX: &str =
    "+incompatible_fix_package_group_reporoot_syntax";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1120 BuildLanguageOptions.INCOMPATIBLE_JAVA_INFO_MERGE_RUNTIME_MODULE_FLAGS
pub const INCOMPATIBLE_JAVA_INFO_MERGE_RUNTIME_MODULE_FLAGS: &str =
    "-incompatible_java_info_merge_runtime_module_flags";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1122 BuildLanguageOptions.INCOMPATIBLE_NO_ATTR_LICENSE
pub const INCOMPATIBLE_NO_ATTR_LICENSE: &str = "+incompatible_no_attr_license";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1123 BuildLanguageOptions.INCOMPATIBLE_NO_IMPLICIT_FILE_EXPORT
pub const INCOMPATIBLE_NO_IMPLICIT_FILE_EXPORT: &str = "-incompatible_no_implicit_file_export";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1125 BuildLanguageOptions.INCOMPATIBLE_NO_RULE_OUTPUTS_PARAM
pub const INCOMPATIBLE_NO_RULE_OUTPUTS_PARAM: &str = "-incompatible_no_rule_outputs_param";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1127 BuildLanguageOptions.INCOMPATIBLE_RUN_SHELL_COMMAND_STRING
pub const INCOMPATIBLE_RUN_SHELL_COMMAND_STRING: &str = "+incompatible_run_shell_command_string";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1118 BuildLanguageOptions.INCOMPATIBLE_DO_NOT_SPLIT_LINKING_CMDLINE
pub const INCOMPATIBLE_DO_NOT_SPLIT_LINKING_CMDLINE: &str =
    "+incompatible_do_not_split_linking_cmdline";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1129 BuildLanguageOptions.INCOMPATIBLE_USE_CC_CONFIGURE_FROM_RULES_CC
pub const INCOMPATIBLE_USE_CC_CONFIGURE_FROM_RULES_CC: &str =
    "-incompatible_use_cc_configure_from_rules";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1131 BuildLanguageOptions.INCOMPATIBLE_UNAMBIGUOUS_LABEL_STRINGIFICATION
pub const INCOMPATIBLE_UNAMBIGUOUS_LABEL_STRINGIFICATION: &str =
    "+incompatible_unambiguous_label_stringification";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1133 BuildLanguageOptions.INCOMPATIBLE_DISABLE_STARLARK_HOST_TRANSITIONS
pub const INCOMPATIBLE_DISABLE_STARLARK_HOST_TRANSITIONS: &str =
    "-incompatible_disable_starlark_host_transitions";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1135 BuildLanguageOptions.INCOMPATIBLE_DISABLE_OBJC_LIBRARY_TRANSITION
pub const INCOMPATIBLE_DISABLE_OBJC_LIBRARY_TRANSITION: &str =
    "+incompatible_disable_objc_library_transition";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1137 BuildLanguageOptions.ADD_GO_EXEC_GROUPS_TO_BINARY_RULES
pub const ADD_GO_EXEC_GROUPS_TO_BINARY_RULES: &str = "-add_go_exec_groups_to_binary_rules";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1139 BuildLanguageOptions.INCOMPATIBLE_FAIL_ON_UNKNOWN_ATTRIBUTES
pub const INCOMPATIBLE_FAIL_ON_UNKNOWN_ATTRIBUTES: &str =
    "+incompatible_fail_on_unknown_attributes";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1141 BuildLanguageOptions.INCOMPATIBLE_ENABLE_PROTO_TOOLCHAIN_RESOLUTION
// Java: com/google/devtools/build/lib/packages/semantics/FlagConstants.java:49 FlagConstants.DEFAULT_INCOMPATIBLE_ENABLE_PROTO_TOOLCHAIN_RESOLUTION_NAME
pub const INCOMPATIBLE_ENABLE_PROTO_TOOLCHAIN_RESOLUTION: &str =
    "+incompatible_enable_proto_toolchain_resolution";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1143 BuildLanguageOptions.INCOMPATIBLE_DISABLE_NON_EXECUTABLE_JAVA_BINARY
pub const INCOMPATIBLE_DISABLE_NON_EXECUTABLE_JAVA_BINARY: &str =
    "-incompatible_disable_non_executable_java_binary";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1145 BuildLanguageOptions.INCOMPATIBLE_DISABLE_TARGET_DEFAULT_PROVIDER_FIELDS
pub const INCOMPATIBLE_DISABLE_TARGET_DEFAULT_PROVIDER_FIELDS: &str =
    "-incompatible_disable_target_default_provider_fields";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1147 BuildLanguageOptions.EXPERIMENTAL_RULE_EXTENSION_API
// Java: com/google/devtools/build/lib/packages/semantics/FlagConstants.java:25 FlagConstants.DEFAULT_EXPERIMENTAL_RULE_EXTENSION_API_NAME
pub const EXPERIMENTAL_RULE_EXTENSION_API: &str = "+experimental_rule_extension_api";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1149 BuildLanguageOptions.EXPERIMENTAL_DORMANT_DEPS
pub const EXPERIMENTAL_DORMANT_DEPS: &str = "-experimental_dormant_deps";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1151 BuildLanguageOptions.EXPERIMENTAL_STARLARK_TYPE_SYNTAX
// Java: com/google/devtools/build/lib/packages/semantics/FlagConstants.java:34 FlagConstants.EXPERIMENTAL_STARLARK_TYPE_SYNTAX_FLAG_NAME
pub const EXPERIMENTAL_STARLARK_TYPE_SYNTAX: &str = "+experimental_starlark_type_syntax";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1153 BuildLanguageOptions.INCOMPATIBLE_ENABLE_DEPRECATED_LABEL_APIS
pub const INCOMPATIBLE_ENABLE_DEPRECATED_LABEL_APIS: &str =
    "+incompatible_enable_deprecated_label_apis";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1155 BuildLanguageOptions.INCOMPATIBLE_STOP_EXPORTING_BUILD_FILE_PATH
pub const INCOMPATIBLE_STOP_EXPORTING_BUILD_FILE_PATH: &str =
    "-incompatible_stop_exporting_build_file_path";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1157 BuildLanguageOptions.INCOMPATIBLE_DISALLOW_CTX_RESOLVE_TOOLS
pub const INCOMPATIBLE_DISALLOW_CTX_RESOLVE_TOOLS: &str =
    "+incompatible_disallow_ctx_resolve_tools";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1159 BuildLanguageOptions.INCOMPATIBLE_SIMPLIFY_UNCONDITIONAL_SELECTS_IN_RULE_ATTRS
pub const INCOMPATIBLE_SIMPLIFY_UNCONDITIONAL_SELECTS_IN_RULE_ATTRS: &str =
    "+incompatible_simplify_unconditional_selects_in_rule_attrs";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1161 BuildLanguageOptions.INCOMPATIBLE_LOCATIONS_PREFERS_EXECUTABLE
pub const INCOMPATIBLE_LOCATIONS_PREFERS_EXECUTABLE: &str =
    "+incompatible_locations_prefers_executable";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1165 BuildLanguageOptions.INCOMPATIBLE_RESOLVE_SELECT_KEYS_EAGERLY
pub const INCOMPATIBLE_RESOLVE_SELECT_KEYS_EAGERLY: &str =
    "-incompatible_resolve_select_keys_eagerly";
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1163 BuildLanguageOptions.EXPERIMENTAL_REPOSITORY_CTX_EXECUTE_WASM
pub const EXPERIMENTAL_REPOSITORY_CTX_EXECUTE_WASM: &str =
    "-experimental_repository_ctx_execute_wasm";
// Java: net/starlark/java/eval/StarlarkSemantics.java:255 StarlarkSemantics.PRINT_TEST_MARKER
pub const STARLARK_SEMANTICS_PRINT_TEST_MARKER: &str = "-print_test_marker";
// Java: net/starlark/java/eval/StarlarkSemantics.java:272 StarlarkSemantics.EXPERIMENTAL_STARLARK_TYPE_CHECKING
pub const STARLARK_SEMANTICS_EXPERIMENTAL_STARLARK_TYPE_CHECKING: &str =
    "-experimental_starlark_type_checking";
// Java: net/starlark/java/eval/StarlarkSemantics.java:264 StarlarkSemantics.EXPERIMENTAL_ENABLE_STARLARK_SET
pub const STARLARK_SEMANTICS_EXPERIMENTAL_ENABLE_STARLARK_SET: &str =
    "+experimental_enable_starlark_set";
// Java: net/starlark/java/eval/StarlarkSemantics.java:267 StarlarkSemantics.INTERNAL_BAZEL_ONLY_UTF_8_BYTE_STRINGS
pub const STARLARK_SEMANTICS_INTERNAL_BAZEL_ONLY_UTF_8_BYTE_STRINGS: &str =
    "-internal_bazel_only_utf_8_byte_strings";
// Java: net/starlark/java/eval/StarlarkSemantics.java:276 StarlarkSemantics.FORCE_STARLARK_STACK_TRACE
pub const STARLARK_SEMANTICS_FORCE_STARLARK_STACK_TRACE: &str = "-force_starlark_stack_trace";

// Non-booleans: StarlarkSemantics.Key<T> (name, default value).
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1171 BuildLanguageOptions.EXPERIMENTAL_BUILTINS_BZL_PATH
pub const EXPERIMENTAL_BUILTINS_BZL_PATH: StarlarkSemanticsKey = StarlarkSemanticsKey {
    name: "experimental_builtins_bzl_path",
    default: || SemanticsValue::String("%bundled%".to_string()),
};
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1173 BuildLanguageOptions.INCOMPATIBLE_AUTOLOAD_EXTERNALLY
pub const INCOMPATIBLE_AUTOLOAD_EXTERNALLY: StarlarkSemanticsKey = StarlarkSemanticsKey {
    name: "incompatible_autoload_externally",
    default: || SemanticsValue::StringList(vec![]),
};
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1184 BuildLanguageOptions.REPOSITORIES_WITHOUT_AUTOLOAD
pub const REPOSITORIES_WITHOUT_AUTOLOAD: StarlarkSemanticsKey = StarlarkSemanticsKey {
    name: "repositories_without_autoloads",
    default: || SemanticsValue::StringList(vec![]),
};
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1186 BuildLanguageOptions.EXPERIMENTAL_BUILTINS_INJECTION_OVERRIDE
pub const EXPERIMENTAL_BUILTINS_INJECTION_OVERRIDE: StarlarkSemanticsKey = StarlarkSemanticsKey {
    name: "experimental_builtins_injection_override",
    default: || SemanticsValue::StringList(vec![]),
};
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1189 BuildLanguageOptions.INCOMPATIBLE_ENFORCE_STARLARK_UTF8
pub const INCOMPATIBLE_ENFORCE_STARLARK_UTF8: StarlarkSemanticsKey = StarlarkSemanticsKey {
    name: "incompatible_enforce_starlark_utf8",
    default: || SemanticsValue::Utf8EnforcementMode(Utf8EnforcementMode::WARNING),
};
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1201 BuildLanguageOptions.MAX_COMPUTATION_STEPS
pub const MAX_COMPUTATION_STEPS: StarlarkSemanticsKey = StarlarkSemanticsKey {
    name: "max_computation_steps",
    default: || SemanticsValue::Long(0),
};
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1203 BuildLanguageOptions.NESTED_SET_DEPTH_LIMIT
pub const NESTED_SET_DEPTH_LIMIT: StarlarkSemanticsKey = StarlarkSemanticsKey {
    name: "nested_set_depth_limit",
    default: || SemanticsValue::Int(3500),
};
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1169 BuildLanguageOptions.INCOMPATIBLE_DISABLE_TRANSITIONS_OPTIONS
pub const INCOMPATIBLE_DISABLE_TRANSITIONS_OPTIONS: StarlarkSemanticsKey = StarlarkSemanticsKey {
    name: "incompatible_disable_transitions_on",
    default: || SemanticsValue::StringList(vec![]),
};
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1193 BuildLanguageOptions.EXPERIMENTAL_STARLARK_TYPES_ALLOWED_PATHS
pub const EXPERIMENTAL_STARLARK_TYPES_ALLOWED_PATHS: StarlarkSemanticsKey = StarlarkSemanticsKey {
    name: "experimental_starlark_types",
    default: || SemanticsValue::StringList(vec![]),
};

/// Every `setFlags` binding, in Java order: (option name, semantics key name).
/// Boolean keys carry their default as a `+`/`-` prefix; other keys are listed with their
/// `StarlarkSemantics.Key` name (default in the constant of the same name).
pub const STARLARK_SEMANTICS_FLAGS: &[(&str, &str)] = &[
    (
        "incompatible_stop_exporting_language_modules",
        INCOMPATIBLE_STOP_EXPORTING_LANGUAGE_MODULES,
    ),
    (
        "incompatible_allow_tags_propagation",
        INCOMPATIBLE_ALLOW_TAGS_PROPAGATION,
    ),
    (
        "experimental_builtins_bzl_path",
        EXPERIMENTAL_BUILTINS_BZL_PATH.name,
    ),
    (
        "incompatible_autoload_externally",
        INCOMPATIBLE_AUTOLOAD_EXTERNALLY.name,
    ),
    (
        "repositories_without_autoloads",
        REPOSITORIES_WITHOUT_AUTOLOAD.name,
    ),
    (
        "incompatible_disable_autoloads_in_main_repo",
        INCOMPATIBLE_DISABLE_AUTOLOADS_IN_MAIN_REPO,
    ),
    ("experimental_builtins_dummy", EXPERIMENTAL_BUILTINS_DUMMY),
    (
        "experimental_builtins_injection_override",
        EXPERIMENTAL_BUILTINS_INJECTION_OVERRIDE.name,
    ),
    ("experimental_bzl_visibility", EXPERIMENTAL_BZL_VISIBILITY),
    ("allow_experimental_loads", ALLOW_EXPERIMENTAL_LOADS),
    ("check_bzl_visibility", CHECK_BZL_VISIBILITY),
    (
        "experimental_enable_android_migration_apis",
        EXPERIMENTAL_ENABLE_ANDROID_MIGRATION_APIS,
    ),
    (
        "experimental_single_package_toolchain_binding",
        EXPERIMENTAL_SINGLE_PACKAGE_TOOLCHAIN_BINDING,
    ),
    (
        "experimental_enable_first_class_macros",
        EXPERIMENTAL_ENABLE_FIRST_CLASS_MACROS,
    ),
    (
        "experimental_enable_scl_dialect",
        EXPERIMENTAL_ENABLE_SCL_DIALECT,
    ),
    (
        "experimental_isolated_extension_usages",
        EXPERIMENTAL_ISOLATED_EXTENSION_USAGES,
    ),
    (
        "incompatible_no_implicit_watch_label",
        INCOMPATIBLE_NO_IMPLICIT_WATCH_LABEL,
    ),
    (
        "experimental_google_legacy_api",
        EXPERIMENTAL_GOOGLE_LEGACY_API,
    ),
    ("experimental_platforms_api", EXPERIMENTAL_PLATFORMS_API),
    (
        "experimental_cc_shared_library",
        EXPERIMENTAL_CC_SHARED_LIBRARY,
    ),
    (
        "experimental_repo_remote_exec",
        EXPERIMENTAL_REPO_REMOTE_EXEC,
    ),
    (
        "experimental_disable_external_package",
        EXPERIMENTAL_DISABLE_EXTERNAL_PACKAGE,
    ),
    (
        "experimental_sibling_repository_layout",
        EXPERIMENTAL_SIBLING_REPOSITORY_LAYOUT,
    ),
    (
        "incompatible_always_check_depset_elements",
        INCOMPATIBLE_ALWAYS_CHECK_DEPSET_ELEMENTS,
    ),
    (
        "incompatible_check_external_repo_source_dir_package_boundary",
        INCOMPATIBLE_CHECK_EXTERNAL_REPO_SOURCE_DIR_PACKAGE_BOUNDARY,
    ),
    (
        "incompatible_disallow_empty_glob",
        INCOMPATIBLE_DISALLOW_EMPTY_GLOB,
    ),
    (
        "incompatible_package_group_has_public_syntax",
        INCOMPATIBLE_PACKAGE_GROUP_HAS_PUBLIC_SYNTAX,
    ),
    (
        "incompatible_fix_package_group_reporoot_syntax",
        INCOMPATIBLE_FIX_PACKAGE_GROUP_REPOROOT_SYNTAX,
    ),
    (
        "incompatible_java_info_merge_runtime_module_flags",
        INCOMPATIBLE_JAVA_INFO_MERGE_RUNTIME_MODULE_FLAGS,
    ),
    ("incompatible_no_attr_license", INCOMPATIBLE_NO_ATTR_LICENSE),
    (
        "incompatible_no_implicit_file_export",
        INCOMPATIBLE_NO_IMPLICIT_FILE_EXPORT,
    ),
    (
        "incompatible_no_rule_outputs_param",
        INCOMPATIBLE_NO_RULE_OUTPUTS_PARAM,
    ),
    (
        "incompatible_run_shell_command_string",
        INCOMPATIBLE_RUN_SHELL_COMMAND_STRING,
    ),
    (
        "internal_starlark_flag_test_canary",
        STARLARK_SEMANTICS_PRINT_TEST_MARKER,
    ),
    (
        "incompatible_do_not_split_linking_cmdline",
        INCOMPATIBLE_DO_NOT_SPLIT_LINKING_CMDLINE,
    ),
    (
        "incompatible_enforce_starlark_utf8",
        INCOMPATIBLE_ENFORCE_STARLARK_UTF8.name,
    ),
    (
        "incompatible_use_cc_configure_from_rules_cc",
        INCOMPATIBLE_USE_CC_CONFIGURE_FROM_RULES_CC,
    ),
    (
        "incompatible_unambiguous_label_stringification",
        INCOMPATIBLE_UNAMBIGUOUS_LABEL_STRINGIFICATION,
    ),
    ("max_computation_steps", MAX_COMPUTATION_STEPS.name),
    ("nested_set_depth_limit", NESTED_SET_DEPTH_LIMIT.name),
    (
        "incompatible_disable_starlark_host_transitions",
        INCOMPATIBLE_DISABLE_STARLARK_HOST_TRANSITIONS,
    ),
    (
        "incompatible_disable_objc_library_transition",
        INCOMPATIBLE_DISABLE_OBJC_LIBRARY_TRANSITION,
    ),
    (
        "incompatible_disable_transitions_on",
        INCOMPATIBLE_DISABLE_TRANSITIONS_OPTIONS.name,
    ),
    (
        "add_go_exec_groups_to_binary_rules",
        ADD_GO_EXEC_GROUPS_TO_BINARY_RULES,
    ),
    (
        "incompatible_fail_on_unknown_attributes",
        INCOMPATIBLE_FAIL_ON_UNKNOWN_ATTRIBUTES,
    ),
    (
        "incompatible_enable_proto_toolchain_resolution",
        INCOMPATIBLE_ENABLE_PROTO_TOOLCHAIN_RESOLUTION,
    ),
    (
        "incompatible_disable_non_executable_java_binary",
        INCOMPATIBLE_DISABLE_NON_EXECUTABLE_JAVA_BINARY,
    ),
    (
        "incompatible_disable_target_default_provider_fields",
        INCOMPATIBLE_DISABLE_TARGET_DEFAULT_PROVIDER_FIELDS,
    ),
    (
        "experimental_rule_extension_api",
        EXPERIMENTAL_RULE_EXTENSION_API,
    ),
    ("experimental_dormant_deps", EXPERIMENTAL_DORMANT_DEPS),
    (
        "experimental_starlark_type_syntax",
        EXPERIMENTAL_STARLARK_TYPE_SYNTAX,
    ),
    (
        "experimental_starlark_type_checking",
        STARLARK_SEMANTICS_EXPERIMENTAL_STARLARK_TYPE_CHECKING,
    ),
    (
        "experimental_starlark_types_allowed_paths",
        EXPERIMENTAL_STARLARK_TYPES_ALLOWED_PATHS.name,
    ),
    (
        "incompatible_enable_deprecated_label_apis",
        INCOMPATIBLE_ENABLE_DEPRECATED_LABEL_APIS,
    ),
    (
        "incompatible_stop_exporting_build_file_path",
        INCOMPATIBLE_STOP_EXPORTING_BUILD_FILE_PATH,
    ),
    (
        "incompatible_disallow_ctx_resolve_tools",
        INCOMPATIBLE_DISALLOW_CTX_RESOLVE_TOOLS,
    ),
    (
        "incompatible_simplify_unconditional_selects_in_rule_attrs",
        INCOMPATIBLE_SIMPLIFY_UNCONDITIONAL_SELECTS_IN_RULE_ATTRS,
    ),
    (
        "incompatible_locations_prefers_executable",
        INCOMPATIBLE_LOCATIONS_PREFERS_EXECUTABLE,
    ),
    (
        "incompatible_resolve_select_keys_eagerly",
        INCOMPATIBLE_RESOLVE_SELECT_KEYS_EAGERLY,
    ),
    (
        "experimental_enable_starlark_set",
        STARLARK_SEMANTICS_EXPERIMENTAL_ENABLE_STARLARK_SET,
    ),
    (
        "internal_starlark_utf_8_byte_strings",
        STARLARK_SEMANTICS_INTERNAL_BAZEL_ONLY_UTF_8_BYTE_STRINGS,
    ),
    (
        "experimental_repository_ctx_execute_wasm",
        EXPERIMENTAL_REPOSITORY_CTX_EXECUTE_WASM,
    ),
    (
        "force_starlark_stack_trace",
        STARLARK_SEMANTICS_FORCE_STARLARK_STACK_TRACE,
    ),
];

impl BuildLanguageOptions {
    // Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:890 BuildLanguageOptions.setFlags
    /// The (key, value) pairs `toStarlarkSemantics` puts into the `StarlarkSemantics` builder,
    /// in Java order. Boolean keys keep their `+`/`-` prefix (`builder.setBool(key, value)`).
    pub fn set_flags(&self) -> Vec<(&'static str, SemanticsValue)> {
        vec![
            (
                INCOMPATIBLE_STOP_EXPORTING_LANGUAGE_MODULES,
                SemanticsValue::Bool(self.incompatible_stop_exporting_language_modules),
            ),
            (
                INCOMPATIBLE_ALLOW_TAGS_PROPAGATION,
                SemanticsValue::Bool(self.experimental_allow_tags_propagation),
            ),
            (
                EXPERIMENTAL_BUILTINS_BZL_PATH.name,
                SemanticsValue::String(self.experimental_builtins_bzl_path.clone()),
            ),
            (
                INCOMPATIBLE_AUTOLOAD_EXTERNALLY.name,
                SemanticsValue::StringList(self.incompatible_autoload_externally.clone()),
            ),
            (
                REPOSITORIES_WITHOUT_AUTOLOAD.name,
                SemanticsValue::StringList(self.repositories_without_autoloads.clone()),
            ),
            (
                INCOMPATIBLE_DISABLE_AUTOLOADS_IN_MAIN_REPO,
                SemanticsValue::Bool(self.incompatible_disable_autoloads_in_main_repo),
            ),
            (
                EXPERIMENTAL_BUILTINS_DUMMY,
                SemanticsValue::Bool(self.experimental_builtins_dummy),
            ),
            (
                EXPERIMENTAL_BUILTINS_INJECTION_OVERRIDE.name,
                SemanticsValue::StringList(self.experimental_builtins_injection_override.clone()),
            ),
            (
                EXPERIMENTAL_BZL_VISIBILITY,
                SemanticsValue::Bool(self.experimental_bzl_visibility),
            ),
            (
                ALLOW_EXPERIMENTAL_LOADS,
                SemanticsValue::Bool(self.allow_experimental_loads),
            ),
            (
                CHECK_BZL_VISIBILITY,
                SemanticsValue::Bool(self.check_bzl_visibility),
            ),
            (
                EXPERIMENTAL_ENABLE_ANDROID_MIGRATION_APIS,
                SemanticsValue::Bool(self.experimental_enable_android_migration_apis),
            ),
            (
                EXPERIMENTAL_SINGLE_PACKAGE_TOOLCHAIN_BINDING,
                SemanticsValue::Bool(self.experimental_single_package_toolchain_binding),
            ),
            (
                EXPERIMENTAL_ENABLE_FIRST_CLASS_MACROS,
                SemanticsValue::Bool(self.experimental_enable_first_class_macros),
            ),
            (
                EXPERIMENTAL_ENABLE_SCL_DIALECT,
                SemanticsValue::Bool(self.experimental_enable_scl_dialect),
            ),
            (
                EXPERIMENTAL_ISOLATED_EXTENSION_USAGES,
                SemanticsValue::Bool(self.experimental_isolated_extension_usages),
            ),
            (
                INCOMPATIBLE_NO_IMPLICIT_WATCH_LABEL,
                SemanticsValue::Bool(self.incompatible_no_implicit_watch_label),
            ),
            (
                EXPERIMENTAL_GOOGLE_LEGACY_API,
                SemanticsValue::Bool(self.experimental_google_legacy_api),
            ),
            (
                EXPERIMENTAL_PLATFORMS_API,
                SemanticsValue::Bool(self.experimental_platforms_api),
            ),
            (
                EXPERIMENTAL_CC_SHARED_LIBRARY,
                SemanticsValue::Bool(self.experimental_cc_shared_library),
            ),
            (
                EXPERIMENTAL_REPO_REMOTE_EXEC,
                SemanticsValue::Bool(self.experimental_repo_remote_exec),
            ),
            (
                EXPERIMENTAL_DISABLE_EXTERNAL_PACKAGE,
                SemanticsValue::Bool(self.experimental_disable_external_package),
            ),
            (
                EXPERIMENTAL_SIBLING_REPOSITORY_LAYOUT,
                SemanticsValue::Bool(self.experimental_sibling_repository_layout),
            ),
            (
                INCOMPATIBLE_ALWAYS_CHECK_DEPSET_ELEMENTS,
                SemanticsValue::Bool(self.incompatible_always_check_depset_elements),
            ),
            (
                INCOMPATIBLE_CHECK_EXTERNAL_REPO_SOURCE_DIR_PACKAGE_BOUNDARY,
                SemanticsValue::Bool(
                    self.incompatible_check_external_repo_source_dir_package_boundary,
                ),
            ),
            (
                INCOMPATIBLE_DISALLOW_EMPTY_GLOB,
                SemanticsValue::Bool(self.incompatible_disallow_empty_glob),
            ),
            (
                INCOMPATIBLE_PACKAGE_GROUP_HAS_PUBLIC_SYNTAX,
                SemanticsValue::Bool(self.incompatible_package_group_has_public_syntax),
            ),
            (
                INCOMPATIBLE_FIX_PACKAGE_GROUP_REPOROOT_SYNTAX,
                SemanticsValue::Bool(self.incompatible_fix_package_group_reporoot_syntax),
            ),
            (
                INCOMPATIBLE_JAVA_INFO_MERGE_RUNTIME_MODULE_FLAGS,
                SemanticsValue::Bool(self.incompatible_java_info_merge_runtime_module_flags),
            ),
            (
                INCOMPATIBLE_NO_ATTR_LICENSE,
                SemanticsValue::Bool(self.incompatible_no_attr_license),
            ),
            (
                INCOMPATIBLE_NO_IMPLICIT_FILE_EXPORT,
                SemanticsValue::Bool(self.incompatible_no_implicit_file_export),
            ),
            (
                INCOMPATIBLE_NO_RULE_OUTPUTS_PARAM,
                SemanticsValue::Bool(self.incompatible_no_rule_outputs_param),
            ),
            (
                INCOMPATIBLE_RUN_SHELL_COMMAND_STRING,
                SemanticsValue::Bool(self.incompatible_run_shell_command_string),
            ),
            (
                STARLARK_SEMANTICS_PRINT_TEST_MARKER,
                SemanticsValue::Bool(self.internal_starlark_flag_test_canary),
            ),
            (
                INCOMPATIBLE_DO_NOT_SPLIT_LINKING_CMDLINE,
                SemanticsValue::Bool(self.incompatible_do_not_split_linking_cmdline),
            ),
            (
                INCOMPATIBLE_ENFORCE_STARLARK_UTF8.name,
                SemanticsValue::Utf8EnforcementMode(self.incompatible_enforce_starlark_utf8),
            ),
            (
                INCOMPATIBLE_USE_CC_CONFIGURE_FROM_RULES_CC,
                SemanticsValue::Bool(self.incompatible_use_cc_configure_from_rules_cc),
            ),
            (
                INCOMPATIBLE_UNAMBIGUOUS_LABEL_STRINGIFICATION,
                SemanticsValue::Bool(self.incompatible_unambiguous_label_stringification),
            ),
            (
                MAX_COMPUTATION_STEPS.name,
                SemanticsValue::Long(self.max_computation_steps),
            ),
            (
                NESTED_SET_DEPTH_LIMIT.name,
                SemanticsValue::Int(self.nested_set_depth_limit),
            ),
            (
                INCOMPATIBLE_DISABLE_STARLARK_HOST_TRANSITIONS,
                SemanticsValue::Bool(self.incompatible_disable_starlark_host_transitions),
            ),
            (
                INCOMPATIBLE_DISABLE_OBJC_LIBRARY_TRANSITION,
                SemanticsValue::Bool(self.incompatible_disable_objc_library_transition),
            ),
            (
                INCOMPATIBLE_DISABLE_TRANSITIONS_OPTIONS.name,
                SemanticsValue::StringList(self.incompatible_disable_transitions_on.clone()),
            ),
            (
                ADD_GO_EXEC_GROUPS_TO_BINARY_RULES,
                SemanticsValue::Bool(self.add_go_exec_groups_to_binary_rules),
            ),
            (
                INCOMPATIBLE_FAIL_ON_UNKNOWN_ATTRIBUTES,
                SemanticsValue::Bool(self.incompatible_fail_on_unknown_attributes),
            ),
            (
                INCOMPATIBLE_ENABLE_PROTO_TOOLCHAIN_RESOLUTION,
                SemanticsValue::Bool(self.incompatible_enable_proto_toolchain_resolution),
            ),
            (
                INCOMPATIBLE_DISABLE_NON_EXECUTABLE_JAVA_BINARY,
                SemanticsValue::Bool(self.incompatible_disable_non_executable_java_binary),
            ),
            (
                INCOMPATIBLE_DISABLE_TARGET_DEFAULT_PROVIDER_FIELDS,
                SemanticsValue::Bool(self.incompatible_disable_target_default_provider_fields),
            ),
            (
                EXPERIMENTAL_RULE_EXTENSION_API,
                SemanticsValue::Bool(self.experimental_rule_extension_api),
            ),
            (
                EXPERIMENTAL_DORMANT_DEPS,
                SemanticsValue::Bool(self.experimental_dormant_deps),
            ),
            (
                EXPERIMENTAL_STARLARK_TYPE_SYNTAX,
                SemanticsValue::Bool(self.experimental_starlark_type_syntax),
            ),
            (
                STARLARK_SEMANTICS_EXPERIMENTAL_STARLARK_TYPE_CHECKING,
                SemanticsValue::Bool(self.experimental_starlark_type_checking),
            ),
            (
                EXPERIMENTAL_STARLARK_TYPES_ALLOWED_PATHS.name,
                SemanticsValue::StringList(self.experimental_starlark_types_allowed_paths.clone()),
            ),
            (
                INCOMPATIBLE_ENABLE_DEPRECATED_LABEL_APIS,
                SemanticsValue::Bool(self.enable_deprecated_label_apis),
            ),
            (
                INCOMPATIBLE_STOP_EXPORTING_BUILD_FILE_PATH,
                SemanticsValue::Bool(self.incompatible_stop_exporting_build_file_path),
            ),
            (
                INCOMPATIBLE_DISALLOW_CTX_RESOLVE_TOOLS,
                SemanticsValue::Bool(self.incompatible_disallow_ctx_resolve_tools),
            ),
            (
                INCOMPATIBLE_SIMPLIFY_UNCONDITIONAL_SELECTS_IN_RULE_ATTRS,
                SemanticsValue::Bool(
                    self.incompatible_simplify_unconditional_selects_in_rule_attrs,
                ),
            ),
            (
                INCOMPATIBLE_LOCATIONS_PREFERS_EXECUTABLE,
                SemanticsValue::Bool(self.incompatible_locations_prefers_executable),
            ),
            (
                INCOMPATIBLE_RESOLVE_SELECT_KEYS_EAGERLY,
                SemanticsValue::Bool(self.incompatible_resolve_select_keys_eagerly),
            ),
            (
                STARLARK_SEMANTICS_EXPERIMENTAL_ENABLE_STARLARK_SET,
                SemanticsValue::Bool(self.experimental_enable_starlark_set),
            ),
            (
                STARLARK_SEMANTICS_INTERNAL_BAZEL_ONLY_UTF_8_BYTE_STRINGS,
                SemanticsValue::Bool(self.internal_starlark_utf8_byte_strings),
            ),
            (
                EXPERIMENTAL_REPOSITORY_CTX_EXECUTE_WASM,
                SemanticsValue::Bool(self.repository_ctx_execute_wasm),
            ),
            (
                STARLARK_SEMANTICS_FORCE_STARLARK_STACK_TRACE,
                SemanticsValue::Bool(self.force_starlark_stack_trace),
            ),
        ]
    }
}

/// Options whose default cannot be re-parsed in a test (none).
#[cfg(test)]
const SKIP_ROUND_TRIP: &[&str] = &[];

#[cfg(test)]
mod tests {
    use super::super::test_util::parse;
    use super::*;

    #[test]
    fn semantics_flags_cover_their_options() {
        // Every option named in the mapping is registered and maps to a value of the key's kind.
        // (A boolean key's +/- prefix is the StarlarkSemantics default, which need not equal the
        // flag default: internal_starlark_utf_8_byte_strings defaults to true.)
        let defaults = BuildLanguageOptions::default();
        let values = defaults.set_flags();
        assert_eq!(values.len(), STARLARK_SEMANTICS_FLAGS.len());
        for ((option, key), (key2, value)) in STARLARK_SEMANTICS_FLAGS.iter().zip(values) {
            assert_eq!(*key, key2);
            assert!(
                BuildLanguageOptions::class()
                    .definitions()
                    .any(|d| d.get_option_name() == *option),
                "{}",
                option
            );
            let is_bool_key = key.starts_with('+') || key.starts_with('-');
            assert_eq!(
                is_bool_key,
                matches!(value, SemanticsValue::Bool(_)),
                "{}",
                option
            );
        }
        assert_eq!(STARLARK_SEMANTICS_FLAGS.len(), 63);
    }

    // Ported from src/test/java/com/google/devtools/build/lib/packages/semantics/ConsistencyTest.java.

    #[test]
    #[ignore = "needs StarlarkSemantics and randomized option/semantics builders (Starlark layer)"]
    fn options_to_semantics() {}

    #[test]
    #[ignore = "needs StarlarkSemantics serialization"]
    fn serialization_round_trip() {}

    #[test]
    fn check_defaults_match() {
        // StarlarkSemantics.DEFAULT: boolean keys default to their +/- prefix, other keys to the
        // Key default; "This flag must be false in Starlark, but true in Bazel by default."
        for (key, value) in BuildLanguageOptions::default().set_flags() {
            let expected = if key == STARLARK_SEMANTICS_INTERNAL_BAZEL_ONLY_UTF_8_BYTE_STRINGS {
                SemanticsValue::Bool(true)
            } else if key.starts_with('+') || key.starts_with('-') {
                SemanticsValue::Bool(key.starts_with('+'))
            } else {
                continue;
            };
            assert_eq!(value, expected, "{}", key);
        }
    }

    #[test]
    #[ignore = "needs StarlarkSemantics (Starlark layer)"]
    fn can_get_builder_from_instance() {}

    #[test]
    fn non_boolean_key_defaults_match_option_defaults() {
        let values: Vec<_> = BuildLanguageOptions::default().set_flags();
        for key in [
            INCOMPATIBLE_DISABLE_TRANSITIONS_OPTIONS,
            EXPERIMENTAL_BUILTINS_BZL_PATH,
            INCOMPATIBLE_AUTOLOAD_EXTERNALLY,
            REPOSITORIES_WITHOUT_AUTOLOAD,
            EXPERIMENTAL_BUILTINS_INJECTION_OVERRIDE,
            INCOMPATIBLE_ENFORCE_STARLARK_UTF8,
            EXPERIMENTAL_STARLARK_TYPES_ALLOWED_PATHS,
            MAX_COMPUTATION_STEPS,
            NESTED_SET_DEPTH_LIMIT,
        ] {
            let (_, v) = values.iter().find(|(k, _)| *k == key.name).unwrap();
            assert_eq!(*v, (key.default)(), "{}", key.name);
        }
    }

    #[test]
    fn utf8_enforcement_bool_or_enum() {
        let o = parse::<BuildLanguageOptions>(&["--incompatible_enforce_starlark_utf8"]);
        assert_eq!(
            o.incompatible_enforce_starlark_utf8,
            Utf8EnforcementMode::ERROR
        );
        let o = parse::<BuildLanguageOptions>(&["--noincompatible_enforce_starlark_utf8"]);
        assert_eq!(
            o.incompatible_enforce_starlark_utf8,
            Utf8EnforcementMode::OFF
        );
        let o = parse::<BuildLanguageOptions>(&["--incompatible_enforce_starlark_utf8=warning"]);
        assert_eq!(
            o.incompatible_enforce_starlark_utf8,
            Utf8EnforcementMode::WARNING
        );
    }

    #[test]
    fn autoload_externally_is_a_sorted_set() {
        let o = parse::<BuildLanguageOptions>(&["--incompatible_autoload_externally=b,a,b"]);
        assert_eq!(o.incompatible_autoload_externally, vec!["a", "b"]);
    }
}
