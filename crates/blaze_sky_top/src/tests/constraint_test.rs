//! Port of `src/test/java/com/google/devtools/build/lib/rules/platform/ConstraintTest.java`:
//! `constraint_setting` and `constraint_value`.

use blaze_analysis::platform::constraint_value_info::ConstraintValueInfo;
use blaze_analysis::platform::platform_provider_utils;
use blaze_cmdline::Label;
use blaze_starlark::structs::StarlarkInfo;
use starlark::values::{OwnedFrozenValue, Value, ValueLike};

use crate::testing::BuildViewTestCase;

fn label(s: &str) -> Label {
    Label::parse_canonical_unchecked(s)
}

/// `info` as a value (the frozen heap is kept alive by `info`).
fn frozen<'v>(info: &'v OwnedFrozenValue) -> Value<'v> {
    // SAFETY: the value is used only while `info` (which keeps its frozen heap alive) is
    // borrowed.
    unsafe { info.unchecked_frozen_value() }.to_value()
}

/// A `BuildViewTestCase` with the class's `@Before`.
fn new_test() -> BuildViewTestCase {
    let t = BuildViewTestCase::new();
    create_constraints(&t);
    t
}

// Java: ConstraintTest.createConstraints
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

// Java: ConstraintTest.testConstraint
#[test]
fn test_constraint() {
    let mut t = new_test();
    let setting = t.get_configured_target("//constraint:basic");
    assert!(setting.is_some());

    let constraint_setting_info =
        platform_provider_utils::constraint_setting(setting.as_ref()).expect("isNotNull");
    assert_eq!(
        constraint_setting_info.label(),
        &label("//constraint:basic")
    );
    assert!(!constraint_setting_info.has_default_constraint_value());
    assert!(constraint_setting_info.default_constraint_value().is_none());

    let foo_value = t.get_configured_target("//constraint:foo");
    assert!(foo_value.is_some());

    let foo_constraint_value_info =
        platform_provider_utils::constraint_value(foo_value.as_ref()).expect("isNotNull");
    assert_eq!(
        foo_constraint_value_info.constraint().label(),
        &label("//constraint:basic")
    );
    assert_eq!(
        foo_constraint_value_info.label(),
        &label("//constraint:foo")
    );

    let bar_value = t.get_configured_target("//constraint:bar");
    assert!(bar_value.is_some());

    let bar_constraint_value_info = platform_provider_utils::constraint_value(bar_value.as_ref())
        .expect("NullPointerException");
    assert_eq!(
        bar_constraint_value_info.constraint().label(),
        &label("//constraint:basic")
    );
    assert_eq!(
        bar_constraint_value_info.label(),
        &label("//constraint:bar")
    );
}

