//! The options classes of Bazel's configuration fragments:
//! `ConfiguredRuleClassProvider.getFragmentRegistry().getOptionsClasses()` for the Bazel 9.3.2
//! module set, from `blaze_build_config`'s port of the rule class provider's configuration parts
//! (`BazelRuleClassProvider.setup` registering every fragment, `FragmentRegistry.create`).

use blaze_build_config::bazel_rule_class_provider::BazelRuleClassProvider;
use blaze_options::definition::OptionsClassRef;

/// The registered `FragmentOptions` classes, ordered by Java binary class name.
// Java: com/google/devtools/build/lib/analysis/config/FragmentRegistry.java:85 FragmentRegistry.getOptionsClasses
pub fn fragment_options_classes() -> Vec<OptionsClassRef> {
    BazelRuleClassProvider::create()
        .get_options_classes()
        .iter()
        .map(|c| c.options_class())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use blaze_optdefs::fragment::android_configuration::AndroidConfigurationOptions;
    use blaze_optdefs::fragment::apple_command_line_options::AppleCommandLineOptions;
    use blaze_optdefs::fragment::bazel_android_configuration::BazelAndroidConfigurationOptions;
    use blaze_optdefs::fragment::bazel_python_configuration::BazelPythonConfigurationOptions;
    use blaze_optdefs::fragment::bazel_rule_class_provider::BazelRuleClassProviderStrictActionEnvOptions;
    use blaze_optdefs::fragment::config_feature_flag_options::ConfigFeatureFlagOptions;
    use blaze_optdefs::fragment::core_options::CoreOptions;
    use blaze_optdefs::fragment::coverage_configuration::CoverageConfigurationCoverageOptions;
    use blaze_optdefs::fragment::cpp_options::CppOptions;
    use blaze_optdefs::fragment::j2objc_command_line_options::J2ObjcCommandLineOptions;
    use blaze_optdefs::fragment::java_options::JavaOptions;
    use blaze_optdefs::fragment::objc_command_line_options::ObjcCommandLineOptions;
    use blaze_optdefs::fragment::platform_options::PlatformOptions;
    use blaze_optdefs::fragment::proto_configuration::ProtoConfigurationOptions;
    use blaze_optdefs::fragment::python_options::PythonOptions;
    use blaze_optdefs::fragment::shell_configuration::ShellConfigurationOptions;
    use blaze_optdefs::fragment::test_configuration::TestConfigurationTestOptions;
    use blaze_options::OptionsClass;

    #[test]
    fn fragment_options_classes_sorted_by_java_class_name() {
        let expected: Vec<OptionsClassRef> = vec![
            PlatformOptions::class(),
            ShellConfigurationOptions::class(),
            CoreOptions::class(),
            CoverageConfigurationCoverageOptions::class(),
            TestConfigurationTestOptions::class(),
            BazelRuleClassProviderStrictActionEnvOptions::class(),
            BazelPythonConfigurationOptions::class(),
            AndroidConfigurationOptions::class(),
            BazelAndroidConfigurationOptions::class(),
            AppleCommandLineOptions::class(),
            ConfigFeatureFlagOptions::class(),
            CppOptions::class(),
            JavaOptions::class(),
            J2ObjcCommandLineOptions::class(),
            ObjcCommandLineOptions::class(),
            ProtoConfigurationOptions::class(),
            PythonOptions::class(),
        ];
        let actual = fragment_options_classes();
        assert_eq!(actual.len(), 17);
        assert_eq!(actual.len(), expected.len());
        for (a, e) in actual.iter().zip(expected.iter()) {
            assert!(std::ptr::eq(*a, *e));
        }
    }

    #[test]
    fn fragment_options_build_an_options_parser() {
        blaze_options::OptionsParser::builder()
            .options_classes(&fragment_options_classes())
            .build()
            .unwrap_or_else(|e| panic!("{}", e.get_message()));
    }
}
