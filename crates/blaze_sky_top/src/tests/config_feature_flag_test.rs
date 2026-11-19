//! Port of `src/test/java/com/google/devtools/build/lib/rules/config/ConfigFeatureFlagTest.java`
//! (a `BuildViewTestCase`): the `config_feature_flag` rule and `ConfigFeatureFlagProvider`.
//!
//! Java's test class builds its own rule class provider (`createRuleClassProvider`):
//! `TestRuleClassProvider.addStandardRules` plus the test rule `feature_flag_setter`
//! (`FeatureFlagSetterRule`, which needs the unported `ConfigFeatureFlagTransitionFactory`). The
//! harness analyzes with Bazel's rule class provider and cannot register it, so the tests that use
//! it are ignored. Every test runs with `--enforce_transitive_configs_for_config_feature_flag`
//! (Java `@Before enforceTransitiveConfigs`).

use std::sync::Arc;

use blaze_analysis::config::config_matching_provider::ConfigMatchingProvider;
use blaze_analysis::config::configured_attribute_mapper::ConfiguredAttributeMapper;
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configured_target_and_data::ConfiguredTargetAndData;
use blaze_cmdline::Label;
use blaze_packages::attribute::attribute_map::AttributeMap;
use blaze_packages::target::Target;
use blaze_packages::types::attr_value::AttrValue;
use blaze_packages::types::type_::Type;
use blaze_rules::config::config_feature_flag_provider::ConfigFeatureFlagProvider;
use indexmap::IndexMap;
use starlark::values::Heap;

use crate::testing::BuildViewTestCase;

/// Java `BuildViewTestCase` with `@Before enforceTransitiveConfigs`.
fn new_test_case() -> BuildViewTestCase {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&["--enforce_transitive_configs_for_config_feature_flag"]);
    t
}

/// `ConfiguredTarget.getConfigConditions()` (empty unless the target is a rule or an alias).
fn get_config_conditions(ct: &ConfiguredTarget) -> Arc<IndexMap<Label, ConfigMatchingProvider>> {
    match ct {
        ConfiguredTarget::Rule(t) => t.get_config_conditions().clone(),
        ConfiguredTarget::Alias(t) => t.get_config_conditions().clone(),
        _ => Arc::new(IndexMap::new()),
    }
}

/// `BuildViewTestCase.getMapperFromConfiguredTargetAndTarget`.
// Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:335 ConfiguredTargetAndData.getAttributeMapperForTesting
fn get_mapper_from_configured_target_and_target(
    ctad: &ConfiguredTargetAndData,
) -> ConfiguredAttributeMapper {
    let Target::Rule(rule) = ctad.get_target() else {
        panic!(
            "ClassCastException: {:?} is not a Rule",
            ctad.get_target_label()
        );
    };
    ConfiguredAttributeMapper::of(
        rule.clone(),
        get_config_conditions(ctad.get_configured_target()),
        ctad.get_configuration().expect("configuration"),
    )
}

/// `ConfigFeatureFlagProvider.fromTarget(getConfiguredTarget(label)).getFlagValue()`.
fn get_flag_value(t: &mut BuildViewTestCase, label: &str) -> Option<String> {
    let target = t
        .get_configured_target(label)
        .unwrap_or_else(|| panic!("NullPointerException: no configured target {label}"));
    let info = ConfigFeatureFlagProvider::from_target(&target)
        .unwrap_or_else(|| panic!("NullPointerException: {label} has no FeatureFlagInfo"));
    ConfigFeatureFlagProvider::of(&info)
        .get_flag_value()
        .map(str::to_owned)
}

// Java: ConfigFeatureFlagTest.configFeatureFlagProvider_fromTargetReturnsNullIfTargetDoesNotExportProvider
#[test]
#[ignore = "needs the test rule feature_flag_setter (FeatureFlagSetterRule, registered by the test's custom rule class provider)"]
fn config_feature_flag_provider_from_target_returns_null_if_target_does_not_export_provider() {
    let mut t = new_test_case();
    t.scratch.file(
        "test/BUILD",
        &[r#"feature_flag_setter(
    name = "top",
    flag_values = {
    },
)
"#],
    );
    let top = t.get_configured_target("//test:top").expect("//test:top");
    assert!(ConfigFeatureFlagProvider::from_target(&top).is_none());
}

