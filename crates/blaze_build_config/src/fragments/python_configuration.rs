//! Port of `rules/python/PythonConfiguration.java`.

use std::any::Any;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_optdefs::fragment::python_options::PythonOptions;
use blaze_options::TriState;

use crate::build_options::BuildOptions;
use crate::fragment::{
    Fragment, FragmentClass, FragmentClassInfo, InvalidConfigurationException, required,
};
use crate::fragment_options_class::PYTHON_OPTIONS;

// Java: com/google/devtools/build/lib/rules/python/PythonConfiguration.java:43 PythonConfiguration
/// The configuration fragment containing information about the various pieces of infrastructure
/// for the Python rules.
#[derive(Debug)]
pub struct PythonConfiguration {
    build_python_zip: TriState,
    /// Whether to include the build label in unstamped builds.
    include_label_in_linkstamp: bool,
    default_to_explicit_init_py: bool,
    native_rules_allowlist: Option<Label>,
    disallow_native_rules: bool,
    disable_py_fragment: bool,
}

/// `PythonConfiguration.class`.
pub static PYTHON_CONFIGURATION: FragmentClassInfo = FragmentClassInfo {
    java_name: "com.google.devtools.build.lib.rules.python.PythonConfiguration",
    starlark_name: Some("py"),
    required_options: || vec![&PYTHON_OPTIONS],
    requires_starlark_options: false,
    constructor: create,
};

fn create(options: &BuildOptions) -> Result<Arc<dyn Fragment>, InvalidConfigurationException> {
    let fragment: Arc<dyn Fragment> = Arc::new(PythonConfiguration::new(options));
    Ok(fragment)
}

impl PythonConfiguration {
    // Java: com/google/devtools/build/lib/rules/python/PythonConfiguration.java:55 PythonConfiguration.<init>
    pub fn new(build_options: &BuildOptions) -> PythonConfiguration {
        let python_options = required::<PythonOptions>(build_options);

        // Only set disablePyFragment, which removes ctx.fragments.py, if all PythonOptions flags
        // are flag aliased. We specially check here to see if any flags lack Starlark flag
        // aliases.
        //
        // This has the clever effect that ctx.fragments.bazel_py can not be disabled for old
        // rules_python versions that don't support Starlark flags. That's because
        // SkyframeExecutor.getFlagAliases() doesn't add aliases on those versions and they're too
        // old to define MODULE.bazel aliases. So needNativeFragment below will be true.
        //
        // TODO: b/453809359 - Remove this extra check Bazel 9+ can read Python flag alias
        // definitions straight from rules_python's MODULE.bazel.
        let flag_aliases = required::<CoreOptions>(build_options).get_command_line_flag_aliases();
        let contains_key = |key: &str| flag_aliases.iter().any(|(k, _)| k == key);
        let need_native_fragment = !contains_key("build_python_zip")
            || !contains_key("incompatible_default_to_explicit_init_py");

        PythonConfiguration {
            build_python_zip: python_options.build_python_zip,
            default_to_explicit_init_py: python_options.incompatible_default_to_explicit_init_py,
            native_rules_allowlist: python_options
                .native_rules_allowlist
                .as_ref()
                .map(|l| l.get().clone()),
            disallow_native_rules: python_options.disallow_native_rules,
            include_label_in_linkstamp: python_options.include_label_in_py_binaries_linkstamp,
            disable_py_fragment: python_options.disable_py_fragment && !need_native_fragment,
        }
    }

    // Java: com/google/devtools/build/lib/rules/python/PythonConfiguration.java:98 PythonConfiguration.getDefaultPythonVersionForStarlark
    /// No-op: PY3 is the default Python version.
    pub fn get_default_python_version_for_starlark(&self) -> &'static str {
        "PY3"
    }

    // Java: com/google/devtools/build/lib/rules/python/PythonConfiguration.java:107 PythonConfiguration.buildPythonZip
    /// Returns whether to build the executable zip file for Python binaries.
    pub fn build_python_zip(&self) -> bool {
        match self.build_python_zip {
            TriState::YES => true,
            TriState::NO => false,
            // PORT: OS.getCurrent() == OS.WINDOWS; Blaze runs on Linux only.
            _ => cfg!(windows),
        }
    }

    // Java: com/google/devtools/build/lib/rules/python/PythonConfiguration.java:123 PythonConfiguration.useToolchains
    /// Returns true if executable Python rules should obtain their runtime from the Python
    /// toolchain rather than legacy flags.
    pub fn use_toolchains(&self) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/rules/python/PythonConfiguration.java:135 PythonConfiguration.defaultToExplicitInitPy
    /// Returns true if executable Python rules should only write out empty `__init__` files to
    /// their runfiles tree when explicitly requested via `legacy_create_init`.
    pub fn default_to_explicit_init_py(&self) -> bool {
        self.default_to_explicit_init_py
    }

    // Java: com/google/devtools/build/lib/rules/python/PythonConfiguration.java:143 PythonConfiguration.getDisallowNativeRules
    pub fn get_disallow_native_rules(&self) -> bool {
        self.disallow_native_rules
    }

    // Java: com/google/devtools/build/lib/rules/python/PythonConfiguration.java:151 PythonConfiguration.getNativeRulesAllowlist
    pub fn get_native_rules_allowlist(&self) -> Option<&Label> {
        self.native_rules_allowlist.as_ref()
    }

    // Java: com/google/devtools/build/lib/rules/python/PythonConfiguration.java:160 PythonConfiguration.isIncludeLabelInLinkstamp
    /// Returns whether the build label is included in unstamped builds.
    pub fn is_include_label_in_linkstamp(&self) -> bool {
        self.include_label_in_linkstamp
    }
}

impl Fragment for PythonConfiguration {
    fn fragment_class(&self) -> FragmentClass {
        &PYTHON_CONFIGURATION
    }

    // Java: com/google/devtools/build/lib/rules/python/PythonConfiguration.java:85 PythonConfiguration.shouldInclude
    fn should_include(&self) -> bool {
        !self.disable_py_fragment
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fragment_options_class::all_known_fragment_options_classes;

    fn create(args: &[&str]) -> PythonConfiguration {
        let options = BuildOptions::of_args(&all_known_fragment_options_classes(), args)
            .unwrap_or_else(|e| panic!("{}", e.get_message()));
        PythonConfiguration::new(&options)
    }

    #[test]
    #[ignore = "needs analysis (AnalysisTestUtil.execOptions)"]
    fn get_exec_copies_most_values() {}

    #[test]
    fn build_python_zip_follows_flag() {
        assert!(create(&["--build_python_zip=true"]).build_python_zip());
        assert!(!create(&["--build_python_zip=false"]).build_python_zip());
        assert!(!create(&[]).build_python_zip());
    }

    #[test]
    fn fragment_kept_without_flag_aliases() {
        // Without Starlark flag aliases for every Python flag the native fragment stays.
        let config = create(&["--incompatible_remove_ctx_py_fragment=true"]);
        assert!(config.should_include());
    }
}
