//! Transcription of `com/google/devtools/build/lib/bazel/BazelStartupOptionsModule.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/bazel/BazelStartupOptionsModule.java:27 BazelStartupOptionsModule.Options
options! {
    /// Bazelrc file flags.
    pub struct BazelStartupOptionsModuleOptions {
        // NOTE: purely decorative, rc files are read by the client.
        #[option(
            name = "bazelrc",
            default_value = "null",
            allow_multiple = true,
            documentation_category = BAZEL_CLIENT_OPTIONS,
            effect_tags = [CHANGES_INPUTS],
            value_help = "<path>",
            help = "The location of the user .bazelrc file containing default values of Bazel options.\n`/dev/null` indicates that all further `--bazelrc`s will be ignored, which is useful to\ndisable the search for a user rc file, e.g. in release builds.\n\nThis option can also be specified multiple times. e.g. with\n`--bazelrc=x.rc --bazelrc=y.rc --bazelrc=/dev/null --bazelrc=z.rc`.\n1. `x.rc` and `y.rc` are read.\n2. `z.rc` is ignored due to the prior `/dev/null`.\nIf unspecified, Bazel uses the first `.bazelrc` file it finds in\nthe following two locations: the workspace directory, then the user's home\ndirectory.\n\nNote: command line options will always supersede any option in bazelrc.\n",
        )]
        pub blazerc: Vec<String>,

        // NOTE: purely decorative, rc files are read by the client.
        #[option(
            name = "system_rc",
            default_value = "true",
            documentation_category = BAZEL_CLIENT_OPTIONS,
            effect_tags = [CHANGES_INPUTS],
            help = "Whether or not to look for the system-wide bazelrc.",
        )]
        pub system_rc: bool,

        // NOTE: purely decorative, rc files are read by the client.
        #[option(
            name = "workspace_rc",
            default_value = "true",
            documentation_category = BAZEL_CLIENT_OPTIONS,
            effect_tags = [CHANGES_INPUTS],
            help = "Whether or not to look for the workspace bazelrc file at `$workspace/.bazelrc`",
        )]
        pub workspace_rc: bool,

        // NOTE: purely decorative, rc files are read by the client.
        #[option(
            name = "home_rc",
            default_value = "true",
            documentation_category = BAZEL_CLIENT_OPTIONS,
            effect_tags = [CHANGES_INPUTS],
            help = "Whether or not to look for the home bazelrc file at `$HOME/.bazelrc`",
        )]
        pub home_rc: bool,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const BAZEL_STARTUP_OPTIONS_MODULE_OPTIONS_OPTION_NAMES: &[&str] =
        &["bazelrc", "system_rc", "workspace_rc", "home_rc"];

    #[test]
    fn bazel_startup_options_module_options_option_names() {
        super::super::test_util::check_option_names(
            BazelStartupOptionsModuleOptions::class(),
            BAZEL_STARTUP_OPTIONS_MODULE_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn bazel_startup_options_module_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<BazelStartupOptionsModuleOptions>(
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
    fn rc_options() {
        let o = parse::<BazelStartupOptionsModuleOptions>(&[
            "--bazelrc=a.rc",
            "--bazelrc=b.rc",
            "--nohome_rc",
        ]);
        assert_eq!(o.blazerc, vec!["a.rc", "b.rc"]);
        assert!(!o.home_rc);
    }
}
