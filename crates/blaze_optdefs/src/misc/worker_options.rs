//! Port of `worker/WorkerOptions.java` (the `OptionsBase` subclass and what it uses).

use std::sync::Arc;
use std::time::Duration;

use blaze_options::{
    AssignmentConverter, CommaSeparatedOptionSetConverter, ConversionContext, Converter,
    DurationConverter, OptionScalar, OptionsParsingException, Value, option_field_type, options,
};

use super::resource_converter::{
    FLAG_SYNTAX, RamResourceConverter, ResourceConverterIntegerConverter,
};

/// `Map.Entry<String, Integer>` as produced by [`MultiResourceConverter`]; key and value may be
/// Java `null`.
///
/// PORT: a local struct instead of a tuple: the orphan rule forbids implementing
/// `blaze_options::FieldType` for a tuple here, and Java's entry can hold nulls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MultiResourceEntry {
    pub key: Option<String>,
    pub value: Option<i32>,
}

impl OptionScalar for MultiResourceEntry {
    // Guava ImmutableEntry.toString: key=value, with "null" for nulls.
    fn java_to_string(&self) -> String {
        format!(
            "{}={}",
            self.key.as_deref().unwrap_or("null"),
            self.value
                .map(|v| v.to_string())
                .unwrap_or_else(|| "null".to_string())
        )
    }
}

option_field_type!(
    MultiResourceEntry,
    "java.util.Map$Entry",
    raw = MultiResourceEntry {
        key: None,
        value: None
    }
);

// Java: com/google/devtools/build/lib/worker/WorkerOptions.java:45 WorkerOptions.MultiResourceConverter
/// Defines a resource converter for named values in the form [name=]value, where the value is
/// `ResourceConverter.FLAG_SYNTAX`. If no name is provided (used when setting a default), the
/// empty string is used as the key. The default value for unspecified mnemonics is defined in
/// `WorkerPoolImpl.createPool`. "auto" currently returns the default.
#[derive(Clone, Copy, Debug, Default)]
pub struct MultiResourceConverter;

impl MultiResourceConverter {
    // Java: com/google/devtools/build/lib/worker/WorkerOptions.java:47 WorkerOptions.MultiResourceConverter.valueConverter
    fn value_converter() -> ResourceConverterIntegerConverter {
        ResourceConverterIntegerConverter::new(Arc::new(|| 0), 0, i32::MAX)
    }
}

impl Converter for MultiResourceConverter {
    // Java: com/google/devtools/build/lib/worker/WorkerOptions.java:51 WorkerOptions.MultiResourceConverter.convert
    fn convert(
        &self,
        input: Option<&str>,
        _: Option<&ConversionContext>,
    ) -> Result<Option<Value>, OptionsParsingException> {
        // TODO(steinman): Make auto value return a reasonable multiplier of host capacity.
        let input = match input {
            None | Some("null") | Some("auto") => {
                return Ok(Some(Value::new(MultiResourceEntry {
                    key: None,
                    value: None,
                })));
            }
            Some(i) => i,
        };
        let Some(pos) = input.find('=') else {
            return Ok(Some(Value::new(MultiResourceEntry {
                key: Some(String::new()),
                value: Some(Self::value_converter().convert_int(input)?),
            })));
        };
        let name = &input[..pos];
        let value = &input[pos + 1..];
        if value == "auto" {
            return Ok(Some(Value::new(MultiResourceEntry {
                key: Some(name.to_string()),
                value: None,
            })));
        }
        Ok(Some(Value::new(MultiResourceEntry {
            key: Some(name.to_string()),
            value: Some(Self::value_converter().convert_int(value)?),
        })))
    }

    // Java: com/google/devtools/build/lib/worker/WorkerOptions.java:72 WorkerOptions.MultiResourceConverter.getTypeDescription
    fn get_type_description(&self) -> String {
        format!("[name=]value, where value is {}", FLAG_SYNTAX)
    }
}

