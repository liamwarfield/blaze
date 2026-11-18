//! Transcription of `com/google/devtools/build/lib/runtime/commands/RunCommand.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:122 RunCommand.RunOptions
options! {
    /// Options for the "run" command.
    pub struct RunCommandRunOptions {
        #[option(
            name = "script_path",
            default_value = "null",
            converter = PathFragmentConverter,
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS, EXECUTION],
            help = concat!(
                "If set, write a shell script to the given file which invokes the target. If this",
                " option is set, the target is not run from %{product}. Use '%{product} run",
                " --script_path=foo //foo && ./foo' to invoke target '//foo' This differs from",
                " '%{product} run //foo' in that the %{product} lock is released and the",
                " executable is connected to the terminal's stdin.",
            ),
        )]
        pub script_path: Option<Foreign<PathFragment>>,

        #[option(
            name = "emit_script_path_in_exec_request",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "If true, emits the ExecRequest with --script_path file value and script contents",
                " instead of writing the script.",
            ),
        )]
        pub emit_script_path_in_exec_request: bool,

        #[option(
            name = "run",
            default_value = "true",
            documentation_category = BAZEL_CLIENT_OPTIONS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "If false, skip running the command line constructed for the built target. Note that",
                " this flag is ignored for all --script_path builds.",
            ),
        )]
        pub run_built_target: bool,

        #[option(
            name = "portable_paths",
            default_value = "false",
            documentation_category = BAZEL_CLIENT_OPTIONS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "If true, includes paths to replace in ExecRequest to make the resulting paths",
                " portable.",
            ),
        )]
        pub portable_paths: bool,

        #[option(
            name = "run_env",
            default_value = "null",
            converter = EnvVarsConverter,
            allow_multiple = true,
            documentation_category = BAZEL_CLIENT_OPTIONS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "Specifies the set of environment variables available to the target to run.",
                " Variables can be either specified by name, in which case the value will be",
                " taken from the invocation environment, by the <code>name=value</code> pair",
                " which sets the value independent of the invocation environment, or by",
                " <code>=name</code>, which unsets the variable of that name. This option can",
                " be used multiple times; for options given for the same variable, the latest",
                " wins, options for different variables accumulate. Note that the executed target",
                " will generally see the full environment of the host except for those variables",
                " that have been explicitly unset.",
            ),
        )]
        pub run_environment: Vec<EnvVar>,

        #[option(
            name = "run_in_cwd",
            default_value = "false",
            documentation_category = BAZEL_CLIENT_OPTIONS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "If true, runs the target in the current working directory instead of the runfile",
                " tree.",
            ),
        )]
        pub run_in_cwd: bool,

        #[option(
            name = "omit_run_args",
            default_value = "true",
            documentation_category = LOGGING,
            effect_tags = [TERMINAL_OUTPUT],
            help = concat!(
                "Specifies whether the arguments passed to the runnable target will be omitted from the",
                " output for privacy reasons. If set to true, the output will not contain the",
                " arguments passed to the target. If set to false, the output will contain the",
                " arguments passed to the target.",
            ),
        )]
        pub run_omit_run_args: bool,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const RUN_COMMAND_RUN_OPTIONS_OPTION_NAMES: &[&str] = &[
        "script_path",
        "emit_script_path_in_exec_request",
        "run",
        "portable_paths",
        "run_env",
        "run_in_cwd",
        "omit_run_args",
    ];

    #[test]
    fn run_command_run_options_option_names() {
        super::super::test_util::check_option_names(
            RunCommandRunOptions::class(),
            RUN_COMMAND_RUN_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn run_command_run_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<RunCommandRunOptions>(SKIP_ROUND_TRIP);
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
    fn run_options() {
        let o = parse::<RunCommandRunOptions>(&[
            "--script_path=~/x.sh",
            "--run_env=A=1",
            "--run_env=B",
        ]);
        assert!(o.script_path.unwrap().get_path_string().ends_with("/x.sh"));
        assert_eq!(o.run_environment.len(), 2);
        assert!(RunCommandRunOptions::default().script_path.is_none());
    }
}
