//! Port of `starlarkdebug/module/StarlarkDebuggerOptions.java` (the `OptionsBase` subclass and what it uses).

use blaze_options::options;

// Java: com/google/devtools/build/lib/starlarkdebug/module/StarlarkDebuggerOptions.java:26 StarlarkDebuggerOptions
options! {
    /// Configuration options for Starlark debugging.
    pub struct StarlarkDebuggerOptions {
        #[option(
            name = "experimental_skylark_debug",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [EXECUTION],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "If true, Blaze will open the Starlark debug server at the start of the build ",
                "invocation, and wait for a debugger to attach before running the build.",
            ),
        )]
        pub debug_starlark: bool,

        #[option(
            name = "experimental_skylark_debug_server_port",
            default_value = "7300",
            documentation_category = UNDOCUMENTED,
            effect_tags = [EXECUTION],
            metadata_tags = [EXPERIMENTAL],
            help = "The port on which the Starlark debug server will listen for connections.",
        )]
        pub debug_server_port: i32,

        #[option(
            name = "experimental_skylark_debug_verbose_logging",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [TERMINAL_OUTPUT],
            metadata_tags = [EXPERIMENTAL],
            help = "Show verbose logs for the debugger.",
        )]
        pub verbose_logs: bool,

        #[option(
            name = "experimental_skylark_debug_reset_analysis",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [EXECUTION, LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "If true, resets analysis before executing the build. Has no effect without",
                " --experimental_skylark_debug",
            ),
        )]
        pub reset_analysis: bool,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::misc::test_support::{check_options_class, parse};

    #[test]
    fn options_class_matches_java() {
        check_options_class::<StarlarkDebuggerOptions>(&[
            "experimental_skylark_debug",
            "experimental_skylark_debug_server_port",
            "experimental_skylark_debug_verbose_logging",
            "experimental_skylark_debug_reset_analysis",
        ]);
    }

    #[test]
    fn parses_flags() {
        let o = parse::<StarlarkDebuggerOptions>(&[
            "--experimental_skylark_debug",
            "--experimental_skylark_debug_server_port=1234",
        ])
        .unwrap();
        assert!(o.debug_starlark);
        assert_eq!(o.debug_server_port, 1234);
    }
}
