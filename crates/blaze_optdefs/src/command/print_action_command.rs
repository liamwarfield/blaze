//! Transcription of `com/google/devtools/build/lib/runtime/commands/PrintActionCommand.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/runtime/commands/PrintActionCommand.java:85 PrintActionCommand.PrintActionOptions
options! {
    /// Options for print_action, used to parse command-line arguments.
    pub struct PrintActionCommandPrintActionOptions {
        #[option(
            name = "print_action_mnemonics",
            default_value = "null",
            allow_multiple = true,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Lists which mnemonics to filter print_action data by, no filtering takes place ",
                "when left empty.",
            ),
        )]
        pub print_action_mnemonics: Vec<String>,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const PRINT_ACTION_COMMAND_PRINT_ACTION_OPTIONS_OPTION_NAMES: &[&str] =
        &["print_action_mnemonics"];

    #[test]
    fn print_action_command_print_action_options_option_names() {
        super::super::test_util::check_option_names(
            PrintActionCommandPrintActionOptions::class(),
            PRINT_ACTION_COMMAND_PRINT_ACTION_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn print_action_command_print_action_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<PrintActionCommandPrintActionOptions>(
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
    fn mnemonics() {
        let o = parse::<PrintActionCommandPrintActionOptions>(&[
            "--print_action_mnemonics=CppCompile",
            "--print_action_mnemonics=Javac",
        ]);
        assert_eq!(o.print_action_mnemonics, vec!["CppCompile", "Javac"]);
    }
}
