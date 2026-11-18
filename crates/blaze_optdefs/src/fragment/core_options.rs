//! Port of `analysis/config/CoreOptions.java`.

use blaze_options::Foreign;
use std::collections::{BTreeSet, HashMap};

use blaze_cmdline::Label;
use blaze_options::{
    AssignmentConverter, BooleanConverter, CommaSeparatedOptionListConverter,
    CommaSeparatedOptionSetConverter, ConversionContext, Converter, EnvVar, EnvVarsConverter,
    TriState, Value, options,
};

use super::auto_cpu_converter::AutoCpuConverter;
use super::compilation_mode::{CompilationMode, CompilationModeConverter};
use super::converters::{
    ConvResult, CustomFlagConverter, FlagAliasConverter, LabelListConverter, LabelToStringEntry,
    LabelToStringEntryConverter, StringToLabelEntry,
};
use super::execution_info_modifier::{ExecutionInfoModifier, ExecutionInfoModifierConverter};
use super::fragment_options::{
    FragmentOptions, dedup_and_sort, java_string_compare, normalize_entries, normalize_env_vars,
};
use super::regex_filter::{RegexFilter, RegexFilterConverter};
use super::run_under::{RunUnder, RunUnderConverter};

// Java: com/google/devtools/build/lib/analysis/config/CoreOptions.java:64 CoreOptions
options! {
    /// Core options affecting a `BuildConfigurationValue` that don't belong in domain-specific
    /// `FragmentOptions`. All options defined here should be universal in that they affect
    /// configuration regardless of which languages a build uses. In other words, this should only
    /// contain options that aren't suitable for Starlark configuration.
    ///
    /// IMPORTANT: when adding new options, be sure to consider whether those values should be
    /// propagated to the exec configuration or not.
    ///
    /// ALSO IMPORTANT: all option types MUST define a toString method that gives identical results
    /// for semantically identical option values. The simplest way to ensure that is to return the
    /// input string.
    pub struct CoreOptions {
        #[option(
            name = "incompatible_filegroup_runfiles_for_data",
            default_value = "true",
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [UNKNOWN],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If true, runfiles of targets listed in the srcs attribute are available to targets",
                " that consume the filegroup as a data dependency.",
            ),
        )]
        pub filegroup_runfiles_for_data: bool,

        #[option(
            name = "scl_config",
            default_value = "null",
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "Name of the scl config defined in PROJECT.scl. Note that this feature is still under",
                " development b/324119879.",
            ),
        )]
        pub scl_config: Option<String>,

        #[option(
            name = "incompatible_merge_genfiles_directory",
            default_value = "true",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "If true, the genfiles directory is folded into the bin directory.",
        )]
        pub merge_genfiles_directory: bool,

        #[option(
            name = "experimental_exec_config",
            default_value = "@_builtins//:common/builtin_exec_platforms.bzl%bazel_exec_transition",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "If set to `//some:label:my.bzl%my_transition`, uses my_transition for `cfg = \"exec\"`\n",
                "semantics instead of Bazel's internal exec transition logic. Else uses Bazel's\n",
                "internal logic.\n",
            ),
        )]
        pub starlark_exec_config: String,

        #[option(
            name = "incompatible_disable_select_on",
            default_value = "",
            converter = CommaSeparatedOptionSetConverter,
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE, NON_CONFIGURABLE],
            help = "List of flags for which the use in `select()` is disabled.",
        )]
        pub disabled_select_options: Vec<String>,

        #[option(
            name = "experimental_propagate_custom_flag",
            default_value = "null",
            allow_multiple = true,
            converter = CustomFlagConverter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [AFFECTS_OUTPUTS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "Which custom flags (starlark flags or defines) to propagate to the exec transition, by\n",
                "key. e.g. if `--define=a=b` should be propagated, set\n",
                "`--experimental_propagate_custom_flag=a`\n",
            ),
        )]
        pub custom_flags_to_propagate: Vec<String>,

        #[option(
            name = "experimental_exclude_defines_from_exec_config",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [AFFECTS_OUTPUTS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "If true, don't propagate `--define`s to the exec transition at default; only propagate\n",
                "defines specified by `--experimental_propagate_custom_flag`.\n",
            ),
        )]
        pub exclude_defines_from_exec_config: bool,

        #[option(
            name = "incompatible_exclude_starlark_flags_from_exec_config",
            old_name = "experimental_exclude_starlark_flags_from_exec_config",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [AFFECTS_OUTPUTS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If true, don't propagate starlark flags to the exec transition at default; only",
                " propagate starlark flags specified in `--experimental_propagate_custom_flag`.",
            ),
        )]
        pub exclude_starlark_flags_from_exec_config: bool,

        #[option(
            name = "experimental_platform_in_output_dir",
            default_value = "Auto",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "If true, a shortname for the target platform is used in the output directory name\n",
                "instead of the CPU. The exact scheme is experimental and subject to change:\n",
                "1. First, in the rare case the `--platforms` option does not have exactly one value, a\n",
                "   hash of the platforms option is used.\n",
                "2. Next, if any shortname for the current platform was registered by\n",
                "   `--experimental_override_name_platform_in_output_dir`, then that shortname is used.\n",
                "3. Then, if `--experimental_use_platforms_in_output_dir_legacy_heuristic` is set, use a\n",
                "   shortname based off the current platform Label.\n",
                "4. Finally, a hash of the platform option is used as a last resort.\n",
            ),
        )]
        pub platform_in_output_dir: TriState,

        #[option(
            name = "experimental_use_platforms_in_output_dir_legacy_heuristic",
            default_value = "true",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "Please only use this flag as part of a suggested migration or testing strategy. Note\n",
                "that the heuristic has known deficiencies and it is suggested to migrate to\n",
                "relying on just `--experimental_override_name_platform_in_output_dir`.\n",
            ),
        )]
        pub use_platforms_in_output_dir_legacy_heuristic: bool,

        #[option(
            name = "experimental_override_platform_cpu_name",
            old_name = "experimental_override_name_platform_in_output_dir",
            old_name_warning = false,
            converter = LabelToStringEntryConverter,
            default_value = "null",
            allow_multiple = true,
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "Each entry should be of the form `label=value` where label refers to a platform and values\n",
                "is the desired shortname to override the platform's CPU name in `$(TARGET_CPU)`\n",
                "make variable and output path. Only used when\n",
                "`--experimental_platform_in_output_dir`, `--incompatible_target_cpu_from_platform` or\n",
                "`--incompatible_bep_cpu_from_platform` is true. Has highest naming priority.\n",
            ),
        )]
        pub override_platform_cpu_name: Vec<LabelToStringEntry>,

        #[option(
            name = "incompatible_limit_platforms_in_output_dir_to",
            converter = LabelListConverter,
            default_value = "",
            documentation_category = UNDOCUMENTED,
            effect_tags = [AFFECTS_OUTPUTS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "Added for gradual rollout of --experimental_platform_in_output_dir to non-exec",
                " configurations. Takes a comma-separated list of platform labels. If set,",
                " --experimental_platform_in_output_dir is only enabled for platforms in this list.",
                " Otherwise, --experimental_platform_in_output_dir applies to all platforms.",
            ),
        )]
        pub limit_output_dir_to_platforms: Vec<Foreign<Label>>,

        #[option(
            name = "incompatible_target_cpu_from_platform",
            default_value = "true",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If specified, the value of the cpu constraint (`@platforms//cpu:cpu`) of\n",
                "the target platform is used to set the `$(TARGET_CPU)` make variable.\n",
            ),
        )]
        pub incompatible_target_cpu_from_platform: bool,

        #[option(
            name = "incompatible_bep_cpu_from_platform",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [AFFECTS_OUTPUTS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If specified, the value of the cpu constraint (@platforms//cpu:cpu) of",
                " the target platform is used to set the Configuration.cpu and",
                " Configuration.platform_name fields in the BEP.",
            ),
        )]
        pub incompatible_bep_cpu_from_platform: bool,

        #[option(
            name = "define",
            converter = AssignmentConverter,
            default_value = "null",
            allow_multiple = true,
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [CHANGES_INPUTS, AFFECTS_OUTPUTS],
            help = concat!(
                "Each `--define` option specifies an assignment for a build variable.\n",
                "In case of multiple values for a variable, the last one wins.\n",
            ),
        )]
        pub command_line_build_variables: Vec<(String, String)>,

        #[option(
            name = "allowed_cpu_values",
            default_value = "",
            converter = CommaSeparatedOptionSetConverter,
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [CHANGES_INPUTS, AFFECTS_OUTPUTS],
            help = "Allowed values for the `--cpu` flag.",
        )]
        pub allowed_cpu_values: Vec<String>,

        #[option(
            name = "cpu",
            default_value = "",
            converter = AutoCpuConverter,
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [CHANGES_INPUTS, AFFECTS_OUTPUTS],
            help = concat!(
                "Deprecated: this flag is not used internally by Blaze although there are legacy platform\n",
                "mappings to allow for backwards compatibility. Do not use this flag, instead use\n",
                "`--platforms` with an appropriate platform definition.\n",
            ),
        )]
        pub cpu: String,

        #[option(
            name = "min_param_file_size",
            default_value = "32768",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS, EXECUTION, ACTION_COMMAND_LINES],
            help = "Minimum command line length before creating a parameter file.",
        )]
        pub min_param_file_size: i32,

        #[option(
            name = "experimental_extended_sanity_checks",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "Enables internal validation checks to make sure that configured target ",
                "implementations only access things they should. Causes a performance hit.",
            ),
        )]
        pub extended_sanity_checks: bool,

        #[option(
            name = "strict_filesets",
            default_value = "false",
            documentation_category = INPUT_STRICTNESS,
            effect_tags = [BUILD_FILE_SEMANTICS, EAGERNESS_TO_EXIT],
            help = concat!(
                "If this option is enabled, filesets crossing package boundaries are reported ",
                "as errors.",
            ),
        )]
        pub strict_filesets: bool,

        #[option(
            name = "experimental_writable_outputs",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            metadata_tags = [EXPERIMENTAL],
            help = "If true, the file permissions of action outputs are set to `0755` instead of `0555`",
        )]
        pub experimental_writable_outputs: bool,

        #[option(
            name = "experimental_strict_fileset_output",
            default_value = "false",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "If this option is enabled, filesets will treat all output artifacts as regular files. ",
                "They will not traverse directories or be sensitive to symlinks.",
            ),
        )]
        pub strict_fileset_output: bool,

        #[option(
            name = "stamp",
            default_value = "false",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = "Stamp binaries with the date, username, hostname, workspace information, etc.",
        )]
        pub stamp_binaries: bool,

        #[option(
            name = "instrumentation_filter",
            converter = RegexFilterConverter,
            default_value = "-/javatests[/:],-/test/java[/:]",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "When coverage is enabled, only rules with names included by the\n",
                "specified regex-based filter will be instrumented. Rules prefixed\n",
                "with '-' are excluded instead. Note that only non-test rules are\n",
                "instrumented unless `--instrument_test_targets` is enabled.\n",
            ),
        )]
        pub instrumentation_filter: RegexFilter,

        #[option(
            name = "instrument_test_targets",
            default_value = "false",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "When coverage is enabled, specifies whether to consider instrumenting test rules.\n",
                "When set, test rules included by `--instrumentation_filter` are instrumented.\n",
                "Otherwise, test rules are always excluded from coverage instrumentation.\n",
            ),
        )]
        pub instrument_test_targets: bool,

        #[option(
            name = "host_cpu",
            default_value = "",
            converter = AutoCpuConverter,
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [CHANGES_INPUTS, AFFECTS_OUTPUTS],
            help = "The host CPU.",
        )]
        pub host_cpu: String,

        #[option(
            name = "compilation_mode",
            abbrev = 'c',
            converter = CompilationModeConverter,
            default_value = "fastbuild",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS, ACTION_COMMAND_LINES],
            help = "Specify the mode the binary will be built in. Values: `fastbuild`, `dbg`, `opt`.",
        )]
        pub compilation_mode: CompilationMode,

        #[option(
            name = "host_compilation_mode",
            converter = CompilationModeConverter,
            default_value = "opt",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS, ACTION_COMMAND_LINES],
            help = concat!(
                "Specify the mode the tools used during the build will be built in. Values:\n",
                "`fastbuild`, `dbg`, `opt`.\n",
            ),
        )]
        pub host_compilation_mode: CompilationMode,

        #[option(
            name = "incompatible_auto_exec_groups",
            default_value = "false",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "When enabled, an exec groups is automatically created for each toolchain used by a rule.\n",
                "For this to work rule needs to specify `toolchain` parameter on its actions. For\n",
                "more information, see [GH-17134].\n",
                "\n",
                "[GH-17134]: https://github.com/bazelbuild/bazel/issues/17134\n",
            ),
        )]
        pub use_auto_exec_groups: bool,

        /// This internal option is a *set* of names of options that have been changed by starlark
        /// transitions at any point in the build at the time of accessing. It contains both native and
        /// starlark options in label form. e.g. "//command_line_option:cpu" for native options and
        /// "//myapp:foo" for starlark options. This is used to regenerate {@code
        /// transitionDirectoryNameFragment} after each starlark transition.
        #[option(
            name = "affected by starlark transition",
            default_value = "",
            converter = EmptyListConverter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOSES_INCREMENTAL_STATE, AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS],
            metadata_tags = [INTERNAL],
        )]
        pub affected_by_starlark_transition: Vec<String>,

        #[option(
            name = "experimental_exec_configuration_distinguisher",
            default_value = "off",
            converter = ExecConfigurationDistinguisherSchemeConverter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [AFFECTS_OUTPUTS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "Please only use this flag as part of a suggested migration or testing strategy due to\n",
                "potential for action conflicts. Controls how the execution transition changes the\n",
                "`--platform_suffix` flag. In `legacy` mode, sets it to a hash of the execution\n",
                "platform. In `fullhash` mode, sets it to a hash of the entire configuration. In `off`\n",
                "mode, does not touch it.\n",
            ),
        )]
        pub exec_configuration_distinguisher_scheme: CoreOptionsExecConfigurationDistinguisherScheme,

        #[option(
            name = "platform_suffix",
            default_value = "null",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [LOSES_INCREMENTAL_STATE, AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS],
            help = "Specifies a suffix to be added to the configuration directory.",
        )]
        pub platform_suffix: Option<String>,

        #[option(
            name = "action_env",
            converter = EnvVarsConverter,
            allow_multiple = true,
            default_value = "null",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [ACTION_COMMAND_LINES],
            help = concat!(
                "Specifies the set of environment variables available to actions with target\n",
                "configuration. Variables can be either specified by `name`, in which case\n",
                "the value will be taken from the invocation environment, by the `name=value`\n",
                "pair which sets the value independent of the invocation environment, or by\n",
                "`=name`, which unsets the variable of that name. This option can be used\n",
                "multiple times; for options given for the same variable, the latest wins,\n",
                "options for different variables accumulate.\n",
                "\n",
                "Note that unless `--incompatible_repo_env_ignores_action_env` is true, all `name=value`\n",
                "pairs will be available to repository rules.\n",
            ),
        )]
        pub action_environment: Vec<EnvVar>,

        #[option(
            name = "host_action_env",
            converter = EnvVarsConverter,
            allow_multiple = true,
            default_value = "null",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [ACTION_COMMAND_LINES],
            help = concat!(
                "Specifies the set of environment variables available to actions with execution\n",
                "configurations. Variables can be either specified by `name`, in which case the\n",
                "value will be taken from the invocation environment, by the `name=value` pair\n",
                "which sets the value independent of the invocation environment, or by\n",
                "`=name`, which unsets the variable of that name. This option can\n",
                "be used multiple times; for options given for the same variable, the latest\n",
                "wins, options for different variables accumulate.\n",
            ),
        )]
        pub host_action_environment: Vec<EnvVar>,

        #[option(
            name = "collect_code_coverage",
            default_value = "false",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "If specified, Bazel will instrument code (using offline instrumentation where\n",
                "possible) and will collect coverage information during tests. Only targets that\n",
                "match `--instrumentation_filter` will be affected. Usually this option should\n",
                "not be specified directly - `bazel coverage` command should be used instead.\n",
            ),
        )]
        pub collect_code_coverage: bool,

        #[option(
            name = "experimental_collect_code_coverage_for_generated_files",
            default_value = "false",
            documentation_category = OUTPUT_PARAMETERS,
            metadata_tags = [EXPERIMENTAL],
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "If specified, Bazel will also generate collect coverage information for generated",
                " files.",
            ),
        )]
        pub collect_code_coverage_for_generated_files: bool,

        #[option(
            name = "build_runfile_manifests",
            default_value = "true",
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "If true, write runfiles manifests for all targets. If false, omit them. Local tests will",
                " fail to run when false.",
            ),
        )]
        pub build_runfile_manifests: bool,

        #[option(
            name = "build_runfile_links",
            default_value = "true",
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "If true, build runfiles symlink forests for all targets.  ",
                "If false, write them only when required by a local action, test or run command.",
            ),
        )]
        pub build_runfile_links: bool,

        #[option(
            name = "incompatible_always_include_files_in_data",
            default_value = "true",
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [AFFECTS_OUTPUTS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If true, native rules add `DefaultInfo.files` of data dependencies to their runfiles,\n",
                "which matches the recommended behavior for Starlark rules ([runfiles features to avoid]).\n",
                "\n",
                "[runfiles features to avoid]: https://bazel.build/extending/rules#runfiles_features_to_avoid\n",
            ),
        )]
        pub always_include_files_to_build_in_data: bool,

        #[option(
            name = "incompatible_compact_repo_mapping_manifest",
            default_value = "true",
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [AFFECTS_OUTPUTS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If enabled, the `{binary}.repo_mapping` file emits a module extension's repo mapping\n",
                "only once instead of once for each repo generated by the extension that\n",
                "contributes runfiles.\n",
            ),
        )]
        pub compact_repo_mapping: bool,

        #[option(
            name = "run_under",
            default_value = "null",
            converter = RunUnderConverter,
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [ACTION_COMMAND_LINES],
            help = concat!(
                "Prefix to insert before the executables for the `test` and `run` commands.\n",
                "If the value is `foo -bar`, and the execution command line is `test_binary -baz`,\n",
                "then the final command line is `foo -bar test_binary -baz`.\n",
                "This can also be a label to an executable target. Some examples are:\n",
                "- `valgrind`\n",
                "- `strace`\n",
                "- `strace -c`\n",
                "- `valgrind --quiet --num-callers=20`\n",
                "- `//package:target`\n",
                "- `//package:target --options`\n",
            ),
        )]
        pub run_under: Option<RunUnder>,

        #[option(
            name = "check_visibility",
            default_value = "true",
            documentation_category = INPUT_STRICTNESS,
            effect_tags = [BUILD_FILE_SEMANTICS],
            metadata_tags = [NON_CONFIGURABLE],
            help = "If disabled, visibility errors in target dependencies are demoted to warnings.",
        )]
        pub check_visibility: bool,

        #[option(
            name = "experimental_enforce_transitive_visibility",
            default_value = "false",
            documentation_category = INPUT_STRICTNESS,
            effect_tags = [BUILD_FILE_SEMANTICS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "If true, enable package()s to set the transitive_visibility attribute to restrict which",
                " packages may depend on them.",
            ),
        )]
        pub enforce_transitive_visibility: bool,

        #[option(
            name = "verbose_visibility_errors",
            default_value = "false",
            documentation_category = LOGGING,
            effect_tags = [BUILD_FILE_SEMANTICS],
            metadata_tags = [NON_CONFIGURABLE],
            help = "If enabled, visibility errors include additional diagnostic information.",
        )]
        pub verbose_visibility_errors: bool,

        #[option(
            name = "incompatible_check_testonly_for_output_files",
            default_value = "false",
            documentation_category = INPUT_STRICTNESS,
            effect_tags = [BUILD_FILE_SEMANTICS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If enabled, check testonly for prerequisite targets that are output files by",
                " looking up the testonly of the generating rule. This matches visibility",
                " checking.",
            ),
        )]
        pub check_testonly_for_output_files: bool,

        #[option(
            name = "check_licenses",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BUILD_FILE_SEMANTICS],
            help = "Deprecated in favor of `--config=check_licenses`.",
        )]
        pub check_licenses: bool,

        #[option(
            name = "enforce_constraints",
            default_value = "true",
            documentation_category = INPUT_STRICTNESS,
            effect_tags = [BUILD_FILE_SEMANTICS],
            help = concat!(
                "Checks the environments each target is compatible with and reports errors if any ",
                "target has dependencies that don't support the same environments",
            ),
            old_name = "experimental_enforce_constraints",
        )]
        pub enforce_constraints: bool,

        #[option(
            name = "experimental_action_listener",
            allow_multiple = true,
            default_value = "null",
            converter = LabelListConverter,
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [EXECUTION],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "Deprecated in favor of aspects. Use `action_listener` to attach an `extra_action` to\n",
                "existing build actions.\n",
            ),
        )]
        pub action_listeners: Vec<Foreign<Label>>,

        #[option(
            name = "experimental_output_directory_naming_scheme",
            default_value = "diff_against_dynamic_baseline",
            converter = OutputDirectoryNamingSchemeConverter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [AFFECTS_OUTPUTS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "Please only use this flag as part of a suggested migration or testing strategy. In\n",
                "`legacy` mode, transitions (generally only Starlark) set and use `affected by Starlark transition`\n",
                "to determine the ST hash. In `diff_against_baseline` mode,\n",
                "`affected by Starlark transition` is ignored and instead ST hash is determined,\n",
                "for all configuration, by diffing against the top-level configuration.\n",
            ),
        )]
        pub output_directory_naming_scheme: CoreOptionsOutputDirectoryNamingScheme,

        #[option(
            name = "is exec configuration",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            metadata_tags = [INTERNAL],
            help = "Shows whether these options are set for an execution configuration.",
        )]
        pub is_exec: bool,

        #[option(
            name = "allow_analysis_failures",
            default_value = "false",
            documentation_category = TESTING,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "If true, an analysis failure of a rule target results in the target's propagation\n",
                "of an instance of `AnalysisFailureInfo` containing the error description, instead\n",
                "of resulting in a build failure.\n",
            ),
        )]
        pub allow_analysis_failures: bool,

        #[option(
            name = "evaluating for analysis test",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            metadata_tags = [INTERNAL],
            help = concat!(
                "If true, targets in the current configuration are being analyzed only for purposes\n",
                "of an analysis test. This, for example, imposes the restriction described by\n",
                "`--analysis_testing_deps_limit`.\n",
            ),
        )]
        pub evaluating_for_analysis_test: bool,

        #[option(
            name = "analysis_testing_deps_limit",
            default_value = "2000",
            documentation_category = TESTING,
            effect_tags = [LOADING_AND_ANALYSIS],
            help = concat!(
                "Sets the maximum number of transitive dependencies through a rule attribute with\n",
                "a `for_analysis_testing` configuration transition.\n",
                "Exceeding this limit will result in a rule error.\n",
            ),
        )]
        pub analysis_testing_deps_limit: i32,

        #[option(
            name = "features",
            allow_multiple = true,
            default_value = "null",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [CHANGES_INPUTS, AFFECTS_OUTPUTS],
            help = concat!(
                "The given features will be enabled or disabled by default for targets built in the target configuration.\n",
                "Specifying `-{feature}` will disable the feature. Negative features always override positive ones.\n",
                "See also `--host_features`.\n",
            ),
        )]
        pub default_features: Vec<String>,

        #[option(
            name = "host_features",
            allow_multiple = true,
            default_value = "null",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [CHANGES_INPUTS, AFFECTS_OUTPUTS],
            help = concat!(
                "The given features will be enabled or disabled by default for targets built in the exec configuration.\n",
                "Specifying `-{feature}` will disable the feature. Negative features always override positive ones.\n",
            ),
        )]
        pub host_features: Vec<String>,

        #[option(
            name = "target_environment",
            converter = LabelListConverter,
            allow_multiple = true,
            default_value = "null",
            documentation_category = INPUT_STRICTNESS,
            effect_tags = [CHANGES_INPUTS],
            help = concat!(
                "Declares this build's target environment. Must be a label reference to an\n",
                "[`environment` rule]. If specified, all top-level targets must be compatible with this\n",
                "environment.\n",
                "\n",
                "See also `--platforms`.\n",
                "\n",
                "[`environment` rule]: https://github.com/bazelbuild/bazel/blob/master/src/main/java/com/google/devtools/build/lib/analysis/constraints/EnvironmentRule.java\n",
            ),
        )]
        pub target_environments: Vec<Foreign<Label>>,

        #[option(
            name = "allow_unresolved_symlinks",
            old_name = "experimental_allow_unresolved_symlinks",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOSES_INCREMENTAL_STATE, LOADING_AND_ANALYSIS],
            help = concat!(
                "If enabled, Bazel allows the use of `ctx.action.declare_symlink()` and the use of\n",
                "`ctx.actions.symlink()` without a target file, thus allowing the creation of\n",
                "unresolved symlinks.\n",
                "\n",
                "Unresolved symlinks inside tree artifacts are not currently supported.\n",
            ),
        )]
        pub allow_unresolved_symlinks: bool,

        #[option(
            name = "experimental_allow_map_directory",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS, EXECUTION],
            metadata_tags = [EXPERIMENTAL, NON_CONFIGURABLE],
            help = concat!(
                "If enabled, Bazel allows the use of ctx.actions.map_directory(). This allows for the",
                " creation of actions based on the files in a directory through a user defined",
                " Starlark function and a <code>template_ctx</code> that supports basic action",
                " generation APIs.",
            ),
        )]
        pub allow_map_directory: bool,

        #[option(
            name = "experimental_output_paths",
            converter = OutputPathsConverter,
            default_value = "off",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [LOSES_INCREMENTAL_STATE, BAZEL_INTERNAL_CONFIGURATION, AFFECTS_OUTPUTS, EXECUTION],
            help = concat!(
                "Which model to use for where in the output tree rules write their outputs, particularly\n",
                "for multi-platform / multi-configuration builds. This is highly experimental. See\n",
                "[GH-6526] for details. Starlark actions can opt into path mapping by adding the key\n",
                "`supports-path-mapping` to the `execution_requirements` dict.\n",
                "\n",
                "[GH-6526]: https://github.com/bazelbuild/bazel/issues/6526\n",
            ),
        )]
        pub output_paths_mode: CoreOptionsOutputPathsMode,

        #[option(
            name = "enable_runfiles",
            default_value = "auto",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = "Enable runfiles symlink tree; By default, it's off on Windows, on on other platforms.",
        )]
        pub enable_runfiles: TriState,

        #[option(
            name = "modify_execution_info",
            allow_multiple = true,
            converter = ExecutionInfoModifierConverter,
            default_value = "null",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION, AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS],
            help = concat!(
                "Add or remove keys from an action's execution info based on action mnemonic.\n",
                "Applies only to actions which support execution info. Many common actions\n",
                "support execution info, e.g. Genrule, CppCompile, Javac, StarlarkAction,\n",
                "TestRunner. When specifying multiple values, order matters because\n",
                "many regexes may apply to the same mnemonic.\n",
                "\n",
                "Syntax: `regex=[+-]key,regex=[+-]key,...`.\n",
                "\n",
                "Examples:\n",
                "- `.*=+x,.*=-y,.*=+z` adds `x` and `z` to, and removes `y` from, the execution info for all actions.\n",
                "- `Genrule=+requires-x` adds `requires-x` to the execution info for all Genrule actions.\n",
                "- `(?!Genrule).*=-requires-x` removes `requires-x` from the execution info for all non-Genrule actions.\n",
            ),
        )]
        pub execution_info_modifier: Vec<ExecutionInfoModifier>,

        #[option(
            name = "incompatible_modify_execution_info_additive",
            default_value = "true",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION, AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "When enabled, passing multiple `--modify_execution_info` flags is additive.\n",
                "When disabled, only the last flag is taken into account.\n",
            ),
        )]
        pub additive_modify_execution_info: bool,

        #[option(
            name = "incompatible_bazel_test_exec_run_under",
            default_value = "true",
            documentation_category = TOOLCHAIN,
            effect_tags = [AFFECTS_OUTPUTS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If enabled, `bazel test --run_under=//:runner` builds `//:runner` in the exec\n",
                "configuration. If disabled, it builds `//:runner` in the target configuration.\n",
                "Bazel executes tests on exec machines, so the former is more correct. This\n",
                "doesn't affect `bazel run`, which always builds `--run_under=//foo` in the\n",
                "target configuration.\n",
            ),
        )]
        pub bazel_test_exec_run_under: bool,

        #[option(
            name = "include_config_fragments_provider",
            default_value = "off",
            converter = IncludeConfigFragmentsEnumConverter,
            documentation_category = UNDOCUMENTED,
            metadata_tags = [HIDDEN],
            effect_tags = [LOADING_AND_ANALYSIS, LOSES_INCREMENTAL_STATE],
            help = concat!(
                "INTERNAL BLAZE DEVELOPER FEATURE: If `direct`, all configured targets expose\n",
                "RequiredConfigFragmentsProvider with the configuration fragments they directly\n",
                "require.\n",
                "If `transitive`, they do the same but also include the fragments their\n",
                "transitive dependencies require. If `off`, the provider is omitted.\n",
                "\n",
                "If not `off`, this also populates `config_setting`'s\n",
                "`ConfigMatchingProvider.requiredFragmentOptions` with the fragment options the\n",
                "`config_setting` requires.\n",
                "\n",
                "Be careful using this feature: it adds memory to every configured target in the\n",
                "build.\n",
            ),
        )]
        pub include_required_config_fragments_provider: CoreOptionsIncludeConfigFragmentsEnum,

        #[option(
            name = "experimental_remotable_source_manifests",
            default_value = "false",
            converter = BooleanConverter,
            documentation_category = EXECUTION_STRATEGY,
            metadata_tags = [EXPERIMENTAL],
            effect_tags = [LOADING_AND_ANALYSIS, EXECUTION],
            help = "Whether to make source manifest actions remotable",
        )]
        pub remotable_source_manifest_actions: bool,

        #[option(
            name = "flag_alias",
            converter = FlagAliasConverter,
            default_value = "null",
            allow_multiple = true,
            documentation_category = GENERIC_INPUTS,
            effect_tags = [CHANGES_INPUTS],
            metadata_tags = [NON_CONFIGURABLE],
            help = concat!(
                "Sets a shorthand name for a Starlark flag. It takes a single key-value pair in the form\n",
                "`{key}={value}` as an argument.\n",
            ),
        )]
        pub command_line_flag_aliases: Vec<StringToLabelEntry>,

        #[option(
            name = "archived_tree_artifact_mnemonics_filter",
            default_value = "-.*",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS, EXECUTION],
            converter = RegexFilterConverter,
            help = concat!(
                "Regex filter for mnemonics of actions for which we should create archived tree",
                " artifacts. This option is a no-op for actions which do not generate tree",
                " artifacts.",
            ),
        )]
        pub archived_artifacts_mnemonics_filter: RegexFilter,

        #[option(
            name = "experimental_debug_selects_always_succeed",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "When set, `select` functions with no matching clause will return an empty value, instead\n",
                "of failing. This is to help use `cquery` diagnose failures in `select`.\n",
            ),
        )]
        pub debug_selects_always_succeed: bool,

        #[option(
            name = "experimental_throttle_action_cache_check",
            default_value = "true",
            converter = BooleanConverter,
            documentation_category = UNDOCUMENTED,
            metadata_tags = [EXPERIMENTAL],
            effect_tags = [EXECUTION],
            help = "Whether to throttle the check whether an action is cached.",
        )]
        pub throttle_action_cache_check: bool,

        #[option(
            name = "use_target_platform_for_tests",
            deprecation_warning = concat!(
                "Tests select an execution platform matching all constraints of the target platform by",
                " default. Instead of using this flag, make sure that all test target platform are",
                " registered as execution platforms.",
            ),
            default_value = "false",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION],
            help = "If true, use the target platform for running tests rather than the test exec group.",
        )]
        pub use_target_platform_for_tests: bool,

        #[option(
            name = "exec_aspects",
            converter = CommaSeparatedOptionListConverter,
            default_value = "null",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [LOADING_AND_ANALYSIS],
            allow_multiple = true,
            help = concat!(
                "Comma-separated list of aspects to be applied to exec-configured targets, regardless of",
                " whether or not they are top-level targets. This is an experimental feature and",
                " is subject to change.",
            ),
        )]
        pub exec_aspects: Vec<String>,
    }
}