// Java: ConfigFeatureFlagTest.configFeatureFlagProvider_containsValueFromConfiguration
#[test]
#[ignore = "needs the test rule feature_flag_setter (FeatureFlagSetterRule, registered by the test's custom rule class provider)"]
fn config_feature_flag_provider_contains_value_from_configuration() {
    let mut t = new_test_case();
    t.scratch.file(
        "test/BUILD",
        &[r#"feature_flag_setter(
    name = "top",
    exports_flag = ":flag",
    flag_values = {
        ":flag": "configured",
    },
    transitive_configs = [":flag"],
)

config_feature_flag(
    name = "flag",
    allowed_values = [
        "default",
        "configured",
        "other",
    ],
)
"#],
    );
    assert_eq!(
        get_flag_value(&mut t, "//test:top").as_deref(),
        Some("configured")
    );
}

// Java: ConfigFeatureFlagTest.configFeatureFlagProvider_usesConfiguredValueOverDefault
#[test]
#[ignore = "needs the test rule feature_flag_setter (FeatureFlagSetterRule, registered by the test's custom rule class provider)"]
fn config_feature_flag_provider_uses_configured_value_over_default() {
    let mut t = new_test_case();
    t.scratch.file(
        "test/BUILD",
        &[r#"feature_flag_setter(
    name = "top",
    exports_flag = ":flag",
    flag_values = {
        ":flag": "configured",
    },
    transitive_configs = [":flag"],
)

config_feature_flag(
    name = "flag",
    allowed_values = [
        "default",
        "configured",
        "other",
    ],
    default_value = "default",
)
"#],
    );
    assert_eq!(
        get_flag_value(&mut t, "//test:top").as_deref(),
        Some("configured")
    );
}

