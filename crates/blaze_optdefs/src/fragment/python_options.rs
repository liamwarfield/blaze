//! Port of `rules/python/PythonOptions.java`.

use super::converters::{LabelConverter, OptionLabel};
use super::python_version::PythonVersion;
use blaze_options::TriState;
use blaze_options::options;

use super::fragment_options::FragmentOptions;

// Java: com/google/devtools/build/lib/rules/python/PythonOptions.java:33 PythonOptions
options! {
    /// Python-related command-line options.
    ///
    /// Due to the migration of the Python version API (see #6583) and the default Python version (see
    /// (see #6647), the Python major version mode (`PY2` vs `PY3`) is a function of multiple
    /// flags. See [`PythonOptions::get_python_version`] for more details.
    pub struct PythonOptions {
        #[option(
            name = "build_python_zip",
            default_value = "auto",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            deprecation_warning = concat!(
                "The '--no' prefix is no longer supported for this flag. Please use",
                " --build_python_zip=false instead.",
            ),
            help = "Deprecated. No-op.",
        )]
        pub build_python_zip: TriState,

        #[option(
            name = "incompatible_default_to_explicit_init_py",
            default_value = "false",
            documentation_category = GENERIC_INPUTS,
            effect_tags = [AFFECTS_OUTPUTS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "This flag changes the default behavior so that __init__.py files are no longer ",
                "automatically created in the runfiles of Python targets. Precisely, when a ",
                "py_binary or py_test target has legacy_create_init set to \"auto\" (the default), ",
                "it is treated as false if and only if this flag is set. See ",
                "https://github.com/bazelbuild/bazel/issues/10076.",
            ),
        )]
        pub incompatible_default_to_explicit_init_py: bool,

        #[option(
            name = "python_native_rules_allowlist",
            documentation_category = INPUT_STRICTNESS,
            effect_tags = [LOADING_AND_ANALYSIS],
            default_value = "null",
            converter = LabelConverter,
            help = concat!(
                "An allowlist (package_group target) to use when enforcing ",
                "--incompatible_python_disallow_native_rules.",
            ),
        )]
        pub native_rules_allowlist: Option<OptionLabel>,

        #[option(
            name = "incompatible_python_disallow_native_rules",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [LOADING_AND_ANALYSIS],
            default_value = "false",
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "When true, an error occurs when using the builtin py_* rules; instead the rule_python",
                " rules should be used. See https://github.com/bazelbuild/bazel/issues/17773 for",
                " more information and migration instructions.",
            ),
        )]
        pub disallow_native_rules: bool,

        #[option(
            name = "experimental_py_binaries_include_label",
            default_value = "false",
            documentation_category = OUTPUT_PARAMETERS,
            metadata_tags = [EXPERIMENTAL],
            effect_tags = [AFFECTS_OUTPUTS],
            help = "py_binary targets include their label even when stamping is disabled.",
        )]
        pub include_label_in_py_binaries_linkstamp: bool,

        #[option(
            name = "incompatible_remove_ctx_py_fragment",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BUILD_FILE_SEMANTICS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "When true, Python build flags are defined with Python rules (in BUIILD files) and",
                " ctx.fragments.py is undefined. This is a migration flag to move all Python flags ",
                " from core Bazel to Python rules.",
            ),
        )]
        pub disable_py_fragment: bool,
    }
}

impl FragmentOptions for PythonOptions {}

impl PythonOptions {
    // Java: com/google/devtools/build/lib/rules/python/PythonOptions.java:105 PythonOptions.getPythonVersion
    /// Returns the Python major version (`PY3`) that targets should be built for.
    pub fn get_python_version(&self) -> PythonVersion {
        PythonVersion::PY3
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{assert_defaults_round_trip, assert_option_names, parse};
    use super::*;

    #[test]
    fn option_names_match_java() {
        assert_option_names::<PythonOptions>(&[
            "build_python_zip",
            "incompatible_default_to_explicit_init_py",
            "python_native_rules_allowlist",
            "incompatible_python_disallow_native_rules",
            "experimental_py_binaries_include_label",
            "incompatible_remove_ctx_py_fragment",
        ]);
    }

    #[test]
    fn defaults_round_trip() {
        assert_defaults_round_trip::<PythonOptions>(&[]);
    }

    #[test]
    fn spot_check_parses() {
        let o = parse::<PythonOptions>(&["--build_python_zip=yes"]).unwrap();
        assert_eq!(o.build_python_zip, TriState::YES);
        assert_eq!(o.get_python_version(), PythonVersion::PY3);
    }
}
