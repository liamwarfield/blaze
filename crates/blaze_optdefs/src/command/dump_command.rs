//! Transcription of `com/google/devtools/build/lib/runtime/commands/DumpCommand.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/runtime/commands/DumpCommand.java:230 DumpCommand.DumpOptions
options! {
    /// NB! Any changes to this class must be kept in sync with anyOutput variable value in the {@link
    /// DumpCommand#exec} method below.
    pub struct DumpCommandDumpOptions {
        #[option(
            name = "packages",
            default_value = "false",
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [BAZEL_MONITORING],
            help = "Dump package cache content.",
        )]
        pub dump_packages: bool,

        #[option(
            name = "action_cache",
            default_value = "false",
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [BAZEL_MONITORING],
            help = "Dump action cache content.",
        )]
        pub dump_action_cache: bool,

        #[option(
            name = "rule_classes",
            default_value = "false",
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [BAZEL_MONITORING],
            help = "Dump rule classes.",
        )]
        pub dump_rule_classes: bool,

        #[option(
            name = "rules",
            default_value = "false",
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [BAZEL_MONITORING],
            help = "Dump rules, including counts and memory usage (if memory is tracked).",
        )]
        pub dump_rules: bool,

        #[option(
            name = "skylark_memory",
            default_value = "null",
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [BAZEL_MONITORING],
            help = concat!(
                "Dumps a pprof-compatible memory profile to the specified path. To learn more please",
                " see https://github.com/google/pprof.",
            ),
        )]
        pub starlark_memory: Option<String>,

        #[option(
            name = "skyframe",
            default_value = "off",
            converter = SkyframeDumpEnumConverter,
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [BAZEL_MONITORING],
            help = "Dump the Skyframe graph.",
        )]
        pub dump_skyframe: SkyframeDumpOption,

        #[option(
            name = "skykey_filter",
            default_value = ".*",
            converter = RegexFilterConverter,
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [BAZEL_MONITORING],
            help = concat!(
                "Regex filter of SkyKey names to output. Only used with --skyframe=deps, rdeps,",
                " function_graph.",
            ),
        )]
        pub sky_key_filter: RegexFilter,

        #[option(
            name = "memory",
            default_value = "null",
            converter = MemoryModeConverter,
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [BAZEL_MONITORING],
            help = "Dump the memory use of the given Skyframe node.",
        )]
        pub memory: Option<MemoryMode>,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const DUMP_COMMAND_DUMP_OPTIONS_OPTION_NAMES: &[&str] = &[
        "packages",
        "action_cache",
        "rule_classes",
        "rules",
        "skylark_memory",
        "skyframe",
        "skykey_filter",
        "memory",
    ];

    #[test]
    fn dump_command_dump_options_option_names() {
        super::super::test_util::check_option_names(
            DumpCommandDumpOptions::class(),
            DUMP_COMMAND_DUMP_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn dump_command_dump_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<DumpCommandDumpOptions>(
            SKIP_ROUND_TRIP,
        );
    }
}

// ---- hand-written below this line (preserved by the generator) ----

use super::skyframe_memory_dumper::DisplayMode;

// Java: com/google/devtools/build/lib/runtime/commands/DumpCommand.java:102 DumpCommand.MemoryCollectionMode
option_enum! {
    /// How to dump Skyframe memory.
    // SHALLOW: the objects owned by a single SkyValue; DEEP: objects reachable from a single
    // SkyValue; TRANSITIVE: objects in the Skyframe transitive closure of a SkyValue; FULL: every
    // object in Skyframe in "shallow" mode.
    pub enum MemoryCollectionMode { SHALLOW, DEEP, TRANSITIVE, FULL }
}

// Java: com/google/devtools/build/lib/runtime/commands/DumpCommand.java:114 DumpCommand.MemorySubjectType
option_enum! {
    /// Whose memory use we should measure.
    pub enum MemorySubjectType { STARLARK_MODULE, PACKAGE, CONFIGURED_TARGET }
}

