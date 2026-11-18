//! Transcription of `com/google/devtools/build/lib/runtime/commands/HelpCommand.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/runtime/commands/HelpCommand.java:101 HelpCommand.Options
options! {
    /// Only to be used to escape the internal hard-coded help texts when outputting HTML from help,
    /// which don't pose a security risk.
    pub struct HelpCommandOptions {
        #[option(
            name = "help_verbosity",
            default_value = "medium",
            converter = HelpVerbosityConverter,
            documentation_category = LOGGING,
            effect_tags = [TERMINAL_OUTPUT],
            help = "Select the verbosity of the help command.",
        )]
        pub help_verbosity: HelpVerbosity,

        #[option(
            name = "long",
            abbrev = 'l',
            default_value = "null",
            documentation_category = LOGGING,
            effect_tags = [TERMINAL_OUTPUT],
            expansion = ["--help_verbosity=long"],
            help = "Show full description of each option, instead of just its name.",
        )]
        pub show_long_form_options: Void,

        #[option(
            name = "short",
            default_value = "null",
            documentation_category = LOGGING,
            effect_tags = [TERMINAL_OUTPUT],
            expansion = ["--help_verbosity=short"],
            help = "Show only the names of the options, not their types or meanings.",
        )]
        pub show_short_form_options: Void,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const HELP_COMMAND_OPTIONS_OPTION_NAMES: &[&str] = &["help_verbosity", "long", "short"];

    #[test]
    fn help_command_options_option_names() {
        super::super::test_util::check_option_names(
            HelpCommandOptions::class(),
            HELP_COMMAND_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn help_command_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<HelpCommandOptions>(SKIP_ROUND_TRIP);
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
    fn verbosity_expansions() {
        assert_eq!(
            HelpCommandOptions::default().help_verbosity,
            HelpVerbosity::MEDIUM
        );
        assert_eq!(
            parse::<HelpCommandOptions>(&["-l"]).help_verbosity,
            HelpVerbosity::LONG
        );
        assert_eq!(
            parse::<HelpCommandOptions>(&["--short"]).help_verbosity,
            HelpVerbosity::SHORT
        );
    }
}
