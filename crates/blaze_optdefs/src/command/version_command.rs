//! Transcription of `com/google/devtools/build/lib/runtime/commands/VersionCommand.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/runtime/commands/VersionCommand.java:48 VersionCommand.VersionOptions
options! {
    /// Options for the "version" command.
    pub struct VersionCommandVersionOptions {
        #[option(
            name = "gnu_format",
            default_value = "false",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS, EXECUTION],
            help = concat!(
                "If set, write the version to stdout using the conventions described in the GNU",
                " standards.",
            ),
        )]
        pub gnu_format: bool,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const VERSION_COMMAND_VERSION_OPTIONS_OPTION_NAMES: &[&str] = &["gnu_format"];

    #[test]
    fn version_command_version_options_option_names() {
        super::super::test_util::check_option_names(
            VersionCommandVersionOptions::class(),
            VERSION_COMMAND_VERSION_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn version_command_version_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<VersionCommandVersionOptions>(
            SKIP_ROUND_TRIP,
        );
    }
}

// ---- hand-written below this line (preserved by the generator) ----

/// Options whose default cannot be re-parsed in a test (none).
#[cfg(test)]
const SKIP_ROUND_TRIP: &[&str] = &[];

#[cfg(test)]
mod tests {
    use super::super::test_util::parse;
    use super::*;

    #[test]
    fn gnu_format() {
        assert!(parse::<VersionCommandVersionOptions>(&["--gnu_format"]).gnu_format);
    }
}