// Java: com/google/devtools/build/lib/runtime/commands/DumpCommand.java:123 DumpCommand.MemoryMode
/// The parsed value of `--memory`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryMode {
    pub collection_mode: MemoryCollectionMode,
    pub display_mode: DisplayMode,
    pub type_: Option<MemorySubjectType>,
    pub needle: Option<String>,
    pub report_transient: bool,
    pub report_configuration: bool,
    pub report_precomputed: bool,
    pub report_workspace_status: bool,
    pub subject: Option<String>,
}

// Java record toString.
impl OptionScalar for MemoryMode {
    fn java_to_string(&self) -> String {
        fn s<T: OptionScalar>(v: &Option<T>) -> String {
            v.as_ref()
                .map(OptionScalar::java_to_string)
                .unwrap_or_else(|| "null".to_string())
        }
        format!(
            "MemoryMode[collectionMode={}, displayMode={}, type={}, needle={}, reportTransient={}, reportConfiguration={}, reportPrecomputed={}, reportWorkspaceStatus={}, subject={}]",
            OptionScalar::java_to_string(&self.collection_mode),
            OptionScalar::java_to_string(&self.display_mode),
            s(&self.type_),
            s(&self.needle),
            self.report_transient,
            self.report_configuration,
            self.report_precomputed,
            self.report_workspace_status,
            s(&self.subject)
        )
    }
}
option_field_type!(
    MemoryMode,
    "com.google.devtools.build.lib.runtime.commands.DumpCommand$MemoryMode",
    raw = MemoryMode {
        collection_mode: MemoryCollectionMode::SHALLOW,
        display_mode: DisplayMode::SUMMARY,
        type_: None,
        needle: None,
        report_transient: true,
        report_configuration: true,
        report_precomputed: true,
        report_workspace_status: true,
        subject: None,
    }
);

// Java: com/google/devtools/build/lib/runtime/commands/DumpCommand.java:135 DumpCommand.MemoryModeConverter
/// Converter for `MemoryCollectionMode`.
#[derive(Clone, Copy, Debug, Default)]
pub struct MemoryModeConverter;

impl MemoryModeConverter {
    // Java: com/google/devtools/build/lib/runtime/commands/DumpCommand.java:142 DumpCommand.MemoryModeConverter.convert
    pub fn convert_mode(&self, input: &str) -> Result<MemoryMode, OptionsParsingException> {
        // The SkyKey designator is frequently a Label, which usually contains a colon so we must
        // not split the argument into an unlimited number of elements
        let items: Vec<&str> = input.splitn(3, ':').collect();
        if items.len() > 3 {
            return Err(OptionsParsingException::new(
                "Should contain at most three segments separated by ':'",
            ));
        }

        let mut collection_mode = None;
        let mut display_mode = None;
        let mut needle = None;
        let mut report_transient = true;
        let mut report_configuration = true;
        let mut report_precomputed = true;
        let mut report_workspace_status = true;

        for word in items[0].split(',') {
            if let Some(rest) = word.strip_prefix("needle=") {
                needle = Some(rest.to_string());
                continue;
            }
            match word {
                "shallow" => collection_mode = Some(MemoryCollectionMode::SHALLOW),
                "deep" => collection_mode = Some(MemoryCollectionMode::DEEP),
                "transitive" => collection_mode = Some(MemoryCollectionMode::TRANSITIVE),
                "full" => collection_mode = Some(MemoryCollectionMode::FULL),
                "summary" => display_mode = Some(DisplayMode::SUMMARY),
                "count" => display_mode = Some(DisplayMode::COUNT),
                "bytes" => display_mode = Some(DisplayMode::BYTES),
                "notransient" => report_transient = false,
                "noconfig" => report_configuration = false,
                "noprecomputed" => report_precomputed = false,
                "noworkspacestatus" => report_workspace_status = false,
                _ => {
                    return Err(OptionsParsingException::new(format!(
                        "Unrecognized word '{}'",
                        word
                    )));
                }
            }
        }

        let Some(collection_mode) = collection_mode else {
            return Err(OptionsParsingException::new("No collection type specified"));
        };
        let Some(display_mode) = display_mode else {
            return Err(OptionsParsingException::new("No display mode specified"));
        };

        if collection_mode == MemoryCollectionMode::FULL {
            return Ok(MemoryMode {
                collection_mode,
                display_mode,
                type_: None,
                needle,
                report_transient,
                report_configuration,
                report_precomputed,
                report_workspace_status,
                subject: None,
            });
        }

        if items.len() != 3 {
            return Err(OptionsParsingException::new(
                "Should be in the form: <flags>:<node type>:<node>",
            ));
        }

        // MemorySubjectType.valueOf(items[1].toUpperCase(Locale.ROOT))
        let upper = items[1].to_uppercase();
        let Some(subject_type) =
            <MemorySubjectType as blaze_options::converters::JavaEnum>::values()
                .iter()
                .find(|t| blaze_options::converters::JavaEnum::name(*t) == upper)
        else {
            return Err(OptionsParsingException::new("Invalid subject type"));
        };

        Ok(MemoryMode {
            collection_mode,
            display_mode,
            type_: Some(*subject_type),
            needle,
            report_transient,
            report_configuration,
            report_precomputed,
            report_workspace_status,
            subject: Some(items[2].to_string()),
        })
    }
}

