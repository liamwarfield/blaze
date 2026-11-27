//! Transcription of `com/google/devtools/build/lib/query2/common/CommonQueryOptions.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/query2/common/CommonQueryOptions.java:36 CommonQueryOptions
options! {
    /// Options shared between blaze query implementations.
    pub struct CommonQueryOptions {
        #[option(
            name = "universe_scope",
            default_value = "",
            converter = CommaSeparatedOptionListConverter,
            documentation_category = QUERY,
            effect_tags = [LOADING_AND_ANALYSIS],
            help = concat!(
                "A comma-separated set of target patterns (additive and subtractive). The query may be",
                " performed in the universe defined by the transitive closure of the specified",
                " targets. This option is used for the query and cquery commands.\n",
                "For cquery, the input to this option is the targets all answers are built under",
                " and so this option may affect configurations and transitions. If this option is",
                " not specified, the top-level targets are assumed to be the targets parsed from",
                " the query expression. Note: For cquery, not specifying this option may cause the",
                " build to break if targets parsed from the query expression are not buildable",
                " with top-level options.",
            ),
        )]
        pub universe_scope: Vec<String>,

        #[option(
            name = "line_terminator_null",
            default_value = "false",
            documentation_category = QUERY,
            effect_tags = [TERMINAL_OUTPUT],
            help = "Whether each format is terminated with \\0 instead of newline.",
        )]
        pub line_terminator_null: bool,

        #[option(
            name = "infer_universe_scope",
            default_value = "false",
            documentation_category = QUERY,
            effect_tags = [LOADING_AND_ANALYSIS],
            help = concat!(
                "If set and --universe_scope is unset, then a value of --universe_scope will be inferred",
                " as the list of unique target patterns in the query expression. Note that the",
                " --universe_scope value inferred for a query expression that uses universe-scoped",
                " functions (e.g.`allrdeps`) may not be what you want, so you should use this",
                " option only if you know what you are doing. See",
                " https://bazel.build/reference/query#sky-query for details and",
                " examples. If --universe_scope is set, then this option's value is ignored. Note:",
                " this option applies only to `query` (i.e. not `cquery`).",
            ),
        )]
        pub infer_universe_scope: bool,

        #[option(
            name = "tool_deps",
            old_name = "host_deps",
            default_value = "true",
            documentation_category = QUERY,
            effect_tags = [BUILD_FILE_SEMANTICS],
            help = concat!(
                "Query: If disabled, dependencies on 'exec configuration' will",
                " not be included in the dependency graph over which the query operates. An 'exec",
                " configuration' dependency edge, such as the one from any 'proto_library' rule to",
                " the Protocol Compiler, usually points to a tool executed during the build rather",
                " than a part of the same 'target' program.\n",
                "Cquery: If disabled, filters out all configured targets which cross an",
                " execution transition from the top-level target that discovered this configured",
                " target. That means if the top-level target is in the target configuration, only",
                " configured targets also in the target configuration will be returned. If the",
                " top-level target is in the exec configuration, only exec configured targets will",
                " be returned. This option will NOT exclude resolved toolchains.",
            ),
        )]
        pub include_tool_deps: bool,

        #[option(
            name = "implicit_deps",
            default_value = "true",
            documentation_category = QUERY,
            effect_tags = [BUILD_FILE_SEMANTICS],
            help = concat!(
                "If enabled, implicit dependencies will be included in the dependency graph over ",
                "which the query operates. An implicit dependency is one that is not explicitly ",
                "specified in the BUILD file but added by bazel. For cquery, this option controls ",
                "filtering resolved toolchains.",
            ),
        )]
        pub include_implicit_deps: bool,

        #[option(
            name = "nodep_deps",
            default_value = "true",
            documentation_category = QUERY,
            effect_tags = [BUILD_FILE_SEMANTICS],
            help = concat!(
                "If enabled, deps from \"nodep\" attributes will be included in the dependency graph ",
                "over which the query operates. A common example of a \"nodep\" attribute is ",
                "\"visibility\". Run and parse the output of `info build-language` to learn about ",
                "all the \"nodep\" attributes in the build language.",
            ),
        )]
        pub include_no_dep_deps: bool,

        #[option(
            name = "include_aspects",
            default_value = "true",
            documentation_category = QUERY,
            effect_tags = [TERMINAL_OUTPUT],
            help = concat!(
                "aquery, cquery: whether to include aspect-generated actions in the output. ",
                "query: no-op (aspects are always followed).",
            ),
        )]
        pub use_aspects: bool,

        #[option(
            name = "incompatible_package_group_includes_double_slash",
            // Java: com/google/devtools/build/lib/query2/common/FlagConstants.java:24 FlagConstants.DEFAULT_INCOMPATIBLE_PACKAGE_GROUP_INCLUDES_DOUBLE_SLASH
            default_value = "true",
            documentation_category = QUERY,
            effect_tags = [TERMINAL_OUTPUT],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If enabled, when outputting package_group's `packages` attribute, the leading `//`",
                " will not be omitted.",
            ),
        )]
        pub incompatible_package_group_includes_double_slash: bool,

        #[option(
            name = "experimental_explicit_aspects",
            default_value = "false",
            documentation_category = QUERY,
            effect_tags = [TERMINAL_OUTPUT],
            help = concat!(
                "aquery, cquery: whether to include aspect-generated actions in the output. ",
                "query: no-op (aspects are always followed).",
            ),
        )]
        pub explicit_aspects: bool,

        #[option(
            name = "consistent_labels",
            default_value = "false",
            documentation_category = QUERY,
            effect_tags = [TERMINAL_OUTPUT],
            help = concat!(
                "If enabled, every query command emits labels as if by the Starlark <code>str</code>",
                " function applied to a <code>Label</code> instance. This is useful for tools that",
                " need to match the output of different query commands and/or labels emitted by",
                " rules. If not enabled, output formatters are free to emit apparent repository",
                " names (relative to the main repository) instead to make the output more",
                " readable.",
            ),
        )]
        pub emit_consistent_labels: bool,

        #[option(
            name = "relative_locations",
            default_value = "false",
            documentation_category = QUERY,
            effect_tags = [TERMINAL_OUTPUT],
            help = concat!(
                "If true, the location of BUILD files in xml and proto outputs will be relative. ",
                "By default, the location output is an absolute path and will not be consistent ",
                "across machines. You can set this option to true to have a consistent result ",
                "across machines.",
            ),
        )]
        pub relative_locations: bool,

        #[option(
            name = "proto:locations",
            default_value = "true",
            documentation_category = QUERY,
            effect_tags = [TERMINAL_OUTPUT],
            help = "Whether to output location information in proto output at all.",
        )]
        pub proto_include_locations: bool,

        #[option(
            name = "proto:default_values",
            default_value = "true",
            documentation_category = QUERY,
            effect_tags = [TERMINAL_OUTPUT],
            help = concat!(
                "If true, attributes whose value is not explicitly specified in the BUILD file are ",
                "included; otherwise they are omitted. This option is applicable to --output=proto",
            ),
        )]
        pub proto_include_default_values: bool,

        #[option(
            name = "proto:flatten_selects",
            default_value = "true",
            documentation_category = QUERY,
            effect_tags = [BUILD_FILE_SEMANTICS],
            help = concat!(
                "If enabled, configurable attributes created by select() are flattened. For list types ",
                "the flattened representation is a list containing each value of the select map ",
                "exactly once. Scalar types are flattened to null.",
            ),
        )]
        pub proto_flatten_selects: bool,

        #[option(
            name = "proto:output_rule_attrs",
            default_value = "all",
            converter = CommaSeparatedOptionListConverter,
            documentation_category = QUERY,
            effect_tags = [TERMINAL_OUTPUT],
            help = concat!(
                "Comma separated list of attributes to include in output. Defaults to all attributes. ",
                "Set to empty string to not output any attribute. ",
                "This option is applicable to --output=proto.",
            ),
        )]
        pub proto_output_rule_attributes: Vec<String>,

        #[option(
            name = "proto:rule_inputs_and_outputs",
            default_value = "true",
            documentation_category = QUERY,
            effect_tags = [TERMINAL_OUTPUT],
            help = "Whether or not to populate the rule_input and rule_output fields.",
        )]
        pub proto_include_rule_inputs_and_outputs: bool,

        #[option(
            name = "proto:include_synthetic_attribute_hash",
            default_value = "false",
            documentation_category = QUERY,
            effect_tags = [TERMINAL_OUTPUT],
            help = "Whether or not to calculate and populate the $internal_attr_hash attribute.",
        )]
        pub proto_include_synthetic_attribute_hash: bool,

        #[option(
            name = "proto:instantiation_stack",
            default_value = "false",
            documentation_category = QUERY,
            effect_tags = [TERMINAL_OUTPUT],
            help = concat!(
                "Populate the instantiation call stack of each rule. ",
                "Note that this requires the stack to be present",
            ),
        )]
        pub proto_include_instantiation_stack: bool,

        #[option(
            name = "proto:definition_stack",
            default_value = "false",
            documentation_category = QUERY,
            effect_tags = [TERMINAL_OUTPUT],
            help = concat!(
                "Populate the definition_stack proto field, which records for each rule instance the ",
                "Starlark call stack at the moment the rule's class was defined.",
            ),
        )]
        pub proto_include_definition_stack: bool,

        #[option(
            name = "proto:include_starlark_rule_env",
            default_value = "true",
            documentation_category = QUERY,
            effect_tags = [TERMINAL_OUTPUT],
            help = concat!(
                "Use the starlark environment in the value of the generated $internal_attr_hash",
                " attribute. This ensures that the starlark rule definition (and its transitive",
                " imports) are part of this identifier.",
            ),
        )]
        pub proto_include_starlark_rule_env: bool,

        #[option(
            name = "proto:include_attribute_source_aspects",
            default_value = "false",
            documentation_category = QUERY,
            effect_tags = [TERMINAL_OUTPUT],
            help = concat!(
                "Populate the source_aspect_name proto field of each Attribute with the source aspect ",
                "that the attribute came from (empty string if it did not).",
            ),
        )]
        pub proto_include_attribute_source_aspects: bool,

        #[option(
            name = "proto:rule_classes",
            default_value = "false",
            documentation_category = QUERY,
            effect_tags = [TERMINAL_OUTPUT],
            help = concat!(
                "Populate the rule_class_key field of each rule; and for the first rule with a given",
                " rule_class_key, also populate its rule_class_info proto field. The rule_class_key",
                " field uniquely identifies a rule class, and the rule_class_info field is a",
                " Stardoc-format rule class API definition.",
            ),
        )]
        pub proto_rule_classes: bool,

        #[option(
            name = "aspect_deps",
            default_value = "conservative",
            converter = AspectResolutionModeConverter,
            documentation_category = QUERY,
            effect_tags = [BUILD_FILE_SEMANTICS],
            help = concat!(
                "How to resolve aspect dependencies when the output format is one of {xml,proto,record}. ",
                "'off' means no aspect dependencies are resolved, 'conservative' (the default) ",
                "means all declared aspect dependencies are added regardless of whether they are ",
                "given the rule class of direct dependencies, 'precise' means that only those ",
                "aspects are added that are possibly active given the rule class of the direct ",
                "dependencies. Note that precise mode requires loading other packages to evaluate ",
                "a single target thus making it slower than the other modes. Also note that even ",
                "precise mode is not completely precise: the decision whether to compute an aspect ",
                "is decided in the analysis phase, which is not run during 'bazel query'.",
            ),
        )]
        pub aspect_deps: AspectResolverMode,

        #[option(
            name = "graph:node_limit",
            default_value = "512",
            documentation_category = QUERY,
            effect_tags = [TERMINAL_OUTPUT],
            help = concat!(
                "The maximum length of the label string for a graph node in the output.  Longer labels",
                " will be truncated; -1 means no truncation.  This option is only applicable to",
                " --output=graph.",
            ),
        )]
        pub graph_node_string_limit: i32,

        #[option(
            name = "graph:factored",
            default_value = "true",
            documentation_category = QUERY,
            effect_tags = [TERMINAL_OUTPUT],
            help = concat!(
                "If true, then the graph will be emitted 'factored', i.e. topologically-equivalent nodes ",
                "will be merged together and their labels concatenated. This option is only ",
                "applicable to --output=graph.",
            ),
        )]
        pub graph_factored: bool,

        #[option(
            name = "query_file",
            default_value = "",
            documentation_category = QUERY,
            effect_tags = [CHANGES_INPUTS],
            help = concat!(
                "If set, query will read the query from the file named here, rather than on the command ",
                "line. It is an error to specify a file here as well as a command-line query.",
            ),
        )]
        pub query_file: String,

        #[option(
            name = "output_file",
            default_value = "",
            documentation_category = QUERY,
            effect_tags = [TERMINAL_OUTPUT],
            help = concat!(
                "When specified, query results will be written directly to this file, and nothing will be",
                " printed to Bazel's standard output stream (stdout). In benchmarks, this is",
                " generally faster than <code>bazel query &gt; file</code>.",
            ),
        )]
        pub output_file: String,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const COMMON_QUERY_OPTIONS_OPTION_NAMES: &[&str] = &[
        "universe_scope",
        "line_terminator_null",
        "infer_universe_scope",
        "tool_deps",
        "implicit_deps",
        "nodep_deps",
        "include_aspects",
        "incompatible_package_group_includes_double_slash",
        "experimental_explicit_aspects",
        "consistent_labels",
        "relative_locations",
        "proto:locations",
        "proto:default_values",
        "proto:flatten_selects",
        "proto:output_rule_attrs",
        "proto:rule_inputs_and_outputs",
        "proto:include_synthetic_attribute_hash",
        "proto:instantiation_stack",
        "proto:definition_stack",
        "proto:include_starlark_rule_env",
        "proto:include_attribute_source_aspects",
        "proto:rule_classes",
        "aspect_deps",
        "graph:node_limit",
        "graph:factored",
        "query_file",
        "output_file",
    ];

    #[test]
    fn common_query_options_option_names() {
        super::super::test_util::check_option_names(
            CommonQueryOptions::class(),
            COMMON_QUERY_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn common_query_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<CommonQueryOptions>(SKIP_ROUND_TRIP);
    }
}

// ---- hand-written below this line (preserved by the generator) ----

// Java: com/google/devtools/build/lib/query2/common/CommonQueryOptions.java:337 CommonQueryOptions.AspectResolutionModeConverter
enum_converter!(pub AspectResolutionModeConverter, AspectResolverMode, "Aspect resolution mode");

impl CommonQueryOptions {
    // Java: com/google/devtools/build/lib/query2/common/CommonQueryOptions.java:65 CommonQueryOptions.getLineTerminator
    /// Ugly workaround since line terminator option default has to be constant expression.
    pub fn get_line_terminator(&self) -> &'static str {
        if self.line_terminator_null {
            return "\0";
        }
        "\n"
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
    fn aspect_deps_and_attributes() {
        let o = parse::<CommonQueryOptions>(&[
            "--aspect_deps=precise",
            "--proto:output_rule_attrs=a,b",
            "--line_terminator_null",
        ]);
        assert_eq!(o.aspect_deps, AspectResolverMode::PRECISE);
        assert_eq!(o.proto_output_rule_attributes, vec!["a", "b"]);
        assert_eq!(o.get_line_terminator(), "\0");
        assert_eq!(
            CommonQueryOptions::default().proto_output_rule_attributes,
            vec!["all"]
        );
    }
}
