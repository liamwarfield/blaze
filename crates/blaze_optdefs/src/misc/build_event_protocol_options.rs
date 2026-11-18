//! Port of `buildeventstream/BuildEventProtocolOptions.java` (the `OptionsBase` subclass and what it uses).

use std::collections::HashMap;
use std::time::Duration;

use blaze_options::{
    AssignmentConverter, ConversionContext, Converter, OptionScalar, OptionsParsingException,
    Value, converter::non_null, enum_converter, option_field_type, options,
};

use super::build_event_context::OutputGroupFileMode;

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventProtocolOptions.java:34 BuildEventProtocolOptions
options! {
    /// Options used to configure the build event protocol.
    pub struct BuildEventProtocolOptions {
        #[option(
            name = "legacy_important_outputs",
            default_value = "false",
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "Use this to suppress generation of the legacy `important_outputs` field in the\n",
                "`TargetComplete` event. `important_outputs` are required for Bazel to ResultStore/BTX\n",
                "integration.\n",
            ),
        )]
        pub legacy_important_outputs: bool,

        #[option(
            name = "experimental_build_event_upload_strategy",
            default_value = "null",
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "Selects how to upload artifacts referenced in the build event protocol. In Bazel\n",
                "the valid options include `local` and `remote`. The default value is `local`.\n",
            ),
        )]
        pub build_event_upload_strategy: Option<String>,

        #[option(
            name = "build_event_upload_max_retries",
            old_name = "experimental_build_event_upload_max_retries",
            default_value = "4",
            documentation_category = LOGGING,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            help = "The maximum number of times Bazel should retry uploading a build event.",
        )]
        pub bes_upload_max_retries: i32,

        #[option(
            name = "experimental_build_event_upload_retry_minimum_delay",
            default_value = "1s",
            documentation_category = LOGGING,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            help = concat!(
                "Initial, minimum delay for exponential backoff retries when BEP upload fails. (exponent:",
                " 1.6)",
            ),
        )]
        pub bes_upload_retry_initial_delay: Duration,

        #[option(
            name = "experimental_stream_log_file_uploads",
            default_value = "false",
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "Stream log file uploads directly to the remote storage rather than writing them to",
                " disk.",
            ),
        )]
        pub streaming_log_file_uploads: bool,

        #[option(
            name = "experimental_build_event_expand_filesets",
            default_value = "false",
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            help = "If true, expand Filesets in the BEP when presenting output files.",
        )]
        pub expand_filesets: bool,

        #[option(
            name = "experimental_bep_target_summary",
            default_value = "false",
            documentation_category = LOGGING,
            effect_tags = [UNKNOWN],
            help = "Whether to publish `TargetSummary` events.",
        )]
        pub publish_target_summary: bool,

        #[option(
            name = "experimental_run_bep_event_include_residue",
            default_value = "false",
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "Whether to include the command-line residue in run build events which could contain the",
                " residue. By default, the residue is not included in run command build events that",
                " could contain the residue.",
            ),
        )]
        pub include_residue_in_run_bep_event: bool,

        #[option(
            name = "experimental_build_event_output_group_mode",
            default_value = "null",
            converter = BuildEventOutputGroupModeConverter,
            allow_multiple = true,
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "Specify how an output group's files will be represented in `TargetComplete`/`AspectComplete`\n",
                "BEP events. Values are an assignment of an output group name to one of\n",
                "`NAMED_SET_OF_FILES_ONLY`, `INLINE_ONLY`, or `BOTH`. The default value is\n",
                "`NAMED_SET_OF_FILES_ONLY`. If an output group is repeated, the final value to\n",
                "appear is used. The default value sets the mode for coverage artifacts to BOTH:\n",
                "`--experimental_build_event_output_group_mode=baseline.lcov=both`\n",
            ),
        )]
        pub output_group_file_modes: Vec<OutputGroupFileModeEntry>,
    }
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventProtocolOptions.java:118 BuildEventProtocolOptions.OutputGroupFileModeConverter
enum_converter!(
    OutputGroupFileModeConverter,
    OutputGroupFileMode,
    "Output group file reporting mode"
);

/// `Map.Entry<String, OutputGroupFileMode>`.
///
/// PORT: a local struct instead of a `(String, OutputGroupFileMode)` tuple: the orphan rule
/// forbids implementing `blaze_options::FieldType` for a tuple in this crate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputGroupFileModeEntry {
    pub key: String,
    pub value: OutputGroupFileMode,
}

impl OptionScalar for OutputGroupFileModeEntry {
    // Guava ImmutableEntry.toString: key=value.
    fn java_to_string(&self) -> String {
        format!("{}={}", self.key, self.value.java_to_string())
    }
}

option_field_type!(
    OutputGroupFileModeEntry,
    "java.util.Map$Entry",
    raw = OutputGroupFileModeEntry {
        key: String::new(),
        value: OutputGroupFileMode::NAMED_SET_OF_FILES_ONLY
    }
);

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventProtocolOptions.java:128 BuildEventProtocolOptions.BuildEventOutputGroupModeConverter
/// Options converter that parses the assignment of an `OutputGroupFileMode` for an output group by
/// name, e.g. `default=fileset` or `baseline.lcov=inline`.
#[derive(Clone, Copy, Debug, Default)]
struct BuildEventOutputGroupModeConverter;

