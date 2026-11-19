//! Port of
//! `src/test/java/com/google/devtools/build/lib/rules/platform/PlatformConfigurationApiTest.java`:
//! the Starlark API of the `platform` configuration fragment.

use blaze_cmdline::Label;
use blaze_starlark::label::LabelValue;
use blaze_starlark::structs::StarlarkInfo;
use starlark::values::{OwnedFrozenValue, Value, ValueLike};

use crate::testing::BuildViewTestCase;

/// `info` as a value (the frozen heap is kept alive by `info`).
fn frozen<'v>(info: &'v OwnedFrozenValue) -> Value<'v> {
    // SAFETY: the value is used only while `info` (which keeps its frozen heap alive) is
    // borrowed.
    unsafe { info.unchecked_frozen_value() }.to_value()
}

/// `(Label) ((StructImpl) info).getValue(name)` of the `result` provider of `//verify:verify`.
fn get_result_label(t: &mut BuildViewTestCase, name: &str) -> Label {
    let my_rule_target = t.get_configured_target("//verify:verify").unwrap();
    let info =
        BuildViewTestCase::get_starlark_provider(&my_rule_target, "//verify:verify.bzl", "result")
            .expect("result");
    let info = StarlarkInfo::from_value(frozen(&info)).expect("ClassCastException: StructImpl");
    let value = info
        .get_field(name)
        .unwrap_or_else(|| panic!("no field {name}"));
    value
        .downcast_ref::<LabelValue>()
        .unwrap_or_else(|| panic!("ClassCastException: {value} is not a Label"))
        .label()
        .clone()
}

// Java: PlatformConfigurationApiTest.testHostPlatform
#[test]
fn test_host_platform() {
    let mut t = BuildViewTestCase::new();
    t.scratch
        .file("platforms/BUILD", &["platform(name = 'test_platform')"]);

    t.scratch.file(
        "verify/verify.bzl",
        &[r#"result = provider()

def _impl(ctx):
    platformConfig = ctx.fragments.platform
    host_platform = platformConfig.host_platform
    return [result(
        host_platform = host_platform,
    )]

verify = rule(
    implementation = _impl,
    fragments = ["platform"],
)
"#],
    );
    t.scratch.file(
        "verify/BUILD",
        &[r#"load(":verify.bzl", "verify")

verify(name = "verify")
"#],
    );

    t.use_configuration(&["--host_platform=//platforms:test_platform"]);

    let host_platform = get_result_label(&mut t, "host_platform");
    assert_eq!(
        host_platform,
        Label::parse_canonical_unchecked("//platforms:test_platform")
    );
}

// Java: PlatformConfigurationApiTest.testTargetPlatform_single
#[test]
fn test_target_platform_single() {
    let mut t = BuildViewTestCase::new();
    t.scratch
        .file("platforms/BUILD", &["platform(name = 'test_platform')"]);

    t.scratch.file(
        "verify/verify.bzl",
        &[r#"result = provider()

def _impl(ctx):
    platformConfig = ctx.fragments.platform
    target_platform = platformConfig.platform
    return [result(
        target_platform = target_platform,
    )]

verify = rule(
    implementation = _impl,
    fragments = ["platform"],
)
"#],
    );
    t.scratch.file(
        "verify/BUILD",
        &[r#"load(":verify.bzl", "verify")

verify(name = "verify")
"#],
    );

    t.use_configuration(&["--platforms=//platforms:test_platform"]);

    let target_platform = get_result_label(&mut t, "target_platform");
    assert_eq!(
        target_platform,
        Label::parse_canonical_unchecked("//platforms:test_platform")
    );
}
