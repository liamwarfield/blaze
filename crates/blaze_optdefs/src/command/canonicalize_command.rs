//! Transcription of `com/google/devtools/build/lib/runtime/commands/CanonicalizeCommand.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/runtime/commands/CanonicalizeCommand.java:72 CanonicalizeCommand.Options
options! {
    /// The 'blaze canonicalize-flags' command.
    pub struct CanonicalizeCommandOptions {
        #[option(
            name = "for_command",
            default_value = "build",
            documentation_category = GENERIC_INPUTS,
            effect_tags = [AFFECTS_OUTPUTS, TERMINAL_OUTPUT],
            help = "The command for which the options should be canonicalized.",
        )]
        pub for_command: String,

        #[option(
            name = "invocation_policy",
            default_value = "",
            documentation_category = GENERIC_INPUTS,
            effect_tags = [AFFECTS_OUTPUTS, TERMINAL_OUTPUT],
            help = "Applies an invocation policy to the options to be canonicalized.",
        )]
        pub invocation_policy: String,

        #[option(
            name = "canonicalize_policy",
            default_value = "false",
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [AFFECTS_OUTPUTS, TERMINAL_OUTPUT],
            help = concat!(
                "Output the canonical policy, after expansion and filtering. To keep the output clean,",
                " the canonicalized command arguments will NOT be shown when this option is set",
                " to true. Note that the command specified by --for_command affects the filtered",
                " policy, and if none is specified, the default command is 'build'.",
            ),
        )]
        pub canonicalize_policy: bool,

        #[option(
            name = "experimental_include_default_values",
            default_value = "true",
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [AFFECTS_OUTPUTS, TERMINAL_OUTPUT],
            help = "Whether Starlark options set to their default values are included in the output.",
        )]
        pub include_default_values: bool,
    }
}

// Java: com/google/devtools/build/lib/runtime/commands/CanonicalizeCommand.java:115 CanonicalizeCommand.FlagClashCanaryOptions
options! {
    /// These options are used by the incompatible_changes_conflict_test.sh integration test, which
    /// confirms that the warning for conflicting expansion options is working correctly. These flags
    /// are undocumented no-ops, and are not to be used by anything outside of that test.
    pub struct CanonicalizeCommandFlagClashCanaryOptions {
        #[option(
            name = "flag_clash_canary",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
        )]
        pub flag_clash_canary: bool,

        #[option(
            name = "flag_clash_canary_expander1",
            default_value = "null",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            expansion = ["--flag_clash_canary=1"],
        )]
        pub flag_clash_canary_expander1: Void,

        #[option(
            name = "flag_clash_canary_expander2",
            default_value = "null",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            expansion = ["--flag_clash_canary=0"],
        )]
        pub flag_clash_canary_expander2: Void,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const CANONICALIZE_COMMAND_OPTIONS_OPTION_NAMES: &[&str] = &[
        "for_command",
        "invocation_policy",
        "canonicalize_policy",
        "experimental_include_default_values",
    ];

    #[test]
    fn canonicalize_command_options_option_names() {
        super::super::test_util::check_option_names(
            CanonicalizeCommandOptions::class(),
            CANONICALIZE_COMMAND_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn canonicalize_command_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<CanonicalizeCommandOptions>(
            SKIP_ROUND_TRIP,
        );
    }

    /// Option names in Java declaration order (extracted from the Java source).
    const CANONICALIZE_COMMAND_FLAG_CLASH_CANARY_OPTIONS_OPTION_NAMES: &[&str] = &[
        "flag_clash_canary",
        "flag_clash_canary_expander1",
        "flag_clash_canary_expander2",
    ];

    #[test]
    fn canonicalize_command_flag_clash_canary_options_option_names() {
        super::super::test_util::check_option_names(
            CanonicalizeCommandFlagClashCanaryOptions::class(),
            CANONICALIZE_COMMAND_FLAG_CLASH_CANARY_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn canonicalize_command_flag_clash_canary_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<
            CanonicalizeCommandFlagClashCanaryOptions,
        >(SKIP_ROUND_TRIP);
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
    fn expansions() {
        let o =
            parse::<CanonicalizeCommandFlagClashCanaryOptions>(&["--flag_clash_canary_expander1"]);
        assert!(o.flag_clash_canary);
        let o = parse::<CanonicalizeCommandFlagClashCanaryOptions>(&[
            "--flag_clash_canary_expander1",
            "--flag_clash_canary_expander2",
        ]);
        assert!(!o.flag_clash_canary);
        let o = parse::<CanonicalizeCommandOptions>(&["--for_command=test"]);
        assert_eq!(o.for_command, "test");
    }
}