impl Converter for BuildEventOutputGroupModeConverter {
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventProtocolOptions.java:134 BuildEventProtocolOptions.BuildEventOutputGroupModeConverter.getTypeDescription
    fn get_type_description(&self) -> String {
        "an output group name followed by an OutputGroupFileMode, e.g. default=both".into()
    }

    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventProtocolOptions.java:139 BuildEventProtocolOptions.BuildEventOutputGroupModeConverter.convert
    fn convert(
        &self,
        input: Option<&str>,
        _: Option<&ConversionContext>,
    ) -> Result<Option<Value>, OptionsParsingException> {
        let (key, value) = AssignmentConverter.convert_entry(non_null(input))?;
        let mode = OutputGroupFileModeConverter
            .convert(Some(&value), None)?
            .expect("EnumConverter returned null")
            .get::<OutputGroupFileMode>();
        Ok(Some(Value::new(OutputGroupFileModeEntry {
            key,
            value: mode,
        })))
    }
}

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventProtocolOptions.java:152 BuildEventProtocolOptions.OutputGroupFileModes
/// A mapping from output group name to the `OutputGroupFileMode` to use for that output group.
pub struct OutputGroupFileModes(Box<dyn Fn(&str) -> OutputGroupFileMode + Send + Sync>);

impl OutputGroupFileModes {
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventProtocolOptions.java:153 BuildEventProtocolOptions.OutputGroupFileModes.getMode
    pub fn get_mode(&self, output_group: &str) -> OutputGroupFileMode {
        (self.0)(output_group)
    }

    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventProtocolOptions.java:155 BuildEventProtocolOptions.OutputGroupFileModes.DEFAULT
    pub fn default_modes() -> OutputGroupFileModes {
        OutputGroupFileModes(Box::new(|_| OutputGroupFileMode::NAMED_SET_OF_FILES_ONLY))
    }
}

impl BuildEventProtocolOptions {
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventProtocolOptions.java:168 BuildEventProtocolOptions.getOutputGroupFileModesMapping
    /// Collects the values in `output_group_file_modes` into a map and returns an
    /// `OutputGroupFileModes` backed by that map and defaulting to `NAMED_SET_OF_FILES_ONLY` for
    /// output groups not in that map.
    ///
    /// This also implements the default value of the
    /// `--experimental_build_event_output_group_mode` option, which as an `allowMultiple` option
    /// cannot specify a default value. The default value sets the mode for coverage artifacts to
    /// BOTH: `--experimental_build_event_output_group_mode=baseline.lcov=both`.
    pub fn get_output_group_file_modes_mapping(&self) -> OutputGroupFileModes {
        // ImmutableMap.Builder.buildKeepingLast: later entries win.
        let mode_map: HashMap<String, OutputGroupFileMode> = self
            .output_group_file_modes
            .iter()
            .map(|e| (e.key.clone(), e.value))
            .collect();
        OutputGroupFileModes(Box::new(move |output_group| {
            mode_map
                .get(output_group)
                .copied()
                .unwrap_or(OutputGroupFileMode::NAMED_SET_OF_FILES_ONLY)
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::misc::test_support::{check_options_class, parse};

    #[test]
    fn options_class_matches_java() {
        check_options_class::<BuildEventProtocolOptions>(&[
            "legacy_important_outputs",
            "experimental_build_event_upload_strategy",
            "build_event_upload_max_retries",
            "experimental_build_event_upload_retry_minimum_delay",
            "experimental_stream_log_file_uploads",
            "experimental_build_event_expand_filesets",
            "experimental_bep_target_summary",
            "experimental_run_bep_event_include_residue",
            "experimental_build_event_output_group_mode",
        ]);
    }

    #[test]
    fn parses_output_group_modes() {
        let o = parse::<BuildEventProtocolOptions>(&[
            "--experimental_build_event_output_group_mode=default=inline_only",
            "--experimental_build_event_output_group_mode=default=both",
            "--experimental_build_event_upload_max_retries=7",
        ])
        .unwrap();
        assert_eq!(o.bes_upload_max_retries, 7);
        assert_eq!(o.output_group_file_modes.len(), 2);
        let m = o.get_output_group_file_modes_mapping();
        assert_eq!(m.get_mode("default"), OutputGroupFileMode::BOTH);
        assert_eq!(
            m.get_mode("other"),
            OutputGroupFileMode::NAMED_SET_OF_FILES_ONLY
        );
        assert_eq!(
            parse::<BuildEventProtocolOptions>(&[
                "--experimental_build_event_output_group_mode=x=bogus"
            ])
            .unwrap_err()
            .get_message(),
            "While parsing option --experimental_build_event_output_group_mode=x=bogus: Not a \
             valid Output group file reporting mode: 'bogus' (should be named_set_of_files_only, \
             inline_only or both)"
        );
    }
}
