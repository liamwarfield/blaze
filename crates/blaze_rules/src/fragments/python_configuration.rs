//! The Starlark values of the `py` fragment (`rules/python/PythonConfiguration.java`) and the
//! `bazel_py` fragment (`bazel/rules/python/BazelPythonConfiguration.java`).

use blaze_build_config::fragments::bazel_python_configuration::{
    BAZEL_PYTHON_CONFIGURATION, BazelPythonConfiguration,
};
use blaze_build_config::fragments::python_configuration::{
    PYTHON_CONFIGURATION, PythonConfiguration,
};
use starlark::environment::MethodsBuilder;
use starlark::starlark_module;

use super::fragment_value;

fragment_value!(
    /// `ctx.fragments.py`: a configuration fragment for Python.
    // Java: com/google/devtools/build/lib/rules/python/PythonConfiguration.java:43 PythonConfiguration
    PythonConfigurationValue,
    PythonConfiguration,
    "py",
    "com.google.devtools.build.lib.rules.python.PythonConfiguration",
    python_configuration_methods
);

fragment_value!(
    /// `ctx.fragments.bazel_py`.
    // Java: com/google/devtools/build/lib/bazel/rules/python/BazelPythonConfiguration.java:41 BazelPythonConfiguration
    BazelPythonConfigurationValue,
    BazelPythonConfiguration,
    "bazel_py",
    "com.google.devtools.build.lib.bazel.rules.python.BazelPythonConfiguration",
    bazel_python_configuration_methods
);

pub(super) fn register() {
    super::register(&PYTHON_CONFIGURATION, PythonConfigurationValue::alloc);
    super::register(
        &BAZEL_PYTHON_CONFIGURATION,
        BazelPythonConfigurationValue::alloc,
    );
}

#[starlark_module]
fn python_configuration_methods(builder: &mut MethodsBuilder) {
    /// No-op: PY3 is the default Python version.
    // Java: com/google/devtools/build/lib/rules/python/PythonConfiguration.java:98 PythonConfiguration.getDefaultPythonVersionForStarlark
    #[starlark(attribute)]
    fn default_python_version(this: &PythonConfigurationValue) -> starlark::Result<&'static str> {
        Ok(this.fragment().get_default_python_version_for_starlark())
    }

    /// The effective value of --build_python_zip
    // Java: com/google/devtools/build/lib/rules/python/PythonConfiguration.java:107 PythonConfiguration.buildPythonZip
    #[starlark(attribute)]
    fn build_python_zip(this: &PythonConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().build_python_zip())
    }

    /// No-op: Python toolchains are always used.
    // Java: com/google/devtools/build/lib/rules/python/PythonConfiguration.java:123 PythonConfiguration.useToolchains
    #[starlark(attribute)]
    fn use_toolchains(this: &PythonConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().use_toolchains())
    }

    /// The value from the --incompatible_default_to_explicit_init_py flag
    // Java: com/google/devtools/build/lib/rules/python/PythonConfiguration.java:135 PythonConfiguration.defaultToExplicitInitPy
    #[starlark(attribute)]
    fn default_to_explicit_init_py(this: &PythonConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().default_to_explicit_init_py())
    }

    /// The value of the --incompatible_python_disallow_native_rules flag.
    // Java: com/google/devtools/build/lib/rules/python/PythonConfiguration.java:143 PythonConfiguration.getDisallowNativeRules
    #[starlark(attribute)]
    fn disallow_native_rules(this: &PythonConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().get_disallow_native_rules())
    }

    /// Whether the build label is included in unstamped builds.
    // Java: com/google/devtools/build/lib/rules/python/PythonConfiguration.java:160 PythonConfiguration.isIncludeLabelInLinkstamp
    #[starlark(attribute)]
    fn include_label_in_linkstamp(this: &PythonConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().is_include_label_in_linkstamp())
    }
}

#[starlark_module]
fn bazel_python_configuration_methods(builder: &mut MethodsBuilder) {
    /// The value of the --python_path flag.
    // Java: com/google/devtools/build/lib/bazel/rules/python/BazelPythonConfiguration.java:135 BazelPythonConfiguration.getPythonPath
    #[starlark(attribute)]
    fn python_path(this: &BazelPythonConfigurationValue) -> starlark::Result<String> {
        Ok(this.fragment().get_python_path().to_owned())
    }

    /// The value of the --experimental_python_import_all_repositories flag.
    // Java: com/google/devtools/build/lib/bazel/rules/python/BazelPythonConfiguration.java:143 BazelPythonConfiguration.getImportAllRepositories
    #[starlark(attribute)]
    fn python_import_all_repositories(
        this: &BazelPythonConfigurationValue,
    ) -> starlark::Result<bool> {
        Ok(this.fragment().get_import_all_repositories())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use blaze_build_config::build_options::BuildOptions;
    use blaze_build_config::fragment::Fragment;
    use blaze_build_config::fragment_options_class::all_known_fragment_options_classes;

    use super::*;

    fn options(args: &[&str]) -> BuildOptions {
        BuildOptions::of_args(&all_known_fragment_options_classes(), args)
            .unwrap_or_else(|e| panic!("{}", e.get_message()))
    }

    #[test]
    fn py_value_reads_the_fragment() {
        let fragment: Arc<dyn Fragment> = Arc::new(PythonConfiguration::new(&options(&[
            "--build_python_zip=true",
        ])));
        let value = PythonConfigurationValue(fragment);
        assert!(value.fragment().build_python_zip());
        assert_eq!(
            value.to_string(),
            "<unknown object com.google.devtools.build.lib.rules.python.PythonConfiguration>"
        );
    }

    #[test]
    fn bazel_py_value_reads_the_fragment() {
        let fragment: Arc<dyn Fragment> = Arc::new(
            BazelPythonConfiguration::new(&options(&["--python_path=/usr/bin/python3"])).unwrap(),
        );
        let value = BazelPythonConfigurationValue(fragment);
        assert_eq!(value.fragment().get_python_path(), "/usr/bin/python3");
    }

    // Java: PythonConfigurationTest.getExec_copiesMostValues
    #[test]
    #[ignore = "needs analysis (AnalysisTestUtil.execOptions)"]
    fn get_exec_copies_most_values() {}
}