// Java: ConfigFeatureFlagTest.configFeatureFlagProvider_starlarkConstructor
#[test]
fn config_feature_flag_provider_starlark_constructor() {
    let mut t = new_test_case();
    t.scratch.file(
        "test/wrapper.bzl",
        &[r#"def _flag_reading_wrapper_impl(ctx):
    pass

flag_reading_wrapper = rule(
    implementation = _flag_reading_wrapper_impl,
    attrs = {"flag": attr.label()},
)

def _flag_propagating_wrapper_impl(ctx):
    return [config_common.FeatureFlagInfo(value = "hello")]

flag_propagating_wrapper = rule(
    implementation = _flag_propagating_wrapper_impl,
)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load(":wrapper.bzl", "flag_propagating_wrapper")

flag_propagating_wrapper(
    name = "propagator",
)

config_setting(
    name = "hello_setting",
    flag_values = {":propagator": "hello"},
)

genrule(
    name = "gen",
    srcs = [],
    outs = ["out"],
    cmd = select({
        ":hello_setting": "hello",
        "//conditions:default": "error",
    }),
)
"#],
    );

    let ctad = t
        .get_configured_target_and_data("//test:gen")
        .expect("//test:gen");
    let attribute_mapper = get_mapper_from_configured_target_and_target(&ctad);
    assert_eq!(
        attribute_mapper.get("cmd", Type::String),
        Some(AttrValue::String("hello".to_owned()))
    );
}

// Java: ConfigFeatureFlagTest.configFeatureFlagProvider_valueIsAccessibleFromStarlark
#[test]
#[ignore = "needs the test rule feature_flag_setter (FeatureFlagSetterRule, registered by the test's custom rule class provider)"]
fn config_feature_flag_provider_value_is_accessible_from_starlark() {
    let mut t = new_test_case();
    t.scratch.file(
        "test/wrapper.bzl",
        &[r#"def _flag_reading_wrapper_impl(ctx):
    pass

flag_reading_wrapper = rule(
    implementation = _flag_reading_wrapper_impl,
    attrs = {"flag": attr.label()},
)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load(":wrapper.bzl", "flag_reading_wrapper")

feature_flag_setter(
    name = "top",
    flag_values = {
        ":flag": "configured",
    },
    transitive_configs = [":flag"],
    deps = [":wrapper"],
)

flag_reading_wrapper(
    name = "wrapper",
    flag = ":flag",
    transitive_configs = [":flag"],
)

config_feature_flag(
    name = "flag",
    allowed_values = [
        "default",
        "configured",
        "other",
    ],
    default_value = "default",
)
"#],
    );
    let _top = t.get_configured_target("//test:top").expect("//test:top");
    unimplemented!(
        "getPrerequisites(top, \"deps\") and ev.eval(\"ruleContext.attr.flag[config_common.FeatureFlagInfo].value\") == \"configured\""
    );
}

// Java: ConfigFeatureFlagTest.configFeatureFlagProvider_validatesValuesUsingAllowedValuesAttribute
#[test]
fn config_feature_flag_provider_validates_values_using_allowed_values_attribute() {
    let mut t = new_test_case();
    t.scratch.file(
        "test/BUILD",
        &[r#"config_feature_flag(
    name = "flag",
    allowed_values = [
        "default",
        "configured",
        "other",
    ],
    default_value = "default",
)
"#],
    );
    let target = t.get_configured_target("//test:flag").expect("//test:flag");
    let info = ConfigFeatureFlagProvider::from_target(&target).expect("FeatureFlagInfo");
    let provider = ConfigFeatureFlagProvider::of(&info);
    assert!(provider.is_valid_value("default"));
    assert!(provider.is_valid_value("configured"));
    assert!(provider.is_valid_value("other"));

    assert!(!provider.is_valid_value("absent"));
    assert!(!provider.is_valid_value("conFigured"));
    assert!(!provider.is_valid_value("  other"));
}

// Java: ConfigFeatureFlagTest.configFeatureFlagProvider_valueValidationIsPossibleFromStarlark
#[test]
#[ignore = "needs BazelEvaluationTestCase evaluating expressions over a StarlarkRuleContext (ev.update(\"ruleContext\", createRuleContext(...)))"]
fn config_feature_flag_provider_value_validation_is_possible_from_starlark() {
    let mut t = new_test_case();
    t.scratch.file(
        "test/wrapper.bzl",
        &[r#"def _flag_reading_wrapper_impl(ctx):
    pass

flag_reading_wrapper = rule(
    implementation = _flag_reading_wrapper_impl,
    attrs = {"flag": attr.label()},
)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load(":wrapper.bzl", "flag_reading_wrapper")

flag_reading_wrapper(
    name = "wrapper",
    flag = ":flag",
    transitive_configs = [":flag"],
)

config_feature_flag(
    name = "flag",
    allowed_values = [
        "default",
        "configured",
        "other",
    ],
    default_value = "default",
)
"#],
    );
    let _wrapper = t
        .get_configured_target("//test:wrapper")
        .expect("//test:wrapper");
    // Java: provider = "ruleContext.attr.flag[config_common.FeatureFlagInfo]"; is_valid_value is
    // True for 'default', 'configured', 'other' and False for 'absent', 'conFigured', '  other'.
    unimplemented!(
        "ev.update(\"ruleContext\", createRuleContext(\"//test:wrapper\")); ev.eval(...)"
    );
}

// Java: ConfigFeatureFlagTest.configFeatureFlagProvider_usesDefaultValueIfConfigurationDoesntSetValue
#[test]
#[ignore = "needs the test rule feature_flag_setter (FeatureFlagSetterRule, registered by the test's custom rule class provider)"]
fn config_feature_flag_provider_uses_default_value_if_configuration_doesnt_set_value() {
    let mut t = new_test_case();
    t.scratch.file(
        "test/BUILD",
        &[r#"feature_flag_setter(
    name = "top",
    exports_flag = ":flag",
    flag_values = {
        ":other": "configured",
    },
    transitive_configs = [
        ":flag",
        ":other",
    ],
)

config_feature_flag(
    name = "flag",
    allowed_values = [
        "other",
        "default",
        "configured",
    ],
    default_value = "default",
)

config_feature_flag(
    name = "other",
    allowed_values = [
        "default",
        "configured",
        "other",
    ],
    default_value = "default",
)
"#],
    );
    assert_eq!(
        get_flag_value(&mut t, "//test:top").as_deref(),
        Some("default")
    );
}

// Java: ConfigFeatureFlagTest.configFeatureFlagProvider_ignoresUnusedFlagWithNeitherDefaultNorConfiguredValueSet
#[test]
#[ignore = "needs the test rule feature_flag_setter (FeatureFlagSetterRule, registered by the test's custom rule class provider)"]
fn config_feature_flag_provider_ignores_unused_flag_with_neither_default_nor_configured_value_set()
{
    let mut t = new_test_case();
    t.scratch.file(
        "test/BUILD",
        &[r#"feature_flag_setter(
    name = "top",
    exports_flag = ":flag",
    flag_values = {
        ":other": "configured",
    },
    transitive_configs = [
        ":flag",
        ":other",
    ],
)

config_feature_flag(
    name = "flag",
    allowed_values = [
        "other",
        "configured",
    ],
)

config_feature_flag(
    name = "other",
    allowed_values = [
        "default",
        "configured",
        "other",
    ],
    default_value = "default",
)
"#],
    );
    assert!(t.get_configured_target("//test:top").is_some());
    t.assert_no_events();
}

// Java: ConfigFeatureFlagTest.configFeatureFlagProvider_throwsErrorIfReadFlagWithNeitherDefaultNorConfiguredValueSet
#[test]
#[ignore = "needs the test rule feature_flag_setter (FeatureFlagSetterRule, registered by the test's custom rule class provider)"]
fn config_feature_flag_provider_throws_error_if_read_flag_with_neither_default_nor_configured_value_set()
 {
    let mut t = new_test_case();
    t.remove_fail_fast_handler(); // expecting an error
    t.scratch.file(
        "test/BUILD",
        &[r#"feature_flag_setter(
    name = "top",
    exports_flag = ":flag",
    flag_values = {
        ":other": "configured",
    },
    transitive_configs = [
        ":flag",
        ":other",
    ],
    deps = [":reader"],
)

filegroup(
    name = "reader",
    srcs = select({
        ":flag@configured": ["a.txt"],
        "//conditions:default": ["b.txt"],
    }),
    transitive_configs = [
        ":flag",
        ":other",
    ],
)

config_setting(
    name = "flag@configured",
    flag_values = {":flag": "configured"},
    transitive_configs = [":flag"],
)

config_feature_flag(
    name = "flag",
    allowed_values = [
        "other",
        "configured",
    ],
)

config_feature_flag(
    name = "other",
    allowed_values = [
        "default",
        "configured",
        "other",
    ],
    default_value = "default",
)
"#],
    );
    assert!(t.get_configured_target("//test:top").is_none());
    t.assert_contains_event(
        "config_setting //test:flag@configured is unresolvable because: Feature flag //test:flag has no default but no value was explicitly specified.",
    );
}

// Java: ConfigFeatureFlagTest.allowedValuesAttribute_cannotBeEmpty
#[test]
fn allowed_values_attribute_cannot_be_empty() {
    let mut t = new_test_case();
    t.remove_fail_fast_handler(); // expecting an error
    t.scratch.file(
        "test/BUILD",
        &[r#"config_feature_flag(
    name = "flag",
    allowed_values = [],
    default_value = "default",
)
"#],
    );
    assert!(t.get_configured_target("//test:flag").is_none());
    t.assert_contains_event(
        "in allowed_values attribute of config_feature_flag rule //test:flag: attribute must be non empty",
    );
}

// Java: ConfigFeatureFlagTest.allowedValuesAttribute_cannotContainDuplicates
#[test]
fn allowed_values_attribute_cannot_contain_duplicates() {
    let mut t = new_test_case();
    t.remove_fail_fast_handler(); // expecting an error
    t.scratch.file(
        "test/BUILD",
        &[r#"config_feature_flag(
    name = "flag",
    allowed_values = [
        "double",
        "double",
        "toil",
        "trouble",
    ],
    default_value = "trouble",
)
"#],
    );
    assert!(t.get_configured_target("//test:flag").is_none());
    t.assert_contains_event(
        "in allowed_values attribute of config_feature_flag rule //test:flag: cannot contain duplicates, but contained multiple of [\"double\"]",
    );
}

// Java: ConfigFeatureFlagTest.defaultValueAttribute_mustBeMemberOfAllowedValuesIfPresent
#[test]
#[ignore = "needs the test rule feature_flag_setter (FeatureFlagSetterRule, registered by the test's custom rule class provider)"]
fn default_value_attribute_must_be_member_of_allowed_values_if_present() {
    let mut t = new_test_case();
    t.remove_fail_fast_handler(); // expecting an error
    t.scratch.file(
        "test/BUILD",
        &[r#"feature_flag_setter(
    name = "top",
    exports_flag = ":flag",
    flag_values = {
        ":flag": "legal",
    },
    transitive_configs = [":flag"],
)

config_feature_flag(
    name = "flag",
    allowed_values = [
        "legal",
        "eagle",
    ],
    default_value = "beagle",
)
"#],
    );
    assert!(t.get_configured_target("//test:top").is_none());
    t.assert_contains_event(
        "in default_value attribute of config_feature_flag rule //test:flag: must be one of [\"eagle\", \"legal\"], but was \"beagle\"",
    );
}

// Java: ConfigFeatureFlagTest.configurationValue_mustBeMemberOfAllowedValuesIfPresent
#[test]
#[ignore = "needs the test rule feature_flag_setter (FeatureFlagSetterRule, registered by the test's custom rule class provider)"]
fn configuration_value_must_be_member_of_allowed_values_if_present() {
    let mut t = new_test_case();
    t.remove_fail_fast_handler(); // expecting an error
    t.scratch.file(
        "test/BUILD",
        &[r#"feature_flag_setter(
    name = "top",
    exports_flag = ":flag",
    flag_values = {
        ":flag": "invalid",
    },
    transitive_configs = [":flag"],
)

config_feature_flag(
    name = "flag",
    allowed_values = [
        "default",
        "configured",
        "other",
    ],
    default_value = "default",
)
"#],
    );
    assert!(t.get_configured_target("//test:top").is_none());
    // TODO(b/140635901): when configurationError is implemented, switch to testing for that
    t.assert_contains_event(
        "in config_feature_flag rule //test:flag: value must be one of [\"configured\", \"default\", \"other\"], but was \"invalid\"",
    );
}

// Java: ConfigFeatureFlagTest.policy_mustContainRulesPackage
#[test]
fn policy_must_contain_rules_package() {
    let mut t = new_test_case();
    t.remove_fail_fast_handler(); // expecting an error
    t.scratch.overwrite_file(
        "tools/allowlists/config_feature_flag/BUILD",
        &["package_group(name = 'config_feature_flag', packages = ['//some/other'])"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"config_feature_flag(
    name = "flag",
    allowed_values = [
        "default",
        "configured",
        "other",
    ],
    default_value = "default",
)
"#],
    );
    assert!(t.get_configured_target("//test:flag").is_none());
    t.assert_contains_event(
        "in config_feature_flag rule //test:flag: the config_feature_flag rule is not available in this package",
    );
}

// Java: ConfigFeatureFlagTest.policy_doesNotBlockRuleIfInPackageGroup
#[test]
fn policy_does_not_block_rule_if_in_package_group() {
    let mut t = new_test_case();
    t.scratch.overwrite_file(
        "tools/allowlists/config_feature_flag/BUILD",
        &["package_group(name = 'config_feature_flag', packages = ['//test'])"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"config_feature_flag(
    name = "flag",
    allowed_values = [
        "default",
        "configured",
        "other",
    ],
    default_value = "default",
)
"#],
    );
    assert!(t.get_configured_target("//test:flag").is_some());
    t.assert_no_events();
}

// Java: ConfigFeatureFlagTest.equalsTester
#[test]
fn equals_tester() {
    Heap::temp(|heap| {
        let always_true =
            || -> blaze_rules::config::config_feature_flag_provider::ValidityPredicate {
                Arc::new(|_| true)
            };
        let groups = [
            // Basic case.
            heap.alloc(ConfigFeatureFlagProvider::create(
                Some("flag1".to_owned()),
                None,
                always_true(),
            )),
            // Will be distinct from the first group because CFFP instances are all distinct.
            heap.alloc(ConfigFeatureFlagProvider::create(
                Some("flag1".to_owned()),
                None,
                always_true(),
            )),
            // Set the error, still distinct from the above.
            heap.alloc(ConfigFeatureFlagProvider::create(
                None,
                Some("error".to_owned()),
                always_true(),
            )),
            // Change the value, still distinct from the above.
            heap.alloc(ConfigFeatureFlagProvider::create(
                Some("flag2".to_owned()),
                None,
                always_true(),
            )),
        ];
        for (i, a) in groups.iter().enumerate() {
            for (j, b) in groups.iter().enumerate() {
                assert_eq!(a.equals(*b).unwrap(), i == j, "groups {i} and {j}");
            }
        }
    });
}
