//! Port of `bazel/rules/python/BazelPythonConfiguration.java` (the fragment; its options are
//! `blaze_optdefs::fragment::bazel_python_configuration`).

use std::any::Any;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_optdefs::fragment::bazel_python_configuration::BazelPythonConfigurationOptions;
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_vfs::PathFragment;

use crate::build_options::BuildOptions;
use crate::fragment::{
    Fragment, FragmentClass, FragmentClassInfo, InvalidConfigurationException, required,
};
use crate::fragment_options_class::{BAZEL_PYTHON_OPTIONS, PYTHON_OPTIONS};

// Java: com/google/devtools/build/lib/bazel/rules/python/BazelPythonConfiguration.java:41 BazelPythonConfiguration
/// Bazel-specific Python configuration.
#[derive(Debug)]
pub struct BazelPythonConfiguration {
    options: BazelPythonConfigurationOptions,
    disable_py_fragment: bool,
}

/// `BazelPythonConfiguration.class`.
pub static BAZEL_PYTHON_CONFIGURATION: FragmentClassInfo = FragmentClassInfo {
    java_name: "com.google.devtools.build.lib.bazel.rules.python.BazelPythonConfiguration",
    starlark_name: Some("bazel_py"),
    required_options: || vec![&BAZEL_PYTHON_OPTIONS, &PYTHON_OPTIONS],
    requires_starlark_options: false,
    constructor: create,
};

fn create(options: &BuildOptions) -> Result<Arc<dyn Fragment>, InvalidConfigurationException> {
    let fragment: Arc<dyn Fragment> = Arc::new(BazelPythonConfiguration::new(options)?);
    Ok(fragment)
}

impl BazelPythonConfiguration {
    // Java: com/google/devtools/build/lib/bazel/rules/python/BazelPythonConfiguration.java:90 BazelPythonConfiguration.<init>
    pub fn new(
        build_options: &BuildOptions,
    ) -> Result<BazelPythonConfiguration, InvalidConfigurationException> {
        let options = required::<BazelPythonConfigurationOptions>(build_options).clone();
        let python_path = Self::python_path_of(&options).to_string();

        // Only set disablePyFragment, which removes ctx.fragments.bazel_py, if all
        // BazelPythonConfiguration flags are flag aliased. We specially check here to see if any
        // flags lack Starlark flag aliases.
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
        let need_native_fragment = !contains_key("python_path")
            || !contains_key("experimental_python_import_all_repositories");
        let disable_py_fragment = options.disable_py_fragment && !need_native_fragment;
        if !python_path.starts_with("python") && !PathFragment::create(&python_path).is_absolute() {
            return Err(InvalidConfigurationException::new(
                "python_path must be an absolute path when it is set.",
            ));
        }
        Ok(BazelPythonConfiguration {
            options,
            disable_py_fragment,
        })
    }

    fn python_path_of(options: &BazelPythonConfigurationOptions) -> &str {
        options.python_path.as_deref().unwrap_or("python")
    }

    // Java: com/google/devtools/build/lib/bazel/rules/python/BazelPythonConfiguration.java:127 BazelPythonConfiguration.getPythonTop
    /// Deprecated. Always returns None. Use toolchain resolution instead.
    pub fn get_python_top(&self) -> Option<&Label> {
        None
    }

    // Java: com/google/devtools/build/lib/bazel/rules/python/BazelPythonConfiguration.java:92 BazelPythonConfiguration.getPythonPath
    /// The value of the --python_path flag.
    pub fn get_python_path(&self) -> &str {
        Self::python_path_of(&self.options)
    }

    // Java: com/google/devtools/build/lib/bazel/rules/python/BazelPythonConfiguration.java:143 BazelPythonConfiguration.getImportAllRepositories
    /// The value of the --experimental_python_import_all_repositories flag.
    pub fn get_import_all_repositories(&self) -> bool {
        self.options.experimental_python_import_all_repositories
    }
}

impl Fragment for BazelPythonConfiguration {
    fn fragment_class(&self) -> FragmentClass {
        &BAZEL_PYTHON_CONFIGURATION
    }

    // Java: com/google/devtools/build/lib/bazel/rules/python/BazelPythonConfiguration.java:120 BazelPythonConfiguration.shouldInclude
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

    // PORT: ConfigurationTestCase.create builds the whole configuration; these tests construct
    // the fragment directly from the parsed options.
    fn create(args: &[&str]) -> Result<BazelPythonConfiguration, InvalidConfigurationException> {
        let options = BuildOptions::of_args(&all_known_fragment_options_classes(), args)
            .unwrap_or_else(|e| panic!("{}", e.get_message()));
        BazelPythonConfiguration::new(&options)
    }

    #[test]
    fn python_path() {
        let config = create(&["--python_path=/pajama"]).unwrap();
        assert_eq!(config.get_python_path(), "/pajama");
    }

    #[test]
    fn python_path_must_be_absolute() {
        let expected = create(&["--python_path=pajama"]).unwrap_err();
        assert!(
            expected
                .get_message()
                .contains("python_path must be an absolute path")
        );
    }
}