// Java: com/google/devtools/build/lib/analysis/config/CoreOptions.java:436 CoreOptions.EmptyListConverter
/// Regardless of input, converts to an empty list. For use with affectedByStarlarkTransition
#[derive(Clone, Copy, Debug, Default)]
pub struct EmptyListConverter;

impl Converter for EmptyListConverter {
    // Java: com/google/devtools/build/lib/analysis/config/CoreOptions.java:438 CoreOptions.EmptyListConverter.convert
    fn convert(&self, _input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::list(vec![])))
    }
    // Java: com/google/devtools/build/lib/analysis/config/CoreOptions.java:443 CoreOptions.EmptyListConverter.getTypeDescription
    fn get_type_description(&self) -> String {
        "Regardless of input, converts to an empty list. For use with affectedByStarlarkTransition"
            .into()
    }
}

blaze_options::option_enum! {
    #[starlark_value]
    // Java: com/google/devtools/build/lib/analysis/config/CoreOptions.java:470 CoreOptions.ExecConfigurationDistinguisherScheme
    /// Values for the --experimental_exec_configuration_distinguisher options
    pub enum CoreOptionsExecConfigurationDistinguisherScheme {
        /// Use hash of selected execution platform for platform_suffix.
        LEGACY,
        /// Do not touch platform_suffix or do anything else.
        OFF,
        /// Use hash of entire configuration (with platform_suffix="") for platform_suffix.
        FULL_HASH,
        /// Set platform_suffix to "exec", instead update `affected by starlark transition`
        DIFF_TO_AFFECTED,
    }
}

