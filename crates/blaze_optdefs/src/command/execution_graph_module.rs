//! Transcription of `com/google/devtools/build/lib/runtime/ExecutionGraphModule.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/runtime/ExecutionGraphModule.java:106 ExecutionGraphModule.ExecutionGraphOptions
options! {
    /// Options for the generated execution graph.
    pub struct ExecutionGraphModuleExecutionGraphOptions {
        #[option(
            name = "experimental_enable_execution_graph_log",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "Enabling this flag makes Blaze write a file of all actions executed during a build. ",
                "Note that this dump may use a different granularity of actions than other APIs, ",
                "and may also contain additional information as necessary to reconstruct the ",
                "full dependency graph in combination with other sources of data.",
            ),
        )]
        pub enable_execution_graph_log: bool,

        #[option(
            name = "experimental_execution_graph_log_path",
            default_value = "",
            documentation_category = UNDOCUMENTED,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "Local path at which the execution path will be written. If this is set, the log will",
                " only be written locally, and not to BEP. If this is set when",
                " experimental_enable_execution_graph_log is disabled, there will be an error. If",
                " this is unset while BEP uploads are disabled and",
                " experimental_enable_execution_graph_log is enabled, the log will be written to",
                " a local default.",
            ),
        )]
        pub execution_graph_log_path: String,

        #[option(
            name = "experimental_execution_graph_log_dep_type",
            default_value = "none",
            converter = DependencyInfoConverter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Selects what kind of dependency information is reported in the action dump. If 'all',",
                " every inter-action edge will be reported.",
            ),
        )]
        pub dep_type: DependencyInfo,

        #[option(
            name = "experimental_execution_graph_log_queue_size",
            default_value = "-1",
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "The size of the action dump queue, where actions are kept before writing. Larger",
                " sizes will increase peak memory usage, but should decrease queue blocking. -1",
                " means unbounded",
            ),
        )]
        pub queue_size: i32,

        #[option(
            name = "experimental_execution_graph_enable_edges_from_filewrite_actions",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            help = "Handle edges from filewrite actions to their inputs correctly.",
        )]
        pub log_file_write_edges: bool,

        #[option(
            name = "experimental_execution_graph_include_change_pruned_actions",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            help = "Whether to include change pruned actions in execution graph.",
        )]
        pub include_change_pruned_actions: bool,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const EXECUTION_GRAPH_MODULE_EXECUTION_GRAPH_OPTIONS_OPTION_NAMES: &[&str] = &[
        "experimental_enable_execution_graph_log",
        "experimental_execution_graph_log_path",
        "experimental_execution_graph_log_dep_type",
        "experimental_execution_graph_log_queue_size",
        "experimental_execution_graph_enable_edges_from_filewrite_actions",
        "experimental_execution_graph_include_change_pruned_actions",
    ];

    #[test]
    fn execution_graph_module_execution_graph_options_option_names() {
        super::super::test_util::check_option_names(
            ExecutionGraphModuleExecutionGraphOptions::class(),
            EXECUTION_GRAPH_MODULE_EXECUTION_GRAPH_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn execution_graph_module_execution_graph_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<
            ExecutionGraphModuleExecutionGraphOptions,
        >(SKIP_ROUND_TRIP);
    }
}

// ---- hand-written below this line (preserved by the generator) ----

// Java: com/google/devtools/build/lib/runtime/ExecutionGraphModule.java:173 ExecutionGraphModule.DependencyInfo
option_enum! {
    /// Which dependency edges to log.
    pub enum DependencyInfo { NONE, RUNFILES, ALL }
}

// Java: com/google/devtools/build/lib/runtime/ExecutionGraphModule.java:180 ExecutionGraphModule.DependencyInfoConverter
enum_converter!(pub DependencyInfoConverter, DependencyInfo, "dependency edge strategy");

/// Options whose default cannot be re-parsed in a test (none).
#[cfg(test)]
const SKIP_ROUND_TRIP: &[&str] = &[];

#[cfg(test)]
mod tests {
    use super::super::test_util::parse;
    use super::*;

    #[test]
    fn dep_type() {
        let o = parse::<ExecutionGraphModuleExecutionGraphOptions>(&[
            "--experimental_execution_graph_log_dep_type=all",
        ]);
        assert_eq!(o.dep_type, DependencyInfo::ALL);
    }
}
