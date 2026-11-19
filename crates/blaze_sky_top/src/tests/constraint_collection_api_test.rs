//! Port of
//! `src/test/java/com/google/devtools/build/lib/rules/platform/ConstraintCollectionApiTest.java`
//! (a `PlatformTestCase`): the Starlark API of `ConstraintCollection` providers.

use blaze_analysis::platform::constraint_collection::ConstraintCollection;
use blaze_analysis::platform::constraint_setting_info::ConstraintSettingInfo;
use blaze_analysis::platform::constraint_value_info::ConstraintValueInfo;
use blaze_cmdline::Label;
use blaze_starlark::structs::StarlarkInfo;
use starlark::values::list::ListRef;
use starlark::values::{OwnedFrozenValue, Value, ValueLike};

use crate::testing::BuildViewTestCase;
use crate::tests::platform_test_case::{
    constraint_builder, fetch_constraint_setting_info, fetch_platform_info, platform_builder,
};

/// `Label.parseCanonicalUnchecked`.
fn label(s: &str) -> Label {
    Label::parse_canonical_unchecked(s)
}

// Java: ConstraintCollectionApiTest.testConstraintSettings
#[test]
fn test_constraint_settings() {
    let mut t = BuildViewTestCase::new();
    constraint_builder("//foo:s1")
        .add_constraint_value("value1")
        .write(&t);
    constraint_builder("//foo:s2")
        .add_constraint_value("value2")
        .write(&t);
    platform_builder("//foo:my_platform")
        .add_constraint("value1")
        .add_constraint("value2")
        .write(&t);

    let constraint_collection = fetch_constraint_collection(&mut t, "//foo:my_platform");
    let constraint_collection = constraint_collection.expect("isNotNull");

    let mut labels = collect_labels(&constraint_collection.constraint_settings());
    labels.sort();
    let mut expected = vec![label("//foo:s1"), label("//foo:s2")];
    expected.sort();
    assert_eq!(labels, expected);
}

// Java: ConstraintCollectionApiTest.testConstraintValue
#[test]
fn test_constraint_value() {
    let mut t = BuildViewTestCase::new();
    constraint_builder("//foo:s1")
        .add_constraint_value("value1")
        .write(&t);
    constraint_builder("//foo:s2")
        .add_constraint_value("value2")
        .write(&t);
    constraint_builder("//foo:unused").write(&t);
    platform_builder("//foo:my_platform")
        .add_constraint("value1")
        .add_constraint("value2")
        .write(&t);

    let constraint_collection =
        fetch_constraint_collection(&mut t, "//foo:my_platform").expect("isNotNull");

    let setting = ConstraintSettingInfo::create(label("//foo:s1"));
    assert!(constraint_collection.has(&setting));
    let value = constraint_collection.get(&setting).expect("isNotNull");
    assert_eq!(value.label(), &label("//foo:value1"));
    assert!(!constraint_collection.has(&ConstraintSettingInfo::create(label("//foo:unused"))));
}

// Java: ConstraintCollectionApiTest.testContraintValue_parent
#[test]
fn test_contraint_value_parent() {
    let mut t = BuildViewTestCase::new();
    constraint_builder("//foo:s1")
        .add_constraint_value("value1")
        .write(&t);
    constraint_builder("//foo:s2")
        .add_constraint_value("value2")
        .write(&t);
    constraint_builder("//foo:s3")
        .add_constraint_value("value3")
        .add_constraint_value("value4")
        .write(&t);
    platform_builder("//foo:p1")
        .add_constraint("value1")
        .add_constraint("value4")
        .write(&t);
    platform_builder("//foo:p2")
        .set_parent("//foo:p1")
        .add_constraint("value2")
        .write(&t);
    platform_builder("//foo:p3")
        .set_parent("//foo:p2")
        .add_constraint("value3")
        .write(&t);

    let constraint_collection = fetch_constraint_collection(&mut t, "//foo:p3").expect("isNotNull");

    let setting = ConstraintSettingInfo::create(label("//foo:s1"));
    assert!(constraint_collection.has(&setting));
    let value = constraint_collection.get(&setting).expect("isNotNull");
    assert_eq!(value.label(), &label("//foo:value1"));

    let setting = ConstraintSettingInfo::create(label("//foo:s2"));
    assert!(constraint_collection.has(&setting));
    let value = constraint_collection.get(&setting).expect("isNotNull");
    assert_eq!(value.label(), &label("//foo:value2"));

    let setting = ConstraintSettingInfo::create(label("//foo:s3"));
    assert!(constraint_collection.has(&setting));
    let value = constraint_collection.get(&setting).expect("isNotNull");
    assert_eq!(value.label(), &label("//foo:value3"));
}

