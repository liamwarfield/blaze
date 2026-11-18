//! Command, runtime, startup, execution, query and module options: the non-fragment
//! `OptionsBase` subclasses of group "command" in `SPLIT.tsv`. One module per Java file; nested
//! `Outer.Options` classes are named `OuterOptions`.
//!
//! The small converters, enums and value classes those options use are ported next to them, or in
//! the helper modules below (one per Java class) when several options files share them.
//!
//! Not transcribed: `common/options/OptionsParserImpl.skippedArgs` (an internal marker field of
//! the parser, owned by `blaze_options`) and the `@Option(...)` mention in `OptionsParser`'s
//! Javadoc; neither is an options class.

// Shared helpers (one per Java class).
pub mod action_execution_context;
pub mod aspect_resolver;
pub mod auto_output_filter;
pub mod caffeine_spec;
pub mod command_line_event;
pub mod digest_hash_function;
pub mod enum_filter_converter;
pub mod event_kind;
pub mod foreign;
pub mod gc_thrashing_detector;
pub mod include_config_fragments;
pub mod info_item_handler;
pub mod local_host_capacity;
pub mod memory_profiler;
pub mod options_utils;
pub mod per_label_options;
pub mod profiler_task;
pub mod regex_filter;
pub mod regex_filter_assignment_converter;
pub mod resource_converter;
pub mod rule_visibility;
pub mod skyframe_memory_dumper;
pub mod test_size;
pub mod test_timeout;

// Options classes.
pub mod analysis_options;
pub mod aquery_options;
pub mod bazel_rules_module;
pub mod bazel_startup_options_module;
pub mod blaze_server_startup_options;
pub mod build_language_options;
pub mod build_request_options;
pub mod canonicalize_command;
pub mod clean_command;
pub mod client_options;
pub mod common_command_options;
pub mod common_query_options;
pub mod cquery_options;
pub mod dump_command;
pub mod execution_graph_module;
pub mod execution_options;
pub mod fetch_options;
pub mod help_command;
pub mod host_jvm_startup_options;
pub mod info_command;
pub mod keep_going_option;
pub mod loading_options;
pub mod loading_phase_threads_option;
pub mod local_diff_awareness;
pub mod memory_pressure_options;
pub mod mobile_install_command;
pub mod output_filtering_module;
pub mod package_options;
pub mod print_action_command;
pub mod query_options;
pub mod remote_analysis_caching_options;
pub mod run_command;
pub mod shutdown_command;
pub mod skyfocus_options;
pub mod test_summary_options;
pub mod ui_options;
pub mod vendor_options;
pub mod version_command;
pub mod workspace_status_action;
pub mod write_adb_args_action;

/// Names every transcribed options file needs: the `blaze_options` API plus the shared helpers.
pub(crate) mod prelude {
    pub use blaze_options::converter::non_null;
    pub use blaze_options::*;
    pub use std::time::Duration;

    /// The result of `Converter::convert`.
    pub type ConvResult = Result<Option<Value>, OptionsParsingException>;

    pub use blaze_cmdline::{Label, PackageIdentifier};
    pub use blaze_vfs::{DigestHashFunction, PathFragment};

