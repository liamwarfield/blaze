//! Analysis-time resolution of late-bound defaults (Java `LateBoundDefault.resolve` of each
//! late-bound default object).
//!
//! In Java every `LateBoundDefault` carries its own resolver lambda. Blaze's loading-phase
//! `blaze_packages::attribute::LateBoundDefault` only names its resolver
//! (`get_resolver_name()`); the resolvers themselves are ported here, keyed by that name.

use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_build_config::build_options::StarlarkOptionValue;
use blaze_build_config::fragment::Fragment;
use blaze_build_config::fragments::android_configuration::AndroidConfiguration;
use blaze_build_config::fragments::apple_configuration::AppleConfiguration;
use blaze_build_config::fragments::bazel_python_configuration::BazelPythonConfiguration;
use blaze_build_config::fragments::coverage_configuration::CoverageConfiguration;
use blaze_build_config::fragments::cpp_configuration::CppConfiguration;
use blaze_build_config::fragments::j2objc_configuration::J2ObjcConfiguration;
use blaze_build_config::fragments::java_configuration::JavaConfiguration;
use blaze_build_config::fragments::proto_configuration::ProtoConfiguration;
use blaze_build_config::fragments::python_configuration::PythonConfiguration;
use blaze_build_config::fragments::test_configuration::TestConfiguration;
use blaze_cmdline::Label;
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_optdefs::fragment::run_under::RunUnder;
use blaze_packages::attribute::{AttributeMap, LateBoundDefault};
use blaze_packages::native_rules::config_rule_classes::ConfigSettingRule;
use blaze_packages::rule_class::rule_class::STARLARK_BUILD_SETTING_DEFAULT_ATTR_NAME;
use blaze_packages::target::Rule;
use blaze_packages::types::{AttrValue, Type};

/// The `input` argument of Java `LateBoundDefault.resolve(rule, attributes, input)`: the
/// configuration (`BuildConfigurationValue.class`), a fragment, or null (`Void.class`).
#[derive(Clone, Copy)]
pub enum LateBoundInput<'a> {
    Configuration(&'a BuildConfigurationValue),
    Fragment(&'a dyn Fragment),
    Null,
}

impl<'a> LateBoundInput<'a> {
    fn configuration(self) -> Option<&'a BuildConfigurationValue> {
        match self {
            LateBoundInput::Configuration(c) => Some(c),
            _ => None,
        }
    }

    fn fragment<T: Fragment>(self) -> &'a T {
        match self {
            LateBoundInput::Fragment(f) => f.as_any().downcast_ref::<T>().unwrap_or_else(|| {
                panic!(
                    "ClassCastException: fragment is not a {}",
                    std::any::type_name::<T>()
                )
            }),
            _ => panic!("NullPointerException: late-bound default without its fragment"),
        }
    }
}

fn label(l: Option<&Label>) -> Option<AttrValue> {
    l.cloned().map(AttrValue::Label)
}

