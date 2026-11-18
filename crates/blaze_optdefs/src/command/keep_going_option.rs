//! Transcription of `com/google/devtools/build/lib/runtime/KeepGoingOption.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/runtime/KeepGoingOption.java:22 KeepGoingOption
options! {
    /// Defines the --keep_going option which is used by multiple commands.
    pub struct KeepGoingOption {
        #[option(
            name = "keep_going",
            abbrev = 'k',
            default_value = "false",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EAGERNESS_TO_EXIT],
            help = concat!(
                "Continue as much as possible after an error.  While the target that failed and those ",
                "that depend on it cannot be analyzed, other prerequisites of these targets can be.",
            ),
        )]
        pub keep_going: bool,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const KEEP_GOING_OPTION_OPTION_NAMES: &[&str] = &["keep_going"];

    #[test]
    fn keep_going_option_option_names() {
        super::super::test_util::check_option_names(
            KeepGoingOption::class(),
            KEEP_GOING_OPTION_OPTION_NAMES,
        );
    }

    #[test]
    fn keep_going_option_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<KeepGoingOption>(SKIP_ROUND_TRIP);
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
    fn abbreviation() {
        assert!(parse::<KeepGoingOption>(&["-k"]).keep_going);
        assert!(!KeepGoingOption::default().keep_going);
    }
}
