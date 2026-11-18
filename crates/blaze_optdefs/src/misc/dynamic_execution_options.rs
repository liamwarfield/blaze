//! Port of `dynamic/DynamicExecutionOptions.java` (the `OptionsBase` subclass and what it uses).

use std::time::Duration;

use blaze_options::{
    ConversionContext, Converter, OptionsParsingException, StringToStringListConverter, Value,
    Void, options,
};

// Java: com/google/devtools/build/lib/dynamic/DynamicExecutionOptions.java:33 DynamicExecutionOptions
options! {
    /// Options related to dynamic spawn execution.
    pub struct DynamicExecutionOptions {
        #[option(
            name = "experimental_spawn_scheduler",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            default_value = "null",
            help = concat!(
                "Enable dynamic execution by running actions locally and remotely in parallel. Bazel ",
                "spawns each action locally and remotely and picks the one that completes first. ",
                "If an action supports workers, the local action will be run in the persistent ",
                "worker mode. To enable dynamic execution for an individual action mnemonic, use ",
                "the `--internal_spawn_scheduler` and `--strategy=<mnemonic>=dynamic` flags ",
                "instead.",
            ),
            expansion = ["--internal_spawn_scheduler", "--spawn_strategy=dynamic"],
            deprecation_warning = concat!(
                "--experimental_spawn_scheduler is deprecated. Using dynamic execution for everything is",
                " rarely a good idea (see https://bazel.build/remote/dynamic). If you really want",
                " to enable dynamic execution globally, pass `--internal_spawn_scheduler ",
                "--spawn_strategy=dynamic`.",
            ),
            deprecated = true,
        )]
        pub experimental_spawn_scheduler: Void,

        #[option(
            name = "internal_spawn_scheduler",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION, HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            default_value = "true",
            help = concat!(
                "Placeholder option so that we can tell in Blaze whether the spawn scheduler was ",
                "enabled.",
            ),
        )]
        pub internal_spawn_scheduler: bool,

        #[option(
            name = "dynamic_local_strategy",
            converter = StringToStringListConverter,
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION, HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            default_value = "null",
            allow_multiple = true,
            help = concat!(
                "The local strategies, in order, to use for the given mnemonic - the first applicable ",
                "strategy is used. For example, `worker,sandboxed` runs actions that support ",
                "persistent workers using the worker strategy, and all others using the sandboxed ",
                "strategy. If no mnemonic is given, the list of strategies is used as the ",
                "fallback for all mnemonics. The default fallback list is `worker,sandboxed`, or",
                "`worker,sandboxed,standalone` if `experimental_local_lockfree_output` is set. ",
                "Takes [mnemonic=]local_strategy[,local_strategy,...]",
            ),
        )]
        pub dynamic_local_strategy: Vec<(String, Vec<String>)>,

        #[option(
            name = "dynamic_remote_strategy",
            converter = StringToStringListConverter,
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION, HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            default_value = "null",
            allow_multiple = true,
            help = concat!(
                "The remote strategies, in order, to use for the given mnemonic - the first applicable ",
                "strategy is used. If no mnemonic is given, the list of strategies is used as the ",
                "fallback for all mnemonics. The default fallback list is `remote`, so this flag ",
                "usually does not need to be set explicitly. ",
                "Takes [mnemonic=]remote_strategy[,remote_strategy,...]",
            ),
        )]
        pub dynamic_remote_strategy: Vec<(String, Vec<String>)>,

        #[option(
            name = "dynamic_local_execution_delay",
            old_name = "experimental_local_execution_delay",
            old_name_warning = false,
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION, HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            default_value = "1000",
            help = concat!(
                "How many milliseconds should local execution be delayed, if remote execution was faster",
                " during a build at least once?",
            ),
        )]
        pub local_execution_delay: i32,

        #[option(
            name = "debug_spawn_scheduler",
            old_name = "experimental_debug_spawn_scheduler",
            old_name_warning = false,
            documentation_category = LOGGING,
            effect_tags = [UNKNOWN],
            default_value = "false",
        )]
        pub debug_spawn_scheduler: bool,

        #[option(
            name = "experimental_dynamic_slow_remote_time",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION, HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            default_value = "0",
            help = concat!(
                "If >0, the time a dynamically run action must run remote-only before we",
                " prioritize its local execution to avoid remote timeouts.",
                " This may hide some problems on the remote execution system. Do not turn this on",
                " without monitoring of remote execution issues.",
            ),
        )]
        pub slow_remote_time: Duration,

        #[option(
            name = "experimental_dynamic_local_load_factor",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION, HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            default_value = "0",
            help = concat!(
                "Controls how much load from dynamic execution to put on the local machine.",
                " This flag adjusts how many actions in dynamic execution we will schedule",
                " concurrently. It is based on the number of CPUs Blaze thinks is available,",
                " which can be controlled with the --local_resources=cpu= flag.",
                "\n",
                "If this flag is 0, all actions are scheduled locally immediately. If > 0,",
                " the amount of actions scheduled locally is limited by the number of CPUs",
                " available. If < 1, the load factor is used to reduce the number of locally",
                " scheduled actions when the number of actions waiting to schedule is high.",
                " This lessens the load on the local machine in the clean build case, where",
                " the local machine does not contribute much.",
            ),
        )]
        pub local_load_factor: f64,

        #[option(
            name = "experimental_dynamic_exclude_tools",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION, HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            default_value = "true",
            help = concat!(
                "When set, targets that are build \"for tool\" are not subject to dynamic execution. Such",
                " targets are extremely unlikely to be built incrementally and thus not worth",
                " spending local cycles on.",
            ),
        )]
        pub exclude_tools: bool,

        #[option(
            name = "experimental_dynamic_ignore_local_signals",
            documentation_category = BUILD_TIME_OPTIMIZATION,
            converter = SignalListConverter,
            effect_tags = [EXECUTION],
            default_value = "null",
            help = concat!(
                "Takes a list of OS signal numbers. If a local branch of dynamic execution",
                " gets killed with any of these signals, the remote branch will be allowed to",
                " finish instead. For persistent workers, this only affects signals that kill",
                " the worker process.",
            ),
        )]
        /// PORT: Java `Set<Integer>`; the converter returns the signals in insertion order without
        /// duplicates (`ImmutableSet`).
        pub ignore_local_signals: Option<Vec<i32>>,
    }
}