/// Java `lateBoundDefault.resolve(rule, attributes, input)`.
// Java: com/google/devtools/build/lib/packages/Attribute.java:1675 Attribute.LateBoundDefault.resolve
pub fn resolve(
    late_bound_default: &LateBoundDefault,
    rule: Option<&Rule>,
    attributes: &dyn AttributeMap,
    input: LateBoundInput<'_>,
) -> Option<AttrValue> {
    let Some(name) = late_bound_default.get_resolver_name() else {
        // `LateBoundDefault.alwaysNull()` and defaults whose resolver is not named.
        if late_bound_default.kind() == blaze_packages::attribute::LateBoundDefaultKind::AlwaysNull
        {
            // Java: com/google/devtools/build/lib/packages/Attribute.java:1700 Attribute.AlwaysNullLateBoundDefault.resolve
            return None;
        }
        blaze_util::unported!(
            "LateBoundDefault.resolve",
            "late-bound default without a resolver: {:?}",
            late_bound_default
        );
    };
    if let Some(field) = name.strip_prefix("configuration_field:") {
        let (fragment, field_name) = field
            .split_once(':')
            .expect("configuration_field:<fragment>:<name>");
        return resolve_configuration_field(fragment, field_name, input);
    }
    match name {
        // Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:120 BaseRuleClasses.ACTION_LISTENER
        "BaseRuleClasses.ACTION_LISTENER" => {
            let configuration = input.configuration()?;
            Some(AttrValue::LabelList(get_action_listeners(configuration)))
        }
        // Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:128 BaseRuleClasses.COVERAGE_SUPPORT_CONFIGURATION_RESOLVER
        "BaseRuleClasses.coverageSupportAttribute" => Some(AttrValue::Label(
            input
                .fragment::<TestConfiguration>()
                .get_coverage_support()
                .clone(),
        )),
        // Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:140 BaseRuleClasses.COVERAGE_REPORT_GENERATOR_CONFIGURATION_RESOLVER
        "BaseRuleClasses.coverageReportGeneratorAttribute" => {
            label(input.fragment::<CoverageConfiguration>().report_generator())
        }
        // Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:162 BaseRuleClasses.RUN_UNDER_EXEC_CONFIG
        "BaseRuleClasses.RUN_UNDER_EXEC_CONFIG" => {
            let config = input.configuration()?;
            if config.is_exec_configuration()
                // This is the opposite of RUN_UNDER_TARGET_CONFIG, so both can't be non-null.
                || !run_under_exec_config_for_tests(config)
                || get_run_under(config).is_none()
            {
                return None;
            }
            run_under_label(config)
        }
        // Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:184 BaseRuleClasses.RUN_UNDER_TARGET_CONFIG
        "BaseRuleClasses.RUN_UNDER_TARGET_CONFIG" => {
            let config = input.configuration()?;
            if config.is_exec_configuration()
                // This is the opposite of RUN_UNDER_EXEC_CONFIG, so both can't be non-null.
                || run_under_exec_config_for_tests(config)
                || get_run_under(config).is_none()
            {
                return None;
            }
            run_under_label(config)
        }
        // Java: com/google/devtools/build/lib/rules/LabelBuildSettings.java:62 LabelBuildSettings.ACTUAL
        "LabelBuildSettings.ACTUAL" => {
            let (Some(rule), Some(configuration)) = (rule, input.configuration()) else {
                return attributes.get(STARLARK_BUILD_SETTING_DEFAULT_ATTR_NAME, Type::NodepLabel);
            };
            let command_line_value = configuration
                .get_options()
                .get_starlark_options()
                .get(rule.get_label());
            let Some(command_line_value) = command_line_value else {
                return attributes.get(STARLARK_BUILD_SETTING_DEFAULT_ATTR_NAME, Type::NodepLabel);
            };
            match command_line_value {
                StarlarkOptionValue::Label(l) => Some(AttrValue::Label(l.clone())),
                other => panic!(
                    "the value of {} should have been converted to a label already, but its type is {:?}",
                    rule.get_label(),
                    other
                ),
            }
        }
        // Java: com/google/devtools/build/lib/rules/config/ConfigRuleClasses.java:172 ConfigRuleClasses.ConfigSettingRule.FLAG_ALIAS_REFERENCES
        "ConfigSettingRule.FLAG_ALIAS_REFERENCES" => {
            let configuration = input.configuration()?;
            let Some(AttrValue::StringDict(settings)) =
                attributes.get(ConfigSettingRule::SETTINGS_ATTRIBUTE, Type::StringDict)
            else {
                // Only true for tests.
                return Some(AttrValue::LabelList(Vec::new()));
            };
            let command_line_flag_aliases = configuration
                .get_options()
                .get::<CoreOptions>()
                .expect("CoreOptions")
                .get_command_line_flag_aliases();
            let mut user_defined_flags = Vec::new();
            for flag_name in settings.keys() {
                if let Some((_, user_defined_flag)) = command_line_flag_aliases
                    .iter()
                    .find(|(k, _)| k == flag_name)
                {
                    user_defined_flags.push(user_defined_flag.clone());
                }
            }
            Some(AttrValue::LabelList(user_defined_flags))
        }
        // Java: com/google/devtools/build/lib/rules/cpp/CcLibcTopAlias.java:30 CcLibcTopAlias.getSysrootAttribute
        "CcLibcTopAlias.SYSROOT" => {
            label(input.fragment::<CppConfiguration>().get_libc_top_label())
        }
        other => blaze_util::unported!("LateBoundDefault.resolve", "resolver {}", other),
    }
}

