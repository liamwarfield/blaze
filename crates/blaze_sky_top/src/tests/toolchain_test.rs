//! Port of `src/test/java/com/google/devtools/build/lib/rules/platform/ToolchainTest.java`: the
//! `toolchain` rule.

use blaze_analysis::config::config_matching_provider::MatchResult;
use blaze_analysis::platform::constraint_setting_info::ConstraintSettingInfo;
use blaze_analysis::platform::constraint_value_info::ConstraintValueInfo;
use blaze_analysis::platform::platform_provider_utils;
use blaze_analysis::platform::toolchain_type_info::ToolchainTypeInfo;
use blaze_cmdline::Label;

use crate::testing::BuildViewTestCase;

fn label(s: &str) -> Label {
    Label::parse_canonical_unchecked(s)
}

/// A `BuildViewTestCase` with the class's `@Before`.
fn new_test() -> BuildViewTestCase {
    let t = BuildViewTestCase::new();
    create_constraints(&t);
    t
}

// Java: ToolchainTest.createConstraints
fn create_constraints(t: &BuildViewTestCase) {
    t.scratch.file(
        "constraint/BUILD",
        &[r#"constraint_setting(name = "basic")

constraint_value(
    name = "foo",
    constraint_setting = ":basic",
)

constraint_value(
    name = "bar",
    constraint_setting = ":basic",
)
"#],
    );
}

const TOOLCHAIN_DEF_BZL: &str = r#"def _impl(ctx):
    toolchain = platform_common.ToolchainInfo(
        data = ctx.attr.data,
    )
    return [toolchain]

toolchain_def = rule(
    implementation = _impl,
    attrs = {
        "data": attr.string(),
    },
)
"#;

const TARGET_SETTING_BUILD: &str = r#"load(":toolchain_def.bzl", "toolchain_def")

toolchain_type(name = "demo_toolchain")

config_setting(
    name = "optimised",
    values = {"compilation_mode": "opt"},
)

toolchain(
    name = "toolchain1",
    target_settings = [":optimised"],
    toolchain = ":toolchain_def1",
    toolchain_type = ":demo_toolchain",
)

toolchain_def(
    name = "toolchain_def1",
    data = "foo",
)
"#;

// Java: ToolchainTest.testToolchain
#[test]
fn test_toolchain() {
    let mut t = new_test();
    t.scratch
        .file("toolchain/toolchain_def.bzl", &[TOOLCHAIN_DEF_BZL]);
    t.scratch.file(
        "toolchain/BUILD",
        &[r#"load(":toolchain_def.bzl", "toolchain_def")

toolchain_type(name = "demo_toolchain")

toolchain(
    name = "toolchain1",
    exec_compatible_with = ["//constraint:foo"],
    target_compatible_with = ["//constraint:bar"],
    toolchain = ":toolchain_def1",
    toolchain_type = ":demo_toolchain",
)

toolchain_def(
    name = "toolchain_def1",
    data = "foo",
)
"#],
    );

    let target = t.get_configured_target("//toolchain:toolchain1");
    assert!(target.is_some());

    let provider =
        platform_provider_utils::declared_toolchain_info(target.as_ref()).expect("isNotNull");
    assert_eq!(
        provider.toolchain_type(),
        &ToolchainTypeInfo::create(label("//toolchain:demo_toolchain"))
    );

    let basic_constraint_setting = ConstraintSettingInfo::create(label("//constraint:basic"));
    assert_eq!(
        provider.exec_constraints().get(&basic_constraint_setting),
        Some(ConstraintValueInfo::create(
            basic_constraint_setting.clone(),
            label("//constraint:foo")
        ))
    );
    assert_eq!(
        provider.target_constraints().get(&basic_constraint_setting),
        Some(ConstraintValueInfo::create(
            basic_constraint_setting.clone(),
            label("//constraint:bar")
        ))
    );

    assert_eq!(
        provider.resolved_toolchain_label(),
        &label("//toolchain:toolchain_def1")
    );
    assert_eq!(provider.target_label(), &label("//toolchain:toolchain1"));
}

// Java: ToolchainTest.testToolchain_targetSetting_matching
#[test]
fn test_toolchain_target_setting_matching() {
    let mut t = new_test();
    t.use_configuration(&["--compilation_mode=opt"]);
    t.scratch
        .file("toolchain/toolchain_def.bzl", &[TOOLCHAIN_DEF_BZL]);
    t.scratch.file("toolchain/BUILD", &[TARGET_SETTING_BUILD]);

    let target = t.get_configured_target("//toolchain:toolchain1");
    let provider = platform_provider_utils::declared_toolchain_info(target.as_ref());

    assert!(target.is_some());
    let provider = provider.expect("isNotNull");
    assert_eq!(
        provider.toolchain_type(),
        &ToolchainTypeInfo::create(label("//toolchain:demo_toolchain"))
    );
    // Ensure target settings completely matches (and not just vacuously e.g. if somehow empty)
    assert!(!provider.target_settings().is_empty());
    assert!(
        provider
            .target_settings()
            .iter()
            .all(|x| matches!(x.result(), MatchResult::Match))
    );
    assert!(
        !provider
            .target_settings()
            .iter()
            .any(|x| matches!(x.result(), MatchResult::InError(_)))
    );
    assert_eq!(
        provider.resolved_toolchain_label(),
        &label("//toolchain:toolchain_def1")
    );
    assert_eq!(provider.target_label(), &label("//toolchain:toolchain1"));
}

// Java: ToolchainTest.testToolchain_targetSetting_nonmatching
#[test]
fn test_toolchain_target_setting_nonmatching() {
    let mut t = new_test();
    t.use_configuration(&["--compilation_mode=fastbuild"]);
    t.scratch
        .file("toolchain/toolchain_def.bzl", &[TOOLCHAIN_DEF_BZL]);
    t.scratch.file("toolchain/BUILD", &[TARGET_SETTING_BUILD]);

    let target = t.get_configured_target("//toolchain:toolchain1");
    let provider = platform_provider_utils::declared_toolchain_info(target.as_ref());

    assert!(target.is_some());
    let provider = provider.expect("isNotNull");
    assert_eq!(
        provider.toolchain_type(),
        &ToolchainTypeInfo::create(label("//toolchain:demo_toolchain"))
    );
    assert!(
        !provider
            .target_settings()
            .iter()
            .any(|x| matches!(x.result(), MatchResult::Match))
    );
    assert_eq!(
        provider.resolved_toolchain_label(),
        &label("//toolchain:toolchain_def1")
    );
    assert_eq!(provider.target_label(), &label("//toolchain:toolchain1"));
}

// Java: ToolchainTest.ruleDefinitionIncorrectlyDependsOnToolchainInstance
#[test]
fn rule_definition_incorrectly_depends_on_toolchain_instance() {
    let mut t = new_test();
    t.scratch.file(
        "toolchain/toolchain_def.bzl",
        &[r#"def _impl(ctx):
    return [platform_common.ToolchainInfo()]

toolchain_def = rule(
    implementation = _impl,
    attrs = {},
)
"#],
    );
    t.scratch.file(
        "toolchain/BUILD",
        &[r#"load(":toolchain_def.bzl", "toolchain_def")

toolchain_type(name = "demo_toolchain")

toolchain(
    name = "toolchain1",
    exec_compatible_with = ["//constraint:foo"],
    target_compatible_with = ["//constraint:bar"],
    toolchain = ":toolchain_def1",
    toolchain_type = ":demo_toolchain",
)

toolchain_def(name = "toolchain_def1")
"#],
    );
    t.scratch.file(
        "rule/rule_def.bzl",
        &[r#"def _impl(ctx):
    pass

my_rule = rule(
    implementation = _impl,
    toolchains = ["//toolchain:toolchain1"],
)
"#],
    );
    t.scratch.file(
        "rule/BUILD",
        &[r#"load("//rule:rule_def.bzl", "my_rule")

my_rule(name = "me")
"#],
    );
    t.remove_fail_fast_handler(); // expect errors
    let target = t.get_configured_target("//rule:me");
    assert!(target.is_none());
    t.assert_contains_event(
        "Target //toolchain:toolchain1 was referenced as a toolchain type, but is a toolchain \
         instance. Is the rule definition for the target you're building setting \
         \"toolchains =\" to a toolchain() instead of the expected toolchain_type()?",
    );
}