    pub use super::action_execution_context::ShowSubcommands;
    pub use super::aspect_resolver::AspectResolverMode;
    pub use super::auto_output_filter::{AutoOutputFilter, AutoOutputFilterConverter};
    pub use super::caffeine_spec::{CaffeineSpec, CaffeineSpecConverter};
    pub use super::command_line_event::{ToolCommandLineEvent, ToolCommandLineEventConverter};
    pub use super::common_query_options::AspectResolutionModeConverter;
    pub use super::digest_hash_function::DigestFunctionConverter;
    pub use super::enum_filter_converter::EnumSet;
    pub use super::event_kind::EventKind;
    pub use super::foreign::Foreign;
    pub use super::gc_thrashing_detector::GcThrashingDetectorLimit;
    pub use super::include_config_fragments::{
        IncludeConfigFragmentsEnum, IncludeConfigFragmentsEnumConverter,
    };
    pub use super::info_item_handler::InfoItemOutputType;
    pub use super::memory_profiler::{
        MemoryProfileStableHeapParameters, MemoryProfileStableHeapParametersConverter,
    };
    pub use super::options_utils::{EmptyToNullPathFragmentConverter, PathFragmentConverter};
    pub use super::per_label_options::{PerLabelOptions, PerLabelOptionsConverter};
    pub use super::profiler_task::ProfilerTask;
    pub use super::regex_filter::{RegexFilter, RegexFilterConverter};
    pub use super::regex_filter_assignment_converter::{
        RegexFilterAssignmentConverter, RegexFilterToStringList,
    };
    pub use super::resource_converter::{
        CpuResourceConverter, ResourceConverterAssignmentConverter, ResourceIntegerConverter,
    };
    pub use super::rule_visibility::RuleVisibility;
    pub use super::test_size::{TestSize, TestSizeFilterConverter};
    pub use super::test_timeout::{TestTimeout, TestTimeoutFilterConverter};
}

#[cfg(test)]
pub(crate) mod test_util {
    //! Checks shared by the generated per-class tests.

    use blaze_options::{OptionsClass, OptionsClassInfo, OptionsParser};

    /// The registered option names equal the names declared in Java. `expected` is in Java
    /// declaration order; the registry lists definitions sorted by name (Java
    /// `OptionsData.getOptionDefinitionsFromClass`), so both sides are sorted.
    pub fn check_option_names(class: &'static OptionsClassInfo, expected: &[&str]) {
        let actual: Vec<&str> = class.definitions().map(|d| d.get_option_name()).collect();
        let mut expected = expected.to_vec();
        expected.sort_unstable();
        assert_eq!(actual, expected, "option names of {}", class.name());
    }

    /// Every option's default parses, and re-parsing the default string explicitly gives the same
    /// options object as `Options.getDefaults`. Skips options that carry no default value to
    /// re-parse (`"null"` defaults, `Void`, `allowMultiple`, expansion and implicit-requirement
    /// options) and the names in `skip`.
    pub fn check_defaults_round_trip<O>(skip: &[&str])
    where
        O: OptionsClass + PartialEq + std::fmt::Debug + Default,
    {
        let defaults = O::default();
        for d in O::class().definitions() {
            let name = d.get_option_name();
            if skip.contains(&name)
                || d.is_special_null_default()
                || d.is_void_field()
                || d.allows_multiple()
                || d.is_expansion_option()
                || d.has_implicit_requirements()
            {
                continue;
            }
            let mut parser = OptionsParser::builder()
                .options_classes(&[O::class()])
                .build()
                .unwrap();
            let arg = format!("--{}={}", name, d.get_unparsed_default_value());
            if let Err(e) = parser.parse(&[arg.as_str()]) {
                panic!("{}: {}", arg, e.get_message());
            }
            let parsed = parser.get_options::<O>().unwrap();
            assert_eq!(parsed, defaults, "re-parsing {}", arg);
        }
    }

    /// Parses `args` over the single class `O`.
    pub fn parse<O: OptionsClass>(args: &[&str]) -> O {
        let mut parser = OptionsParser::builder()
            .options_classes(&[O::class()])
            .build()
            .unwrap();
        if let Err(e) = parser.parse(args) {
            panic!("{:?}: {}", args, e.get_message());
        }
        parser.get_options::<O>().unwrap()
    }

    /// Parses `args` over the single class `O` and returns the error message.
    pub fn parse_err<O: OptionsClass>(args: &[&str]) -> String {
        let mut parser = OptionsParser::builder()
            .options_classes(&[O::class()])
            .build()
            .unwrap();
        parser.parse(args).unwrap_err().get_message().to_string()
    }
}