/// Java `StarlarkLateBoundDefault.resolve`: invokes the fragment's
/// `@StarlarkConfigurationField(name = field_name)` method.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkLateBoundDefault.java:57 StarlarkLateBoundDefault.resolve
fn resolve_configuration_field(
    fragment: &str,
    field_name: &str,
    input: LateBoundInput<'_>,
) -> Option<AttrValue> {
    match (fragment, field_name) {
        // Java: com/google/devtools/build/lib/analysis/test/CoverageConfiguration.java:86 CoverageConfiguration.outputGenerator
        ("coverage", "output_generator") => {
            label(input.fragment::<CoverageConfiguration>().output_generator())
        }
        // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:320 AppleConfiguration.getXcodeConfigLabel
        ("apple", "xcode_config_label") => label(Some(
            input
                .fragment::<AppleConfiguration>()
                .get_xcode_config_label(),
        )),
        ("cpp", _) => {
            let cpp = input.fragment::<CppConfiguration>();
            match field_name {
                // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:309 CppConfiguration.getFdoZipper
                "zipper" => cpp.get_fdo_zipper().map(AttrValue::Label),
                // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:424 CppConfiguration.customMalloc
                "custom_malloc" => label(cpp.custom_malloc()),
                // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:632 CppConfiguration.getFdoOptimizeLabel
                "fdo_optimize" => label(cpp.get_fdo_optimize_label()),
                // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:683 CppConfiguration.getFdoPrefetchHintsLabel
                "fdo_prefetch_hints" => label(cpp.get_fdo_prefetch_hints_label()),
                // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:696 CppConfiguration.getFdoProfileLabel
                "fdo_profile" => label(cpp.get_fdo_profile_label()),
                // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:701 CppConfiguration.getCSFdoProfileLabel
                "cs_fdo_profile" => label(cpp.get_cs_fdo_profile_label()),
                // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:709 CppConfiguration.getPropellerOptimizeLabel
                "propeller_optimize" => label(cpp.get_propeller_optimize_label()),
                // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:720 CppConfiguration.getXFdoProfileLabel
                "xbinary_fdo" => label(cpp.get_x_fdo_profile_label()),
                // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:733 CppConfiguration.getMemProfProfileLabel
                "memprof_profile" => label(cpp.get_mem_prof_profile_label()),
                // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:756 CppConfiguration.getLibcTopLabel
                "libc_top" => label(cpp.get_libc_top_label()),
                // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:958 CppConfiguration.getProtoProfilePath
                "proto_profile_path" => label(cpp.get_proto_profile_path()),
                other => blaze_util::unported!("CppConfiguration configuration field", "{}", other),
            }
        }
        ("java", _) => {
            let java = input.fragment::<JavaConfiguration>();
            match field_name {
                // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:236 JavaConfiguration.getJavaLauncherLabel
                "launcher" => label(java.get_java_launcher_label()),
                // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:246 JavaConfiguration.getProguardBinary
                "proguard_top" => label(java.get_proguard_binary()),
                // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:305 JavaConfiguration.getBytecodeOptimizerLabel
                "bytecode_optimizer" => label(java.get_bytecode_optimizer_label()),
                // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:319 JavaConfiguration.getBytecodeOptimizerLabelForJavaToolchain
                "java_toolchain_bytecode_optimizer" => {
                    label(java.get_bytecode_optimizer_label_for_java_toolchain())
                }
                // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:332 JavaConfiguration.getLocalJavaOptimizationConfiguration
                "local_java_optimization_configuration" => {
                    label(java.get_local_java_optimization_configuration())
                }
                other => {
                    blaze_util::unported!("JavaConfiguration configuration field", "{}", other)
                }
            }
        }
        // Java: com/google/devtools/build/lib/bazel/rules/python/BazelPythonConfiguration.java:124 BazelPythonConfiguration.getPythonTop
        ("bazel_py", "python_top") => label(
            input
                .fragment::<BazelPythonConfiguration>()
                .get_python_top(),
        ),
        // Java: com/google/devtools/build/lib/rules/objc/J2ObjcConfiguration.java:100 J2ObjcConfiguration.getDeadCodeReport
        ("j2objc", "dead_code_report") => {
            label(input.fragment::<J2ObjcConfiguration>().dead_code_report())
        }
        ("proto", _) => {
            let proto = input.fragment::<ProtoConfiguration>();
            match field_name {
                // Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:188 ProtoConfiguration.protoCompiler
                "proto_compiler" => label(proto.proto_compiler()),
                // Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:197 ProtoConfiguration.protoToolchainForJava
                "proto_toolchain_for_java" => label(proto.proto_toolchain_for_java()),
                // Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:206 ProtoConfiguration.protoToolchainForJ2objc
                "proto_toolchain_for_j2objc" => label(proto.proto_toolchain_for_j2objc()),
                // Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:215 ProtoConfiguration.protoToolchainForJavaLite
                "proto_toolchain_for_java_lite" => label(proto.proto_toolchain_for_java_lite()),
                // Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:224 ProtoConfiguration.protoToolchainForCc
                "proto_toolchain_for_cc" => label(proto.proto_toolchain_for_cc()),
                other => {
                    blaze_util::unported!("ProtoConfiguration configuration field", "{}", other)
                }
            }
        }
        ("android", _) => {
            let android = input.fragment::<AndroidConfiguration>();
            match field_name {
                // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1390 AndroidConfiguration.getLegacyMainDexListGenerator
                "legacy_main_dex_list_generator" => {
                    label(android.get_legacy_main_dex_list_generator())
                }
                // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1399 AndroidConfiguration.getOptimizingDexer
                "optimizing_dexer" => label(android.get_optimizing_dexer()),
                other => {
                    blaze_util::unported!("AndroidConfiguration configuration field", "{}", other)
                }
            }
        }
        // Java: com/google/devtools/build/lib/rules/python/PythonConfiguration.java:147 PythonConfiguration.getNativeRulesAllowlist
        ("py", "native_rules_allowlist") => label(
            input
                .fragment::<PythonConfiguration>()
                .get_native_rules_allowlist(),
        ),
        (fragment, field) => {
            blaze_util::unported!("StarlarkLateBoundDefault.resolve", "{}.{}", fragment, field)
        }
    }
}