// Java: com/google/devtools/build/lib/dynamic/DynamicExecutionOptions.java:177 DynamicExecutionOptions.SignalListConverter
/// Converts comma-separated lists of signal numbers into a set of signal numbers.
#[derive(Clone, Copy, Debug, Default)]
pub struct SignalListConverter;

impl Converter for SignalListConverter {
    // Java: com/google/devtools/build/lib/dynamic/DynamicExecutionOptions.java:179 DynamicExecutionOptions.SignalListConverter.convert
    fn convert(
        &self,
        input: Option<&str>,
        _: Option<&ConversionContext>,
    ) -> Result<Option<Value>, OptionsParsingException> {
        let input = match input {
            None | Some("null") => return Ok(Some(Value::list_of(Vec::<i32>::new()))),
            Some(i) => i,
        };
        // Splitter.on(",").split(input) always yields at least one part.
        let parts = input.split(',');
        let mut signals: Vec<i32> = Vec::new();
        for p in parts {
            // String.trim(): strips chars <= ' '.
            let trimmed = p.trim_matches(|c: char| c <= ' ');
            match super::resource_converter::guava_ints_try_parse(trimmed) {
                Some(signal_num) if signal_num > 0 => {
                    if !signals.contains(&signal_num) {
                        signals.push(signal_num);
                    }
                }
                _ => {
                    return Err(OptionsParsingException::new(format!(
                        "No such signal {}",
                        trimmed
                    )));
                }
            }
        }
        Ok(Some(Value::list_of(signals)))
    }

