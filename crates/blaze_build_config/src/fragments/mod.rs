//! The configuration fragments of Bazel 9.3.2's rule class provider: one module per Java
//! `Fragment` subclass, and their registration ([`bazel_fragments`]).

pub mod android_configuration;
pub mod apple_configuration;
pub mod bazel_android_configuration;
pub mod bazel_python_configuration;
pub mod bazel_rule_class_provider;
pub mod config_feature_flag_configuration;
pub mod coverage_configuration;
pub mod cpp_configuration;
pub mod j2objc_configuration;
pub mod java_configuration;
pub mod objc_configuration;
pub mod platform_configuration;
pub mod proto_configuration;
pub mod python_configuration;
pub mod shell_configuration;
pub mod test_configuration;

use crate::fragment::FragmentClass;

use android_configuration::ANDROID_CONFIGURATION;
use apple_configuration::APPLE_CONFIGURATION;
use bazel_android_configuration::BAZEL_ANDROID_CONFIGURATION;
use bazel_python_configuration::BAZEL_PYTHON_CONFIGURATION;
use bazel_rule_class_provider::STRICT_ACTION_ENV_CONFIGURATION;
use config_feature_flag_configuration::CONFIG_FEATURE_FLAG_CONFIGURATION;
use coverage_configuration::COVERAGE_CONFIGURATION;
use cpp_configuration::CPP_CONFIGURATION;
use j2objc_configuration::J2OBJC_CONFIGURATION;
use java_configuration::JAVA_CONFIGURATION;
use objc_configuration::OBJC_CONFIGURATION;
use platform_configuration::PLATFORM_CONFIGURATION;
use proto_configuration::PROTO_CONFIGURATION;
use python_configuration::PYTHON_CONFIGURATION;
use shell_configuration::SHELL_CONFIGURATION;
use test_configuration::TEST_CONFIGURATION;

// Java: com/google/devtools/build/lib/bazel/rules/BazelRuleClassProvider.java:208 BazelRuleClassProvider.setup
/// The fragments `BazelRuleClassProvider.setup` registers, as
/// `(configurationFragmentClasses, universalFragments)` of `ConfiguredRuleClassProvider.Builder`
/// (both in registration order; `FragmentRegistry.create` dedups and sorts them).
///
/// `RULE_SETS` run in order: `BAZEL_SETUP` (`addUniversalConfigurationFragment`, which also
/// calls `addConfigurationFragment`, for `ShellConfiguration`, `PlatformConfiguration` and
/// `StrictActionEnvConfiguration`), `CoreRules`, `GenericRules` (none), `ConfigRules`,
/// `PlatformRules` (`PlatformConfiguration` again), `PROTO_RULES`, `CcRules`, `JavaRules`,
/// `ANDROID_RULES`, `PYTHON_RULES`, `ObjcRules`, `TestingSupportRules`, `PACKAGING_RULES` and
/// `ToolchainRules` (none).
pub fn bazel_fragments() -> (Vec<FragmentClass>, Vec<FragmentClass>) {
    let mut all: Vec<FragmentClass> = Vec::new();
    let mut universal: Vec<FragmentClass> = Vec::new();

    // BAZEL_SETUP
    for f in [
        &SHELL_CONFIGURATION,
        &PLATFORM_CONFIGURATION,
        &STRICT_ACTION_ENV_CONFIGURATION,
    ] {
        universal.push(f);
        all.push(f);
    }
    // CoreRules
    all.push(&TEST_CONFIGURATION);
    all.push(&COVERAGE_CONFIGURATION);
    // ConfigRules
    all.push(&CONFIG_FEATURE_FLAG_CONFIGURATION);
    // PlatformRules
    all.push(&PLATFORM_CONFIGURATION);
    // PROTO_RULES
    all.push(&PROTO_CONFIGURATION);
    // CcRules
    all.push(&CPP_CONFIGURATION);
    // JavaRules
    all.push(&JAVA_CONFIGURATION);
    // ANDROID_RULES
    all.push(&ANDROID_CONFIGURATION);
    all.push(&BAZEL_ANDROID_CONFIGURATION);
    // PYTHON_RULES
    all.push(&PYTHON_CONFIGURATION);
    all.push(&BAZEL_PYTHON_CONFIGURATION);
    // ObjcRules
    all.push(&OBJC_CONFIGURATION);
    all.push(&APPLE_CONFIGURATION);
    // j2objc shouldn't be here!
    all.push(&J2OBJC_CONFIGURATION);

    (all, universal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_options::BuildOptions;
    use crate::fragment::{FragmentFactory, FragmentRegistry};
    use crate::fragment_options_class::{CORE_OPTIONS, all_known_fragment_options_classes};

    #[test]
    fn registry_options_match_known_options_classes() {
        let (all, universal) = bazel_fragments();
        let registry = FragmentRegistry::create(all, universal, vec![&CORE_OPTIONS]);
        assert_eq!(registry.get_all_fragments().len(), 16);
        assert_eq!(registry.get_universal_fragments().len(), 3);
        let names: Vec<&str> = registry
            .get_options_classes()
            .iter()
            .map(|c| c.get_name())
            .collect();
        let mut expected: Vec<&str> = all_known_fragment_options_classes()
            .iter()
            .map(|c| c.get_name())
            .collect();
        expected.sort();
        assert_eq!(names, expected);
    }

    #[test]
    fn every_fragment_builds_from_default_options() {
        let options = BuildOptions::get_default_build_options_for_fragments(
            &all_known_fragment_options_classes(),
        );
        let (all, _) = bazel_fragments();
        let factory = FragmentFactory;
        for fragment_class in all {
            let fragment = factory
                .create_fragment(&options, fragment_class)
                .unwrap_or_else(|e| panic!("{:?}: {}", fragment_class, e.get_message()));
            if let Some(fragment) = fragment {
                assert!(std::ptr::eq(fragment.fragment_class(), fragment_class));
            }
        }
    }
}