// Java: com/google/devtools/build/lib/analysis/config/CoreOptions.java:482 CoreOptions.ExecConfigurationDistinguisherSchemeConverter
blaze_options::enum_converter!(
    pub ExecConfigurationDistinguisherSchemeConverter,
    CoreOptionsExecConfigurationDistinguisherScheme,
    "Exec transition configuration distinguisher scheme"
);

blaze_options::option_enum! {
    #[starlark_value]
    // Java: com/google/devtools/build/lib/analysis/config/CoreOptions.java:740 CoreOptions.OutputDirectoryNamingScheme
    /// Values for the --experimental_output_directory_naming_scheme options
    pub enum CoreOptionsOutputDirectoryNamingScheme {
        /// Use `affected by starlark transition` to track configuration changes
        LEGACY,
        /// Produce name based on diff from some baseline BuildOptions (usually top-level)
        DIFF_AGAINST_BASELINE,
        /// Like DIFF_AGAINST_BASELINE, but compare against post-exec baseline if isExec is set.
        DIFF_AGAINST_DYNAMIC_BASELINE,
    }
}

// Java: com/google/devtools/build/lib/analysis/config/CoreOptions.java:750 CoreOptions.OutputDirectoryNamingSchemeConverter
blaze_options::enum_converter!(
    pub OutputDirectoryNamingSchemeConverter,
    CoreOptionsOutputDirectoryNamingScheme,
    "Output directory naming scheme"
);

