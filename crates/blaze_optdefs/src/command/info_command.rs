//! Transcription of `com/google/devtools/build/lib/runtime/commands/InfoCommand.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/runtime/commands/InfoCommand.java:113 InfoCommand.Options
options! {
    /// Options for the info command.
    pub struct InfoCommandOptions {
        #[option(
            name = "show_make_env",
            default_value = "false",
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS, TERMINAL_OUTPUT],
            help = "Include the \"Make\" environment in the output.",
        )]
        pub show_make_environment: bool,

        #[option(
            name = "info_output_type",
            default_value = "stdout",
            converter = InfoItemOutputTypeConverter,
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS, TERMINAL_OUTPUT],
            help = concat!(
                "If stdout, results are directly printed to the console. If response_proto, the info",
                " command results are packed in response extensions.",
            ),
        )]
        pub info_output_type: InfoItemOutputType,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const INFO_COMMAND_OPTIONS_OPTION_NAMES: &[&str] = &["show_make_env", "info_output_type"];

    #[test]
    fn info_command_options_option_names() {
        super::super::test_util::check_option_names(
            InfoCommandOptions::class(),
            INFO_COMMAND_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn info_command_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<InfoCommandOptions>(SKIP_ROUND_TRIP);
    }
}

// ---- hand-written below this line (preserved by the generator) ----

// Java: com/google/devtools/build/lib/runtime/commands/InfoCommand.java:134 InfoCommand.InfoItemOutputTypeConverter
enum_converter!(pub InfoItemOutputTypeConverter, InfoItemOutputType, "InfoItem output type");

/// Options whose default cannot be re-parsed in a test (none).
#[cfg(test)]
const SKIP_ROUND_TRIP: &[&str] = &[];

#[cfg(test)]
mod tests {
    use super::super::test_util::parse;
    use super::*;

    #[test]
    fn output_type() {
        let o =
            parse::<InfoCommandOptions>(&["--info_output_type=response_proto", "--show_make_env"]);
        assert_eq!(o.info_output_type, InfoItemOutputType::RESPONSE_PROTO);
        assert!(o.show_make_environment);
    }
}
