//! Transcription of `com/google/devtools/build/lib/bazel/commands/VendorOptions.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/bazel/commands/VendorOptions.java:23 VendorOptions
options! {
    /// Defines the options specific to Bazel's vendor command.
    pub struct VendorOptions {
        #[option(
            name = "repo",
            default_value = "null",
            allow_multiple = true,
            documentation_category = BZLMOD,
            effect_tags = [CHANGES_INPUTS],
            help = "Only vendors the specified repository, which can be either `@apparent_repo_name` or\n`@@canonical_repo_name`. This option can be set multiple times.\n",
        )]
        pub repos: Vec<String>,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const VENDOR_OPTIONS_OPTION_NAMES: &[&str] = &["repo"];

    #[test]
    fn vendor_options_option_names() {
        super::super::test_util::check_option_names(
            VendorOptions::class(),
            VENDOR_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn vendor_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<VendorOptions>(SKIP_ROUND_TRIP);
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
    fn repos() {
        assert_eq!(parse::<VendorOptions>(&["--repo=@x"]).repos, vec!["@x"]);
    }
}