blaze_options::option_enum! {
    #[starlark_value]
    // Java: com/google/devtools/build/lib/analysis/config/CoreOptions.java:913 CoreOptions.OutputPathsMode
    /// Values for --experimental_output_paths.
    pub enum CoreOptionsOutputPathsMode {
        /// Use the production output path model.
        OFF,
        /// Strip the config prefix (i.e. `/x86-fastbuild/` from output paths for actions that are
        /// registered to support this feature.
        STRIP,
    }
}

// Java: com/google/devtools/build/lib/analysis/config/CoreOptions.java:926 CoreOptions.OutputPathsConverter
blaze_options::enum_converter!(
    pub OutputPathsConverter,
    CoreOptionsOutputPathsMode,
    "output path mode"
);

blaze_options::option_enum! {
    #[starlark_value]
    // Java: com/google/devtools/build/lib/analysis/config/CoreOptions.java:1173 CoreOptions.IncludeConfigFragmentsEnum
    /// Ways configured targets may provide the `Fragment`s they require.
    pub enum CoreOptionsIncludeConfigFragmentsEnum {
        /// Don't offer the provider at all. This is best for most builds, which don't use this
        /// information and don't need the extra memory hit over every configured target.
        OFF,
        /// Provide the fragments required *directly* by this rule.
        DIRECT,
        /// Provide the fragments required by this rule and its transitive dependencies.
        TRANSITIVE,
    }
}