// Java: ConstraintTest.testConstraint_defaultValue
#[test]
fn test_constraint_default_value() {
    let mut t = new_test();
    t.scratch.file(
        "constraint_default/BUILD",
        &[r#"constraint_setting(
    name = "basic",
    default_constraint_value = ":foo",
)

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

    let setting = t.get_configured_target("//constraint_default:basic");
    assert!(setting.is_some());
    let constraint_setting_info =
        platform_provider_utils::constraint_setting(setting.as_ref()).expect("isNotNull");
    assert!(constraint_setting_info.has_default_constraint_value());

    let foo_value = t.get_configured_target("//constraint_default:foo");
    assert!(foo_value.is_some());
    let foo_constraint_value_info =
        platform_provider_utils::constraint_value(foo_value.as_ref()).expect("isNotNull");

    assert_eq!(
        constraint_setting_info.default_constraint_value(),
        Some(foo_constraint_value_info)
    );
}

// Java: ConstraintTest.testConstraint_defaultValue_differentPackageFails
#[test]
fn test_constraint_default_value_different_package_fails() {
    let mut t = new_test();
    t.scratch.file(
        "other/BUILD",
        &[r#"constraint_value(
    name = "other",
    constraint_setting = "//constraint_default:basic",
)
"#],
    );
    t.check_error(
        "constraint_default",
        "basic",
        "same package",
        &[
            "constraint_setting(name = 'basic',",
            "    default_constraint_value = '//other:other',",
            ")",
        ],
    );
}

// Java: ConstraintTest.testConstraint_defaultValue_nonExistentTargetFails
#[test]
fn test_constraint_default_value_non_existent_target_fails() {
    let mut t = new_test();
    t.check_error(
        "constraint_default",
        "basic",
        "default constraint value '//constraint_default:food' does not exist",
        &[
            "constraint_setting(name = 'basic',",
            "    default_constraint_value = ':food',",
            "    )",
            "constraint_value(name = 'foo',",
            "    constraint_setting = ':basic',",
            ")",
        ],
    );
}

const VERIFY_DEFAULT_VALUE_BZL: &str = r#"result = provider()

def _impl(ctx):
    constraint_setting = ctx.attr.constraint_setting[platform_common.ConstraintSettingInfo]
    default_value = constraint_setting.default_constraint_value
    has_default_value = constraint_setting.has_default_constraint_value
    return [result(
        default_value = default_value,
        has_default_value = has_default_value,
    )]

verify = rule(
    implementation = _impl,
    attrs = {
        "constraint_setting": attr.label(
            providers = [platform_common.ConstraintSettingInfo],
        ),
    },
)
"#;

const VERIFY_DEFAULT_VALUE_BUILD: &str = r#"load(":verify.bzl", "verify")

verify(
    name = "verify",
    constraint_setting = "//constraint_default:basic",
)
"#;

// Java: ConstraintTest.testConstraint_defaultValue_starlark
#[test]
fn test_constraint_default_value_starlark() {
    let mut t = new_test();
    t.set_build_language_options(&["--experimental_platforms_api=true"]);
    t.scratch.file(
        "constraint_default/BUILD",
        &[r#"constraint_setting(
    name = "basic",
    default_constraint_value = ":foo",
)

constraint_value(
    name = "foo",
    constraint_setting = ":basic",
)
"#],
    );

    t.scratch
        .file("verify/verify.bzl", &[VERIFY_DEFAULT_VALUE_BZL]);
    t.scratch
        .file("verify/BUILD", &[VERIFY_DEFAULT_VALUE_BUILD]);

    let my_rule_target = t.get_configured_target("//verify:verify").unwrap();
    let info =
        BuildViewTestCase::get_starlark_provider(&my_rule_target, "//verify:verify.bzl", "result")
            .expect("result");
    let info = StarlarkInfo::from_value(frozen(&info)).expect("ClassCastException: StructImpl");

    let default_constraint_value = info
        .get_field("default_value")
        .and_then(|v| v.downcast_ref::<ConstraintValueInfo>())
        .expect("isNotNull");
    assert_eq!(
        default_constraint_value.label(),
        &label("//constraint_default:foo")
    );
    assert_eq!(
        default_constraint_value.constraint().label(),
        &label("//constraint_default:basic")
    );

    let has_constraint_value = info
        .get_field("has_default_value")
        .and_then(|v| v.unpack_bool())
        .expect("ClassCastException: boolean");
    assert!(has_constraint_value);
}

// Java: ConstraintTest.testConstraint_refinesConstraintValue
#[test]
fn test_constraint_refines_constraint_value() {
    let mut t = new_test();
    t.scratch.file(
        "libc/BUILD",
        &[r#"constraint_setting(name = "libc")

constraint_value(
    name = "glibc",
    constraint_setting = ":libc",
)
"#],
    );
    t.scratch.file(
        "libc/glibc/BUILD",
        &[r#"constraint_setting(
    name = "version",
    refines_constraint_value = "//libc:glibc",
)

constraint_value(
    name = "2.42",
    constraint_setting = ":version",
)
"#],
    );

    let setting = t.get_configured_target("//libc/glibc:version");
    assert!(setting.is_some());
    let setting_info =
        platform_provider_utils::constraint_setting(setting.as_ref()).expect("isNotNull");
    assert_eq!(
        setting_info
            .refined_constraint_value()
            .expect("NullPointerException")
            .label(),
        &label("//libc:glibc")
    );
    assert_eq!(setting_info.refinement_chain(), vec![label("//libc:glibc")]);
}

// Java: ConstraintTest.testConstraint_refinesConstraintValue_alias
#[test]
fn test_constraint_refines_constraint_value_alias() {
    let mut t = new_test();
    t.scratch.file(
        "libc/BUILD",
        &[r#"constraint_setting(name = "libc")

constraint_value(
    name = "glibc",
    constraint_setting = ":libc",
)

alias(
    name = "glibc_alias",
    actual = ":glibc",
)
"#],
    );
    t.scratch.file(
        "libc/glibc/BUILD",
        &[r#"constraint_setting(
    name = "version",
    refines_constraint_value = "//libc:glibc_alias",
)

constraint_value(
    name = "2.42",
    constraint_setting = ":version",
)
"#],
    );

    let setting = t.get_configured_target("//libc/glibc:version");
    let setting_info =
        platform_provider_utils::constraint_setting(setting.as_ref()).expect("isNotNull");
    // The alias is resolved to the underlying constraint_value.
    assert_eq!(
        setting_info
            .refined_constraint_value()
            .expect("NullPointerException")
            .label(),
        &label("//libc:glibc")
    );
    assert_eq!(setting_info.refinement_chain(), vec![label("//libc:glibc")]);
}

// Java: ConstraintTest.testConstraint_refinesConstraintValue_chain
#[test]
fn test_constraint_refines_constraint_value_chain() {
    let mut t = new_test();
    t.scratch.file(
        "a/BUILD",
        &[r#"constraint_setting(name = "a")

constraint_value(
    name = "a1",
    constraint_setting = ":a",
)
"#],
    );
    t.scratch.file(
        "b/BUILD",
        &[r#"constraint_setting(
    name = "b",
    refines_constraint_value = "//a:a1",
)

constraint_value(
    name = "b1",
    constraint_setting = ":b",
)
"#],
    );
    t.scratch.file(
        "c/BUILD",
        &[r#"constraint_setting(
    name = "c",
    refines_constraint_value = "//b:b1",
)

constraint_value(
    name = "c1",
    constraint_setting = ":c",
)
"#],
    );

    let setting = t.get_configured_target("//c:c");
    let setting_info =
        platform_provider_utils::constraint_setting(setting.as_ref()).expect("isNotNull");
    assert_eq!(
        setting_info.refinement_chain(),
        vec![label("//b:b1"), label("//a:a1")]
    );
}

// Java: ConstraintTest.testConstraint_refinesConstraintValue_withDefault
#[test]
fn test_constraint_refines_constraint_value_with_default() {
    let mut t = new_test();
    t.scratch.file(
        "libc/BUILD",
        &[r#"constraint_setting(name = "libc")

constraint_value(
    name = "glibc",
    constraint_setting = ":libc",
)
"#],
    );
    t.scratch.file(
        "libc/glibc/BUILD",
        &[r#"constraint_setting(
    name = "version",
    default_constraint_value = ":unknown",
    refines_constraint_value = "//libc:glibc",
)

constraint_value(
    name = "unknown",
    constraint_setting = ":version",
)

constraint_value(
    name = "2.42",
    constraint_setting = ":version",
)
"#],
    );

    let setting = t.get_configured_target("//libc/glibc:version");
    let setting_info =
        platform_provider_utils::constraint_setting(setting.as_ref()).expect("isNotNull");
    assert!(setting_info.has_default_constraint_value());
    assert_eq!(
        setting_info
            .refined_constraint_value()
            .expect("NullPointerException")
            .label(),
        &label("//libc:glibc")
    );
}

// Java: ConstraintTest.testConstraint_defaultValue_notSet_starlark
#[test]
fn test_constraint_default_value_not_set_starlark() {
    let mut t = new_test();
    t.set_build_language_options(&["--experimental_platforms_api=true"]);
    t.scratch.file(
        "constraint_default/BUILD",
        &["constraint_setting(name = 'basic')"],
    );

    t.scratch
        .file("verify/verify.bzl", &[VERIFY_DEFAULT_VALUE_BZL]);
    t.scratch
        .file("verify/BUILD", &[VERIFY_DEFAULT_VALUE_BUILD]);

    let my_rule_target = t.get_configured_target("//verify:verify").unwrap();
    let info =
        BuildViewTestCase::get_starlark_provider(&my_rule_target, "//verify:verify.bzl", "result")
            .expect("result");
    let info = StarlarkInfo::from_value(frozen(&info)).expect("ClassCastException: StructImpl");

    assert!(info.get_field("default_value").expect("field").is_none());

    let has_constraint_value = info
        .get_field("has_default_value")
        .and_then(|v| v.unpack_bool())
        .expect("ClassCastException: boolean");
    assert!(!has_constraint_value);
}
