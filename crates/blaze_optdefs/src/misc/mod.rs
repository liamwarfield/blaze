//! Repository, remote, auth, sandbox, worker, build event service/stream, debug, profiler,
//! metrics, dynamic execution, local execution, include scanning and coverage options: the
//! `OptionsBase` subclasses of group "misc" in `SPLIT.tsv`, one module per Java class, plus the
//! small converters and value types they use.

// Shared converters and value types.
pub mod build_event_context;
pub mod extension_arg;
pub mod module_arg;
pub use crate::common::options_utils;
pub mod remote_build_event_upload_mode;
pub mod remote_outputs_mode;
pub use crate::common::resource_converter;

// Options classes.
pub mod auth_and_tls_options;
pub mod bazel_coverage_report_module;
pub mod build_event_protocol_options;
pub mod build_event_service_options;
pub mod build_event_stream_options;
pub mod command_profiler_module;
pub mod common_remote_options;
pub mod debugging_options;
pub mod dynamic_execution_options;
pub mod include_scanning_options;
pub mod local_execution_options;
pub mod metrics_module;
pub mod mod_options;
pub mod package_metrics_module;
pub mod post_gc_memory_use_recorder;
pub mod remote_options;
pub mod remote_startup_options;
pub mod repository_options;
pub mod sandbox_options;
pub mod starlark_debugger_options;
pub mod worker_options;

#[cfg(test)]
pub(crate) mod test_support {
    use blaze_options::{OptionsBase, OptionsClass, OptionsParser};

    /// Checks one transcribed options class:
    /// - the registered option names equal `expected_names` (extracted from the Java source, in
    ///   declaration order);
    /// - an `OptionsParser` over the class builds, and parsing nothing yields `T::default()`;
    /// - every non-null default round-trips: `--name=<defaultValue>` parses to the same value as
    ///   the evaluated default.
    pub fn check_options_class<T>(expected_names: &[&str])
    where
        T: OptionsClass + OptionsBase + Default + PartialEq + std::fmt::Debug,
    {
        // The parser sees options sorted by name (Java `OptionsData.getAllAnnotatedFieldsSorted`);
        // compare as sorted lists.
        let mut names: Vec<&str> = T::class()
            .definitions()
            .map(|d| d.get_option_name())
            .collect();
        names.sort_unstable();
        let mut expected: Vec<&str> = expected_names.to_vec();
        expected.sort_unstable();
        assert_eq!(names, expected, "option names of {}", T::class().name());

        let mut parser = OptionsParser::builder()
            .options_classes(&[T::class()])
            .build()
            .unwrap_or_else(|e| panic!("{}: {}", T::class().name(), e.get_message()));
        parser.parse(&[]).unwrap();
        let parsed: T = parser.get_options::<T>().unwrap();
        assert_eq!(parsed, T::default());

        for d in T::class().definitions() {
            if d.is_special_null_default() || d.is_expansion_option() || d.is_void_field() {
                continue;
            }
            let default = d.get_default_value(None);
            let arg = format!(
                "--{}={}",
                d.get_option_name(),
                d.get_unparsed_default_value()
            );
            let mut parser = OptionsParser::builder()
                .options_classes(&[T::class()])
                .build()
                .unwrap();
            parser
                .parse(&[arg.as_str()])
                .unwrap_or_else(|e| panic!("{}: {}", arg, e.get_message()));
            let parsed = parser.get_options::<T>().unwrap();
            let value = d.get_value(&parsed);
            assert_eq!(
                value,
                default,
                "default of --{} round-trips",
                d.get_option_name()
            );
        }
    }

    pub(crate) use crate::common::local_host_capacity::with_local_host_capacity;

    /// Parses `args` over `T` alone.
    pub fn parse<T: OptionsClass + OptionsBase>(
        args: &[&str],
    ) -> Result<T, blaze_options::OptionsParsingException> {
        let mut parser = OptionsParser::builder()
            .options_classes(&[T::class()])
            .build()
            .unwrap();
        parser.parse(args)?;
        Ok(parser.get_options::<T>().unwrap())
    }
}
