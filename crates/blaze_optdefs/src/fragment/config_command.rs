//! The options of `runtime/commands/ConfigCommand.java` (`ConfigCommand.ConfigOptions`).
//!
//! PORT: ConfigOptions extends OptionsBase, not FragmentOptions; it lives here because
//! `SPLIT.tsv` assigned `ConfigCommand.java` to the "fragment" group. The command itself is not
//! ported here.

use blaze_options::options;

blaze_options::option_enum! {
    // Java: com/google/devtools/build/lib/runtime/commands/ConfigCommand.java:86 ConfigCommand.OutputType
    pub enum ConfigCommandOutputType { TEXT, JSON }
}

// Java: com/google/devtools/build/lib/runtime/commands/ConfigCommand.java:102 ConfigCommand.ConfigOptions.OutputTypeConverter
blaze_options::enum_converter!(pub OutputTypeConverter, ConfigCommandOutputType, "output type");

// Java: com/google/devtools/build/lib/runtime/commands/ConfigCommand.java:92 ConfigCommand.ConfigOptions
options! {
    /// Options for the "config" command.
    pub struct ConfigCommandConfigOptions {
        #[option(
            name = "dump_all",
            default_value = "false",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = "If set, dump all known configurations instead of just the ids.",
        )]
        pub dump_all: bool,

        #[option(
            name = "output",
            converter = OutputTypeConverter,
            default_value = "text",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = "Formats the output of displayed results. Can be one of: 'text', 'json'. ",
        )]
        pub output_type: ConfigCommandOutputType,
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{assert_defaults_round_trip, assert_option_names, parse};
    use super::*;

    #[test]
    fn option_names_match_java() {
        assert_option_names::<ConfigCommandConfigOptions>(&["dump_all", "output"]);
    }

    #[test]
    fn defaults_round_trip() {
        assert_defaults_round_trip::<ConfigCommandConfigOptions>(&[]);
    }

    #[test]
    fn spot_check_parses() {
        let o = parse::<ConfigCommandConfigOptions>(&["--output=JSON", "--dump_all"]).unwrap();
        assert_eq!(o.output_type, ConfigCommandOutputType::JSON);
        assert!(o.dump_all);
        assert_eq!(
            parse::<ConfigCommandConfigOptions>(&["--output=xml"])
                .unwrap_err()
                .get_message(),
            "While parsing option --output=xml: Not a valid output type: 'xml' (should be text \
             or json)"
        );
    }
}
