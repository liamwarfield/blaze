//! Port of `bazel/debug/DebuggingOptions.java` (the `OptionsBase` subclass and what it uses).

use blaze_options::Foreign;
use blaze_options::options;
use blaze_vfs::PathFragment;

use super::options_utils::PathFragmentConverter;

// Java: com/google/devtools/build/lib/bazel/debug/DebuggingOptions.java:25 DebuggingOptions
options! {
    /// Options for debugging and verbosity tools.
    pub struct DebuggingOptions {
        #[option(
            name = "experimental_workspace_rules_log_file",
            default_value = "null",
            category = "verbosity",
            documentation_category = LOGGING,
            effect_tags = [UNKNOWN],
            converter = PathFragmentConverter,
            help = "Log certain Workspace Rules events into this file as delimited WorkspaceEvent protos.",
        )]
        pub workspace_rules_log_file: Option<Foreign<PathFragment>>,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::misc::test_support::{check_options_class, parse};

    #[test]
    fn options_class_matches_java() {
        check_options_class::<DebuggingOptions>(&["experimental_workspace_rules_log_file"]);
    }

    #[test]
    fn parses_log_file() {
        let o = parse::<DebuggingOptions>(&["--experimental_workspace_rules_log_file=/tmp/x.log"])
            .unwrap();
        assert_eq!(
            o.workspace_rules_log_file.unwrap().get_path_string(),
            "/tmp/x.log"
        );
        assert!(
            DebuggingOptions::default()
                .workspace_rules_log_file
                .is_none()
        );
    }
}
