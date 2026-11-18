//! Transcription of `com/google/devtools/build/lib/runtime/commands/CleanCommand.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/runtime/commands/CleanCommand.java:69 CleanCommand.Options
options! {
    /// An interface for special options for the clean command.
    pub struct CleanCommandOptions {
        #[option(
            name = "expunge",
            default_value = "false",
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            help = concat!(
                "If true, clean removes the entire working tree for this %{product} instance, ",
                "which includes all %{product}-created temporary and build output files, ",
                "and stops the %{product} server if it is running.",
            ),
        )]
        pub expunge: bool,

        #[option(
            name = "expunge_async",
            default_value = "null",
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            expansion = ["--expunge", "--async"],
            help = concat!(
                "If specified, clean asynchronously removes the entire working tree for ",
                "this %{product} instance, which includes all %{product}-created temporary and ",
                "build output files, and stops the %{product} server if it is running. When ",
                "this command completes, it will be safe to execute new commands in the same ",
                "client, even though the deletion may continue in the background.",
            ),
        )]
        pub expunge_async: Void,

        #[option(
            name = "async",
            default_value = "false",
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            help = concat!(
                "If true, output cleaning is asynchronous. When this command completes, it will be safe ",
                "to execute new commands in the same client, even though the deletion may continue ",
                "in the background.",
            ),
        )]
        pub async_: bool,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const CLEAN_COMMAND_OPTIONS_OPTION_NAMES: &[&str] = &["expunge", "expunge_async", "async"];

    #[test]
    fn clean_command_options_option_names() {
        super::super::test_util::check_option_names(
            CleanCommandOptions::class(),
            CLEAN_COMMAND_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn clean_command_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<CleanCommandOptions>(SKIP_ROUND_TRIP);
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
    fn expunge_async_expansion() {
        let o = parse::<CleanCommandOptions>(&["--expunge_async"]);
        assert!(o.expunge);
        assert!(o.async_);
    }
}
