//! Port of `exec/local/LocalExecutionOptions.java` (the `OptionsBase` subclass and what it uses).

use std::time::Duration;

use blaze_options::{RegexPatternConverter, RegexPatternOption, options};

// Java: com/google/devtools/build/lib/exec/local/LocalExecutionOptions.java:27 LocalExecutionOptions
options! {
    /// Local execution options.
    pub struct LocalExecutionOptions {
        #[option(
            name = "local_termination_grace_seconds",
            old_name = "local_sigkill_grace_seconds",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            default_value = "15",
            help = concat!(
                "Time to wait between terminating a local process due to timeout and forcefully ",
                "shutting it down.",
            ),
        )]
        pub local_sigkill_grace_seconds: i32,

        #[option(
            name = "allowed_local_actions_regex",
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            converter = RegexPatternConverter,
            default_value = "null",
            help = concat!(
                "A regex whitelist for action types which may be run locally. If unset, ",
                "all actions are allowed to execute locally",
            ),
        )]
        pub allowed_local_action: Option<RegexPatternOption>,

        #[option(
            name = "experimental_local_lockfree_output",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [EXECUTION],
            help = concat!(
                "When true, the local spawn runner doesn't lock the output tree during dynamic ",
                "execution. Instead, spawns are allowed to execute until they are explicitly ",
                "interrupted by a faster remote action.",
            ),
        )]
        pub local_lockfree_output: bool,

        #[option(
            name = "experimental_process_wrapper_graceful_sigterm",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [EXECUTION],
            help = concat!(
                "When true, make the process-wrapper propagate SIGTERMs (used by the dynamic scheduler ",
                "to stop process trees) to the subprocesses themselves, giving them the grace ",
                "period in --local_termination_grace_seconds before forcibly sending a SIGKILL.",
            ),
        )]
        pub process_wrapper_graceful_sigterm: bool,

        #[option(
            name = "experimental_local_retries_on_crash",
            default_value = "0",
            documentation_category = UNDOCUMENTED,
            effect_tags = [EXECUTION],
            help = concat!(
                "Number of times to retry a local action when we detect that it crashed. This exists ",
                "to workaround a bug in OSXFUSE which is tickled by the use of the dynamic ",
                "scheduler and --experimental_local_lockfree_output due to constant process ",
                "churn. The bug can be triggered by a cancelled process that ran *before* the ",
                "process we are trying to run, introducing corruption in its file reads.",
            ),
        )]
        pub local_retries_on_crash: i32,
    }
}

impl LocalExecutionOptions {
    // Java: com/google/devtools/build/lib/exec/local/LocalExecutionOptions.java:86 LocalExecutionOptions.getLocalSigkillGraceSeconds
    pub fn get_local_sigkill_grace_seconds(&self) -> Duration {
        // TODO(ulfjack): Change localSigkillGraceSeconds type to Duration.
        // Java: Duration.ofSeconds(long) accepts negative values; std Duration cannot hold them.
        if self.local_sigkill_grace_seconds < 0 {
            blaze_util::unported!("Duration.ofSeconds (negative duration)");
        }
        Duration::from_secs(self.local_sigkill_grace_seconds as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::misc::test_support::{check_options_class, parse};

    #[test]
    fn options_class_matches_java() {
        check_options_class::<LocalExecutionOptions>(&[
            "local_termination_grace_seconds",
            "allowed_local_actions_regex",
            "experimental_local_lockfree_output",
            "experimental_process_wrapper_graceful_sigterm",
            "experimental_local_retries_on_crash",
        ]);
    }

    #[test]
    fn parses_flags() {
        let o = parse::<LocalExecutionOptions>(&[
            "--local_sigkill_grace_seconds=3",
            "--allowed_local_actions_regex=Genrule.*",
        ])
        .unwrap();
        assert_eq!(o.get_local_sigkill_grace_seconds(), Duration::from_secs(3));
        assert!(o.allowed_local_action.unwrap().matches("GenruleFoo"));
        assert_eq!(
            LocalExecutionOptions::default().get_local_sigkill_grace_seconds(),
            Duration::from_secs(15)
        );
    }
}
