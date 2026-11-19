//! Port of `src/test/java/com/google/devtools/build/lib/analysis/platform/ToolchainInfoTest.java`
//! (the `BuildViewTestCase` test; `toolchainInfo_equalsTester` is a unit test in
//! `blaze_analysis::platform::platform_common`).

use blaze_analysis::configuredtargets::target_value::StarlarkTarget;
use blaze_analysis::info::downcast_info;
use blaze_analysis::platform::platform_provider_utils;
use blaze_analysis::platform::toolchain_info::FrozenToolchainInfo;
use blaze_cmdline::Label;

use crate::testing::BuildViewTestCase;

// Java: ToolchainInfoTest.toolchainInfoConstructor
#[test]
fn toolchain_info_constructor() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "test/toolchain/my_toolchain.bzl",
        &[r#"def _impl(ctx):
    toolchain = platform_common.ToolchainInfo(
        extra_label = ctx.attr.extra_label,
        extra_str = ctx.attr.extra_str,
    )
    return [toolchain]

my_toolchain = rule(
    implementation = _impl,
    attrs = {
        "extra_label": attr.label(),
        "extra_str": attr.string(),
    },
)
"#],
    );
    t.scratch.file(
        "test/toolchain/BUILD",
        &[r#"load("//test/toolchain:my_toolchain.bzl", "my_toolchain")

toolchain_type(name = "my_toolchain_type")

filegroup(name = "dep")

my_toolchain(
    name = "toolchain",
    extra_label = ":dep",
    extra_str = "foo",
)
"#],
    );

    let toolchain = t.get_configured_target("//test/toolchain:toolchain");
    assert!(toolchain.is_some());

    let provider = platform_provider_utils::toolchain(toolchain.as_ref()).expect("isNotNull");
    let provider =
        downcast_info::<FrozenToolchainInfo>(&provider).expect("ClassCastException: ToolchainInfo");

    let extra_label = provider.get_value("extra_label").expect("isNotNull");
    let extra_label = StarlarkTarget::configured_target_of(extra_label)
        .unwrap_or_else(|| panic!("ClassCastException: {extra_label} is not a ConfiguredTarget"));
    assert_eq!(
        extra_label.get_label(),
        Label::parse_canonical_unchecked("//test/toolchain:dep")
    );
    assert_eq!(
        provider.get_value("extra_str").and_then(|v| v.unpack_str()),
        Some("foo")
    );
}
