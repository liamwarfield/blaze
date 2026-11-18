//! Port of `remote/options/RemoteStartupOptions.java` (the `OptionsBase` subclass and what it uses).

use blaze_options::options;

// Java: com/google/devtools/build/lib/remote/options/RemoteStartupOptions.java:26 RemoteStartupOptions
options! {
    /// Additional startup options provided by the `RemoteModule`.
    pub struct RemoteStartupOptions {
        #[option(
            name = "experimental_remote_repo_contents_cache",
            default_value = "false",
            documentation_category = REMOTE,
            effect_tags = [LOADING_AND_ANALYSIS],
            help = concat!(
                "If enabled, the remote cache will be used to store the results of reproducible repository\n",
                "rules. If a repository rule needs to be evaluated and its result is already in the remote\n",
                "cache, the contents of the repository will be kept in an in-memory file system and are\n",
                "only downloaded when needed, either by Bazel itself or an action that runs locally.\n",
            ),
        )]
        pub use_remote_repo_contents_cache: bool,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::misc::test_support::{check_options_class, parse};

    #[test]
    fn options_class_matches_java() {
        check_options_class::<RemoteStartupOptions>(&["experimental_remote_repo_contents_cache"]);
    }

    #[test]
    fn parses_flag() {
        let o =
            parse::<RemoteStartupOptions>(&["--experimental_remote_repo_contents_cache"]).unwrap();
        assert!(o.use_remote_repo_contents_cache);
    }
}
