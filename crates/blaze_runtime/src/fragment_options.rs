//! The options classes of Bazel's configuration fragments:
//! `ConfiguredRuleClassProvider.getFragmentRegistry().getOptionsClasses()` for the Bazel 9.3.2
//! module set.
//!
//! PORT: move to blaze_config/blaze_rules once the rule class provider is ported.
//!
//! Derivation:
//! - `BlazeRuntime.Builder.build` calls `initializeRuleClasses` on every module. In Bazel only
//!   `BazelRulesModule` (`BazelRuleClassProvider.setup`) and `BazelRepositoryModule` (a Starlark
//!   bootstrap only, no fragments) implement it.
//! - `BazelRuleClassProvider.setup` runs `RULE_SETS` in order. The rule sets that register
//!   fragments or options:
//!   - `BAZEL_SETUP`: universal fragments `ShellConfiguration`, `PlatformConfiguration`,
//!     `StrictActionEnvConfiguration`; `addConfigurationOptions(CoreOptions.class)`.
//!   - `CoreRules`: `TestConfiguration`, `CoverageConfiguration`.
//!   - `ConfigRules`: `ConfigFeatureFlagConfiguration`.
//!   - `PlatformRules`: `PlatformConfiguration`.
//!   - `PROTO_RULES`: `ProtoConfiguration`.
//!   - `CcRules`: `CppConfiguration`.
//!   - `JavaRules`: `JavaConfiguration`.
//!   - `ANDROID_RULES`: `AndroidConfiguration`, `BazelAndroidConfiguration`.
//!   - `PYTHON_RULES`: `PythonConfiguration`, `BazelPythonConfiguration`.
//!   - `ObjcRules`: `ObjcConfiguration`, `AppleConfiguration`, `J2ObjcConfiguration`.
//!     (`GenericRules`, `TestingSupportRules`, `PACKAGING_RULES` and `ToolchainRules` register no
//!     fragments.)
//! - `ConfiguredRuleClassProvider.Builder.build` calls `FragmentRegistry.create(fragments,
//!   universalFragments, configurationOptions)`, which collects `Fragment.requiredOptions` (the
//!   `@RequiresOptions(options = ...)` of each fragment) plus the additional options into an
//!   `ImmutableSortedSet` ordered by `BuildOptions.LEXICAL_FRAGMENT_OPTIONS_COMPARATOR`
//!   (`Class.getName`, the binary class name).

use blaze_options::OptionsClass;
use blaze_options::definition::OptionsClassRef;

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