/// Java `BuildConfigurationValue.getActionListeners`.
// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:796 BuildConfigurationValue.getActionListeners
fn get_action_listeners(configuration: &BuildConfigurationValue) -> Vec<Label> {
    core_options(configuration)
        .action_listeners
        .iter()
        .map(|l| l.get().clone())
        .collect()
}

/// Java `BuildConfigurationValue.getRunUnder`.
// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:741 BuildConfigurationValue.getRunUnder
fn get_run_under(configuration: &BuildConfigurationValue) -> Option<&RunUnder> {
    core_options(configuration).run_under.as_ref()
}

/// Java `BuildConfigurationValue.runUnderExecConfigForTests`.
// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:746 BuildConfigurationValue.runUnderExecConfigForTests
fn run_under_exec_config_for_tests(configuration: &BuildConfigurationValue) -> bool {
    core_options(configuration).bazel_test_exec_run_under
}

/// `config.getRunUnder() instanceof LabelRunUnder runUnder ? runUnder.label() : null`.
fn run_under_label(configuration: &BuildConfigurationValue) -> Option<AttrValue> {
    match get_run_under(configuration) {
        Some(RunUnder::LabelRunUnder { label, .. }) => Some(AttrValue::Label(label.clone())),
        _ => None,
    }
}

fn core_options(configuration: &BuildConfigurationValue) -> &CoreOptions {
    configuration
        .get_options()
        .get::<CoreOptions>()
        .expect("CoreOptions")
}