impl Converter for MemoryModeConverter {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(self.convert_mode(non_null(input))?)))
    }
    // Java: com/google/devtools/build/lib/runtime/commands/DumpCommand.java:136 DumpCommand.MemoryModeConverter.getTypeDescription
    fn get_type_description(&self) -> String {
        "memory mode".into()
    }
}

// Java: com/google/devtools/build/lib/runtime/commands/DumpCommand.java:305 DumpCommand.SkyframeDumpOption
option_enum! {
    /// Different ways to dump information about Skyframe.
    pub enum SkyframeDumpOption {
        OFF, SUMMARY, COUNT, VALUE, DEPS, RDEPS, FUNCTION_GRAPH, ACTIVE_DIRECTORIES,
        ACTIVE_DIRECTORIES_FRONTIER_DEPS,
    }
}

// Java: com/google/devtools/build/lib/runtime/commands/DumpCommand.java:318 DumpCommand.SkyframeDumpEnumConverter
enum_converter!(pub SkyframeDumpEnumConverter, SkyframeDumpOption, "Skyframe Dump option");

/// Options whose default cannot be re-parsed in a test (none).
#[cfg(test)]
const SKIP_ROUND_TRIP: &[&str] = &[];

#[cfg(test)]
mod tests {
    use super::super::test_util::{parse, parse_err};
    use super::*;

    #[test]
    fn memory_mode() {
        let o =
            parse::<DumpCommandDumpOptions>(&["--memory=deep,count,needle=x:package://foo:bar"]);
        let m = o.memory.unwrap();
        assert_eq!(m.type_, Some(MemorySubjectType::PACKAGE));
        assert_eq!(m.subject.as_deref(), Some("//foo:bar"));
        assert_eq!(m.needle.as_deref(), Some("x"));
        let o = parse::<DumpCommandDumpOptions>(&["--memory=full,bytes"]);
        assert_eq!(
            OptionScalar::java_to_string(&o.memory.unwrap()),
            "MemoryMode[collectionMode=FULL, displayMode=BYTES, type=null, needle=null, reportTransient=true, reportConfiguration=true, reportPrecomputed=true, reportWorkspaceStatus=true, subject=null]"
        );
        assert_eq!(
            parse_err::<DumpCommandDumpOptions>(&["--memory=deep"]),
            "While parsing option --memory=deep: No display mode specified"
        );
        let o = parse::<DumpCommandDumpOptions>(&["--skyframe=deps"]);
        assert_eq!(o.dump_skyframe, SkyframeDumpOption::DEPS);
    }
}
