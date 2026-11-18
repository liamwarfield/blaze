//! The options of `bazel/rules/python/BazelPythonConfiguration.java` (`BazelPythonConfiguration.Options`). The fragment itself is not ported here.

use blaze_options::options;

use super::fragment_options::FragmentOptions;

// Java: com/google/devtools/build/lib/bazel/rules/python/BazelPythonConfiguration.java:44 BazelPythonConfiguration.Options
options! {
    /// Bazel-specific Python configuration options.
    pub struct BazelPythonConfigurationOptions {
        #[option(
            name = "python_path",
            default_value = "null",
            documentation_category = TOOLCHAIN,
            effect_tags = [LOADING_AND_ANALYSIS, AFFECTS_OUTPUTS],
            help = concat!(
                "The absolute path of the Python interpreter invoked to run Python targets on the\n",
                "target platform. Deprecated; disabled by `--incompatible_use_python_toolchains`.\n",
            ),
        )]
        pub python_path: Option<String>,

        #[option(
            name = "experimental_python_import_all_repositories",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "If true, the roots of repositories in the runfiles tree are added to `PYTHONPATH`, so\n",
                "that imports like `import mytoplevelpackage.package.module` are valid.\n",
                "Regardless of whether this flag is true, the runfiles root itself is also\n",
                "added to the `PYTHONPATH`, so\n",
                "`import myreponame.mytoplevelpackage.package.module` is valid. The latter form\n",
                "is less likely to experience import name collisions.\n",
            ),
        )]
        pub experimental_python_import_all_repositories: bool,

        #[option(
            name = "incompatible_remove_ctx_bazel_py_fragment",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BUILD_FILE_SEMANTICS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "When true, Python build flags are defined with Python rules (in BUIILD files) and the",
                " ctx.fragments.bazel_py attribute is not present. This is a migration flag to",
                " move all Python flags from core Bazel to Python rules.",
            ),
        )]
        pub disable_py_fragment: bool,
    }
}

impl FragmentOptions for BazelPythonConfigurationOptions {}

#[cfg(test)]
mod tests {
    use super::super::test_util::{assert_defaults_round_trip, assert_option_names, parse};
    use super::*;

    #[test]
    fn option_names_match_java() {
        assert_option_names::<BazelPythonConfigurationOptions>(&[
            "python_path",
            "experimental_python_import_all_repositories",
            "incompatible_remove_ctx_bazel_py_fragment",
        ]);
    }

    #[test]
    fn defaults_round_trip() {
        assert_defaults_round_trip::<BazelPythonConfigurationOptions>(&[]);
    }

    #[test]
    fn spot_check_parses() {
        let o =
            parse::<BazelPythonConfigurationOptions>(&["--python_path=/usr/bin/python3"]).unwrap();
        assert_eq!(o.python_path.as_deref(), Some("/usr/bin/python3"));
        assert!(o.disable_py_fragment);
    }
}