// Java: com/google/devtools/build/lib/worker/WorkerOptions.java:36 WorkerOptions
options! {
    /// Options related to worker processes.
    pub struct WorkerOptions {
        #[option(
            name = "worker_max_instances",
            converter = MultiResourceConverter,
            default_value = "null",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION, HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            help = concat!(
                "How many instances of each kind of persistent worker may be ",
                "launched if you use the 'worker' strategy. May be specified as [name=value] to ",
                "give a different value per mnemonic. The limit is based on worker keys, which are ",
                "differentiated based on mnemonic, but also on startup flags and environment, so ",
                "there can in some cases be more workers per mnemonic than this flag specifies. ",
                "Takes ",
                "an integer, or a keyword (\"auto\", \"HOST_CPUS\", \"HOST_RAM\"), optionally followed by an operation ([-|*]<float>) eg. \"auto\", \"HOST_CPUS*.5\"",
                ". 'auto' calculates a reasonable default based on machine capacity. ",
                "\"=value\" sets a default for unspecified mnemonics.",
            ),
            allow_multiple = true,
        )]
        pub worker_max_instances: Vec<MultiResourceEntry>,

        #[option(
            name = "worker_max_multiplex_instances",
            old_name = "experimental_worker_max_multiplex_instances",
            converter = MultiResourceConverter,
            default_value = "null",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION, HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            help = concat!(
                "How many WorkRequests a multiplex worker process may receive in parallel if you use the",
                " 'worker' strategy with --worker_multiplex. May be specified as ",
                "[name=value] to give a different value per mnemonic. The limit is based on worker ",
                "keys, which are differentiated based on mnemonic, but also on startup flags and ",
                "environment, so there can in some cases be more workers per mnemonic than this ",
                "flag specifies. Takes ",
                "an integer, or a keyword (\"auto\", \"HOST_CPUS\", \"HOST_RAM\"), optionally followed by an operation ([-|*]<float>) eg. \"auto\", \"HOST_CPUS*.5\"",
                ". 'auto' calculates a reasonable default based on machine capacity. ",
                "\"=value\" sets a default for unspecified mnemonics.",
            ),
            allow_multiple = true,
        )]
        pub worker_max_multiplex_instances: Vec<MultiResourceEntry>,

        #[option(
            name = "worker_quit_after_build",
            default_value = "false",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION, HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            help = "If enabled, all workers quit after a build is done.",
        )]
        pub worker_quit_after_build: bool,

        #[option(
            name = "worker_verbose",
            default_value = "false",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [UNKNOWN],
            help = "If enabled, prints verbose messages when workers are started, shutdown, ...",
        )]
        pub worker_verbose: bool,

        #[option(
            name = "worker_extra_flag",
            converter = AssignmentConverter,
            default_value = "null",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION, HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            help = concat!(
                "Extra command-flags that will be passed to worker processes in addition to ",
                "--persistent_worker, keyed by mnemonic (e.g. --worker_extra_flag=Javac=--debug.",
            ),
            allow_multiple = true,
        )]
        pub worker_extra_flags: Vec<(String, String)>,

        #[option(
            name = "worker_sandboxing",
            default_value = "false",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION],
            help = concat!(
                "If enabled, singleplex workers will run in a sandboxed environment. Singleplex workers",
                " are always sandboxed when running under the dynamic execution strategy,",
                " irrespective of this flag.",
            ),
        )]
        pub worker_sandboxing: bool,

        #[option(
            name = "worker_multiplex",
            old_name = "experimental_worker_multiplex",
            default_value = "true",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION, HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            help = "If enabled, workers will use multiplexing if they support it. ",
        )]
        pub worker_multiplex: bool,

        #[option(
            name = "experimental_worker_cancellation",
            default_value = "false",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION],
            help = "If enabled, Bazel may send cancellation requests to workers that support them.",
        )]
        pub worker_cancellation: bool,

        #[option(
            name = "experimental_worker_multiplex_sandboxing",
            default_value = "false",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION],
            help = concat!(
                "If enabled, multiplex workers with a 'supports-multiplex-sandboxing' execution",
                " requirement will run in a sandboxed environment, using a separate sandbox",
                " directory per work request. Multiplex workers with the execution requirement are",
                " always sandboxed when running under the dynamic execution strategy,",
                " irrespective of this flag.",
            ),
        )]
        pub multiplex_sandboxing: bool,

        #[option(
            name = "experimental_worker_strict_flagfiles",
            default_value = "false",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION],
            help = concat!(
                "If enabled, actions arguments for workers that do not follow the worker specification",
                " will cause an error. Worker arguments must have exactly one @flagfile argument",
                " as the last of its list of arguments.",
            ),
        )]
        pub strict_flagfiles: bool,

        #[option(
            name = "experimental_total_worker_memory_limit_mb",
            converter = RamResourceConverter,
            default_value = "0",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION, HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            help = concat!(
                "If this limit is greater than zero idle workers might be killed if the total memory",
                " usage of all  workers exceed the limit.",
            ),
        )]
        pub total_worker_memory_limit_mb: i32,

        #[option(
            name = "experimental_worker_use_cgroups_on_linux",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [EXECUTION],
            help = concat!(
                "On linux, run all workers in its own cgroup (without any limits set) and use the",
                " cgroup's own resource accounting for memory measurements. This is overridden by",
                " --experimental_worker_sandbox_hardening for sandboxed workers.",
            ),
        )]
        pub use_cgroups_on_linux: bool,

        #[option(
            name = "experimental_worker_sandbox_hardening",
            default_value = "false",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION],
            help = concat!(
                "If enabled, workers are run in a hardened sandbox, if the implementation allows it. If",
                " hardening is enabled then tmp directories are distinct for different workers.",
            ),
        )]
        pub sandbox_hardening: bool,

        #[option(
            name = "experimental_shrink_worker_pool",
            default_value = "false",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION, HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            help = concat!(
                "If enabled, could shrink worker pool if worker memory pressure is high. This flag works",
                " only when flag experimental_total_worker_memory_limit_mb is enabled.",
            ),
        )]
        pub shrink_worker_pool: bool,

        #[option(
            name = "experimental_worker_metrics_poll_interval",
            converter = DurationConverter,
            default_value = "5s",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION, HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            help = concat!(
                "The interval between collecting worker metrics and possibly attempting evictions. ",
                "Cannot effectively be less than 1s for performance reasons.",
            ),
        )]
        pub worker_metrics_poll_interval: Duration,

        #[option(
            name = "experimental_worker_memory_limit_mb",
            converter = RamResourceConverter,
            default_value = "0",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION, HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            help = concat!(
                "If this limit is greater than zero, workers might be killed if the memory usage of the ",
                "worker exceeds the limit. If not used together with dynamic execution and ",
                "`--experimental_dynamic_ignore_local_signals=9`, this may crash your build.",
            ),
        )]
        pub worker_memory_limit_mb: i32,

        #[option(
            name = "experimental_worker_sandbox_inmemory_tracking",
            default_value = "null",
            allow_multiple = true,
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION],
            help = concat!(
                "A worker key mnemonic for which the contents of the sandbox directory are tracked in",
                " memory. This may improve build performance at the cost of additional memory",
                " usage. Only affects sandboxed workers. May be specified multiple times for",
                " different mnemonics.",
            ),
        )]
        pub worker_sandbox_in_memory_tracking: Vec<String>,

        #[option(
            name = "experimental_worker_allowlist",
            converter = CommaSeparatedOptionSetConverter,
            default_value = "null",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION, HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            help = "If non-empty, only allow using persistent workers with the given worker key mnemonic.",
        )]
        pub allowlist: Option<Vec<String>>,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::misc::test_support::{check_options_class, parse};

    #[test]
    fn options_class_matches_java() {
        check_options_class::<WorkerOptions>(&[
            "worker_max_instances",
            "worker_max_multiplex_instances",
            "worker_quit_after_build",
            "worker_verbose",
            "worker_extra_flag",
            "worker_sandboxing",
            "worker_multiplex",
            "experimental_worker_cancellation",
            "experimental_worker_multiplex_sandboxing",
            "experimental_worker_strict_flagfiles",
            "experimental_total_worker_memory_limit_mb",
            "experimental_worker_use_cgroups_on_linux",
            "experimental_worker_sandbox_hardening",
            "experimental_shrink_worker_pool",
            "experimental_worker_metrics_poll_interval",
            "experimental_worker_memory_limit_mb",
            "experimental_worker_sandbox_inmemory_tracking",
            "experimental_worker_allowlist",
        ]);
    }

    #[test]
    fn parses_flags() {
        let o = parse::<WorkerOptions>(&[
            "--worker_max_instances=4",
            "--worker_max_instances=Javac=auto",
            "--worker_max_instances=Foo=auto*2",
            "--worker_max_instances=auto",
            "--worker_extra_flag=Javac=--debug",
            "--experimental_worker_allowlist=a,b,a",
            "--experimental_worker_memory_limit_mb=100",
        ])
        .unwrap();
        assert_eq!(
            o.worker_max_instances,
            vec![
                MultiResourceEntry {
                    key: Some(String::new()),
                    value: Some(4)
                },
                MultiResourceEntry {
                    key: Some("Javac".into()),
                    value: None
                },
                MultiResourceEntry {
                    key: Some("Foo".into()),
                    value: Some(0)
                },
                MultiResourceEntry {
                    key: None,
                    value: None
                },
            ]
        );
        assert_eq!(
            o.worker_extra_flags,
            vec![("Javac".into(), "--debug".into())]
        );
        assert_eq!(o.allowlist, Some(vec!["a".to_string(), "b".to_string()]));
        assert_eq!(o.worker_memory_limit_mb, 100);
        assert_eq!(
            parse::<WorkerOptions>(&["--worker_max_instances=x=-2"])
                .unwrap_err()
                .get_message(),
            "While parsing option --worker_max_instances=x=-2: Value '(-2.000000)' must be at \
             least 0.000000."
        );
    }

    // Java tests: src/test/java/com/google/devtools/build/lib/worker/MultiResourceConverterTest.java

    use crate::misc::resource_converter::ResourceConverterIntegerConverter;
    use crate::misc::test_support::with_local_host_capacity;
    use blaze_options::Converter;

    fn multi(s: &str) -> MultiResourceEntry {
        MultiResourceConverter
            .convert(Some(s), None)
            .unwrap()
            .unwrap()
            .get::<MultiResourceEntry>()
    }

    fn resource_converter() -> ResourceConverterIntegerConverter {
        ResourceConverterIntegerConverter::new(
            Arc::new(|| panic!("NullPointerException: supplier returned null")),
            1,
            i32::MAX,
        )
    }

    #[test]
    fn convert_mnemonic_equals_auto_returns_default() {
        assert_eq!(multi("someMnemonic=auto").value, None);
    }

    #[test]
    fn convert_mnemonic_equals_keyword_equals_resource_converter_convert_keyword() {
        with_local_host_capacity((1000.0, 8.0), || {
            assert_eq!(
                multi("someMnemonic=HOST_CPUS-1").value,
                Some(resource_converter().convert_int("HOST_CPUS-1").unwrap())
            );
        });
    }

    #[test]
    fn convert_auto_returns_default() {
        assert_eq!(multi("auto").value, None);
    }

    #[test]
    fn convert_keyword_equals_resource_converter_convert_keyword() {
        with_local_host_capacity((1000.0, 8.0), || {
            assert_eq!(
                multi("HOST_CPUS-1").value,
                Some(resource_converter().convert_int("HOST_CPUS-1").unwrap())
            );
        });
    }

    #[test]
    fn convert_mnemonic_saves_correct_key() {
        assert_eq!(
            multi("someMnemonic=10").key.as_deref(),
            Some("someMnemonic")
        );
    }

    #[test]
    fn convert_auto_sets_empty_string_aka_default_as_key() {
        assert_eq!(multi("auto").key, None);
    }
}