    // Java: com/google/devtools/build/lib/dynamic/DynamicExecutionOptions.java:202 DynamicExecutionOptions.SignalListConverter.getTypeDescription
    fn get_type_description(&self) -> String {
        "a comma-separated list of signal numbers".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::misc::test_support::{check_options_class, parse};

    #[test]
    fn options_class_matches_java() {
        check_options_class::<DynamicExecutionOptions>(&[
            "experimental_spawn_scheduler",
            "internal_spawn_scheduler",
            "dynamic_local_strategy",
            "dynamic_remote_strategy",
            "dynamic_local_execution_delay",
            "debug_spawn_scheduler",
            "experimental_dynamic_slow_remote_time",
            "experimental_dynamic_local_load_factor",
            "experimental_dynamic_exclude_tools",
            "experimental_dynamic_ignore_local_signals",
        ]);
    }

    #[test]
    fn parses_flags() {
        let o = parse::<DynamicExecutionOptions>(&[
            "--dynamic_local_strategy=Javac=worker,sandboxed",
            "--dynamic_local_strategy=local",
            "--experimental_dynamic_ignore_local_signals=9, 15,9",
            "--experimental_local_execution_delay=500",
        ])
        .unwrap();
        assert_eq!(
            o.dynamic_local_strategy,
            vec![
                (
                    "Javac".to_string(),
                    vec!["worker".to_string(), "sandboxed".to_string()]
                ),
                (String::new(), vec!["local".to_string()]),
            ]
        );
        assert_eq!(o.ignore_local_signals, Some(vec![9, 15]));
        assert_eq!(o.local_execution_delay, 500);
        assert_eq!(
            parse::<DynamicExecutionOptions>(&["--experimental_dynamic_ignore_local_signals=0"])
                .unwrap_err()
                .get_message(),
            "While parsing option --experimental_dynamic_ignore_local_signals=0: No such signal 0"
        );
    }

    #[test]
    fn experimental_spawn_scheduler_expands() {
        use blaze_options::OptionsClass;
        let d = DynamicExecutionOptions::class()
            .definitions()
            .find(|d| d.get_option_name() == "experimental_spawn_scheduler")
            .unwrap();
        assert_eq!(
            d.get_option_expansion(),
            &["--internal_spawn_scheduler", "--spawn_strategy=dynamic"]
        );
        assert!(d.is_deprecated());
        // --spawn_strategy belongs to ExecutionOptions, so expanding over this class alone fails.
        assert_eq!(
            parse::<DynamicExecutionOptions>(&["--experimental_spawn_scheduler"])
                .unwrap_err()
                .get_message(),
            "Unrecognized option: --spawn_strategy=dynamic"
        );
    }

    // Java tests: src/test/java/com/google/devtools/build/lib/dynamic/DynamicExecutionOptionsTest.java

    fn signals(s: Option<&str>) -> Result<Vec<i32>, OptionsParsingException> {
        Ok(SignalListConverter
            .convert(s, None)?
            .unwrap()
            .as_list()
            .unwrap()
            .iter()
            .map(|v| v.get::<i32>())
            .collect())
    }

    #[test]
    fn test_signal_name_converter_converts_integers() {
        assert_eq!(signals(Some("9")).unwrap(), vec![9]);
        assert_eq!(signals(Some("1,12,64")).unwrap(), vec![1, 12, 64]);
        assert_eq!(signals(Some("1,2,1,4")).unwrap(), vec![1, 2, 4]);
        assert_eq!(signals(Some("9")).unwrap(), vec![9]);
        assert_eq!(signals(Some("  12 , 14\t")).unwrap(), vec![12, 14]);
    }

    #[test]
    fn test_signal_name_converter_bad_inputs() {
        assert!(signals(None).unwrap().is_empty());
        assert!(signals(Some("null")).unwrap().is_empty());
        assert!(signals(Some("\t  ")).is_err());
        assert!(signals(Some("")).is_err());
        assert!(signals(Some("-1")).is_err());
        assert!(signals(Some("5,,6")).is_err());
        assert!(signals(Some("5.3")).is_err());
        assert!(signals(Some("")).is_err());
        assert!(signals(Some("SIGTERM")).is_err());
    }
}
