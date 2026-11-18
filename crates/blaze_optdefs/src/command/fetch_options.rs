//! Transcription of `com/google/devtools/build/lib/bazel/commands/FetchOptions.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/bazel/commands/FetchOptions.java:23 FetchOptions
options! {
    /// Defines the options specific to Bazel's fetch command
    pub struct FetchOptions {
        #[option(
            name = "all",
            default_value = "false",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [CHANGES_INPUTS],
            help = "Fetches all external repositories necessary for building any target or repository.\nThis is the default if no other flags and arguments are provided. Only works\nwhen `--enable_bzlmod` is on.\n",
        )]
        pub all: bool,

        #[option(
            name = "configure",
            default_value = "false",
            documentation_category = BZLMOD,
            effect_tags = [CHANGES_INPUTS],
            help = "Only fetches repositories marked as `configure` for system-configuration purpose. Only\nworks when `--enable_bzlmod` is on.\n",
        )]
        pub configure: bool,

        #[option(
            name = "repo",
            default_value = "null",
            allow_multiple = true,
            documentation_category = BZLMOD,
            effect_tags = [CHANGES_INPUTS],
            help = "Only fetches the specified repository, which can be either `@apparent_repo_name` or\n`@@canonical_repo_name`. Only works when `--enable_bzlmod` is on.\n",
        )]
        pub repos: Vec<String>,

        #[option(
            name = "force",
            default_value = "false",
            documentation_category = BZLMOD,
            effect_tags = [CHANGES_INPUTS],
            help = "Ignore existing repository if any and force fetch the repository again. Only works when\n`--enable_bzlmod` is on.\n",
        )]
        pub force: bool,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const FETCH_OPTIONS_OPTION_NAMES: &[&str] = &["all", "configure", "repo", "force"];

    #[test]
    fn fetch_options_option_names() {
        super::super::test_util::check_option_names(
            FetchOptions::class(),
            FETCH_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn fetch_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<FetchOptions>(SKIP_ROUND_TRIP);
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
        let o = parse::<FetchOptions>(&["--repo=@a", "--repo=@b", "--force"]);
        assert_eq!(o.repos, vec!["@a", "@b"]);
        assert!(o.force);
    }
}
