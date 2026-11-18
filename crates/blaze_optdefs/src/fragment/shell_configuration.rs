//! The options of `analysis/ShellConfiguration.java` (`ShellConfiguration.Options`). The fragment itself is not ported here.

use crate::misc::options_utils::PathFragmentConverter;
use blaze_options::Foreign;
use blaze_options::options;
use blaze_vfs::PathFragment;

use super::fragment_options::FragmentOptions;

// Java: com/google/devtools/build/lib/analysis/ShellConfiguration.java:71 ShellConfiguration.Options
options! {
    /// An option that tells Bazel where the shell is.
    pub struct ShellConfigurationOptions {
        #[option(
            name = "shell_executable",
            converter = PathFragmentConverter,
            default_value = "null",
            documentation_category = UNCATEGORIZED,
            effect_tags = [LOADING_AND_ANALYSIS],
            help = concat!(
                "Absolute path to the shell executable for Bazel to use. If this is unset, but the\n",
                "`BAZEL_SH` environment variable is set on the first Bazel invocation (that starts\n",
                "up a Bazel server), Bazel uses that. If neither is set, Bazel uses a hard-coded\n",
                "default path depending on the operating system it runs on;\n",
                "- Windows: `c:/msys64/usr/bin/bash.exe`\n",
                "- FreeBSD: `/usr/local/bin/bash`\n",
                "- All others: `/bin/bash`.\n",
                "\n",
                "Note that using a shell that is not compatible with `bash` may lead\n",
                "to build failures or runtime failures of the generated binaries.\n",
            ),
        )]
        pub shell_executable: Option<Foreign<PathFragment>>,
    }
}

impl FragmentOptions for ShellConfigurationOptions {}

#[cfg(test)]
mod tests {
    use super::super::test_util::{assert_defaults_round_trip, assert_option_names, parse};
    use super::*;

    #[test]
    fn option_names_match_java() {
        assert_option_names::<ShellConfigurationOptions>(&["shell_executable"]);
    }

    #[test]
    fn defaults_round_trip() {
        assert_defaults_round_trip::<ShellConfigurationOptions>(&[]);
    }

    #[test]
    fn spot_check_parses() {
        let o = parse::<ShellConfigurationOptions>(&["--shell_executable=/bin/zsh"]).unwrap();
        assert_eq!(o.shell_executable.unwrap().get_path_string(), "/bin/zsh");
        assert!(
            parse::<ShellConfigurationOptions>(&[])
                .unwrap()
                .shell_executable
                .is_none()
        );
    }
}