// Java: com/google/devtools/build/lib/analysis/config/CoreOptions.java:1186 CoreOptions.IncludeConfigFragmentsEnumConverter
blaze_options::enum_converter!(
    pub IncludeConfigFragmentsEnumConverter,
    CoreOptionsIncludeConfigFragmentsEnum,
    "include config fragments provider option"
);

impl CoreOptions {
    // Java: com/google/devtools/build/lib/analysis/config/CoreOptions.java:774 CoreOptions.useBaselineForOutputDirectoryNamingScheme
    pub fn use_baseline_for_output_directory_naming_scheme(&self) -> bool {
        match self.output_directory_naming_scheme {
            CoreOptionsOutputDirectoryNamingScheme::DIFF_AGAINST_BASELINE
            | CoreOptionsOutputDirectoryNamingScheme::DIFF_AGAINST_DYNAMIC_BASELINE => true,
            CoreOptionsOutputDirectoryNamingScheme::LEGACY => false,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/config/CoreOptions.java:1138 CoreOptions.getPlatformCpuNameOverride
    pub fn get_platform_cpu_name_override(&self, platform: &Label) -> Option<String> {
        // As highest priority, use the last entry that matches in name override option.
        self.override_platform_cpu_name
            .iter()
            .filter(|e| e.key == *platform)
            .map(|e| e.value.clone())
            .next_back()
    }

    // Java: com/google/devtools/build/lib/analysis/config/CoreOptions.java:1146 CoreOptions.usePlatformInOutputDir
    pub fn use_platform_in_output_dir(&self, platform: &Label) -> bool {
        if self.is_exec {
            return self.platform_in_output_dir == TriState::YES
                || self.platform_in_output_dir == TriState::AUTO;
        }

        if self.platform_in_output_dir == TriState::YES {
            return self.limit_output_dir_to_platforms.is_empty()
                || self
                    .limit_output_dir_to_platforms
                    .iter()
                    .any(|l| l.get() == platform);
        }

        false
    }

    // Java: com/google/devtools/build/lib/analysis/config/CoreOptions.java:1160 CoreOptions.getCommandLineFlagAliases
    /// `ImmutableMap.copyOf(commandLineFlagAliases)`, in insertion order.
    // PORT: Java memoizes the map in a volatile field (cleared by clone()); it is recomputed here.
    pub fn get_command_line_flag_aliases(&self) -> Vec<(String, Label)> {
        let mut result: Vec<(String, Label)> = Vec::new();
        for e in &self.command_line_flag_aliases {
            if let Some((_, existing)) = result.iter().find(|(k, _)| *k == e.key) {
                // ImmutableMap.Builder rejects duplicate keys.
                panic!(
                    "IllegalArgumentException: Multiple entries with same key: {}={} and {}={}",
                    e.key, e.value, e.key, existing
                );
            }
            result.push((e.key.clone(), e.value.clone()));
        }
        result
    }

    // Java: com/google/devtools/build/lib/analysis/config/CoreOptions.java:1194 CoreOptions.getNormalizedCommandLineBuildVariables
    /// Normalizes --define flags, preserving the last one to appear in the event of conflicts.
    pub fn get_normalized_command_line_build_variables(&self) -> Vec<(String, String)> {
        sort_entries(normalize_entries(&self.command_line_build_variables))
    }

    // Java: com/google/devtools/build/lib/analysis/config/CoreOptions.java:1211 CoreOptions.getNormalizedFeatures
    /// Normalizes --features flags by sorting the values and having disables win over enables.
    fn get_normalized_features(features: &[String]) -> Vec<String> {
        // Parse out the features into a Map<String, boolean>, where the boolean represents
        // whether the feature is enabled or disabled.
        let mut feature_to_state: HashMap<&str, bool> = HashMap::new();
        for feature in features {
            if let Some(stripped) = feature.strip_prefix('-') {
                // disable always wins.
                feature_to_state.insert(stripped, false);
            } else if !feature_to_state.contains_key(feature.as_str()) {
                // enable feature only if it does not already have a state.
                // If existing state is enabled, no need to do extra work.
                // If existing state is disabled, it wins.
                feature_to_state.insert(feature, true);
            }
        }
        // Partition into enabled/disabled features.
        let mut enabled: BTreeSet<JavaOrdered> = BTreeSet::new();
        let mut disabled: BTreeSet<JavaOrdered> = BTreeSet::new();
        for (k, v) in feature_to_state {
            if v {
                enabled.insert(JavaOrdered(k.to_string()));
            } else {
                disabled.insert(JavaOrdered(k.to_string()));
            }
        }
        // Rebuild the set of features.
        // Since we used TreeSet the features come out in a deterministic order.
        let mut result: Vec<String> = enabled.into_iter().map(|s| s.0).collect();
        result.extend(disabled.into_iter().map(|x| format!("-{}", x.0)));
        result
    }
}

/// A `String` ordered like Java's `String.compareTo`, for `TreeSet<String>`.
#[derive(PartialEq, Eq)]
struct JavaOrdered(String);

impl Ord for JavaOrdered {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        java_string_compare(&self.0, &other.0)
    }
}

impl PartialOrd for JavaOrdered {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// Java: com/google/devtools/build/lib/analysis/config/CoreOptions.java:1200 CoreOptions.sortEntries
/// Sort the map entries by key (a stable sort, as Java's sorted stream).
fn sort_entries<V: Clone>(entries: Vec<(String, V)>) -> Vec<(String, V)> {
    let mut sorted = entries;
    sorted.sort_by(|a, b| java_string_compare(&a.0, &b.0));
    sorted
}

impl FragmentOptions for CoreOptions {
    // Java: com/google/devtools/build/lib/analysis/config/CoreOptions.java:1245 CoreOptions.getNormalized
    fn get_normalized(&self) -> CoreOptions {
        let mut result = self.clone();
        result.allowed_cpu_values = dedup_and_sort(Some(&self.allowed_cpu_values));
        result.command_line_build_variables =
            sort_entries(normalize_entries(&self.command_line_build_variables));

        // Normalize features.
        result.default_features = Self::get_normalized_features(&self.default_features);

        result.action_environment = normalize_env_vars(&self.action_environment);
        result.host_action_environment = normalize_env_vars(&self.host_action_environment);
        let aliases: Vec<(String, Label)> = self
            .command_line_flag_aliases
            .iter()
            .map(|e| (e.key.clone(), e.value.clone()))
            .collect();
        result.command_line_flag_aliases = sort_entries(normalize_entries(&aliases))
            .into_iter()
            .map(|(key, value)| StringToLabelEntry { key, value })
            .collect();

        result
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{assert_defaults_round_trip, assert_option_names, parse};
    use super::*;

    #[test]
    fn option_names_match_java() {
        assert_option_names::<CoreOptions>(&[
            "incompatible_filegroup_runfiles_for_data",
            "scl_config",
            "incompatible_merge_genfiles_directory",
            "experimental_exec_config",
            "incompatible_disable_select_on",
            "experimental_propagate_custom_flag",
            "experimental_exclude_defines_from_exec_config",
            "incompatible_exclude_starlark_flags_from_exec_config",
            "experimental_platform_in_output_dir",
            "experimental_use_platforms_in_output_dir_legacy_heuristic",
            "experimental_override_platform_cpu_name",
            "incompatible_limit_platforms_in_output_dir_to",
            "incompatible_target_cpu_from_platform",
            "incompatible_bep_cpu_from_platform",
            "define",
            "allowed_cpu_values",
            "cpu",
            "min_param_file_size",
            "experimental_extended_sanity_checks",
            "strict_filesets",
            "experimental_writable_outputs",
            "experimental_strict_fileset_output",
            "stamp",
            "instrumentation_filter",
            "instrument_test_targets",
            "host_cpu",
            "compilation_mode",
            "host_compilation_mode",
            "incompatible_auto_exec_groups",
            "affected by starlark transition",
            "experimental_exec_configuration_distinguisher",
            "platform_suffix",
            "action_env",
            "host_action_env",
            "collect_code_coverage",
            "experimental_collect_code_coverage_for_generated_files",
            "build_runfile_manifests",
            "build_runfile_links",
            "incompatible_always_include_files_in_data",
            "incompatible_compact_repo_mapping_manifest",
            "run_under",
            "check_visibility",
            "experimental_enforce_transitive_visibility",
            "verbose_visibility_errors",
            "incompatible_check_testonly_for_output_files",
            "check_licenses",
            "enforce_constraints",
            "experimental_action_listener",
            "experimental_output_directory_naming_scheme",
            "is exec configuration",
            "allow_analysis_failures",
            "evaluating for analysis test",
            "analysis_testing_deps_limit",
            "features",
            "host_features",
            "target_environment",
            "allow_unresolved_symlinks",
            "experimental_allow_map_directory",
            "experimental_output_paths",
            "enable_runfiles",
            "modify_execution_info",
            "incompatible_modify_execution_info_additive",
            "incompatible_bazel_test_exec_run_under",
            "include_config_fragments_provider",
            "experimental_remotable_source_manifests",
            "flag_alias",
            "archived_tree_artifact_mnemonics_filter",
            "experimental_debug_selects_always_succeed",
            "experimental_throttle_action_cache_check",
            "use_target_platform_for_tests",
            "exec_aspects",
        ]);
    }

    #[test]
    fn defaults_round_trip() {
        // "affected by starlark transition": EmptyListConverter ignores its input, so it is
        // comparable too; nothing is skipped.
        assert_defaults_round_trip::<CoreOptions>(&[]);
    }

    #[test]
    fn spot_check_parses() {
        let o = parse::<CoreOptions>(&[
            "--cpu=k8",
            "--compilation_mode=opt",
            "--define=a=1",
            "--define=b=2",
            "--features=foo",
            "--run_under=//tools:wrap -v",
            "--platform_suffix=sfx",
            "--incompatible_disable_select_on=b,a,b",
        ])
        .unwrap();
        assert_eq!(o.cpu, "k8");
        assert_eq!(o.compilation_mode, CompilationMode::OPT);
        assert_eq!(
            o.command_line_build_variables,
            vec![
                ("a".to_string(), "1".to_string()),
                ("b".to_string(), "2".to_string())
            ]
        );
        assert_eq!(o.default_features, vec!["foo".to_string()]);
        assert_eq!(o.run_under.as_ref().unwrap().options(), ["-v".to_string()]);
        assert_eq!(o.platform_suffix.as_deref(), Some("sfx"));
        assert_eq!(o.disabled_select_options, vec!["a", "b"]);
        let e = parse::<CoreOptions>(&["--compilation_mode=fast"]).unwrap_err();
        assert_eq!(
            e.get_message(),
            "While parsing option --compilation_mode=fast: Not a valid compilation mode: 'fast' \
             (should be fastbuild, dbg or opt)"
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/analysis/config/CoreOptionsTest.java
    mod core_options_test {
        use super::super::super::test_util::{assert_same, create_with_prefix};
        use super::*;

        const FEATURES_PREFIX: &str = "--features=";
        const DEFINE_PREFIX: &str = "--define=";
        const FLAG_ALIAS_PREFIX: &str = "--flag_alias=";

        fn c(prefix: &str, args: &[&str]) -> CoreOptions {
            create_with_prefix::<CoreOptions>(prefix, args)
        }

        #[test]
        fn test_features_ordering_of_positive_features() {
            assert_same(
                &c(FEATURES_PREFIX, &["foo", "bar"]),
                &c(FEATURES_PREFIX, &["bar", "foo"]),
            );
        }

        #[test]
        fn test_features_duplicate_features() {
            assert_same(
                &c(FEATURES_PREFIX, &["foo", "bar"]),
                &c(FEATURES_PREFIX, &["bar", "foo", "bar"]),
            );
        }

        #[test]
        fn test_features_disabling_wins() {
            assert_same(
                &c(FEATURES_PREFIX, &["foo", "-foo", "bar"]),
                &c(FEATURES_PREFIX, &["-foo", "bar"]),
            );
        }

        #[test]
        fn test_defines_duplicate_key() {
            // Last one should win
            assert_same(
                &c(DEFINE_PREFIX, &["a=1", "a=2"]),
                &c(DEFINE_PREFIX, &["a=2"]),
            );
        }

        #[test]
        fn test_defines_order_of_keys() {
            assert_same(
                &c(DEFINE_PREFIX, &["a=1", "c=3", "b=2"]),
                &c(DEFINE_PREFIX, &["b=2", "a=1", "c=3"]),
            );
        }

        #[test]
        fn testflag_alias_duplicate_key() {
            // Last one should win
            assert_same(
                &c(FLAG_ALIAS_PREFIX, &["a=//one", "a=//two"]),
                &c(FLAG_ALIAS_PREFIX, &["a=//two"]),
            );
        }

        #[test]
        fn test_flag_alias_order_of_keys() {
            assert_same(
                &c(FLAG_ALIAS_PREFIX, &["a=//one", "c=//three", "b=//two"]),
                &c(FLAG_ALIAS_PREFIX, &["b=//two", "a=//one", "c=//three"]),
            );
        }
    }
}