/// A fragment registered with the rule class provider: its Java binary class name is not needed
/// (fragment order does not reach the options order), only its `@RequiresOptions` classes, each
/// with its Java binary class name (the sort key).
struct RegisteredFragment {
    required_options: fn() -> Vec<(&'static str, OptionsClassRef)>,
}

/// Fragments in registration order (`configurationFragmentClasses`), with their
/// `@RequiresOptions`.
// Java: com/google/devtools/build/lib/bazel/rules/BazelRuleClassProvider.java:327 BazelRuleClassProvider.RULE_SETS
fn registered_fragments() -> Vec<RegisteredFragment> {
    vec![
        // BAZEL_SETUP
        // Java: com/google/devtools/build/lib/analysis/ShellConfiguration.java:33 ShellConfiguration
        RegisteredFragment {
            required_options: || {
                vec![(
                    "com.google.devtools.build.lib.analysis.ShellConfiguration$Options",
                    ShellConfigurationOptions::class(),
                )]
            },
        },
        // Java: com/google/devtools/build/lib/analysis/PlatformConfiguration.java:33 PlatformConfiguration
        RegisteredFragment {
            required_options: platform_configuration_options,
        },
        // Java: com/google/devtools/build/lib/bazel/rules/BazelRuleClassProvider.java:110 BazelRuleClassProvider.StrictActionEnvConfiguration
        RegisteredFragment {
            required_options: || {
                vec![(
                    "com.google.devtools.build.lib.bazel.rules.BazelRuleClassProvider$StrictActionEnvOptions",
                    BazelRuleClassProviderStrictActionEnvOptions::class(),
                )]
            },
        },
        // CoreRules
        // Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:56 TestConfiguration
        RegisteredFragment {
            required_options: || {
                vec![(
                    "com.google.devtools.build.lib.analysis.test.TestConfiguration$TestOptions",
                    TestConfigurationTestOptions::class(),
                )]
            },
        },
        // Java: com/google/devtools/build/lib/analysis/test/CoverageConfiguration.java:35 CoverageConfiguration
        RegisteredFragment {
            required_options: || {
                vec![
                    (CORE_OPTIONS_NAME, CoreOptions::class()),
                    (
                        "com.google.devtools.build.lib.analysis.test.CoverageConfiguration$CoverageOptions",
                        CoverageConfigurationCoverageOptions::class(),
                    ),
                ]
            },
        },
        // ConfigRules
        // Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlagConfiguration.java:27 ConfigFeatureFlagConfiguration
        RegisteredFragment {
            required_options: || {
                vec![(
                    "com.google.devtools.build.lib.rules.config.ConfigFeatureFlagOptions",
                    ConfigFeatureFlagOptions::class(),
                )]
            },
        },
        // PlatformRules
        RegisteredFragment {
            required_options: platform_configuration_options,
        },
        // PROTO_RULES
        // Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:45 ProtoConfiguration
        RegisteredFragment {
            required_options: || {
                vec![(
                    "com.google.devtools.build.lib.rules.proto.ProtoConfiguration$Options",
                    ProtoConfigurationOptions::class(),
                )]
            },
        },
        // CcRules
        // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:51 CppConfiguration
        RegisteredFragment {
            required_options: || {
                vec![(
                    "com.google.devtools.build.lib.rules.cpp.CppOptions",
                    CppOptions::class(),
                )]
            },
        },
        // JavaRules
        // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:46 JavaConfiguration
        RegisteredFragment {
            required_options: || {
                vec![
                    (
                        "com.google.devtools.build.lib.rules.java.JavaOptions",
                        JavaOptions::class(),
                    ),
                    (PLATFORM_OPTIONS_NAME, PlatformOptions::class()),
                ]
            },
        },
        // ANDROID_RULES
        // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:46 AndroidConfiguration
        RegisteredFragment {
            required_options: || {
                vec![(
                    "com.google.devtools.build.lib.rules.android.AndroidConfiguration$Options",
                    AndroidConfigurationOptions::class(),
                )]
            },
        },
        // Java: com/google/devtools/build/lib/rules/android/BazelAndroidConfiguration.java:35 BazelAndroidConfiguration
        RegisteredFragment {
            required_options: || {
                vec![(
                    "com.google.devtools.build.lib.rules.android.BazelAndroidConfiguration$Options",
                    BazelAndroidConfigurationOptions::class(),
                )]
            },
        },
        // PYTHON_RULES
        // Java: com/google/devtools/build/lib/rules/python/PythonConfiguration.java:43 PythonConfiguration
        RegisteredFragment {
            required_options: || vec![(PYTHON_OPTIONS_NAME, PythonOptions::class())],
        },
        // Java: com/google/devtools/build/lib/bazel/rules/python/BazelPythonConfiguration.java:41 BazelPythonConfiguration
        RegisteredFragment {
            required_options: || {
                vec![
                    (
                        "com.google.devtools.build.lib.bazel.rules.python.BazelPythonConfiguration$Options",
                        BazelPythonConfigurationOptions::class(),
                    ),
                    (PYTHON_OPTIONS_NAME, PythonOptions::class()),
                ]
            },
        },
        // ObjcRules
        // Java: com/google/devtools/build/lib/rules/objc/ObjcConfiguration.java:39 ObjcConfiguration
        RegisteredFragment {
            required_options: || {
                vec![(
                    "com.google.devtools.build.lib.rules.objc.ObjcCommandLineOptions",
                    ObjcCommandLineOptions::class(),
                )]
            },
        },
        // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:48 AppleConfiguration
        RegisteredFragment {
            required_options: || {
                vec![(
                    "com.google.devtools.build.lib.rules.apple.AppleCommandLineOptions",
                    AppleCommandLineOptions::class(),
                )]
            },
        },
        // Java: com/google/devtools/build/lib/rules/objc/J2ObjcConfiguration.java:38 J2ObjcConfiguration
        RegisteredFragment {
            required_options: || {
                vec![(
                    "com.google.devtools.build.lib.rules.objc.J2ObjcCommandLineOptions",
                    J2ObjcCommandLineOptions::class(),
                )]
            },
        },
    ]
}

const CORE_OPTIONS_NAME: &str = "com.google.devtools.build.lib.analysis.config.CoreOptions";
const PLATFORM_OPTIONS_NAME: &str = "com.google.devtools.build.lib.analysis.PlatformOptions";
const PYTHON_OPTIONS_NAME: &str = "com.google.devtools.build.lib.rules.python.PythonOptions";

fn platform_configuration_options() -> Vec<(&'static str, OptionsClassRef)> {
    vec![(PLATFORM_OPTIONS_NAME, PlatformOptions::class())]
}

/// `addConfigurationOptions` calls (`configurationOptions`): `BAZEL_SETUP` adds `CoreOptions`.
// Java: com/google/devtools/build/lib/bazel/rules/BazelRuleClassProvider.java:224 BazelRuleClassProvider.BAZEL_SETUP
fn additional_options() -> Vec<(&'static str, OptionsClassRef)> {
    vec![(CORE_OPTIONS_NAME, CoreOptions::class())]
}

/// The registered `FragmentOptions` classes, ordered by Java binary class name.
// Java: com/google/devtools/build/lib/analysis/config/FragmentRegistry.java:36 FragmentRegistry.create
pub fn fragment_options_classes() -> Vec<OptionsClassRef> {
    // ImmutableSortedSet.orderedBy(LEXICAL_FRAGMENT_OPTIONS_COMPARATOR): sorted by name, the
    // first of equal elements kept.
    let mut options_classes: Vec<(&'static str, OptionsClassRef)> = Vec::new();
    let mut add = |(name, class): (&'static str, OptionsClassRef)| {
        if !options_classes.iter().any(|(n, _)| *n == name) {
            options_classes.push((name, class));
        }
    };
    for fragment in registered_fragments() {
        for option in (fragment.required_options)() {
            add(option);
        }
    }
    for option in additional_options() {
        add(option);
    }
    // Java's String.compareTo compares UTF-16 code units; the names are ASCII, so byte order is
    // the same.
    options_classes.sort_by(|a, b| a.0.cmp(b.0));
    options_classes
        .into_iter()
        .map(|(_, class)| class)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