// Java: ConstraintCollectionApiTest.testConstraintValue_starlark
#[test]
fn test_constraint_value_starlark() {
    let mut t = BuildViewTestCase::new();
    t.set_build_language_options(&["--experimental_platforms_api=true"]);
    constraint_builder("//foo:s1")
        .add_constraint_value("value1")
        .write(&t);
    constraint_builder("//foo:s2")
        .add_constraint_value("value2")
        .write(&t);
    platform_builder("//foo:my_platform")
        .add_constraint("value1")
        .add_constraint("value2")
        .write(&t);

    t.scratch.file(
        "verify/verify.bzl",
        &[r#"result = provider()

def _impl(ctx):
    platform = ctx.attr.platform[platform_common.PlatformInfo]
    constraint_setting = ctx.attr.constraint_setting[platform_common.ConstraintSettingInfo]
    constraint_collection = platform.constraints
    value_from_index = constraint_collection[constraint_setting]
    value_from_get = constraint_collection.get(constraint_setting)
    used_constraints = constraint_collection.constraint_settings
    has_constraint = constraint_collection.has(constraint_setting)
    has_constraint_value = constraint_collection.has_constraint_value(value_from_get)
    return [result(
        value_from_index = value_from_index,
        value_from_get = value_from_get,
        used_constraints = used_constraints,
        has_constraint = has_constraint,
        has_constraint_value = has_constraint_value,
    )]

verify = rule(
    implementation = _impl,
    attrs = {
        "platform": attr.label(providers = [platform_common.PlatformInfo]),
        "constraint_setting": attr.label(
            providers = [platform_common.ConstraintSettingInfo],
        ),
    },
)
"#],
    );
    t.scratch.file(
        "verify/BUILD",
        &[r#"load(":verify.bzl", "verify")

verify(
    name = "verify",
    constraint_setting = "//foo:s1",
    platform = "//foo:my_platform",
)
"#],
    );

    let my_rule_target = t.get_configured_target("//verify:verify").unwrap();
    let info =
        BuildViewTestCase::get_starlark_provider(&my_rule_target, "//verify:verify.bzl", "result")
            .expect("result provider");
    let info_value = frozen(&info);
    let info = StarlarkInfo::from_value(info_value).expect("ClassCastException: StructImpl");

    let constraint_value_from_index = info
        .get_field("value_from_index")
        .and_then(|v| v.downcast_ref::<ConstraintValueInfo>())
        .expect("isNotNull");
    assert_eq!(constraint_value_from_index.label(), &label("//foo:value1"));

    let constraint_value_from_get = info
        .get_field("value_from_get")
        .and_then(|v| v.downcast_ref::<ConstraintValueInfo>())
        .expect("isNotNull");
    assert_eq!(constraint_value_from_get.label(), &label("//foo:value1"));

    let used_constraints = info.get_field("used_constraints").expect("isNotNull");
    let used_constraints: Vec<ConstraintSettingInfo> = ListRef::from_value(used_constraints)
        .expect("ClassCastException: Sequence")
        .iter()
        .map(|v| {
            v.downcast_ref::<ConstraintSettingInfo>()
                .expect("ClassCastException: ConstraintSettingInfo")
                .clone()
        })
        .collect();
    assert_contains_exactly_settings(
        &used_constraints,
        &[
            ConstraintSettingInfo::create(label("//foo:s1")),
            ConstraintSettingInfo::create(label("//foo:s2")),
        ],
    );

    let has_constraint = info
        .get_field("has_constraint")
        .and_then(|v| v.unpack_bool())
        .expect("ClassCastException: boolean");
    assert!(has_constraint);

    let has_constraint_value = info
        .get_field("has_constraint_value")
        .and_then(|v| v.unpack_bool())
        .expect("ClassCastException: boolean");
    assert!(has_constraint_value);
}

// Java: ConstraintCollectionApiTest.testGet_defaultConstraintValues
#[test]
fn test_get_default_constraint_values() {
    let mut t = BuildViewTestCase::new();
    constraint_builder("//constraint/default:basic")
        .default_constraint_value("foo")
        .add_constraint_value("bar")
        .write(&t);
    constraint_builder("//constraint/default:other").write(&t);

    platform_builder("//constraint/default:plat_with_default").write(&t);
    platform_builder("//constraint/default:plat_without_default")
        .add_constraint("bar")
        .write(&t);

    let basic_constraint_setting =
        fetch_constraint_setting_info(&mut t, "//constraint/default:basic").expect("basic");
    let other_constraint_setting =
        fetch_constraint_setting_info(&mut t, "//constraint/default:other").expect("other");

    let constraint_collection_with_default =
        fetch_constraint_collection(&mut t, "//constraint/default:plat_with_default")
            .expect("isNotNull");
    assert!(constraint_collection_with_default.has(&basic_constraint_setting));
    let value = constraint_collection_with_default
        .get(&basic_constraint_setting)
        .expect("isNotNull");
    assert_eq!(value.label(), &label("//constraint/default:foo"));
    assert!(!constraint_collection_with_default.has(&other_constraint_setting));
    assert!(
        constraint_collection_with_default
            .get(&other_constraint_setting)
            .is_none()
    );

    let constraint_collection_without_default =
        fetch_constraint_collection(&mut t, "//constraint/default:plat_without_default")
            .expect("isNotNull");
    assert!(constraint_collection_with_default.has(&basic_constraint_setting));
    let value = constraint_collection_without_default
        .get(&basic_constraint_setting)
        .expect("isNotNull");
    assert_eq!(value.label(), &label("//constraint/default:bar"));
}

// Java: ConstraintCollectionApiTest.collectLabels
fn collect_labels(settings: &[ConstraintSettingInfo]) -> Vec<Label> {
    let mut labels: Vec<Label> = Vec::new();
    for setting in settings {
        // Java: Collectors.toSet().
        if !labels.contains(setting.label()) {
            labels.push(setting.label().clone());
        }
    }
    labels
}

// Java: ConstraintCollectionApiTest.fetchConstraintCollection
fn fetch_constraint_collection(
    t: &mut BuildViewTestCase,
    platform_label: &str,
) -> Option<ConstraintCollection> {
    let platform_info = fetch_platform_info(t, platform_label)?;
    Some(platform_info.constraints().clone())
}

/// `assertThat(actual).containsExactly(expected...)` for constraint settings (any order).
fn assert_contains_exactly_settings(
    actual: &[ConstraintSettingInfo],
    expected: &[ConstraintSettingInfo],
) {
    let ok = actual.len() == expected.len()
        && expected.iter().all(|e| actual.contains(e))
        && actual.iter().all(|a| expected.contains(a));
    assert!(
        ok,
        "containsExactly: actual {actual:?}, expected {expected:?}"
    );
}

/// `info` as a value (the frozen heap is kept alive by `info`).
fn frozen<'v>(info: &'v OwnedFrozenValue) -> Value<'v> {
    // SAFETY: the value is used only while `info` (which keeps its frozen heap alive) is
    // borrowed.
    unsafe { info.unchecked_frozen_value() }.to_value()
}
