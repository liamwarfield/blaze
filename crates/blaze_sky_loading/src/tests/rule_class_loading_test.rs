//! Ports of the Java tests that assert the error events of rule instantiation ("no such
//! attribute", "missing value for mandatory attribute", "is not configurable") and need nothing
//! but package loading. They come from analysis test classes (the Java class is named above each
//! test); `getTarget` / `getConfiguredTarget` there only force the package to load.
//!
//! Java: src/test/java/com/google/devtools/build/lib/analysis/RuleConfiguredTargetTest.java,
//! rules/AliasTest.java, analysis/ConfigurableAttributesTest.java,
//! starlark/StarlarkIntegrationTest.java.
//!
//! "is duplicated in the" is covered by `package_factory_test::test_duplicated_dependencies`, and
//! "invalid value in" by `starlark_defined_aspects_loading_test::aspect_parameters_bad_value`.
//! `RuleClassTest` and `RuleFactoryTest` build rule classes and packages in Java, without BUILD
//! files; their stubs stay in blaze_packages.

use crate::tests::package_loading_test_case::{PackageLoadingTestCase, assert_contains_event};

// Java: RuleConfiguredTargetTest.testNativeRuleAttrSetToNoneFails
#[test]
fn test_native_rule_attr_set_to_none_fails() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--incompatible_fail_on_unknown_attributes"]);
    t.scratch_file(
        "p/BUILD",
        &["genrule(name = 'genrule', srcs = ['a.java'], outs = ['b'], cmd = '', bat = None)"],
    );

    let pkg = t.get_package("p");
    // Java `getTarget("//p:genrule")` does not throw: the rule exists, in error.
    pkg.target("genrule");

    pkg.assert_contains_event("no such attribute 'bat' in 'genrule' rule");
}

// Java: RuleConfiguredTargetTest.testNativeRuleAttrSetToNoneDoesntFails
#[test]
fn test_native_rule_attr_set_to_none_doesnt_fails() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--noincompatible_fail_on_unknown_attributes"]);
    t.scratch_file(
        "p/BUILD",
        &["genrule(name = 'genrule', srcs = ['a.java'], outs = ['b'], cmd = '', bat = None)"],
    );

    t.get_package_without_errors("p").target("genrule");
}

// Java: RuleConfiguredTargetTest.testStarlarkRuleAttrSetToNoneFails
#[test]
fn test_starlark_rule_attr_set_to_none_fails() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--incompatible_fail_on_unknown_attributes"]);
    t.scratch_file(
        "p/rule.bzl",
        &["def _impl(ctx):", "    pass", "", "my_rule = rule(_impl)"],
    );
    t.scratch_file(
        "p/BUILD",
        &[
            "load(\":rule.bzl\", \"my_rule\")",
            "",
            "my_rule(",
            "    name = \"my_target\",",
            "    bat = None,",
            ")",
        ],
    );

    let pkg = t.get_package("p");
    // Java `getTarget("//p:my_target")` does not throw: the rule exists, in error.
    pkg.target("my_target");

    pkg.assert_contains_event("no such attribute 'bat' in 'my_rule' rule");
}

// Java: RuleConfiguredTargetTest.testStarlarkRuleAttrSetToNoneDoesntFail
#[test]
fn test_starlark_rule_attr_set_to_none_doesnt_fail() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--noincompatible_fail_on_unknown_attributes"]);
    t.scratch_file(
        "p/rule.bzl",
        &["def _impl(ctx):", "    pass", "", "my_rule = rule(_impl)"],
    );
    t.scratch_file(
        "p/BUILD",
        &[
            "load(\":rule.bzl\", \"my_rule\")",
            "",
            "my_rule(",
            "    name = \"my_target\",",
            "    bat = None,",
            ")",
        ],
    );

    t.get_package_without_errors("p").target("my_target");
}

// Java: AliasTest.assertNoLicensesAttribute
#[test]
fn assert_no_licenses_attribute() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "a/BUILD",
        &[
            "filegroup(name = \"a\")",
            "",
            "alias(",
            "    name = \"b\",",
            "    actual = \":a\",",
            "    licenses = [\"unencumbered\"],",
            ")",
        ],
    );

    let (_, events) = t.evaluate_package("a");
    assert_contains_event(&events, "no such attribute 'licenses' in 'alias' rule");
}

// Java: AliasTest.testNoActual (`checkError("a", "a", message, "alias(name='a')")`: the configured
// target is null or has errors, and the event is reported).
#[test]
fn test_no_actual() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("a/BUILD", &["alias(name='a')"]);

    let events = t.load_failure_events("a");
    assert_contains_event(&events, "missing value for mandatory attribute 'actual'");
}

/// Attributes of type {@link BuildType#OUTPUT_LIST} are not configurable.
// Java: ConfigurableAttributesTest.outputListTypeNotConfigurable
#[test]
fn output_list_type_not_configurable() {
    let t = PackageLoadingTestCase::new();
    // Java: ConfigurableAttributesTest.writeConfigRules
    t.scratch_file(
        "conditions/BUILD",
        &[
            "config_setting(",
            "    name = 'a',",
            "    values = {'foo': 'a'})",
            "config_setting(",
            "    name = 'b',",
            "    values = {'foo': 'b'})",
        ],
    );
    t.scratch_file(
        "foo/BUILD",
        &[
            "genrule(",
            "    name = 'generator',",
            "    srcs = [],",
            "    outs = select({",
            "        '//conditions:a': ['a.out'],",
            "        '//conditions:default': ['default.out']})",
            ")",
        ],
    );

    let (_, events) = t.evaluate_package("foo");
    assert_contains_event(&events, "attribute \"outs\" is not configurable");
}

// Java: StarlarkIntegrationTest.testBuildSettingRule_noDefault
#[test]
fn test_build_setting_rule_no_default() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "test/rules.bzl",
        &[
            "def _impl(ctx): return None",
            "build_setting_rule = rule(_impl, build_setting = config.string())",
        ],
    );
    t.scratch_file(
        "test/BUILD",
        &[
            "load('//test:rules.bzl', 'build_setting_rule')",
            "build_setting_rule(name = 'my_build_setting')",
        ],
    );

    let (_, events) = t.evaluate_package("test");
    assert_contains_event(
        &events,
        "missing value for mandatory attribute 'build_setting_default' in 'build_setting_rule' \
         rule",
    );
}
