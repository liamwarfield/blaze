//! Transcription of `com/google/devtools/build/lib/runtime/commands/ShutdownCommand.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/runtime/commands/ShutdownCommand.java:41 ShutdownCommand.Options
options! {
    /// The 'blaze shutdown' command.
    pub struct ShutdownCommandOptions {
        #[option(
            name = "iff_heap_size_greater_than",
            default_value = "0",
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [LOSES_INCREMENTAL_STATE, EAGERNESS_TO_EXIT],
            help = concat!(
                "Iff non-zero, then shutdown will only shut down the server if the total memory (in MB) ",
                "consumed by the JVM exceeds this value.",
            ),
        )]
        pub heap_size_limit: i32,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const SHUTDOWN_COMMAND_OPTIONS_OPTION_NAMES: &[&str] = &["iff_heap_size_greater_than"];

    #[test]
    fn shutdown_command_options_option_names() {
        super::super::test_util::check_option_names(
            ShutdownCommandOptions::class(),
            SHUTDOWN_COMMAND_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn shutdown_command_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<ShutdownCommandOptions>(
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
    fn heap_size() {
        let o = parse::<ShutdownCommandOptions>(&["--iff_heap_size_greater_than=10"]);
        assert_eq!(o.heap_size_limit, 10);
    }
}
